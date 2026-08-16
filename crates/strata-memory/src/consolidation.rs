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
                let summary = ended
                    .summary
                    .clone()
                    .unwrap_or_else(|| format!("Session {} ended with status {:?}", ended.session_id, ended.final_state));
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
                let content = format!(
                    "Observation from {}: {}",
                    obs.source,
                    obs.content
                );
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

        let mut failure = FailurePattern::new(
            signature.clone(),
            pattern_name,
            description,
            mitigation,
        );
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
                    if obs.observation_type.contains("hypothesis") || obs.observation_type.contains("plan") {
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
    } else if err_lower.contains("permission") || err_lower.contains("denied") || err_lower.contains("unauthorized") {
        format!("Verify authentication credentials and permissions for '{tool_name}'.")
    } else if err_lower.contains("not found") || err_lower.contains("404") || err_lower.contains("no such") {
        format!("Check path/identifier validity before calling '{tool_name}'.")
    } else if err_lower.contains("rate limit") || err_lower.contains("429") {
        format!("Apply rate-limiting delay before invoking '{tool_name}'.")
    } else if err_lower.contains("syntax") || err_lower.contains("parse") || err_lower.contains("json") {
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
    } else if err_lower.contains("permission") || err_lower.contains("denied") || err_lower.contains("timeout") || err_lower.contains("timed out") || err_lower.contains("fail") {
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
