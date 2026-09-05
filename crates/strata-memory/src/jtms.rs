use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use strata_core::errors::StrataError;
use strata_core::schemas::{FactStatus, JtmsAuditRow, SemanticFact};
use strata_core::state::MemoryTier;
use uuid::Uuid;

use crate::embedding::cosine_similarity;
use crate::store::SqliteStore;

/// Resolution strategy when a conflict is detected between semantic facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Supersede old fact: mark old fact Deprecated (OUT) pointing to new fact, make new fact Active (IN) with incremented version.
    Supersede,
    /// Reject new candidate: old fact remains Active (IN), candidate is marked Deprecated / Outlier.
    Reject,
    /// Coexist: both facts remain Active (IN) under distinct conditions, components, or scopes.
    Coexist,
    /// Invalidate: mark fact Stale (OUT) due to prerequisite retraction or downstream cascade.
    Invalidate,
}

/// Represents a detected conflict match between an existing fact and a new candidate fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictMatch {
    pub existing_fact_id: Uuid,
    pub existing_statement: String,
    pub similarity: f32,
    pub has_lexical_contradiction: bool,
    pub contradiction_cues: Vec<String>,
    #[serde(default = "default_resolution")]
    pub resolution: ConflictResolution,
    #[serde(default)]
    pub reason: String,
}

fn default_resolution() -> ConflictResolution {
    ConflictResolution::Supersede
}

/// Justification-based Truth Maintenance System (JTMS v2) & Deterministic Belief Revision Engine.
#[derive(Debug, Clone)]
pub struct TruthMaintenanceSystem {
    /// Cosine similarity threshold above which statements are considered semantic candidates for conflict
    pub similarity_threshold: f32,
}

impl TruthMaintenanceSystem {
    pub fn new(similarity_threshold: f32) -> Self {
        Self {
            similarity_threshold: similarity_threshold.clamp(0.0, 1.0),
        }
    }

    pub fn with_default_threshold() -> Self {
        Self::new(0.85)
    }

    /// Check if two statements belong to clearly orthogonal architectural domains or disjoint subsystems.
    /// Returns true if the statements can safely coexist without contradiction.
    pub fn is_orthogonal_coexistence(&self, text_a: &str, text_b: &str) -> bool {
        let a = text_a.to_lowercase();
        let b = text_b.to_lowercase();

        // 1. Frontend vs Backend tier
        let frontend_cues = [
            "frontend",
            "ui",
            "client-side",
            "react",
            "vue",
            "svelte",
            "tailwind",
            "css",
            "html",
            "browser",
        ];
        let backend_cues = [
            "backend",
            "server-side",
            "axum",
            "actix",
            "tokio",
            "microservice",
            "daemon",
            "server",
        ];
        let a_is_fe = frontend_cues.iter().any(|c| a.contains(c));
        let b_is_fe = frontend_cues.iter().any(|c| b.contains(c));
        let a_is_be = backend_cues.iter().any(|c| a.contains(c));
        let b_is_be = backend_cues.iter().any(|c| b.contains(c));
        if (a_is_fe && b_is_be && !a_is_be && !b_is_fe)
            || (a_is_be && b_is_fe && !a_is_fe && !b_is_be)
        {
            return true;
        }

        // 2. Relational Database vs Cache / In-Memory
        let relational_cues = [
            "relational",
            "primary database",
            "oltp",
            "postgres",
            "postgresql",
            "mysql",
            "mariadb",
            "sqlite",
        ];
        let cache_cues = [
            "cache",
            "caching",
            "in-memory",
            "redis",
            "memcached",
            "keydb",
            "dragonfly",
        ];
        let a_is_rel = relational_cues.iter().any(|c| a.contains(c));
        let b_is_rel = relational_cues.iter().any(|c| b.contains(c));
        let a_is_cache = cache_cues.iter().any(|c| a.contains(c));
        let b_is_cache = cache_cues.iter().any(|c| b.contains(c));
        if (a_is_rel && b_is_cache && !a_is_cache && !b_is_rel)
            || (a_is_cache && b_is_rel && !a_is_rel && !b_is_cache)
        {
            return true;
        }

        // 3. Telemetry Subsystems: Metrics vs Traces vs Logs
        let metrics_cues = ["metric", "metrics", "prometheus", "statsd", "grafana"];
        let traces_cues = [
            "trace",
            "traces",
            "tracing",
            "opentelemetry",
            "jaeger",
            "zipkin",
        ];
        let logs_cues = ["log", "logs", "logging", "loki", "fluentd", "syslog"];
        let a_is_m = metrics_cues.iter().any(|c| a.contains(c));
        let b_is_m = metrics_cues.iter().any(|c| b.contains(c));
        let a_is_t = traces_cues.iter().any(|c| a.contains(c));
        let b_is_t = traces_cues.iter().any(|c| b.contains(c));
        let a_is_l = logs_cues.iter().any(|c| a.contains(c));
        let b_is_l = logs_cues.iter().any(|c| b.contains(c));
        let telemetry_distinct = (a_is_m && b_is_t)
            || (a_is_t && b_is_m)
            || (a_is_m && b_is_l)
            || (a_is_l && b_is_m)
            || (a_is_t && b_is_l)
            || (a_is_l && b_is_t);
        if telemetry_distinct {
            return true;
        }

        // 4. Operation Paths: Read vs Write
        let read_cues = [
            "read operations",
            "reads",
            "read replica",
            "read pool",
            "queries",
        ];
        let write_cues = [
            "write operations",
            "writes",
            "primary node",
            "master node",
            "mutations",
        ];
        let a_is_r = read_cues.iter().any(|c| a.contains(c));
        let b_is_r = read_cues.iter().any(|c| b.contains(c));
        let a_is_w = write_cues.iter().any(|c| a.contains(c));
        let b_is_w = write_cues.iter().any(|c| b.contains(c));
        if (a_is_r && b_is_w && !a_is_w && !b_is_r) || (a_is_w && b_is_r && !a_is_r && !b_is_w) {
            return true;
        }

        // 5. Environment Qualifiers: Staging vs Production
        let staging_cues = [
            "staging environment",
            "staging",
            "development environment",
            "dev environment",
            "local dev",
        ];
        let prod_cues = [
            "production environment",
            "production cluster",
            "prod environment",
            "live environment",
        ];
        let a_is_stg = staging_cues.iter().any(|c| a.contains(c));
        let b_is_stg = staging_cues.iter().any(|c| b.contains(c));
        let a_is_prod = prod_cues.iter().any(|c| a.contains(c));
        let b_is_prod = prod_cues.iter().any(|c| b.contains(c));
        if (a_is_stg && b_is_prod && !a_is_prod && !b_is_stg)
            || (a_is_prod && b_is_stg && !a_is_stg && !b_is_prod)
        {
            return true;
        }

        // 6. Platform Qualifiers: Mobile vs Web
        let mobile_cues = ["mobile app", "mobile", "ios", "android", "swift", "kotlin"];
        let web_cues = [
            "web app",
            "web client",
            "desktop browser",
            "chrome",
            "firefox",
        ];
        let a_is_mob = mobile_cues.iter().any(|c| a.contains(c));
        let b_is_mob = mobile_cues.iter().any(|c| b.contains(c));
        let a_is_web = web_cues.iter().any(|c| a.contains(c));
        let b_is_web = web_cues.iter().any(|c| b.contains(c));
        if (a_is_mob && b_is_web && !a_is_web && !b_is_mob)
            || (a_is_web && b_is_mob && !a_is_mob && !b_is_web)
        {
            return true;
        }

        // 7. Security contexts: User Passwords vs API Authentication
        let pw_cues = ["user password", "passwords", "argon2", "bcrypt", "pbkdf2"];
        let api_key_cues = [
            "api key",
            "api keys",
            "jwt",
            "bearer token",
            "oauth2",
            "oidc",
            "sha-256",
        ];
        let a_is_pw = pw_cues.iter().any(|c| a.contains(c));
        let b_is_pw = pw_cues.iter().any(|c| b.contains(c));
        let a_is_ak = api_key_cues.iter().any(|c| a.contains(c));
        let b_is_ak = api_key_cues.iter().any(|c| b.contains(c));
        if (a_is_pw && b_is_ak && !a_is_ak && !b_is_pw)
            || (a_is_ak && b_is_pw && !a_is_pw && !b_is_ak)
        {
            return true;
        }

        // 8. Testing methodologies: Unit testing vs E2E testing
        let unit_cues = [
            "unit test",
            "unit tests",
            "cargo test",
            "jest unit",
            "pytest unit",
        ];
        let e2e_cues = [
            "e2e test",
            "e2e tests",
            "integration tests",
            "playwright",
            "cypress",
        ];
        let a_is_unit = unit_cues.iter().any(|c| a.contains(c));
        let b_is_unit = unit_cues.iter().any(|c| b.contains(c));
        let a_is_e2e = e2e_cues.iter().any(|c| a.contains(c));
        let b_is_e2e = e2e_cues.iter().any(|c| b.contains(c));
        if (a_is_unit && b_is_e2e && !a_is_e2e && !b_is_unit)
            || (a_is_e2e && b_is_unit && !a_is_unit && !b_is_e2e)
        {
            return true;
        }

        // 9. Distinct Configuration Properties
        let thread_cues = ["worker threads", "thread pool", "concurrency workers"];
        let conn_cues = [
            "connection pool",
            "database pool",
            "max connections",
            "pool size",
        ];
        let a_is_th = thread_cues.iter().any(|c| a.contains(c));
        let b_is_th = thread_cues.iter().any(|c| b.contains(c));
        let a_is_co = conn_cues.iter().any(|c| a.contains(c));
        let b_is_co = conn_cues.iter().any(|c| b.contains(c));
        if (a_is_th && b_is_co && !a_is_co && !b_is_th)
            || (a_is_co && b_is_th && !a_is_th && !b_is_co)
        {
            return true;
        }

        // 10. Database Server/Engine vs Client Driver/Pool
        let driver_cues = [
            "sqlx",
            "diesel",
            "sea-orm",
            "connection pool",
            "database pool",
            "client pool",
            "jdbc",
        ];
        let server_cues = [
            "primary database",
            "storage engine",
            "database server",
            "database cluster",
        ];
        let a_is_drv = driver_cues.iter().any(|c| a.contains(c));
        let b_is_drv = driver_cues.iter().any(|c| b.contains(c));
        let a_is_srv = server_cues.iter().any(|c| a.contains(c));
        let b_is_srv = server_cues.iter().any(|c| b.contains(c));
        if (a_is_drv && b_is_srv && !a_is_srv && !b_is_drv)
            || (a_is_srv && b_is_drv && !a_is_drv && !b_is_srv)
        {
            return true;
        }

        // 11. Application Repository/Service Layer vs Database Infrastructure
        let repo_cues = [
            "repository",
            "user repository",
            "order repository",
            "data access layer",
            "dao",
        ];
        let a_is_repo = repo_cues.iter().any(|c| a.contains(c));
        let b_is_repo = repo_cues.iter().any(|c| b.contains(c));
        if (a_is_repo && b_is_srv && !a_is_srv && !b_is_repo)
            || (a_is_srv && b_is_repo && !a_is_repo && !b_is_srv)
        {
            return true;
        }

        false
    }

    /// Detect lexical contradiction cues between two statements.
    pub fn detect_lexical_contradiction(&self, text_a: &str, text_b: &str) -> (bool, Vec<String>) {
        if self.is_orthogonal_coexistence(text_a, text_b) {
            return (false, Vec::new());
        }

        let a_lower = text_a.to_lowercase();
        let b_lower = text_b.to_lowercase();

        let mut cues = Vec::new();
        let mut has_direct_antonym = false;

        // Antonym, protocol, and polarity pairs
        let antonym_pairs = [
            ("always", "never"),
            ("enable", "disable"),
            ("enabled", "disabled"),
            ("true", "false"),
            ("use", "avoid"),
            ("use", "do not use"),
            ("supported", "unsupported"),
            ("deprecated", "active"),
            ("required", "optional"),
            ("allow", "deny"),
            ("allowed", "forbidden"),
            ("allowed", "disallowed"),
            ("increase", "decrease"),
            ("must", "must not"),
            ("should", "should not"),
            ("can", "cannot"),
            ("is", "is not"),
            ("valid", "invalid"),
            ("success", "failure"),
            ("public", "private"),
            ("sync", "async"),
            ("synchronous", "asynchronous"),
            ("mutable", "immutable"),
            ("blocking", "non-blocking"),
            ("strict", "permissive"),
            ("mandatory", "optional"),
            ("rest", "grpc"),
            ("json", "protobuf"),
            ("mysql", "postgres"),
            ("mysql", "postgresql"),
            ("sqlite", "postgres"),
            ("sqlite", "postgresql"),
            ("monolith", "microservices"),
            ("monolith", "event-driven"),
            ("hs256", "rs256"),
            ("tls 1.2", "tls 1.3"),
            ("http", "https"),
            ("session", "bearer token"),
            ("basic auth", "oauth2"),
            ("root", "unprivileged"),
            ("plaintext", "encrypted"),
            ("unencrypted", "encrypted"),
        ];

        for (word1, word2) in antonym_pairs {
            let (has_1_in_a, has_2_in_a) = (a_lower.contains(word1), a_lower.contains(word2));
            let (has_1_in_b, has_2_in_b) = (b_lower.contains(word1), b_lower.contains(word2));

            if (has_1_in_a && has_2_in_b) || (has_2_in_a && has_1_in_b) {
                cues.push(format!("Polarity opposition: '{word1}' vs '{word2}'"));
                has_direct_antonym = true;
            }
        }

        // Negation & migration keywords present in only one of the texts
        let negation_words = [
            "not",
            "never",
            "no longer",
            "deprecated",
            "deprecating",
            "removed",
            "disabled",
            "avoid",
            "cannot",
            "migrated",
            "migration",
            "replaced",
            "replaces",
            "supersedes",
            "superseded",
            "disallowed",
            "forbidden",
            "instead of",
            "transitioned to",
            "switched to",
            "swapped for",
        ];
        for neg in negation_words {
            let in_a = a_lower.contains(neg);
            let in_b = b_lower.contains(neg);
            if in_a != in_b {
                cues.push(format!("Asymmetric negation/migration keyword: '{neg}'"));
            }
        }

        let is_contradiction = has_direct_antonym || cues.len() >= 2;
        (is_contradiction, cues)
    }

    /// Deterministic Priority Arbitration between an existing active fact and a candidate fact.
    ///
    /// Defines a strict total ordering:
    /// 1. Human approval authority: `approved_by_human` (`true` > `false`).
    /// 2. Memory Tier authority: `Core` > `Working` > `Peripheral`.
    /// 3. Explicit Migration / Supersession phrasing in statement.
    /// 4. Temporal timestamp precedence: Newer evidence/creation timestamp represents updated truth.
    /// 5. Confidence and Importance scores.
    /// 6. Deterministic tie-breaker: Lexicographical UUID comparison.
    pub fn arbitrate_pair(
        &self,
        existing: &SemanticFact,
        candidate: &SemanticFact,
        cues: &[String],
    ) -> (ConflictResolution, String) {
        // 1. Human Approval Authority (Absolute protection for approved truth)
        if existing.approved_by_human && !candidate.approved_by_human {
            return (
                ConflictResolution::Reject,
                "Existing fact possesses explicit human approval authority (Core retention protection)".to_string(),
            );
        }
        if candidate.approved_by_human && !existing.approved_by_human {
            return (
                ConflictResolution::Supersede,
                "Candidate fact possesses explicit human approval authority over unapproved existing fact".to_string(),
            );
        }

        // 2. Memory Tier Authority (Core > Working > Peripheral)
        let tier_rank = |tier: MemoryTier| match tier {
            MemoryTier::Core => 3,
            MemoryTier::Working => 2,
            MemoryTier::Peripheral => 1,
        };
        let cand_tier = tier_rank(candidate.tier);
        let exist_tier = tier_rank(existing.tier);
        if exist_tier > cand_tier {
            return (
                ConflictResolution::Reject,
                format!(
                    "Existing memory tier ({:?}) dominates candidate lower tier ({:?})",
                    existing.tier, candidate.tier
                ),
            );
        } else if cand_tier > exist_tier {
            return (
                ConflictResolution::Supersede,
                format!(
                    "Candidate memory tier ({:?}) supersedes lower tier ({:?})",
                    candidate.tier, existing.tier
                ),
            );
        }

        let existing_lower = existing.statement.to_lowercase();
        let candidate_lower = candidate.statement.to_lowercase();

        // 3. Explicit migration or supersession intent in text
        let migration_phrases = [
            "migrated to",
            "migrating to",
            "switched to",
            "swapped for",
            "transitioned to",
            "replaced by",
            "replaces",
            "supersedes",
            "superseded by",
            "deprecating",
            "deprecated",
            "upgraded to",
            "no longer uses",
            "now uses",
            "instead of",
        ];

        let cand_has_migration = migration_phrases
            .iter()
            .any(|k| candidate_lower.contains(k));
        let exist_has_migration = migration_phrases.iter().any(|k| existing_lower.contains(k));

        if cand_has_migration && !exist_has_migration {
            return (
                ConflictResolution::Supersede,
                format!(
                    "Candidate explicitly declares migration/replacement intent: '{}'",
                    cues.join(", ")
                ),
            );
        } else if exist_has_migration
            && !cand_has_migration
            && existing.created_at >= candidate.created_at
        {
            return (
                ConflictResolution::Reject,
                format!(
                    "Existing active fact explicitly declared migration/replacement intent: '{}'",
                    cues.join(", ")
                ),
            );
        }

        // 4. Temporal Timestamp Precedence (Newer truth supersedes older truth)
        if candidate.created_at > existing.created_at {
            let reason = format!(
                "Newer temporal timestamp ({}) supersedes older belief ({}) with updated evidence",
                candidate.created_at.to_rfc3339(),
                existing.created_at.to_rfc3339()
            );
            return (ConflictResolution::Supersede, reason);
        } else if existing.created_at > candidate.created_at {
            let reason = format!(
                "Existing active fact is chronologically newer ({}) than candidate ({})",
                existing.created_at.to_rfc3339(),
                candidate.created_at.to_rfc3339()
            );
            return (ConflictResolution::Reject, reason);
        }

        // 5. Confidence & Evidence Grounding
        if candidate.confidence > existing.confidence + 0.05 {
            return (
                ConflictResolution::Supersede,
                format!(
                    "Candidate confidence ({:.2}) exceeds existing ({:.2})",
                    candidate.confidence, existing.confidence
                ),
            );
        } else if existing.confidence > candidate.confidence + 0.05 {
            return (
                ConflictResolution::Reject,
                format!(
                    "Existing confidence ({:.2}) exceeds candidate ({:.2})",
                    existing.confidence, candidate.confidence
                ),
            );
        }

        if candidate.importance > existing.importance + 0.10 {
            return (
                ConflictResolution::Supersede,
                format!(
                    "Candidate importance ({:.2}) exceeds existing ({:.2})",
                    candidate.importance, existing.importance
                ),
            );
        } else if existing.importance > candidate.importance + 0.10 {
            return (
                ConflictResolution::Reject,
                format!(
                    "Existing importance ({:.2}) exceeds candidate ({:.2})",
                    existing.importance, candidate.importance
                ),
            );
        }

        // 6. Deterministic Tie-Breaker (UUID total order)
        if candidate.id.to_string() > existing.id.to_string() {
            (
                ConflictResolution::Supersede,
                "Deterministic tie-breaker: candidate UUID ordering".to_string(),
            )
        } else {
            (
                ConflictResolution::Reject,
                "Deterministic tie-breaker: existing UUID ordering".to_string(),
            )
        }
    }

    /// Find candidate conflicts against a list of existing facts and their embeddings.
    pub fn find_conflicts(
        &self,
        new_fact: &SemanticFact,
        new_embedding: &[f32],
        existing_facts: &[(SemanticFact, Vec<f32>)],
    ) -> Vec<ConflictMatch> {
        let mut conflicts = Vec::new();

        for (existing, emb) in existing_facts {
            // Only check against active facts and distinct IDs
            if existing.id == new_fact.id || existing.status != FactStatus::Active {
                continue;
            }

            // Scope compatibility check (only check if scopes overlap)
            if !existing.scope.is_compatible(&new_fact.scope) {
                continue;
            }

            // Check if statements are orthogonally distinct components
            if self.is_orthogonal_coexistence(&existing.statement, &new_fact.statement) {
                continue;
            }

            let sim = cosine_similarity(new_embedding, emb);
            let (lexical_contradiction, cues) =
                self.detect_lexical_contradiction(&existing.statement, &new_fact.statement);

            // Trigger conflict if:
            // 1. High embedding similarity (> threshold) and lexical contradiction cues found
            // 2. Extremely high embedding similarity (> 0.94) with different statements
            // 3. Same category and direct negation / polarity cues
            // 4. Direct lexical contradiction with overlapping tokens
            let same_statement = existing
                .statement
                .trim()
                .eq_ignore_ascii_case(new_fact.statement.trim());
            let is_conflict = (!same_statement
                && lexical_contradiction
                && (sim >= 0.35 || existing.category == new_fact.category || cues.len() >= 2))
                || (sim >= self.similarity_threshold && lexical_contradiction)
                || (sim >= 0.94
                    && !same_statement
                    && !lexical_contradiction
                    && !self.is_orthogonal_coexistence(&existing.statement, &new_fact.statement));

            if is_conflict {
                let (resolution, reason) = self.arbitrate_pair(existing, new_fact, &cues);
                conflicts.push(ConflictMatch {
                    existing_fact_id: existing.id,
                    existing_statement: existing.statement.clone(),
                    similarity: sim,
                    has_lexical_contradiction: lexical_contradiction,
                    contradiction_cues: cues,
                    resolution,
                    reason,
                });
            }
        }

        conflicts
    }

    /// Propagates downstream invalidations when a fact is superseded or retracted.
    ///
    /// Every fact that depends on `invalidated_fact_id` transitions deterministically
    /// to `FactStatus::Stale` (OUT), cascading transitively down the dependency DAG.
    /// Full audit rows are recorded for every invalidated belief.
    pub fn propagate_downstream_invalidation(
        &self,
        store: &SqliteStore,
        invalidated_fact_id: &Uuid,
        reason_prefix: &str,
    ) -> Result<Vec<Uuid>, StrataError> {
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();
        let mut invalidated_ids = Vec::new();

        queue.push_back(*invalidated_fact_id);
        visited.insert(*invalidated_fact_id);

        while let Some(current_id) = queue.pop_front() {
            let dependent_ids = store.get_downstream_dependent_fact_ids(&current_id)?;
            for dep_id in dependent_ids {
                if visited.insert(dep_id) {
                    if let Some(mut dep_fact) = store.get_semantic_fact(&dep_id)? {
                        if dep_fact.status == FactStatus::Active {
                            dep_fact.mark_stale();
                            store.insert_or_update_semantic_fact(&dep_fact)?;

                            let audit = JtmsAuditRow::new(
                                current_id,
                                dep_id,
                                "invalidation",
                                format!("{reason_prefix}: Prerequisite fact {current_id} became inactive"),
                            );
                            store.insert_jtms_audit(&audit)?;

                            invalidated_ids.push(dep_id);
                            queue.push_back(dep_id);
                        }
                    }
                }
            }
        }

        Ok(invalidated_ids)
    }

    /// Apply belief update to SQLite store when revising facts.
    pub fn apply_belief_update(
        &self,
        store: &SqliteStore,
        old_fact_id: &Uuid,
        new_fact: &mut SemanticFact,
        resolution_type: ConflictResolution,
    ) -> Result<(), StrataError> {
        match resolution_type {
            ConflictResolution::Supersede => {
                if let Some(mut old_fact) = store.get_semantic_fact(old_fact_id)? {
                    // Deprecate old fact (OUT)
                    old_fact.status = FactStatus::Deprecated;
                    old_fact.replaced_by = Some(new_fact.id);
                    old_fact.last_updated_at = Utc::now();
                    if let Some(ref mut anchor) = old_fact.code_anchor {
                        anchor.invalidate();
                    }
                    store.insert_or_update_semantic_fact(&old_fact)?;

                    // Record audit row for superseded fact
                    let audit = JtmsAuditRow::new(
                        new_fact.id,
                        old_fact.id,
                        "supersede",
                        format!("Fact {} superseded by fact {}", old_fact.id, new_fact.id),
                    );
                    store.insert_jtms_audit(&audit)?;

                    // Propagate downstream invalidation for the losing old fact
                    self.propagate_downstream_invalidation(
                        store,
                        &old_fact.id,
                        &format!(
                            "Downstream invalidation after fact {} was superseded",
                            old_fact.id
                        ),
                    )?;

                    // Activate new fact (IN) with incremented version
                    new_fact.status = FactStatus::Active;
                    new_fact.version = old_fact.version.max(new_fact.version) + 1;
                    new_fact.last_updated_at = Utc::now();
                    store.insert_or_update_semantic_fact(new_fact)?;
                } else {
                    // Old fact not found; simply insert new fact
                    new_fact.status = FactStatus::Active;
                    store.insert_or_update_semantic_fact(new_fact)?;
                }
            }
            ConflictResolution::Reject => {
                // Reject new candidate (OUT / Deprecated)
                new_fact.status = FactStatus::Deprecated;
                new_fact.replaced_by = Some(*old_fact_id);
                new_fact.last_updated_at = Utc::now();
                store.insert_or_update_semantic_fact(new_fact)?;

                // Record audit row for rejected candidate
                let audit = JtmsAuditRow::new(
                    *old_fact_id,
                    new_fact.id,
                    "reject",
                    format!(
                        "Candidate fact {} rejected in favor of existing fact {}",
                        new_fact.id, old_fact_id
                    ),
                );
                store.insert_jtms_audit(&audit)?;

                // Propagate downstream invalidation for candidate if needed
                self.propagate_downstream_invalidation(
                    store,
                    &new_fact.id,
                    &format!(
                        "Downstream invalidation after candidate fact {} was rejected",
                        new_fact.id
                    ),
                )?;
            }
            ConflictResolution::Coexist => {
                // Keep both active
                new_fact.status = FactStatus::Active;
                store.insert_or_update_semantic_fact(new_fact)?;
            }
            ConflictResolution::Invalidate => {
                new_fact.status = FactStatus::Stale;
                new_fact.last_updated_at = Utc::now();
                if let Some(ref mut anchor) = new_fact.code_anchor {
                    anchor.invalidate();
                }
                store.insert_or_update_semantic_fact(new_fact)?;

                self.propagate_downstream_invalidation(
                    store,
                    &new_fact.id,
                    &format!(
                        "Downstream invalidation after fact {} was marked stale",
                        new_fact.id
                    ),
                )?;
            }
        }

        Ok(())
    }

    /// Check for conflicts and automatically resolve & upsert into SQLite store with 100% replay consistency.
    pub fn resolve_and_upsert(
        &self,
        store: &SqliteStore,
        new_fact: &mut SemanticFact,
        new_embedding: &[f32],
    ) -> Result<Vec<ConflictMatch>, StrataError> {
        // Fetch existing active facts with embeddings
        let existing = store.get_semantic_facts_with_embeddings(None, Some(FactStatus::Active))?;
        let active_with_embs: Vec<(SemanticFact, Vec<f32>)> = existing
            .into_iter()
            .filter_map(|(f, e)| e.map(|vec| (f, vec)))
            .collect();

        let conflicts = self.find_conflicts(new_fact, new_embedding, &active_with_embs);

        // Check if any existing fact defeats this candidate
        let defeated_by = conflicts
            .iter()
            .find(|c| c.resolution == ConflictResolution::Reject);

        if let Some(defeat) = defeated_by {
            // Existing fact wins, candidate loses (marked Deprecated / OUT)
            new_fact.status = FactStatus::Deprecated;
            new_fact.replaced_by = Some(defeat.existing_fact_id);
            new_fact.last_updated_at = Utc::now();
            store.insert_or_update_semantic_fact(new_fact)?;

            let audit = JtmsAuditRow::new(
                defeat.existing_fact_id,
                new_fact.id,
                "reject",
                &defeat.reason,
            )
            .with_cues(defeat.contradiction_cues.clone())
            .with_similarity(defeat.similarity);
            store.insert_jtms_audit(&audit)?;

            self.propagate_downstream_invalidation(
                store,
                &new_fact.id,
                "Candidate rejected during JTMS replay-consistent arbitration",
            )?;
        } else {
            // Candidate wins against all conflicts (or has no conflicts)
            let mut highest_old_version = 0;

            for conflict in &conflicts {
                if conflict.resolution == ConflictResolution::Supersede {
                    if let Some(mut old_fact) =
                        store.get_semantic_fact(&conflict.existing_fact_id)?
                    {
                        highest_old_version = highest_old_version.max(old_fact.version);

                        // Deprecate losing old fact (OUT)
                        old_fact.status = FactStatus::Deprecated;
                        old_fact.replaced_by = Some(new_fact.id);
                        old_fact.last_updated_at = Utc::now();
                        if let Some(ref mut anchor) = old_fact.code_anchor {
                            anchor.invalidate();
                        }
                        store.insert_or_update_semantic_fact(&old_fact)?;

                        let audit = JtmsAuditRow::new(
                            new_fact.id,
                            old_fact.id,
                            "supersede",
                            &conflict.reason,
                        )
                        .with_cues(conflict.contradiction_cues.clone())
                        .with_similarity(conflict.similarity);
                        store.insert_jtms_audit(&audit)?;

                        // Propagate downstream invalidation for the old fact
                        self.propagate_downstream_invalidation(
                            store,
                            &old_fact.id,
                            &format!(
                                "Prerequisite fact {} was superseded by {}",
                                old_fact.id, new_fact.id
                            ),
                        )?;
                    }
                }
            }

            // Candidate becomes Active (IN) with version incremented
            new_fact.status = FactStatus::Active;
            if highest_old_version > 0 {
                new_fact.version = highest_old_version + 1;
            }
            new_fact.last_updated_at = Utc::now();
            store.insert_or_update_semantic_fact(new_fact)?;
        }

        // Store embedding
        store.update_semantic_fact_embedding(&new_fact.id, new_embedding)?;

        Ok(conflicts)
    }

    /// Verifies if a belief is currently valid (`Active` status and all prerequisite beliefs are `Active`).
    pub fn is_belief_valid(
        &self,
        store: &SqliteStore,
        fact_id: &Uuid,
    ) -> Result<bool, StrataError> {
        let Some(fact) = store.get_semantic_fact(fact_id)? else {
            return Ok(false);
        };

        if fact.status != FactStatus::Active {
            return Ok(false);
        }

        // Check all prerequisites in fact_dependencies
        let prereq_ids = store.get_upstream_prerequisite_fact_ids(fact_id)?;
        for pid in prereq_ids {
            if let Some(prereq) = store.get_semantic_fact(&pid)? {
                if prereq.status != FactStatus::Active {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Recomputes truth maintenance justifications across all semantic facts in SQLite.
    /// Traverses the dependency graph and marks any fact Stale if its prerequisites are no longer Active.
    pub fn recompute_justifications(&self, store: &SqliteStore) -> Result<usize, StrataError> {
        let all_active = store.get_all_semantic_facts(None, Some(FactStatus::Active), 10000)?;
        let mut invalidated_count = 0;

        for fact in all_active {
            let is_valid = self.is_belief_valid(store, &fact.id)?;
            if !is_valid {
                let mut invalidated_fact = fact.clone();
                invalidated_fact.mark_stale();
                store.insert_or_update_semantic_fact(&invalidated_fact)?;

                let audit = JtmsAuditRow::new(
                    fact.id,
                    fact.id,
                    "invalidation",
                    "Periodic justification recomputation detected invalid/inactive prerequisites",
                );
                store.insert_jtms_audit(&audit)?;

                self.propagate_downstream_invalidation(
                    store,
                    &fact.id,
                    "Prerequisites invalidated during global justification sweep",
                )?;
                invalidated_count += 1;
            }
        }

        Ok(invalidated_count)
    }
}
