use chrono::{DateTime, Utc};
use strata_core::errors::StrataError;
use strata_core::schemas::{DecayConfig, DecayMetrics, FactStatus, SemanticFact};
use strata_core::state::{MemoryRecord, MemoryTier};

use crate::store::SqliteStore;

/// Calculator implementing ACT-R power-law activation and Ebbinghaus forgetting curve models,
/// with full support for the Tri-Tier cognitive memory model (Core, Working, Peripheral).
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

    /// Evaluate decay metrics according to the formal Tri-Tier memory model:
    /// - Core Tier: Frozen decay (R=1.0, activation=10.0, never expired)
    /// - Working Tier: Active session context (high retention R=0.95, never expired during session)
    /// - Peripheral Tier: Exponential Ebbinghaus decay eligible for Cold Storage archiving
    pub fn evaluate_decay_with_tier(
        &self,
        tier: MemoryTier,
        importance: f32,
        confidence: f32,
        created_at: DateTime<Utc>,
        access_times: &[DateTime<Utc>],
        current_time: DateTime<Utc>,
    ) -> DecayMetrics {
        // 1. Core Tier is completely frozen: Permanent retention 1.0, immune to time decay
        if tier == MemoryTier::Core || importance >= self.config.invariant_threshold {
            let stability = self.calculate_stability(access_times.len() as u32, importance);
            return DecayMetrics {
                activation: 10.0,
                retention: 1.0,
                stability,
                is_expired: false,
            };
        }

        // 2. Working Tier maintains high operational retention throughout the active session
        if tier == MemoryTier::Working {
            let stability = self.calculate_stability(access_times.len() as u32, importance.max(0.7));
            return DecayMetrics {
                activation: 5.0,
                retention: 0.95,
                stability,
                is_expired: false,
            };
        }

        // 3. Peripheral Tier decays exponentially according to ACT-R & Ebbinghaus curves
        self.evaluate_decay(importance, confidence, created_at, access_times, current_time)
    }

    /// Raw decay evaluation for peripheral memories.
    pub fn evaluate_decay(
        &self,
        importance: f32,
        confidence: f32,
        created_at: DateTime<Utc>,
        access_times: &[DateTime<Utc>],
        current_time: DateTime<Utc>,
    ) -> DecayMetrics {
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
        self.evaluate_decay_with_tier(
            record.tier,
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
        self.evaluate_decay_with_tier(
            fact.tier,
            fact.importance,
            fact.confidence,
            fact.created_at,
            access_times,
            current_time,
        )
    }

    /// Prunes expired peripheral memories below the threshold, moving them to cold storage.
    /// Core Tier memories are protected and never pruned.
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
            // Core Tier is immune to pruning
            if fact.tier == MemoryTier::Core || fact.importance >= self.config.invariant_threshold {
                report.core_protected += 1;
                continue;
            }

            if fact.tier == MemoryTier::Working {
                report.working_active += 1;
                continue;
            }

            let logs = store.get_memory_access_logs(&fact.id)?;
            let metrics = self.evaluate_semantic_fact(&fact, &logs, now);

            if metrics.retention < prune_thresh {
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
            // Core Tier is immune to pruning
            if mem.tier == MemoryTier::Core || mem.importance >= self.config.invariant_threshold {
                report.core_protected += 1;
                continue;
            }

            if mem.tier == MemoryTier::Working {
                report.working_active += 1;
                continue;
            }

            let logs = store.get_memory_access_logs(&mem.id)?;
            let metrics = self.evaluate_memory_record(&mem, &logs, now);

            if metrics.retention < prune_thresh {
                // Move expired peripheral memory to cold storage
                let _ = store.archive_to_cold_storage(&mem.id)?;
                report.memories_archived += 1;
                report.memories_pruned += 1;
            }
        }

        Ok(report)
    }
}

/// Report detailing memory decay and cold storage archiving results.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PruneReport {
    pub memories_pruned: usize,
    pub memories_archived: usize,
    pub facts_pruned: usize,
    pub skills_pruned: usize,
    pub core_protected: usize,
    pub working_active: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use strata_core::state::{MemoryRecord, MemoryTier, MemoryType, Scope};

    #[test]
    fn test_tri_tier_decay_behavior() {
        let calc = DecayCalculator::with_default_config();
        let now = Utc::now();
        let old_time = now - Duration::days(365);

        // 1. Core Tier: Frozen decay (R=1.0, activation=10.0, never expired)
        let core_metrics = calc.evaluate_decay_with_tier(
            MemoryTier::Core,
            0.5,
            1.0,
            old_time,
            &[],
            now,
        );
        assert_eq!(core_metrics.retention, 1.0);
        assert_eq!(core_metrics.activation, 10.0);
        assert!(!core_metrics.is_expired);

        // 2. Working Tier: Active session retention (R=0.95, activation=5.0, never expired)
        let working_metrics = calc.evaluate_decay_with_tier(
            MemoryTier::Working,
            0.5,
            1.0,
            old_time,
            &[],
            now,
        );
        assert_eq!(working_metrics.retention, 0.95);
        assert_eq!(working_metrics.activation, 5.0);
        assert!(!working_metrics.is_expired);

        // 3. Peripheral Tier: Exponential decay (after 365 days, retention ~0, expired)
        let peripheral_metrics = calc.evaluate_decay_with_tier(
            MemoryTier::Peripheral,
            0.5,
            1.0,
            old_time,
            &[],
            now,
        );
        assert!(peripheral_metrics.retention < 0.05);
        assert!(peripheral_metrics.is_expired);
    }

    #[test]
    fn test_cold_storage_archiving_and_restoration() {
        let store = SqliteStore::open_in_memory().unwrap();

        let now = Utc::now();
        let old_time = now - Duration::hours(500);

        // Create 1 Core, 1 Working, and 1 Peripheral memory
        let mut core_mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Security rule: Never hardcode JWT secrets",
            Scope::Global,
        );
        core_mem.tier = MemoryTier::Core;
        core_mem.created_at = old_time;
        store.insert_or_update_memory(&core_mem).unwrap();

        let mut working_mem = MemoryRecord::new(
            MemoryType::Episodic,
            "Currently refactoring auth middleware in auth.rs",
            Scope::Session("sess-1".to_string()),
        );
        working_mem.tier = MemoryTier::Working;
        working_mem.created_at = old_time;
        store.insert_or_update_memory(&working_mem).unwrap();

        let mut peripheral_mem = MemoryRecord::new(
            MemoryType::NegativePattern,
            "Temporary observation about slow cargo test run yesterday",
            Scope::Global,
        );
        peripheral_mem.tier = MemoryTier::Peripheral;
        peripheral_mem.created_at = old_time;
        store.insert_or_update_memory(&peripheral_mem).unwrap();

        // Perform prune
        let calc = DecayCalculator::with_default_config();
        let report = calc.prune_expired(&store, Some(0.1), Some(now)).unwrap();

        // Assert Core and Working are protected, Peripheral is archived
        assert_eq!(report.core_protected, 1);
        assert_eq!(report.working_active, 1);
        assert_eq!(report.memories_archived, 1);
        assert_eq!(report.memories_pruned, 1);

        // Active memories should now have exactly 2 records (Core and Working)
        let active = store.get_all_memories(None, None, 10).unwrap();
        assert_eq!(active.len(), 2);
        assert!(active.iter().any(|m| m.id == core_mem.id && m.is_core()));
        assert!(active.iter().any(|m| m.id == working_mem.id && m.is_working()));
        assert!(!active.iter().any(|m| m.id == peripheral_mem.id));

        // Cold storage should contain the peripheral memory
        assert_eq!(store.get_cold_storage_count().unwrap(), 1);
        let cold = store.get_cold_storage_memories(None, None, 10).unwrap();
        assert_eq!(cold.len(), 1);
        assert_eq!(cold[0].id, peripheral_mem.id);

        // Deep search in cold storage via FTS
        let cold_searched = store
            .get_cold_storage_memories(Some("observation"), None, 10)
            .unwrap();
        assert_eq!(cold_searched.len(), 1);
        assert_eq!(cold_searched[0].id, peripheral_mem.id);

        // Restore peripheral memory back into active memory as Working tier
        let restored = store
            .restore_from_cold_storage(&peripheral_mem.id, Some(MemoryTier::Working))
            .unwrap();
        assert!(restored);

        // Cold storage is now empty
        assert_eq!(store.get_cold_storage_count().unwrap(), 0);

        // Active memories now has 3 records, and restored memory has Working tier
        let active_after = store.get_all_memories(None, None, 10).unwrap();
        assert_eq!(active_after.len(), 3);
        let restored_record = active_after
            .iter()
            .find(|m| m.id == peripheral_mem.id)
            .unwrap();
        assert_eq!(restored_record.tier, MemoryTier::Working);
    }
}
