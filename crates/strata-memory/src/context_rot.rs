use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::state::MemoryRecord;

use crate::store::SqliteStore;

/// Outcome of a memory injection into an agent execution context or command generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionOutcome {
    Success,
    Failure,
    Rejected,
    Error,
}

impl InjectionOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Rejected => "rejected",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for InjectionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for InjectionOutcome {
    type Err = StrataError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "success" => Ok(Self::Success),
            "failure" => Ok(Self::Failure),
            "rejected" => Ok(Self::Rejected),
            "error" => Ok(Self::Error),
            other => Err(StrataError::Validation(format!(
                "Invalid InjectionOutcome '{}'",
                other
            ))),
        }
    }
}

/// Record tracking a single memory injection into an agent context and its observed execution outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryInjectionRecord {
    pub id: Uuid,
    pub memory_id: Uuid,
    pub session_id: String,
    pub task_id: Option<String>,
    pub command: Option<String>,
    pub outcome: InjectionOutcome,
    pub created_at: DateTime<Utc>,
}

impl MemoryInjectionRecord {
    pub fn new(memory_id: Uuid, session_id: impl Into<String>, outcome: InjectionOutcome) -> Self {
        Self {
            id: Uuid::new_v4(),
            memory_id,
            session_id: session_id.into(),
            task_id: None,
            command: None,
            outcome,
            created_at: Utc::now(),
        }
    }

    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }
}

/// Aggregated efficacy metrics for a memory over its injection history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEfficacyMetric {
    pub memory_id: Uuid,
    pub total_injections: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub rejection_rate: f32,
    pub efficacy_score: f32,
    pub is_poisoned: bool,
    pub penalty_factor: f32,
}

impl MemoryEfficacyMetric {
    pub fn compute(
        memory_id: Uuid,
        outcomes: &[MemoryInjectionRecord],
        config: &ContextRotConfig,
    ) -> Self {
        let total_injections = outcomes.len();
        let mut success_count = 0;
        let mut failure_count = 0;

        for rec in outcomes {
            match rec.outcome {
                InjectionOutcome::Success => success_count += 1,
                InjectionOutcome::Failure
                | InjectionOutcome::Rejected
                | InjectionOutcome::Error => failure_count += 1,
            }
        }

        let rejection_rate = if total_injections > 0 {
            failure_count as f32 / total_injections as f32
        } else {
            0.0
        };

        let efficacy_score = if total_injections > 0 {
            success_count as f32 / total_injections as f32
        } else {
            0.0
        };

        let is_poisoned =
            total_injections >= config.min_samples && rejection_rate > config.rejection_threshold;

        let penalty_factor = if is_poisoned {
            config.penalty_multiplier
        } else {
            1.0
        };

        Self {
            memory_id,
            total_injections,
            success_count,
            failure_count,
            rejection_rate,
            efficacy_score,
            is_poisoned,
            penalty_factor,
        }
    }
}

/// Configuration thresholds for the Context Rot & Context Poisoning Runtime Detector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextRotConfig {
    /// Rejection/failure threshold above which a memory is classified as poisoned (default: 0.60).
    pub rejection_threshold: f32,
    /// Minimum injection samples required before classifying as poisoned (default: 3).
    pub min_samples: usize,
    /// Multiplier applied to decay importance and confidence when poisoned (default: 0.1).
    pub penalty_multiplier: f32,
}

impl Default for ContextRotConfig {
    fn default() -> Self {
        Self {
            rejection_threshold: 0.60,
            min_samples: 3,
            penalty_multiplier: 0.1,
        }
    }
}

/// Runtime detector that monitors injection outcomes of memories and suppresses context poisoning.
#[derive(Debug, Clone, Default)]
pub struct ContextRotDetector {
    pub config: ContextRotConfig,
}

impl ContextRotDetector {
    pub fn new(config: ContextRotConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(ContextRotConfig::default())
    }

    pub fn record_injection_outcome(
        &self,
        store: &SqliteStore,
        record: &MemoryInjectionRecord,
    ) -> Result<(), StrataError> {
        store.record_injection_outcome(record)
    }

    pub fn evaluate_memory_efficacy(
        &self,
        store: &SqliteStore,
        memory_id: &Uuid,
    ) -> Result<MemoryEfficacyMetric, StrataError> {
        let outcomes = store.get_injection_outcomes_for_memory(memory_id)?;
        Ok(MemoryEfficacyMetric::compute(
            *memory_id,
            &outcomes,
            &self.config,
        ))
    }

    pub fn detect_poisoned_memories(
        &self,
        store: &SqliteStore,
    ) -> Result<Vec<MemoryEfficacyMetric>, StrataError> {
        let all_metrics = store.get_all_memory_injection_metrics(
            self.config.min_samples,
            self.config.rejection_threshold,
        )?;
        Ok(all_metrics.into_iter().filter(|m| m.is_poisoned).collect())
    }

    pub fn apply_penalty_decay(&self, store: &SqliteStore) -> Result<usize, StrataError> {
        let poisoned = self.detect_poisoned_memories(store)?;
        let mut decayed_count = 0;

        for metric in poisoned {
            if let Some(mut memory) = store.get_memory(&metric.memory_id)? {
                memory.importance =
                    (memory.importance * self.config.penalty_multiplier).clamp(0.01, 1.0);
                memory.confidence =
                    (memory.confidence * self.config.penalty_multiplier).clamp(0.01, 1.0);

                let reason = format!(
                    "Context rot: rejection rate {:.2} ({}/{} failures) exceeds threshold {:.2}",
                    metric.rejection_rate,
                    metric.failure_count,
                    metric.total_injections,
                    self.config.rejection_threshold
                );

                if let Some(obj) = memory.metadata.as_object_mut() {
                    obj.insert("quarantined".to_string(), serde_json::Value::Bool(true));
                    obj.insert("suppressed".to_string(), serde_json::Value::Bool(true));
                    obj.insert(
                        "quarantine_reason".to_string(),
                        serde_json::Value::String(reason.clone()),
                    );
                    obj.insert(
                        "quarantined_at".to_string(),
                        serde_json::Value::String(Utc::now().to_rfc3339()),
                    );
                } else {
                    let mut map = serde_json::Map::new();
                    map.insert("quarantined".to_string(), serde_json::Value::Bool(true));
                    map.insert("suppressed".to_string(), serde_json::Value::Bool(true));
                    map.insert(
                        "quarantine_reason".to_string(),
                        serde_json::Value::String(reason.clone()),
                    );
                    map.insert(
                        "quarantined_at".to_string(),
                        serde_json::Value::String(Utc::now().to_rfc3339()),
                    );
                    memory.metadata = serde_json::Value::Object(map);
                }

                store.insert_or_update_memory(&memory)?;
                let _ = store.quarantine_memory(&metric.memory_id, &reason);
                decayed_count += 1;
            }
        }

        Ok(decayed_count)
    }

    pub fn filter_suppressed_memories(
        &self,
        store: &SqliteStore,
        records: &mut Vec<MemoryRecord>,
    ) -> Result<(), StrataError> {
        let mut filtered = Vec::with_capacity(records.len());
        for rec in records.drain(..) {
            let is_meta_suppressed = rec
                .metadata
                .get("quarantined")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || rec
                    .metadata
                    .get("suppressed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                || rec
                    .metadata
                    .get("status")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| {
                        s.eq_ignore_ascii_case("quarantined")
                            || s.eq_ignore_ascii_case("suppressed")
                    });

            if is_meta_suppressed {
                continue;
            }

            if store.is_memory_quarantined(&rec.id)? {
                continue;
            }

            filtered.push(rec);
        }
        *records = filtered;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_injection_outcome_serde_and_str() {
        assert_eq!(InjectionOutcome::Success.as_str(), "success");
        assert_eq!(InjectionOutcome::Failure.as_str(), "failure");
        assert_eq!(InjectionOutcome::Rejected.as_str(), "rejected");
        assert_eq!(InjectionOutcome::Error.as_str(), "error");

        assert_eq!(
            "success".parse::<InjectionOutcome>().unwrap(),
            InjectionOutcome::Success
        );
        assert_eq!(
            "FAILURE".parse::<InjectionOutcome>().unwrap(),
            InjectionOutcome::Failure
        );
        assert_eq!(
            "rejected".parse::<InjectionOutcome>().unwrap(),
            InjectionOutcome::Rejected
        );
        assert_eq!(
            "error".parse::<InjectionOutcome>().unwrap(),
            InjectionOutcome::Error
        );
    }

    #[test]
    fn test_efficacy_metric_computation() {
        let mem_id = Uuid::new_v4();
        let config = ContextRotConfig::default(); // min_samples: 3, threshold: 0.60

        // 1 sample: not poisoned even if failed
        let outcomes_1 = vec![MemoryInjectionRecord::new(
            mem_id,
            "s1",
            InjectionOutcome::Failure,
        )];
        let m1 = MemoryEfficacyMetric::compute(mem_id, &outcomes_1, &config);
        assert_eq!(m1.total_injections, 1);
        assert_eq!(m1.failure_count, 1);
        assert_eq!(m1.success_count, 0);
        assert!(
            !m1.is_poisoned,
            "Insufficient samples for poisoning verdict"
        );

        // 5 samples: 4 failures, 1 success -> 80% failure rate > 60%
        let mut outcomes_5 = Vec::new();
        for _ in 0..4 {
            outcomes_5.push(MemoryInjectionRecord::new(
                mem_id,
                "s1",
                InjectionOutcome::Failure,
            ));
        }
        outcomes_5.push(MemoryInjectionRecord::new(
            mem_id,
            "s1",
            InjectionOutcome::Success,
        ));

        let m5 = MemoryEfficacyMetric::compute(mem_id, &outcomes_5, &config);
        assert_eq!(m5.total_injections, 5);
        assert_eq!(m5.failure_count, 4);
        assert_eq!(m5.success_count, 1);
        assert_eq!(m5.rejection_rate, 0.80);
        assert_eq!(m5.efficacy_score, 0.20);
        assert!(m5.is_poisoned);
        assert_eq!(m5.penalty_factor, 0.1);
    }
}
