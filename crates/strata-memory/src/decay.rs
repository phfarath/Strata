use chrono::{DateTime, Utc};
use strata_core::errors::StrataError;
use strata_core::schemas::{DecayConfig, DecayMetrics, FactStatus, SemanticFact};
use strata_core::state::MemoryRecord;

use crate::store::SqliteStore;

/// Calculator implementing ACT-R power-law activation and Ebbinghaus forgetting curve models.
#[derive(Debug, Clone)]
pub struct DecayCalculator {
    pub config: DecayConfig,
}

impl DecayCalculator {
    pub fn new(config: DecayConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self {
            config: DecayConfig::default(),
        }
    }

    /// Calculate ACT-R Base-Level Activation:
    /// A_m = alpha * ln(sum_{k=1}^n t_k^(-d)) + beta * I_m + gamma * C_m
    ///
    /// where:
    /// - t_k is elapsed time in hours since the k-th access
    /// - d is the power-law decay exponent (default 0.5)
    /// - I_m is the memory importance [0.0, 1.0]
    /// - C_m is the memory confidence [0.0, 1.0]
    pub fn calculate_act_r_activation(
        &self,
        elapsed_times_hours: &[f32],
        importance: f32,
        confidence: f32,
    ) -> f32 {
        let alpha = self.config.alpha;
        let beta = self.config.beta;
        let gamma = self.config.gamma;
        let d = self.config.d;

        let imp = importance.clamp(0.0, 1.0);
        let conf = confidence.clamp(0.0, 1.0);

        let power_sum: f32 = if elapsed_times_hours.is_empty() {
            // If no access history, default to 1 unit elapsed
            1.0
        } else {
            elapsed_times_hours
                .iter()
                .map(|&t| {
                    let safe_t = t.max(0.0001); // Avoid division by zero
                    safe_t.powf(-d)
                })
                .sum()
        };

        let safe_sum = power_sum.max(1e-6);
        let frequency_component = safe_sum.ln();

        alpha * frequency_component + beta * imp + gamma * conf
    }

    /// Calculate memory stability S_m in hours:
    /// S_m = S_0 * (1.0 + lambda * ln(u + 1.0) + mu * I_m)
    ///
    /// where:
    /// - S_0 is base stability in hours (config.s0)
    /// - u is usage / access count
    /// - I_m is memory importance
    pub fn calculate_stability(&self, access_count: u32, importance: f32) -> f32 {
        let s0 = self.config.s0.max(0.001);
        let lambda = self.config.lambda;
        let mu = self.config.mu;
        let imp = importance.clamp(0.0, 1.0);

        let access_factor = (access_count as f32 + 1.0).ln();
        let stability = s0 * (1.0 + lambda * access_factor + mu * imp);
        stability.max(0.001)
    }

    /// Calculate Ebbinghaus Retention probability R_m(t):
    /// R_m(t) = exp(-t / S_m)
    ///
    /// where:
    /// - t is elapsed time in hours since last access / creation
    /// - S_m is current stability in hours
    pub fn calculate_ebbinghaus_retention(&self, elapsed_hours: f32, stability: f32) -> f32 {
        let safe_t = elapsed_hours.max(0.0);
        let safe_s = stability.max(0.001);
        (-safe_t / safe_s).exp().clamp(0.0, 1.0)
    }

    /// Evaluate decay metrics for given memory parameters.
    pub fn evaluate_decay(
        &self,
        importance: f32,
        confidence: f32,
        created_at: DateTime<Utc>,
        access_times: &[DateTime<Utc>],
        current_time: DateTime<Utc>,
    ) -> DecayMetrics {
        // If importance reaches invariant threshold, it never expires and retains 1.0
        if importance >= self.config.invariant_threshold {
            let stability = self.calculate_stability(access_times.len() as u32, importance);
            return DecayMetrics {
                activation: 10.0,
                retention: 1.0,
                stability,
                is_expired: false,
            };
        }

        // Calculate elapsed time in hours for all access events
        let elapsed_hours: Vec<f32> = if access_times.is_empty() {
            let diff = (current_time - created_at).num_seconds().max(0) as f32 / 3600.0;
            vec![diff]
        } else {
            access_times
                .iter()
                .map(|t| (current_time - *t).num_seconds().max(0) as f32 / 3600.0)
                .collect()
        };

        // Time since most recent access (or creation)
        let last_access_elapsed_hours = elapsed_hours
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        let access_count = access_times.len() as u32;
        let stability = self.calculate_stability(access_count, importance);
        let activation = self.calculate_act_r_activation(&elapsed_hours, importance, confidence);
        let retention = self.calculate_ebbinghaus_retention(last_access_elapsed_hours, stability);
        let is_expired = retention < self.config.prune_threshold;

        DecayMetrics {
            activation,
            retention,
            stability,
            is_expired,
        }
    }

    /// Evaluate decay metrics for a `MemoryRecord`.
    pub fn evaluate_memory_record(
        &self,
        record: &MemoryRecord,
        access_times: &[DateTime<Utc>],
        current_time: DateTime<Utc>,
    ) -> DecayMetrics {
        self.evaluate_decay(
            record.importance,
            record.confidence,
            record.created_at,
            access_times,
            current_time,
        )
    }

    /// Evaluate decay metrics for a `SemanticFact`.
    pub fn evaluate_semantic_fact(
        &self,
        fact: &SemanticFact,
        access_times: &[DateTime<Utc>],
        current_time: DateTime<Utc>,
    ) -> DecayMetrics {
        self.evaluate_decay(
            fact.importance,
            fact.confidence,
            fact.created_at,
            access_times,
            current_time,
        )
    }

    /// Prune expired memories from SQLite store whose retention is below the prune threshold.
    /// Returns count of pruned memories.
    pub fn prune_expired(
        &self,
        store: &SqliteStore,
        threshold: Option<f32>,
        current_time: Option<DateTime<Utc>>,
    ) -> Result<PruneReport, StrataError> {
        let now = current_time.unwrap_or_else(Utc::now);
        let prune_thresh = threshold.unwrap_or(self.config.prune_threshold);

        let mut report = PruneReport::default();

        // 1. Check semantic facts
        let facts = store.get_all_semantic_facts(None, Some(FactStatus::Active), 10000)?;
        for fact in facts {
            // If invariant, skip
            if fact.importance >= self.config.invariant_threshold {
                continue;
            }

            let logs = store.get_memory_access_logs(&fact.id)?;
            let metrics = self.evaluate_semantic_fact(&fact, &logs, now);

            if metrics.retention < prune_thresh {
                // Mark fact as deprecated / outlier or delete
                let mut updated = fact.clone();
                updated.status = FactStatus::Deprecated;
                updated.last_updated_at = now;
                store.insert_or_update_semantic_fact(&updated)?;
                report.facts_pruned += 1;
            }
        }

        // 2. Check general memories table
        let memories = store.get_all_memories(None, None, 10000)?;
        for mem in memories {
            if mem.importance >= self.config.invariant_threshold {
                continue;
            }
            let logs = store.get_memory_access_logs(&mem.id)?;
            let metrics = self.evaluate_memory_record(&mem, &logs, now);

            if metrics.retention < prune_thresh {
                store.delete_memory(&mem.id)?;
                report.memories_pruned += 1;
            }
        }

        Ok(report)
    }
}

/// Report detailing pruned memory records.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PruneReport {
    pub memories_pruned: usize,
    pub facts_pruned: usize,
    pub skills_pruned: usize,
}
