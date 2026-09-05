use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use strata_core::errors::StrataError;
use strata_core::events::{Event, EventPayload};
use strata_core::state::{
    DigestOutput, FailurePattern, FailureSeverity, MemoryHandle, MemoryRecord, MemoryType, Scope,
};

use crate::store::SqliteStore;

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

    /// Record a tool failure and consolidate it into the failure_patterns store.
    pub fn record_tool_failure(
        &self,
        store: &SqliteStore,
        tool_name: &str,
        error_msg: &str,
        context: &str,
        scope: Option<&Scope>,
    ) -> Result<FailurePattern, StrataError> {
        // Build normalized signature
        let normalized_error = normalize_error_message(error_msg);
        let mut hasher = DefaultHasher::new();
        tool_name.hash(&mut hasher);
        normalized_error.hash(&mut hasher);
        let sig_hash = hasher.finish();
        let signature = format!("fail:{tool_name}:{sig_hash:016x}");

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

        // 2. Fetch key memory pointers in session & global scope
        let top_memories = store.get_all_memories(Some(&session_scope), None, 5)?;
        let key_pointers: Vec<MemoryHandle> = top_memories
            .into_iter()
            .map(|m| m.to_handle(Some(m.importance)))
            .collect();

        // 3. Fetch top known failure warnings
        let failure_warnings = store.search_failures(None, Some(&session_scope), 3)?;

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
