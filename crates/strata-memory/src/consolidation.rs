use chrono::Utc;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use strata_core::errors::StrataError;
use strata_core::events::{Event, EventPayload};
use strata_core::schemas::{EpisodicMemory, SemanticFact, SignalScores};
use strata_core::state::{
    DigestOutput, FailurePattern, FailureSeverity, MemoryHandle, MemoryRecord, MemoryType, Scope,
};

use crate::clustering::{SpatialClusterer, VectorPoint};
use crate::decay::DecayCalculator;
use crate::embedding::EmbeddingProvider;
use crate::enricher::{CanonicalTemplateEnricher, MemoryEnricher};
use crate::jtms::TruthMaintenanceSystem;
use crate::pipeline::ConsolidationResult;
use crate::procedural_mining::TrajectoryMiner;
use crate::store::SqliteStore;
use crate::subconscious::SubconsciousBuffer;

pub struct Consolidator;

impl Consolidator {
    pub fn new() -> Self {
        Self
    }

    /// Extract memories from a canonical event if applicable.
    pub fn extract_from_event(&self, event: &Event) -> Option<MemoryRecord> {
        let scope = Scope::Session(event.session_id.clone());

        match &event.payload {
            EventPayload::SessionEnded(ended) => {
                let summary = ended.summary.clone().unwrap_or_else(|| {
                    format!(
                        "Session {} ended with status {:?}",
                        ended.session_id, ended.final_state
                    )
                });
                let content = format!(
                    "Session {}: final_state={:?}, reason={:?}, summary={}",
                    ended.session_id, ended.final_state, ended.reason, summary
                );
                let mut record = MemoryRecord::new(MemoryType::Episodic, content, scope)
                    .with_summary(summary)
                    .with_importance(0.7);
                record.created_at = ended.timestamp;
                Some(record)
            }
            EventPayload::TaskCompleted(task) => {
                let content = format!(
                    "Task {} ({}) - Outcome: {}",
                    task.task_id,
                    if task.success { "SUCCESS" } else { "FAILURE" },
                    task.outcome_summary
                );
                let importance = if task.success { 0.6 } else { 0.8 };
                let mem_type = if task.success {
                    MemoryType::Procedural
                } else {
                    MemoryType::NegativePattern
                };
                let mut record = MemoryRecord::new(mem_type, content, scope)
                    .with_summary(task.outcome_summary.clone())
                    .with_importance(importance);
                record.created_at = task.timestamp;
                Some(record)
            }
            EventPayload::ObservationReceived(obs) => {
                let content = format!("Observation from {}: {}", obs.source, obs.content);
                let mut record = MemoryRecord::new(MemoryType::Semantic, content, scope)
                    .with_summary(format!("Observation ({})", obs.observation_type))
                    .with_importance(0.5);
                record.created_at = obs.timestamp;
                Some(record)
            }
            EventPayload::ErrorObserved(err) => {
                let content = format!(
                    "Error {}: {} (Severity: {})",
                    err.error_type, err.message, err.severity
                );
                let mut record = MemoryRecord::new(MemoryType::NegativePattern, content, scope)
                    .with_summary(format!("Error: {}", err.error_type))
                    .with_importance(0.85);
                record.created_at = err.timestamp;
                Some(record)
            }
            _ => None,
        }
    }

    /// Dereference memory handles to full MemoryRecords.
    pub fn dereference_handles(
        &self,
        store: &SqliteStore,
        handles: &[MemoryHandle],
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let mut records = Vec::with_capacity(handles.len());
        for handle in handles {
            if let Some(record) = store.get_memory(&handle.id)? {
                records.push(record);
            }
        }
        Ok(records)
    }

    /// Stable signature for a tool failure: tool name plus normalized error message.
    pub fn failure_signature(tool_name: &str, error_msg: &str) -> String {
        let normalized_error = normalize_error_message(error_msg);
        let mut hasher = DefaultHasher::new();
        tool_name.hash(&mut hasher);
        normalized_error.hash(&mut hasher);
        let sig_hash = hasher.finish();
        format!("fail:{tool_name}:{sig_hash:016x}")
    }

    /// Record a tool failure and consolidate it into the failure_patterns store.
    pub fn record_tool_failure(
        &self,
        store: &SqliteStore,
        tool_name: &str,
        error_msg: &str,
        context: &str,
        scope: Option<&Scope>,
    ) -> Result<FailurePattern, StrataError> {
        let signature = Self::failure_signature(tool_name, error_msg);

        let pattern_name = format!("ToolFailure_{tool_name}");
        let description = format!("Tool '{tool_name}' failed: {error_msg}");
        let trigger_condition = if context.is_empty() {
            format!("Invoking tool '{tool_name}'")
        } else {
            format!("Invoking tool '{tool_name}' with context: {context}")
        };
        let mitigation = generate_mitigation_for_error(tool_name, error_msg);
        let severity = classify_error_severity(error_msg);
        let pattern_scope = scope.cloned().unwrap_or(Scope::Global);

        let mut failure =
            FailurePattern::new(signature.clone(), pattern_name, description, mitigation);
        failure.trigger_condition = trigger_condition;
        failure.error_type = format!("ToolExecutionError({tool_name})");
        failure.severity = severity;
        failure.scope = pattern_scope;
        failure.metadata = serde_json::json!({
            "tool_name": tool_name,
            "raw_error": error_msg,
            "context": context,
        });

        // Upsert into store (increments occurrences if exists)
        store.upsert_failure_pattern(&failure)?;

        // Return current full state of pattern
        let persisted = store
            .get_failure_pattern_by_signature(&signature)?
            .unwrap_or(failure);

        Ok(persisted)
    }

    /// Get known failure patterns relevant to the current query and scope.
    pub fn get_known_failures(
        &self,
        store: &SqliteStore,
        query: Option<&str>,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        store.search_failures(query, scope, limit)
    }

    /// Generate a compact working digest (~300-500 tokens) for the given session.
    pub fn generate_digest(
        &self,
        store: &SqliteStore,
        session_id: &str,
        max_tokens: Option<usize>,
    ) -> Result<DigestOutput, StrataError> {
        self.generate_digest_with_active_files(store, session_id, max_tokens, &[])
    }

    /// Generate a compact working digest (~300-500 tokens) for the given session,
    /// biasing `key_pointers` and `failure_warnings` toward `active_files` and their
    /// 1-hop / 2-hop import graph neighbors.
    pub fn generate_digest_with_active_files(
        &self,
        store: &SqliteStore,
        session_id: &str,
        max_tokens: Option<usize>,
        active_files: &[String],
    ) -> Result<DigestOutput, StrataError> {
        let max_tok = max_tokens.unwrap_or(450);
        let session_scope = Scope::Session(session_id.to_string());

        // 1. Fetch recent events for session
        let recent_events = store.get_events(session_id, None, Some(20))?;

        // Extract decisions and hypotheses from recent events
        let mut recent_decisions = Vec::new();
        let mut active_hypotheses = Vec::new();

        for event in &recent_events {
            match &event.payload {
                EventPayload::TaskCompleted(task) => {
                    recent_decisions.push(format!(
                        "Task {}: {} (Result: {})",
                        task.task_id,
                        if task.success { "Succeeded" } else { "Failed" },
                        task.outcome_summary
                    ));
                }
                EventPayload::ToolResultReceived(res) => {
                    if res.is_error {
                        recent_decisions.push(format!(
                            "Tool '{}' reported error: {:?}",
                            res.tool_name, res.result
                        ));
                    }
                }
                EventPayload::ObservationReceived(obs) => {
                    if obs.observation_type.contains("hypothesis")
                        || obs.observation_type.contains("plan")
                    {
                        active_hypotheses.push(format!("{}: {}", obs.source, obs.content));
                    }
                }
                _ => {}
            }
        }

        // 2. Fetch and rank key memory pointers in session & global scope
        let key_pointers: Vec<MemoryHandle> = if active_files.is_empty() {
            let mut top_memories = store.get_all_memories(Some(&session_scope), None, 10)?;
            top_memories.retain(|m| {
                let is_suppressed = m
                    .metadata
                    .get("quarantined")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || m.metadata
                        .get("suppressed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    || m.metadata
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map_or(false, |s| {
                            s.eq_ignore_ascii_case("quarantined")
                                || s.eq_ignore_ascii_case("suppressed")
                        });
                !is_suppressed
            });
            top_memories.truncate(5);
            top_memories
                .into_iter()
                .map(|m| m.to_handle(Some(m.importance)))
                .collect()
        } else {
            let prox = crate::retrieval::ProximityBiasEngine::from_store(
                store,
                crate::retrieval::ProximityBiasConfig::default(),
            )?;
            let hop_map = prox.compute_hop_distances(active_files);
            let mut candidates = store.get_all_memories(Some(&session_scope), None, 50)?;
            candidates.retain(|m| {
                let is_suppressed = m
                    .metadata
                    .get("quarantined")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || m.metadata
                        .get("suppressed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    || m.metadata
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map_or(false, |s| {
                            s.eq_ignore_ascii_case("quarantined")
                                || s.eq_ignore_ascii_case("suppressed")
                        });
                !is_suppressed
            });
            let mut scored: Vec<(MemoryRecord, f32)> = candidates
                .into_iter()
                .map(|m| {
                    let boost = prox.memory_proximity_boost_with_map(&m, &hop_map);
                    let score = m.importance + boost;
                    (m, score)
                })
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scored
                .into_iter()
                .take(5)
                .map(|(m, score)| m.to_handle(Some(score)))
                .collect()
        };

        // 3. Fetch top known failure warnings
        let failure_warnings: Vec<FailurePattern> = if active_files.is_empty() {
            store.search_failures(None, Some(&session_scope), 3)?
        } else {
            let prox = crate::retrieval::ProximityBiasEngine::from_store(
                store,
                crate::retrieval::ProximityBiasConfig::default(),
            )?;
            let hop_map = prox.compute_hop_distances(active_files);
            let mut candidates = store.search_failures(None, Some(&session_scope), 20)?;
            candidates.sort_by(|a, b| {
                let boost_a = prox.failure_proximity_boost_with_map(a, &hop_map);
                let boost_b = prox.failure_proximity_boost_with_map(b, &hop_map);
                let base_a = (a.occurrences as f32) * 0.05;
                let base_b = (b.occurrences as f32) * 0.05;
                (base_b + boost_b)
                    .partial_cmp(&(base_a + boost_a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            candidates.truncate(3);
            candidates
        };

        // 4. Construct compact summary
        let summary = format!(
            "Session '{}': {} events logged, {} pointers available, {} known failure warnings.",
            session_id,
            recent_events.len(),
            key_pointers.len(),
            failure_warnings.len()
        );

        let mut digest = DigestOutput::new(session_id, summary);
        digest.recent_decisions = recent_decisions;
        digest.active_hypotheses = active_hypotheses;
        digest.key_pointers = key_pointers;
        digest.failure_warnings = failure_warnings;

        // 5. Estimate token count and trim if necessary
        digest.estimated_tokens = estimate_digest_tokens(&digest);
        if digest.estimated_tokens > max_tok {
            // Trim lists to fit budget
            if digest.recent_decisions.len() > 3 {
                digest.recent_decisions.truncate(3);
            }
            if digest.failure_warnings.len() > 2 {
                digest.failure_warnings.truncate(2);
            }
            digest.estimated_tokens = estimate_digest_tokens(&digest);
        }

        Ok(digest)
    }
}

impl Default for Consolidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize error messages to create stable signatures (strips numbers, quotes, hex strings).
fn normalize_error_message(err: &str) -> String {
    let mut normalized = String::with_capacity(err.len());
    let mut in_num = false;

    for c in err.chars() {
        if c.is_ascii_digit() || c == '\'' || c == '\"' || c == '`' {
            if !in_num {
                normalized.push('_');
                in_num = true;
            }
        } else {
            in_num = false;
            normalized.push(c.to_ascii_lowercase());
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Derive actionable mitigation recommendation for known error classes.
fn generate_mitigation_for_error(tool_name: &str, error_msg: &str) -> String {
    let err_lower = error_msg.to_lowercase();
    if err_lower.contains("timeout") || err_lower.contains("timed out") {
        format!("Increase timeout threshold or retry tool '{tool_name}' with backoff.")
    } else if err_lower.contains("permission")
        || err_lower.contains("denied")
        || err_lower.contains("unauthorized")
    {
        format!("Verify authentication credentials and permissions for '{tool_name}'.")
    } else if err_lower.contains("not found")
        || err_lower.contains("404")
        || err_lower.contains("no such")
    {
        format!("Check path/identifier validity before calling '{tool_name}'.")
    } else if err_lower.contains("rate limit") || err_lower.contains("429") {
        format!("Apply rate-limiting delay before invoking '{tool_name}'.")
    } else if err_lower.contains("syntax")
        || err_lower.contains("parse")
        || err_lower.contains("json")
    {
        format!("Validate and sanitize parameters format before calling '{tool_name}'.")
    } else {
        format!("Review input parameters and pre-conditions for '{tool_name}'.")
    }
}

/// Classify error severity based on error characteristics.
fn classify_error_severity(error_msg: &str) -> FailureSeverity {
    let err_lower = error_msg.to_lowercase();
    if err_lower.contains("fatal") || err_lower.contains("panic") || err_lower.contains("corrupt") {
        FailureSeverity::Critical
    } else if err_lower.contains("permission")
        || err_lower.contains("denied")
        || err_lower.contains("timeout")
        || err_lower.contains("timed out")
        || err_lower.contains("fail")
    {
        FailureSeverity::High
    } else if err_lower.contains("warn") || err_lower.contains("retry") {
        FailureSeverity::Low
    } else {
        FailureSeverity::Medium
    }
}

/// Estimate tokens from digest representation (~1 token per 3.5 chars).
fn estimate_digest_tokens(digest: &DigestOutput) -> usize {
    let mut total_chars = digest.summary.len();
    for d in &digest.recent_decisions {
        total_chars += d.len();
    }
    for h in &digest.active_hypotheses {
        total_chars += h.len();
    }
    for p in &digest.key_pointers {
        total_chars += p.title.len() + p.summary.len();
    }
    for f in &digest.failure_warnings {
        total_chars += f.description.len() + f.mitigation.len();
    }
    (total_chars + 3) / 4
}

// ============================================================================
// ACT-R Mathematical Decay & Pruning Engine
// ============================================================================

/// Calculate ACT-R base-level activation score (0.0 to 1.0) for a memory record.
/// Takes into account:
/// - Base importance (0.0 to 1.0)
/// - Recency elapsed since last access (or creation) with power-law decay (exponent 0.5)
/// - Access frequency boost: ln(1 + access_count)
pub fn calculate_act_r_activation(
    record: &MemoryRecord,
    eval_time: chrono::DateTime<chrono::Utc>,
) -> f32 {
    let status = record
        .metadata
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("Active");

    if status.eq_ignore_ascii_case("deprecated") || status.eq_ignore_ascii_case("archived") {
        return 0.0;
    }

    let effective_time = record.last_accessed_at.unwrap_or(record.created_at);
    let seconds_diff = (eval_time - effective_time).num_seconds().max(0) as f32;
    let days_diff = seconds_diff / 86400.0;

    // Decay rate inversely proportional to importance (high importance decays much slower)
    let decay_rate = (1.0 - record.importance).clamp(0.02, 1.0);
    let power_law_factor = 1.0 / (1.0 + days_diff * decay_rate).powf(0.5);

    // Frequency boost from access count (ACT-R frequency component)
    let access_boost = ((1.0 + record.access_count as f32).ln() * 0.15).min(0.4);

    let activation = (record.importance * power_law_factor + access_boost).clamp(0.0, 1.0);
    activation
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PruneReport {
    pub evaluated: usize,
    pub retained: usize,
    pub pruned: usize,
    pub threshold: f32,
}

pub fn prune_decayed_memories(
    store: &SqliteStore,
    threshold: f32,
    scope: Option<&Scope>,
    eval_time: chrono::DateTime<chrono::Utc>,
) -> Result<PruneReport, StrataError> {
    let all_memories = store.get_all_memories(scope, None, 100_000)?;
    let mut evaluated = 0;
    let mut retained = 0;
    let mut pruned = 0;

    for memory in all_memories {
        evaluated += 1;
        let act = calculate_act_r_activation(&memory, eval_time);
        if act < threshold {
            store.delete_memory(&memory.id)?;
            pruned += 1;
        } else {
            retained += 1;
        }
    }

    Ok(PruneReport {
        evaluated,
        retained,
        pruned,
        threshold,
    })
}

// ============================================================================
// JTMS (Justification-based Truth Maintenance System) Engine
// ============================================================================

pub struct JtmsEngine;

impl JtmsEngine {
    pub fn new() -> Self {
        Self
    }

    /// Ingest a semantic fact with JTMS contradiction detection and belief revision.
    pub async fn ingest_fact(
        &self,
        store: &SqliteStore,
        reasoning_engine: Option<&(dyn strata_core::traits::ReasoningEngine + Send + Sync)>,
        new_fact: &strata_reasoning::prompts::SemanticFact,
        scope: Scope,
    ) -> Result<MemoryRecord, StrataError> {
        let existing_memories =
            store.get_all_memories(Some(&scope), Some(&[MemoryType::Semantic]), 50)?;
        let mut fact_to_insert = new_fact.clone();

        for existing in &existing_memories {
            let old_fact = strata_reasoning::prompts::SemanticFact::from_memory_record(existing);
            if old_fact.status.eq_ignore_ascii_case("deprecated") {
                continue;
            }

            // Check if there is potential contradiction / overlap
            let is_match = Self::check_topic_overlap(&old_fact, new_fact);
            if is_match {
                let classification = if let Some(engine) = reasoning_engine {
                    let prompt = strata_reasoning::prompts::build_jtms_arbitration_prompt(
                        &old_fact, new_fact,
                    );
                    if let Ok(res_str) = engine.prompt(None, &prompt, None).await {
                        strata_reasoning::prompts::parse_jtms_arbitration(&res_str)
                            .map(|r| r.classification)
                            .unwrap_or_else(|_| Self::heuristic_classify(&old_fact, new_fact))
                    } else {
                        Self::heuristic_classify(&old_fact, new_fact)
                    }
                } else {
                    Self::heuristic_classify(&old_fact, new_fact)
                };

                match classification {
                    strata_reasoning::prompts::JtmsClassification::Update => {
                        let new_id = fact_to_insert.id.unwrap_or_else(uuid::Uuid::new_v4);
                        fact_to_insert.id = Some(new_id);
                        fact_to_insert.version = old_fact.version + 1;

                        // Deprecate existing fact
                        let mut deprecated_old = existing.clone();
                        let mut meta = deprecated_old.metadata.clone();
                        if !meta.is_object() {
                            meta = serde_json::json!({});
                        }
                        meta["status"] = serde_json::json!("Deprecated");
                        meta["replaced_by"] = serde_json::json!(new_id.to_string());
                        deprecated_old.metadata = meta;
                        store.insert_or_update_memory(&deprecated_old)?;

                        // Set supersedes on new fact
                        let mut new_meta = fact_to_insert.metadata.clone();
                        if !new_meta.is_object() {
                            new_meta = serde_json::json!({});
                        }
                        new_meta["supersedes"] =
                            serde_json::json!(old_fact.id.map(|u| u.to_string()));
                        fact_to_insert.metadata = new_meta;
                        break;
                    }
                    strata_reasoning::prompts::JtmsClassification::Duplicate => {
                        let mut reinforced = existing.clone();
                        reinforced.access_count += 1;
                        reinforced.confidence = (reinforced.confidence + 0.1).min(1.0);
                        store.insert_or_update_memory(&reinforced)?;
                        return Ok(reinforced);
                    }
                    _ => {}
                }
            }
        }

        let record = fact_to_insert.to_memory_record(scope);
        store.insert_or_update_memory(&record)?;
        Ok(record)
    }

    fn check_topic_overlap(
        old_fact: &strata_reasoning::prompts::SemanticFact,
        new_fact: &strata_reasoning::prompts::SemanticFact,
    ) -> bool {
        let old_lower = old_fact.statement.to_lowercase();
        let new_lower = new_fact.statement.to_lowercase();

        let keywords = [
            "architecture",
            "database",
            "storage",
            "protocol",
            "api",
            "framework",
            "auth",
            "cache",
            "network",
            "transport",
            "engine",
            "system",
            "format",
            "rest",
            "grpc",
            "sqlite",
            "postgres",
            "json",
            "protobuf",
        ];

        let mut matches = 0;
        for kw in &keywords {
            if old_lower.contains(kw) && new_lower.contains(kw) {
                matches += 1;
            }
        }

        matches >= 1
            || (!old_fact.tags.is_empty()
                && old_fact.tags.iter().any(|t| new_fact.tags.contains(t)))
    }

    fn heuristic_classify(
        old_fact: &strata_reasoning::prompts::SemanticFact,
        new_fact: &strata_reasoning::prompts::SemanticFact,
    ) -> strata_reasoning::prompts::JtmsClassification {
        let old_lower = old_fact.statement.to_lowercase();
        let new_lower = new_fact.statement.to_lowercase();

        if old_lower == new_lower {
            return strata_reasoning::prompts::JtmsClassification::Duplicate;
        }

        let update_indicators = [
            "migrated to",
            "switched to",
            "replaced by",
            "upgraded to",
            "deprecated",
            "transitioned to",
            "instead of",
            "changed to",
            "uses grpc",
            "uses protobuf",
            "moved to",
            "adopt",
            "refactored to",
        ];

        for ind in &update_indicators {
            if new_lower.contains(ind) {
                return strata_reasoning::prompts::JtmsClassification::Update;
            }
        }

        if (old_lower.contains("rest") && new_lower.contains("grpc"))
            || (old_lower.contains("json") && new_lower.contains("protobuf"))
            || (old_lower.contains("sqlite") && new_lower.contains("postgres"))
        {
            return strata_reasoning::prompts::JtmsClassification::Update;
        }

        strata_reasoning::prompts::JtmsClassification::Update
    }
}

// ============================================================================
// Consolidation Pipeline
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Default)]
pub struct ConsolidationReport {
    pub session_id: Option<String>,
    pub episodic_created: usize,
    pub semantic_created: usize,
    pub procedural_created: usize,
    pub negative_created: usize,
    pub deprecated_facts: usize,
    pub total_consolidated: usize,
}

pub struct ConsolidationPipeline {
    store: std::sync::Arc<SqliteStore>,
    reasoning_engine: std::sync::Arc<dyn strata_core::traits::ReasoningEngine>,
    jtms: JtmsEngine,
}

impl ConsolidationPipeline {
    pub fn new(
        store: std::sync::Arc<SqliteStore>,
        reasoning_engine: std::sync::Arc<dyn strata_core::traits::ReasoningEngine>,
    ) -> Self {
        Self {
            store,
            reasoning_engine,
            jtms: JtmsEngine::new(),
        }
    }

    pub async fn consolidate_session(
        &self,
        session_id: &str,
    ) -> Result<ConsolidationReport, StrataError> {
        let events = self.store.get_events(session_id, None, None)?;
        if events.is_empty() {
            return Ok(ConsolidationReport {
                session_id: Some(session_id.to_string()),
                ..Default::default()
            });
        }

        let prompt = strata_reasoning::prompts::build_distillation_prompt(&events);
        let response = self
            .reasoning_engine
            .prompt(
                Some("You are the cognitive memory consolidation engine for Strata. Extract durable knowledge conforming strictly to JSON schema."),
                &prompt,
                None,
            )
            .await?;

        let distillation = strata_reasoning::prompts::parse_distillation_output(&response)
            .unwrap_or_else(|_| fallback_extract_distillation(&events));

        let mut report = ConsolidationReport {
            session_id: Some(session_id.to_string()),
            ..Default::default()
        };

        let session_scope = Scope::Session(session_id.to_string());

        // 1. Episodic Memories
        for ep in distillation.episodic_memories {
            let record = MemoryRecord::new(MemoryType::Episodic, ep.content, session_scope.clone())
                .with_summary(ep.summary)
                .with_importance(ep.importance)
                .with_tags(ep.tags);
            self.store.insert_or_update_memory(&record)?;
            report.episodic_created += 1;
        }

        // 2. Semantic Facts with JTMS Belief Revision
        for fact in distillation.semantic_facts {
            let before_deprecated = count_deprecated_memories(&self.store, &session_scope)?;
            self.jtms
                .ingest_fact(
                    &self.store,
                    Some(self.reasoning_engine.as_ref()),
                    &fact,
                    session_scope.clone(),
                )
                .await?;
            let after_deprecated = count_deprecated_memories(&self.store, &session_scope)?;
            if after_deprecated > before_deprecated {
                report.deprecated_facts += after_deprecated - before_deprecated;
            }
            report.semantic_created += 1;
        }

        // 3. Procedural Skills
        for skill in distillation.procedural_skills {
            let record = skill.to_memory_record(session_scope.clone());
            self.store.insert_or_update_memory(&record)?;
            report.procedural_created += 1;
        }

        // 4. Negative Patterns
        for neg in distillation.negative_patterns {
            let mut pattern = FailurePattern::new(
                neg.signature,
                neg.pattern_name,
                neg.description,
                neg.mitigation,
            );
            pattern.trigger_condition = neg.trigger_condition;
            pattern.severity = neg.severity.parse().unwrap_or(FailureSeverity::High);
            if let Some(et) = neg.error_type {
                pattern.error_type = et;
            }
            pattern.scope = session_scope.clone();
            self.store.upsert_failure_pattern(&pattern)?;
            report.negative_created += 1;
        }

        report.total_consolidated = report.episodic_created
            + report.semantic_created
            + report.procedural_created
            + report.negative_created;

        Ok(report)
    }

    pub async fn consolidate_all(&self) -> Result<ConsolidationReport, StrataError> {
        let session_ids = self.store.get_session_ids()?;
        let mut total_report = ConsolidationReport::default();

        for sid in session_ids {
            let rep = self.consolidate_session(&sid).await?;
            total_report.episodic_created += rep.episodic_created;
            total_report.semantic_created += rep.semantic_created;
            total_report.procedural_created += rep.procedural_created;
            total_report.negative_created += rep.negative_created;
            total_report.deprecated_facts += rep.deprecated_facts;
            total_report.total_consolidated += rep.total_consolidated;
        }

        Ok(total_report)
    }

    pub async fn prune_decayed(&self, threshold: f32) -> Result<PruneReport, StrataError> {
        prune_decayed_memories(&self.store, threshold, None, chrono::Utc::now())
    }
}

fn count_deprecated_memories(store: &SqliteStore, scope: &Scope) -> Result<usize, StrataError> {
    let memories = store.get_all_memories(Some(scope), Some(&[MemoryType::Semantic]), 10_000)?;
    let count = memories
        .iter()
        .filter(|m| m.metadata.get("status").and_then(|v| v.as_str()) == Some("Deprecated"))
        .count();
    Ok(count)
}

fn fallback_extract_distillation(
    events: &[Event],
) -> strata_reasoning::prompts::DistillationOutput {
    let mut out = strata_reasoning::prompts::DistillationOutput::default();
    let consolidator = Consolidator::new();

    for event in events {
        if let Some(record) = consolidator.extract_from_event(event) {
            match record.memory_type {
                MemoryType::Episodic => {
                    out.episodic_memories
                        .push(strata_reasoning::prompts::EpisodicMemoryItem {
                            summary: record
                                .summary
                                .unwrap_or_else(|| "Session event".to_string()),
                            content: record.content,
                            importance: record.importance,
                            tags: record.tags,
                        });
                }
                MemoryType::Semantic => {
                    out.semantic_facts.push(
                        strata_reasoning::prompts::SemanticFact::new(record.content)
                            .with_importance(record.importance)
                            .with_tags(record.tags),
                    );
                }
                MemoryType::Procedural => {
                    let step = strata_reasoning::prompts::ProceduralStep {
                        step_number: 1,
                        action: record.content.clone(),
                        tool_name: None,
                        expected_outcome: record.summary.clone(),
                    };
                    out.procedural_skills
                        .push(strata_reasoning::prompts::ProceduralSkill::new(
                            record
                                .summary
                                .unwrap_or_else(|| "Procedural Task".to_string()),
                            record.content,
                            vec![step],
                        ));
                }
                MemoryType::NegativePattern => {
                    out.negative_patterns
                        .push(strata_reasoning::prompts::NegativePatternItem {
                            signature: format!("event_{:?}", event.id),
                            pattern_name: "EventFailure".to_string(),
                            description: record.content,
                            trigger_condition: "Tool failure".to_string(),
                            mitigation: "Inspect error log".to_string(),
                            severity: "high".to_string(),
                            error_type: Some("ErrorObserved".to_string()),
                        });
                }
            }
        }
    }

    out
}

// ============================================================================
// Neuro-Symbolic & Deterministic Memory Consolidator
// ============================================================================

/// Neuro-Symbolic memory consolidation engine that operates 100% deterministically and offline in Rust.
/// Combines SubconsciousBuffer (RecMem), NREM Pruning & Invalidation (SleepGate/ACT-R/JTMS),
/// REM Spatial Clustering (Louvain/Medoids), and Trajectory Folding (PrefixSpan).
pub struct NeuroSymbolicConsolidator {
    pub store: Arc<SqliteStore>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub jtms: TruthMaintenanceSystem,
    pub decay: DecayCalculator,
    pub subconscious: SubconsciousBuffer,
    pub clusterer: SpatialClusterer,
    pub trajectory_miner: TrajectoryMiner,
    pub enricher: Arc<dyn MemoryEnricher>,
}

impl NeuroSymbolicConsolidator {
    pub fn new(store: Arc<SqliteStore>, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            store,
            embedder,
            jtms: TruthMaintenanceSystem::with_default_threshold(),
            decay: DecayCalculator::with_default_config(),
            subconscious: SubconsciousBuffer::with_default_config(),
            clusterer: SpatialClusterer::default(),
            trajectory_miner: TrajectoryMiner::new(),
            enricher: Arc::new(CanonicalTemplateEnricher::new()),
        }
    }

    pub fn with_enricher(mut self, enricher: Arc<dyn MemoryEnricher>) -> Self {
        self.enricher = enricher;
        self
    }

    /// Consolidate a session completely offline without calling an LLM for structured knowledge formation.
    pub async fn consolidate_session(
        &mut self,
        session_id: &str,
    ) -> Result<ConsolidationResult, StrataError> {
        let events = self.store.get_events(session_id, None, None)?;
        let mut result = ConsolidationResult::default();
        result.events_processed = events.len();
        result.session_id = Some(session_id.to_string());

        if events.is_empty() {
            return Ok(result);
        }

        // 1. Subconscious Ingestion & Recurrence Gating
        let mut points_to_cluster = Vec::new();
        let mut candidate_facts = Vec::new();

        for event in &events {
            let text = match &event.payload {
                EventPayload::TaskStarted(t) => {
                    format!("TaskStarted: {} - {:?}", t.title, t.description)
                }
                EventPayload::TaskCompleted(t) => {
                    format!("TaskCompleted: {} - {}", t.task_id, t.outcome_summary)
                }
                EventPayload::ErrorObserved(e) => {
                    format!("ErrorObserved {}: {}", e.error_type, e.message)
                }
                EventPayload::ToolInvoked(inv) => {
                    format!("ToolInvoked: {} with input {:?}", inv.tool_name, inv.input)
                }
                EventPayload::ToolResultReceived(res) => format!(
                    "ToolResult: {} (error: {}) {:?}",
                    res.tool_name, res.is_error, res.result
                ),
                EventPayload::ObservationReceived(obs) => {
                    format!("Observation: {} - {}", obs.source, obs.content)
                }
                EventPayload::SessionEnded(se) => format!("SessionEnded: {:?}", se.final_state),
                _ => format!("Event: {:?}", event.id),
            };

            let emb = self.embedder.embed_text(&text).await?;
            let importance = match &event.payload {
                EventPayload::ErrorObserved(_) => 0.85,
                EventPayload::TaskCompleted(t) => {
                    if t.success {
                        0.80
                    } else {
                        0.70
                    }
                }
                EventPayload::ObservationReceived(obs) => {
                    let c = obs.content.to_string().to_lowercase();
                    if c.contains("architecture")
                        || c.contains("database")
                        || c.contains("migrated")
                        || c.contains("protocol")
                        || c.contains("decision")
                    {
                        0.95 // High-importance bypass
                    } else {
                        0.60
                    }
                }
                _ => 0.50,
            };

            let _gating = self
                .subconscious
                .ingest(event.clone(), emb.clone(), importance);

            if let EventPayload::ObservationReceived(obs) = &event.payload {
                let content_str = obs.content.as_str().unwrap_or_default().trim();
                if !content_str.is_empty() {
                    let fact = SemanticFact::new(
                        content_str,
                        "architectural_decision",
                        Scope::Session(session_id.to_string()),
                    )
                    .with_importance(importance);
                    candidate_facts.push((fact, emb.clone()));
                }
            }

            points_to_cluster.push(VectorPoint {
                id: format!("ev-{}", event.id),
                embedding: emb,
                payload: text,
            });
        }

        // 2. NREM Phase (Poda, Error Deduplication, and JTMS Arbitration)
        let consolidator = Consolidator::new();
        for event in &events {
            match &event.payload {
                EventPayload::ErrorObserved(err) => {
                    let _ = consolidator.record_tool_failure(
                        &self.store,
                        &err.error_type,
                        &err.message,
                        "",
                        Some(&Scope::Session(session_id.to_string())),
                    );
                }
                EventPayload::ToolResultReceived(res) if res.is_error => {
                    let _ = consolidator.record_tool_failure(
                        &self.store,
                        &res.tool_name,
                        &format!("{:?}", res.result),
                        "",
                        Some(&Scope::Session(session_id.to_string())),
                    );
                }
                _ => {}
            }
        }

        for (mut fact, emb) in candidate_facts {
            let conflicts = self.jtms.resolve_and_upsert(&self.store, &mut fact, &emb)?;
            result.conflicts_resolved += conflicts.len();
            self.store.update_semantic_fact_embedding(&fact.id, &emb)?;
            result.semantic_facts.push(fact);
        }

        let prune_rep = self.decay.prune_expired(&self.store, None, None)?;
        result.memories_pruned =
            prune_rep.memories_pruned + prune_rep.facts_pruned + prune_rep.skills_pruned;

        // 3. REM Phase (Trajectory Folding & Spatial Clustering with Medoids)
        let recovery_skills = self.trajectory_miner.mine_recovery_trajectories(&events);
        let task_skills = self.trajectory_miner.mine_successful_task_patterns(&events);

        for skill in recovery_skills.into_iter().chain(task_skills.into_iter()) {
            let skill_text = format!("{} {}", skill.name, skill.description);
            if let Ok(skill_emb) = self.embedder.embed_text(&skill_text).await {
                self.store.insert_or_update_procedural_skill(&skill)?;
                let _ = self
                    .store
                    .update_procedural_skill_embedding(&skill.id, &skill_emb);
            } else {
                self.store.insert_or_update_procedural_skill(&skill)?;
            }
            result.procedural_skills.push(skill);
        }

        if !points_to_cluster.is_empty() {
            let clusters = self.clusterer.cluster(points_to_cluster);
            for cluster in clusters {
                let medoid_point = &cluster.points[cluster.medoid_index];
                let raw_summary = &medoid_point.payload;
                let title = format!("Session {} Cluster {}", session_id, cluster.cluster_id);
                let enriched_summary = self
                    .enricher
                    .enrich_summary(
                        &title,
                        raw_summary,
                        &serde_json::json!({
                            "cluster_size": cluster.points.len(),
                            "medoid_id": cluster.medoid_id,
                        }),
                    )
                    .await;

                let time_start = events.first().map(|e| e.timestamp).unwrap_or_else(Utc::now);
                let time_end = events.last().map(|e| e.timestamp).unwrap_or_else(Utc::now);
                let actor = events
                    .first()
                    .map(|e| e.agent_id.clone())
                    .unwrap_or_else(|| "agent".to_string());

                let ep = EpisodicMemory::new(
                    session_id,
                    &actor,
                    &enriched_summary,
                    time_start,
                    time_end,
                )
                .with_signals(SignalScores {
                    success: 1.0,
                    frustration: 0.0,
                    novelty: 0.70,
                    importance: 0.75,
                });
                self.store.insert_episodic_memory(&ep)?;
                result.episodic_memories.push(ep);
            }
        }

        Ok(result)
    }

    pub async fn consolidate_all(&mut self) -> Result<ConsolidationResult, StrataError> {
        let session_ids = self.store.get_session_ids()?;
        let mut total = ConsolidationResult::default();

        for sid in session_ids {
            let res = self.consolidate_session(&sid).await?;
            total.episodic_memories.extend(res.episodic_memories);
            total.semantic_facts.extend(res.semantic_facts);
            total.procedural_skills.extend(res.procedural_skills);
            total.conflicts_resolved += res.conflicts_resolved;
            total.memories_pruned += res.memories_pruned;
            total.events_processed += res.events_processed;
        }

        Ok(total)
    }
}

// ============================================================================
// KAIROS-Style Background Auto-Dreaming Engine (< 5MB RAM, Zero-LLM)
// ============================================================================

/// Parse a human-friendly duration string (e.g., "500ms", "30s", "5m", "2h", "1d", or plain seconds "60").
pub fn parse_dream_duration(raw: &str) -> Result<std::time::Duration, StrataError> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(StrataError::Validation(
            "Duration string cannot be empty".to_string(),
        ));
    }

    let (num_str, multiplier_ms) = if let Some(v) = trimmed.strip_suffix("ms") {
        (v.trim(), 1.0_f64)
    } else if let Some(v) = trimmed.strip_suffix("secs") {
        (v.trim(), 1_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix("sec") {
        (v.trim(), 1_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix('s') {
        (v.trim(), 1_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix("mins") {
        (v.trim(), 60_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix("min") {
        (v.trim(), 60_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix('m') {
        (v.trim(), 60_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix("hrs") {
        (v.trim(), 3_600_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix("hr") {
        (v.trim(), 3_600_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix('h') {
        (v.trim(), 3_600_000.0_f64)
    } else if let Some(v) = trimmed.strip_suffix('d') {
        (v.trim(), 86_400_000.0_f64)
    } else {
        (trimmed.as_str(), 1_000.0_f64)
    };

    let value: f64 = num_str.parse().map_err(|_| {
        StrataError::Validation(format!("Invalid duration value in '{raw}'"))
    })?;

    if !value.is_finite() || value < 0.0 {
        return Err(StrataError::Validation(format!(
            "Duration must be non-negative and finite: '{raw}'"
        )));
    }

    let total_ms = (value * multiplier_ms).round() as u64;
    Ok(std::time::Duration::from_millis(total_ms))
}

/// Configuration for the KAIROS-style background Auto-Dreaming engine.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DreamConfig {
    /// Inactivity duration required before triggering an idle dream cycle (default 60s).
    pub idle_timeout: std::time::Duration,
    /// Number of new/unconsolidated sessions that triggers a dream cycle (default 3).
    pub session_threshold: usize,
    /// Lexical/token Jaccard similarity threshold (0.0 to 1.0) to fuse duplicate memories (default 0.75).
    pub merge_similarity_threshold: f32,
    /// ACT-R activation threshold below which decayed memories are pruned during dreaming (default 0.05).
    pub prune_threshold: f32,
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            idle_timeout: std::time::Duration::from_secs(60),
            session_threshold: 3,
            merge_similarity_threshold: 0.75,
            prune_threshold: 0.05,
        }
    }
}

impl DreamConfig {
    pub fn new(idle_timeout: std::time::Duration, session_threshold: usize) -> Self {
        Self {
            idle_timeout,
            session_threshold,
            ..Default::default()
        }
    }

    pub fn with_idle_timeout(mut self, idle_timeout: std::time::Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    pub fn with_session_threshold(mut self, session_threshold: usize) -> Self {
        self.session_threshold = session_threshold;
        self
    }

    pub fn with_merge_similarity_threshold(mut self, threshold: f32) -> Self {
        self.merge_similarity_threshold = threshold.clamp(0.1, 1.0);
        self
    }

    pub fn with_prune_threshold(mut self, threshold: f32) -> Self {
        self.prune_threshold = threshold.clamp(0.0, 1.0);
        self
    }
}

/// Reason why a background dream cycle was triggered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DreamTriggerReason {
    /// Triggered because the workspace remained idle for at least `idle_timeout`.
    IdleTimeout { idle_ms: u64 },
    /// Triggered because at least `threshold` new sessions were observed since the last dream cycle.
    SessionThreshold {
        new_sessions: usize,
        threshold: usize,
    },
    /// Explicitly invoked dream cycle.
    Manual,
}

impl Default for DreamTriggerReason {
    fn default() -> Self {
        Self::Manual
    }
}

/// Telemetry report emitted after a background Auto-Dreaming cycle completes.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DreamReport {
    pub trigger: DreamTriggerReason,
    pub memories_merged: usize,
    pub contradictions_resolved: usize,
    pub episodes_consolidated: usize,
    pub rollups_created: usize,
    pub memories_pruned: usize,
    pub sessions_processed: usize,
}

impl DreamReport {
    pub fn has_changes(&self) -> bool {
        self.memories_merged > 0
            || self.contradictions_resolved > 0
            || self.episodes_consolidated > 0
            || self.rollups_created > 0
            || self.memories_pruned > 0
    }
}

/// Lightweight (< 5MB RAM), zero-LLM KAIROS-style Auto-Dreaming engine.
/// Runs asynchronously in the background to consolidate facts, merge duplicates,
/// resolve contradictions via JTMS, and prune decayed memories without blocking active agent sessions.
pub struct DreamEngine {
    store: Arc<SqliteStore>,
    config: DreamConfig,
    jtms: TruthMaintenanceSystem,
    dreamed_sessions: std::collections::HashSet<String>,
    explicit_sessions: std::collections::HashSet<String>,
    last_dream_fingerprint: Option<u64>,
    last_activity_override: Option<chrono::DateTime<Utc>>,
}

impl DreamEngine {
    pub fn new(store: Arc<SqliteStore>, config: DreamConfig) -> Self {
        Self {
            store,
            config,
            jtms: TruthMaintenanceSystem::with_default_threshold(),
            dreamed_sessions: std::collections::HashSet::new(),
            explicit_sessions: std::collections::HashSet::new(),
            last_dream_fingerprint: None,
            last_activity_override: None,
        }
    }

    pub fn with_default_config(store: Arc<SqliteStore>) -> Self {
        Self::new(store, DreamConfig::default())
    }

    pub fn config(&self) -> &DreamConfig {
        &self.config
    }

    /// Record an explicit session identifier observed by the daemon.
    pub fn record_session(&mut self, session_id: impl Into<String>) {
        let sid = session_id.into();
        if !sid.trim().is_empty() {
            self.explicit_sessions.insert(sid);
        }
    }

    /// Notify the dream engine of agent activity at the given timestamp.
    pub fn notify_activity(&mut self, timestamp: chrono::DateTime<Utc>) {
        self.last_activity_override = Some(match self.last_activity_override {
            Some(prev) if prev > timestamp => prev,
            _ => timestamp,
        });
    }

    /// Collect all distinct session IDs across events, episodes, memories, and explicit notifications.
    pub fn collect_active_sessions(
        &self,
    ) -> Result<std::collections::HashSet<String>, StrataError> {
        let mut sessions = self.explicit_sessions.clone();

        for sid in self.store.get_session_ids()? {
            if !sid.is_empty() {
                sessions.insert(sid);
            }
        }

        for ep in self.store.list_episodes_by_tier(1, None, 5_000)? {
            if !ep.session_id.is_empty() {
                sessions.insert(ep.session_id);
            }
        }

        for mem in self.store.get_all_memories(None, None, 5_000)? {
            if let Scope::Session(sid) = mem.scope {
                if !sid.is_empty() {
                    sessions.insert(sid);
                }
            }
        }

        for fact in self.store.get_all_semantic_facts(None, None, 5_000)? {
            if let Scope::Session(sid) = fact.scope {
                if !sid.is_empty() {
                    sessions.insert(sid);
                }
            }
        }

        Ok(sessions)
    }

    /// Determine the most recent activity timestamp in the store or explicit override.
    pub fn latest_activity_timestamp(&self) -> Result<Option<chrono::DateTime<Utc>>, StrataError> {
        let mut latest = self.last_activity_override;
        let mut update_max = |ts: chrono::DateTime<Utc>| {
            latest = Some(match latest {
                Some(cur) if cur >= ts => cur,
                _ => ts,
            });
        };

        for mem in self.store.get_all_memories(None, None, 5_000)? {
            update_max(mem.created_at);
            update_max(mem.updated_at);
        }

        for fact in self.store.get_all_semantic_facts(None, None, 5_000)? {
            update_max(fact.created_at);
            update_max(fact.last_updated_at);
        }

        for ep in self.store.list_episodes_by_tier(1, None, 5_000)? {
            if let Some(ts) = chrono::DateTime::from_timestamp(ep.created_at, 0) {
                update_max(ts);
            }
        }

        for ev in self.store.get_all_events()? {
            update_max(ev.timestamp);
        }

        Ok(latest)
    }

    fn compute_store_fingerprint(&self) -> Result<u64, StrataError> {
        let memories = self.store.get_all_memories(None, None, 5_000)?;
        let facts = self.store.get_all_semantic_facts(
            None,
            Some(strata_core::schemas::FactStatus::Active),
            5_000,
        )?;
        let episodes = self.store.list_episodes_by_tier(1, None, 5_000)?;
        let events = self.store.get_all_events()?;

        if memories.is_empty()
            && facts.is_empty()
            && episodes.is_empty()
            && events.is_empty()
            && self.explicit_sessions.is_empty()
        {
            return Ok(0);
        }

        let mut hasher = DefaultHasher::new();
        for m in &memories {
            if !is_memory_deprecated(m) {
                m.id.hash(&mut hasher);
                m.updated_at.timestamp_millis().hash(&mut hasher);
            }
        }
        for f in &facts {
            f.id.hash(&mut hasher);
            f.version.hash(&mut hasher);
        }
        for ep in &episodes {
            if !ep.consolidated {
                ep.id.hash(&mut hasher);
            }
        }
        events.len().hash(&mut hasher);
        self.explicit_sessions.len().hash(&mut hasher);

        let hash = hasher.finish();
        Ok(if hash == 0 { 1 } else { hash })
    }

    /// Evaluate whether a background dream cycle should fire at `now`.
    /// Checks both the session count threshold (`every N sessions`) and inactivity timeout (`idle_timeout`).
    pub fn should_dream(
        &self,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<DreamTriggerReason>, StrataError> {
        let active_sessions = self.collect_active_sessions()?;
        let pending_sessions = active_sessions.difference(&self.dreamed_sessions).count();

        if self.config.session_threshold > 0 && pending_sessions >= self.config.session_threshold {
            return Ok(Some(DreamTriggerReason::SessionThreshold {
                new_sessions: pending_sessions,
                threshold: self.config.session_threshold,
            }));
        }

        let fingerprint = self.compute_store_fingerprint()?;
        if fingerprint != 0 && self.last_dream_fingerprint != Some(fingerprint) {
            if let Some(latest_ts) = self.latest_activity_timestamp()? {
                let elapsed_ms = (now - latest_ts).num_milliseconds().max(0) as u64;
                let timeout_ms = self.config.idle_timeout.as_millis() as u64;
                if elapsed_ms >= timeout_ms {
                    return Ok(Some(DreamTriggerReason::IdleTimeout {
                        idle_ms: elapsed_ms,
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Evaluate trigger conditions at `now` and run the dream cycle if needed.
    pub fn dream_if_needed(
        &mut self,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<DreamReport>, StrataError> {
        if let Some(trigger) = self.should_dream(now)? {
            let report = self.execute_dream_cycle(trigger, now)?;
            Ok(Some(report))
        } else {
            Ok(None)
        }
    }

    /// Unconditionally run a full KAIROS-style dream cycle now.
    pub fn run_dream_cycle(&mut self) -> Result<DreamReport, StrataError> {
        self.execute_dream_cycle(DreamTriggerReason::Manual, Utc::now())
    }

    fn execute_dream_cycle(
        &mut self,
        trigger: DreamTriggerReason,
        now: chrono::DateTime<Utc>,
    ) -> Result<DreamReport, StrataError> {
        let mut report = DreamReport {
            trigger,
            ..Default::default()
        };

        let active_sessions = self.collect_active_sessions()?;
        let new_sessions_count = active_sessions.difference(&self.dreamed_sessions).count();
        report.sessions_processed = new_sessions_count;

        // 1. Episodic Pyramid Rollup & Hippocampal Episode Consolidation
        let pyramid = crate::pyramid::EpisodicPyramidStore::new(Arc::clone(&self.store));
        if let Ok(rollups) = pyramid.process_idle_episodes(None) {
            report.rollups_created = rollups.len();
        }
        let hippo = self.store.consolidate_episodes(None)?;
        report.episodes_consolidated = hippo.episodes_processed;
        report.contradictions_resolved += hippo.contradictions_resolved;

        // 2. MemoryRecord Fusion (Deduplication) & Contradiction Resolution
        let (mem_merged, mem_contradictions) = self.consolidate_memory_records()?;
        report.memories_merged += mem_merged;
        report.contradictions_resolved += mem_contradictions;

        // 3. SemanticFact Fusion (Deduplication) & Contradiction Resolution
        let (fact_merged, fact_contradictions) = self.consolidate_semantic_facts()?;
        report.memories_merged += fact_merged;
        report.contradictions_resolved += fact_contradictions;

        // 4. ACT-R Decay Pruning (only active non-deprecated records whose activation decayed below threshold)
        if self.config.prune_threshold > 0.0 {
            report.memories_pruned = self.prune_decayed_active_memories(now)?;
        }

        // Update session and fingerprint watermarks
        for sid in active_sessions {
            self.dreamed_sessions.insert(sid);
        }
        self.last_dream_fingerprint = Some(self.compute_store_fingerprint()?);

        Ok(report)
    }

    fn consolidate_memory_records(&self) -> Result<(usize, usize), StrataError> {
        let mut records = self.store.get_all_memories(None, None, 10_000)?;
        records.retain(|r| !is_memory_deprecated(r));
        records.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));

        let mut merged_count = 0;
        let mut contradictions_count = 0;
        let mut retired_ids = std::collections::HashSet::new();

        for i in 0..records.len() {
            if retired_ids.contains(&records[i].id) {
                continue;
            }

            for j in (i + 1)..records.len() {
                if retired_ids.contains(&records[i].id) || retired_ids.contains(&records[j].id) {
                    continue;
                }

                if !scopes_compatible(&records[i].scope, &records[j].scope) {
                    continue;
                }

                let text_a = &records[i].content;
                let text_b = &records[j].content;

                let (has_lex_contradiction, mut cues) =
                    self.jtms.detect_lexical_contradiction(text_a, text_b);
                if is_numeric_template_update(text_a, text_b) {
                    cues.push("Numeric value update on identical statement template".to_string());
                }

                let is_contradiction = has_lex_contradiction || !cues.is_empty();
                let shares_topic =
                    has_dream_topic_overlap(text_a, &records[i].tags, text_b, &records[j].tags);

                // Case 1: Duplicate / Near-Duplicate Fusion
                if !is_contradiction && records[i].memory_type == records[j].memory_type {
                    let sim = lexical_jaccard_similarity(text_a, text_b);
                    let emb_sim = match (&records[i].embedding, &records[j].embedding) {
                        (Some(ea), Some(eb)) => crate::embedding::cosine_similarity(ea, eb),
                        _ => 0.0,
                    };
                    let norm_equal =
                        normalize_dream_statement(text_a) == normalize_dream_statement(text_b);

                    if norm_equal
                        || sim >= self.config.merge_similarity_threshold
                        || emb_sim >= 0.92
                    {
                        let (survivor_idx, dup_idx) =
                            if records[j].approved_by_human && !records[i].approved_by_human {
                                (j, i)
                            } else {
                                (i, j)
                            };

                        let dup = records[dup_idx].clone();
                        let survivor = &mut records[survivor_idx];

                        survivor.access_count =
                            survivor.access_count.max(1) + dup.access_count.max(1);
                        survivor.importance = survivor.importance.max(dup.importance);
                        survivor.confidence =
                            (survivor.confidence.max(dup.confidence) + 0.05).min(1.0);
                        survivor.approved_by_human =
                            survivor.approved_by_human || dup.approved_by_human;
                        for tag in dup.tags {
                            if !survivor.tags.contains(&tag) {
                                survivor.tags.push(tag);
                            }
                        }
                        for ev in dup.evidence_ids {
                            if !survivor.evidence_ids.contains(&ev) {
                                survivor.evidence_ids.push(ev);
                            }
                        }
                        let mut meta = survivor.metadata.clone();
                        if !meta.is_object() {
                            meta = serde_json::json!({});
                        }
                        let mut merged_list: Vec<String> = meta
                            .get("merged_ids")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|x| x.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        merged_list.push(dup.id.to_string());
                        meta["merged_ids"] = serde_json::json!(merged_list);
                        survivor.metadata = meta;
                        survivor.updated_at = Utc::now();

                        self.store.insert_or_update_memory(survivor)?;
                        self.store.delete_memory(&dup.id)?;
                        retired_ids.insert(dup.id);
                        merged_count += 1;
                        continue;
                    }
                }

                // Case 2: Contradiction Resolution (Belief Revision)
                if shares_topic && is_contradiction {
                    let (winner_idx, loser_idx) =
                        if records[i].approved_by_human && !records[j].approved_by_human {
                            (i, j)
                        } else {
                            (j, i)
                        };

                    let winner_id = records[winner_idx].id;
                    let loser_id = records[loser_idx].id;

                    // Deprecate losing memory record
                    {
                        let loser = &mut records[loser_idx];
                        let mut meta = loser.metadata.clone();
                        if !meta.is_object() {
                            meta = serde_json::json!({});
                        }
                        meta["status"] = serde_json::json!("Deprecated");
                        meta["replaced_by"] = serde_json::json!(winner_id.to_string());
                        loser.metadata = meta;
                        loser.updated_at = Utc::now();
                        self.store.insert_or_update_memory(loser)?;
                    }

                    // Mark winning memory record as superseding the old one
                    {
                        let winner = &mut records[winner_idx];
                        let mut meta = winner.metadata.clone();
                        if !meta.is_object() {
                            meta = serde_json::json!({});
                        }
                        let prev_ver = meta.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
                        meta["version"] = serde_json::json!(prev_ver + 1);
                        meta["supersedes"] = serde_json::json!(loser_id.to_string());
                        winner.metadata = meta;
                        winner.updated_at = Utc::now();
                        self.store.insert_or_update_memory(winner)?;
                    }

                    retired_ids.insert(loser_id);
                    contradictions_count += 1;
                }
            }
        }

        Ok((merged_count, contradictions_count))
    }

    fn consolidate_semantic_facts(&self) -> Result<(usize, usize), StrataError> {
        let mut facts = self.store.get_all_semantic_facts(
            None,
            Some(strata_core::schemas::FactStatus::Active),
            10_000,
        )?;
        facts.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));

        let mut merged_count = 0;
        let mut contradictions_count = 0;
        let mut retired_ids = std::collections::HashSet::new();

        for i in 0..facts.len() {
            if retired_ids.contains(&facts[i].id) {
                continue;
            }

            for j in (i + 1)..facts.len() {
                if retired_ids.contains(&facts[i].id) || retired_ids.contains(&facts[j].id) {
                    continue;
                }

                if !scopes_compatible(&facts[i].scope, &facts[j].scope) {
                    continue;
                }

                let stmt_a = &facts[i].statement;
                let stmt_b = &facts[j].statement;

                let (has_lex_contradiction, mut cues) =
                    self.jtms.detect_lexical_contradiction(stmt_a, stmt_b);
                if is_numeric_template_update(stmt_a, stmt_b) {
                    cues.push("Numeric value update on identical statement template".to_string());
                }

                let is_contradiction = has_lex_contradiction || !cues.is_empty();
                let shares_topic =
                    has_dream_topic_overlap(stmt_a, &facts[i].tags, stmt_b, &facts[j].tags);

                // Case 1: Duplicate / Near-Duplicate Fusion
                if !is_contradiction {
                    let sim = lexical_jaccard_similarity(stmt_a, stmt_b);
                    let norm_equal =
                        normalize_dream_statement(stmt_a) == normalize_dream_statement(stmt_b);

                    if norm_equal || sim >= self.config.merge_similarity_threshold {
                        let (survivor_idx, dup_idx) =
                            if facts[j].approved_by_human && !facts[i].approved_by_human {
                                (j, i)
                            } else {
                                (i, j)
                            };

                        let dup_id = facts[dup_idx].id;
                        let survivor_id = facts[survivor_idx].id;
                        let dup_clone = facts[dup_idx].clone();

                        {
                            let survivor = &mut facts[survivor_idx];
                            survivor.importance = survivor.importance.max(dup_clone.importance);
                            survivor.confidence =
                                (survivor.confidence.max(dup_clone.confidence) + 0.05).min(1.0);
                            survivor.approved_by_human =
                                survivor.approved_by_human || dup_clone.approved_by_human;
                            for tag in dup_clone.tags {
                                if !survivor.tags.contains(&tag) {
                                    survivor.tags.push(tag);
                                }
                            }
                            for ev in dup_clone.evidence {
                                if !survivor
                                    .evidence
                                    .iter()
                                    .any(|e| e.source_id == ev.source_id)
                                {
                                    survivor.evidence.push(ev);
                                }
                            }
                            survivor.last_updated_at = Utc::now();
                            self.store.insert_or_update_semantic_fact(survivor)?;
                        }

                        {
                            let dup = &mut facts[dup_idx];
                            dup.deprecate_and_replace(survivor_id);
                            self.store.insert_or_update_semantic_fact(dup)?;
                        }

                        retired_ids.insert(dup_id);
                        merged_count += 1;
                        continue;
                    }
                }

                // Case 2: Contradiction Resolution via JTMS Arbitration
                if shares_topic && is_contradiction {
                    let (resolution, _reason) =
                        self.jtms.arbitrate_pair(&facts[i], &facts[j], &cues);
                    match resolution {
                        crate::jtms::ConflictResolution::Supersede => {
                            let new_id = facts[j].id;
                            let old_id = facts[i].id;
                            let old_ver = facts[i].version;

                            facts[i].deprecate_and_replace(new_id);
                            self.store.insert_or_update_semantic_fact(&facts[i])?;

                            facts[j].version = facts[j].version.max(old_ver) + 1;
                            facts[j].last_updated_at = Utc::now();
                            self.store.insert_or_update_semantic_fact(&facts[j])?;

                            retired_ids.insert(old_id);
                            contradictions_count += 1;
                        }
                        crate::jtms::ConflictResolution::Reject => {
                            let old_id = facts[i].id;
                            let new_id = facts[j].id;

                            facts[j].deprecate_and_replace(old_id);
                            self.store.insert_or_update_semantic_fact(&facts[j])?;

                            retired_ids.insert(new_id);
                            contradictions_count += 1;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok((merged_count, contradictions_count))
    }

    fn prune_decayed_active_memories(
        &self,
        eval_time: chrono::DateTime<Utc>,
    ) -> Result<usize, StrataError> {
        let all_memories = self.store.get_all_memories(None, None, 10_000)?;
        let mut pruned = 0;

        for memory in all_memories {
            if is_memory_deprecated(&memory) || memory.approved_by_human {
                continue;
            }
            let act = calculate_act_r_activation(&memory, eval_time);
            if act < self.config.prune_threshold {
                self.store.delete_memory(&memory.id)?;
                pruned += 1;
            }
        }

        Ok(pruned)
    }
}

fn is_memory_deprecated(record: &MemoryRecord) -> bool {
    record
        .metadata
        .get("status")
        .and_then(|v| v.as_str())
        .map_or(false, |s| {
            s.eq_ignore_ascii_case("deprecated") || s.eq_ignore_ascii_case("archived")
        })
}

fn scopes_compatible(a: &Scope, b: &Scope) -> bool {
    a == b || a.is_compatible(b) || b.is_compatible(a)
}

fn normalize_dream_statement(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokenize_meaningful_words(text: &str) -> std::collections::HashSet<String> {
    const STOPWORDS: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "to", "for", "in", "of", "on", "with", "and",
        "or", "by", "at", "from", "as", "be", "this", "that", "it", "we", "our", "use", "uses",
        "using", "used", "now", "new", "old",
    ];
    normalize_dream_statement(text)
        .split_whitespace()
        .filter(|w| w.len() >= 2 && !STOPWORDS.contains(w))
        .map(String::from)
        .collect()
}

fn lexical_jaccard_similarity(a: &str, b: &str) -> f32 {
    let set_a = tokenize_meaningful_words(a);
    let set_b = tokenize_meaningful_words(b);
    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

fn has_dream_topic_overlap(
    text_a: &str,
    tags_a: &[String],
    text_b: &str,
    tags_b: &[String],
) -> bool {
    if !tags_a.is_empty() && tags_a.iter().any(|t| tags_b.contains(t)) {
        return true;
    }
    let words_a = tokenize_meaningful_words(text_a);
    let words_b = tokenize_meaningful_words(text_b);
    let shared_count = words_a.intersection(&words_b).count();
    shared_count >= 1
}

fn is_numeric_template_update(text_a: &str, text_b: &str) -> bool {
    let norm_a = normalize_dream_statement(text_a);
    let norm_b = normalize_dream_statement(text_b);
    if norm_a == norm_b {
        return false;
    }
    let has_digit_a = norm_a.chars().any(|c| c.is_ascii_digit());
    let has_digit_b = norm_b.chars().any(|c| c.is_ascii_digit());
    if !has_digit_a || !has_digit_b {
        return false;
    }
    let tpl_a = normalize_error_message(&norm_a);
    let tpl_b = normalize_error_message(&norm_b);
    tpl_a == tpl_b && tpl_a.len() >= 5
}

#[cfg(test)]
mod dream_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_parse_dream_duration_formats() {
        assert_eq!(
            parse_dream_duration("500ms").unwrap(),
            Duration::from_millis(500)
        );
        assert_eq!(parse_dream_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_dream_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(
            parse_dream_duration("2h").unwrap(),
            Duration::from_secs(7200)
        );
        assert_eq!(parse_dream_duration("45").unwrap(), Duration::from_secs(45));
        assert!(parse_dream_duration("").is_err());
        assert!(parse_dream_duration("invalid").is_err());
    }

    #[test]
    fn test_dream_cycle_merges_duplicate_memories_and_facts() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());

        // Insert duplicate MemoryRecords in the same scope
        let mut m1 = MemoryRecord::new(
            MemoryType::Semantic,
            "Authentication service uses Ed25519 signatures for agent tokens",
            Scope::Project("strata".to_string()),
        )
        .with_importance(0.7)
        .with_confidence(0.8)
        .with_tags(vec!["auth".to_string()]);
        m1.access_count = 2;

        let mut m2 = MemoryRecord::new(
            MemoryType::Semantic,
            "Authentication service uses Ed25519 signatures for agent tokens!",
            Scope::Project("strata".to_string()),
        )
        .with_importance(0.9)
        .with_confidence(0.85)
        .with_tags(vec!["security".to_string()]);
        m2.access_count = 3;

        store.insert_or_update_memory(&m1).unwrap();
        store.insert_or_update_memory(&m2).unwrap();

        // Insert duplicate SemanticFacts
        let f1 = SemanticFact::new(
            "IPC transport uses bounded Named Pipe frames",
            "architecture",
            Scope::Project("strata".to_string()),
        )
        .with_confidence(0.8);
        let f2 = SemanticFact::new(
            "IPC transport uses bounded Named Pipe frames",
            "architecture",
            Scope::Project("strata".to_string()),
        )
        .with_confidence(0.9);
        store.insert_or_update_semantic_fact(&f1).unwrap();
        store.insert_or_update_semantic_fact(&f2).unwrap();

        let mut dream = DreamEngine::with_default_config(store.clone());
        let report = dream.run_dream_cycle().unwrap();

        assert_eq!(report.memories_merged, 2);

        // Verify MemoryRecord fusion
        let remaining_memories = store.get_all_memories(None, None, 10).unwrap();
        assert_eq!(remaining_memories.len(), 1);
        let merged = &remaining_memories[0];
        assert_eq!(merged.access_count, 5);
        assert!((merged.importance - 0.9).abs() < 1e-5);
        assert!(merged.confidence >= 0.89);
        assert!(merged.tags.contains(&"auth".to_string()));
        assert!(merged.tags.contains(&"security".to_string()));

        // Verify SemanticFact fusion
        let active_facts = store
            .get_all_semantic_facts(None, Some(strata_core::schemas::FactStatus::Active), 10)
            .unwrap();
        assert_eq!(active_facts.len(), 1);
        assert!(active_facts[0].confidence >= 0.94);
    }

    #[test]
    fn test_dream_cycle_resolves_contradictions_in_memories_and_facts() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let now = Utc::now();

        // 1. Contradicting MemoryRecords (SQLite -> PostgreSQL migration)
        let mut old_mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Primary database uses SQLite for storage",
            Scope::Project("backend".to_string()),
        );
        old_mem.created_at = now - chrono::Duration::minutes(10);
        old_mem.updated_at = old_mem.created_at;

        let mut new_mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Primary database migrated to PostgreSQL instead of SQLite",
            Scope::Project("backend".to_string()),
        );
        new_mem.created_at = now - chrono::Duration::minutes(1);
        new_mem.updated_at = new_mem.created_at;

        store.insert_or_update_memory(&old_mem).unwrap();
        store.insert_or_update_memory(&new_mem).unwrap();

        // 2. Contradicting SemanticFacts (REST -> gRPC)
        let mut old_fact = SemanticFact::new(
            "Internal service transport uses REST over JSON",
            "architecture",
            Scope::Project("backend".to_string()),
        );
        old_fact.created_at = now - chrono::Duration::minutes(10);

        let mut new_fact = SemanticFact::new(
            "Internal service transport switched to gRPC with Protobuf",
            "architecture",
            Scope::Project("backend".to_string()),
        );
        new_fact.created_at = now - chrono::Duration::minutes(1);

        store.insert_or_update_semantic_fact(&old_fact).unwrap();
        store.insert_or_update_semantic_fact(&new_fact).unwrap();

        let mut dream = DreamEngine::with_default_config(store.clone());
        let report = dream.run_dream_cycle().unwrap();

        assert_eq!(report.contradictions_resolved, 2);

        // Old MemoryRecord must be marked Deprecated and replaced_by new_mem
        let old_mem_loaded = store.get_memory(&old_mem.id).unwrap().unwrap();
        assert_eq!(
            old_mem_loaded.metadata.get("status").and_then(|v| v.as_str()),
            Some("Deprecated")
        );
        assert_eq!(
            old_mem_loaded
                .metadata
                .get("replaced_by")
                .and_then(|v| v.as_str()),
            Some(new_mem.id.to_string().as_str())
        );

        let new_mem_loaded = store.get_memory(&new_mem.id).unwrap().unwrap();
        assert_eq!(
            new_mem_loaded
                .metadata
                .get("supersedes")
                .and_then(|v| v.as_str()),
            Some(old_mem.id.to_string().as_str())
        );

        // Old SemanticFact must be Deprecated and new SemanticFact Active with version 2
        let old_fact_loaded = store.get_semantic_fact(&old_fact.id).unwrap().unwrap();
        assert_eq!(
            old_fact_loaded.status,
            strata_core::schemas::FactStatus::Deprecated
        );
        assert_eq!(old_fact_loaded.replaced_by, Some(new_fact.id));

        let new_fact_loaded = store.get_semantic_fact(&new_fact.id).unwrap().unwrap();
        assert_eq!(
            new_fact_loaded.status,
            strata_core::schemas::FactStatus::Active
        );
        assert_eq!(new_fact_loaded.version, 2);
    }

    #[test]
    fn test_dream_triggers_on_idle_timeout_and_every_n_sessions() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let config = DreamConfig::new(Duration::from_secs(30), 3);
        let mut dream = DreamEngine::new(store.clone(), config);

        let t0 = Utc::now();

        // Add memory in session 1
        let mut m1 = MemoryRecord::new(
            MemoryType::Semantic,
            "Worker pool size is 4",
            Scope::Session("sess-1".to_string()),
        );
        m1.created_at = t0;
        m1.updated_at = t0;
        store.insert_or_update_memory(&m1).unwrap();

        // Before idle timeout (10s < 30s) and only 1 session (< 3): should not dream
        let early = dream.dream_if_needed(t0 + chrono::Duration::seconds(10)).unwrap();
        assert!(early.is_none());

        // After idle timeout (35s >= 30s): triggers IdleTimeout dream!
        let idle_report = dream
            .dream_if_needed(t0 + chrono::Duration::seconds(35))
            .unwrap()
            .expect("Should trigger dream after idle timeout");
        assert!(matches!(
            idle_report.trigger,
            DreamTriggerReason::IdleTimeout { .. }
        ));

        // Subsequent check without new activity should be a no-op
        assert!(dream
            .dream_if_needed(t0 + chrono::Duration::seconds(40))
            .unwrap()
            .is_none());

        // Now add 3 new sessions ("sess-2", "sess-3", "sess-4") and check BEFORE idle timeout
        let t1 = t0 + chrono::Duration::seconds(50);
        for sid in ["sess-2", "sess-3", "sess-4"] {
            let mut m = MemoryRecord::new(
                MemoryType::Episodic,
                format!("Completed work in {sid}"),
                Scope::Session(sid.to_string()),
            );
            m.created_at = t1;
            m.updated_at = t1;
            store.insert_or_update_memory(&m).unwrap();
        }

        // Even at t1 + 1s (idle < 30s), 3 new sessions reached threshold 3 -> triggers SessionThreshold!
        let sess_report = dream
            .dream_if_needed(t1 + chrono::Duration::seconds(1))
            .unwrap()
            .expect("Should trigger dream after N=3 new sessions");
        assert_eq!(
            sess_report.trigger,
            DreamTriggerReason::SessionThreshold {
                new_sessions: 3,
                threshold: 3
            }
        );
    }
}

