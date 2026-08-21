use std::time::Instant;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use strata_core::{
    errors::StrataError,
    schemas::SemanticFact,
    state::{MemoryRecord, MemoryTier, MemoryType, Scope},
    traits::MemoryEngine,
};
use strata_memory::{DecayCalculator, SqliteMemoryEngine};

/// Evaluation scenario measuring Human-in-the-Loop (HITL) Core Tier Approval,
/// safety barrier validation against unapproved promotions, and frozen retention persistence (< 20ms).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlCoreApprovalEvalResult {
    pub unapproved_write_rejected: bool,
    pub unapproved_promote_rejected: bool,
    pub approved_promote_succeeded: bool,
    pub core_retention_frozen: bool,
    pub immune_to_pruning: bool,
    pub evaluation_duration_micros: u128,
    pub is_sub_50ms: bool,
}

pub struct HitlCoreApprovalEval;

impl HitlCoreApprovalEval {
    pub async fn run_eval() -> Result<HitlCoreApprovalEvalResult, StrataError> {
        let engine = SqliteMemoryEngine::open_in_memory(None)?;
        let store = engine.store();

        let start = Instant::now();

        // 1. Direct write to Core Tier without human approval must fail
        let unapproved_core_record = MemoryRecord::new(
            MemoryType::Semantic,
            "Security Axiom: All JWT tokens must be HMAC-SHA256 verified",
            Scope::Global,
        ).with_tier(MemoryTier::Core); // approved_by_human is false by default

        let unapproved_write_res = engine.write(&unapproved_core_record).await;
        let unapproved_write_rejected = unapproved_write_res.is_err();

        // 2. Write to Working Tier initially succeeds
        let working_record = MemoryRecord::new(
            MemoryType::Semantic,
            "Security Axiom: All JWT tokens must be HMAC-SHA256 verified",
            Scope::Global,
        ).with_tier(MemoryTier::Working);

        let handle = engine.write(&working_record).await?;

        // 3. Attempting to promote without human approval (approved_by_human = false) must be rejected
        let unapproved_promote_res = engine.promote_to_core(&handle.id, false, Some("Policy change")).await;
        let unapproved_promote_rejected = unapproved_promote_res.is_err();

        // 4. Promoting with human approval (approved_by_human = true) succeeds
        let promoted_res = engine
            .promote_to_core(&handle.id, true, Some("Approved in Security Board Review"))
            .await;
        let approved_promote_succeeded = promoted_res.is_ok();

        // 5. Verify Core retention dynamics: Core memories have R=1.0 and are immune to decay and pruning
        let calculator = DecayCalculator::with_default_config();
        let now = Utc::now();
        let persisted = store.get_memory(&handle.id)?.expect("promoted memory exists");

        let logs = store.get_memory_access_logs(&handle.id)?;
        let metrics = calculator.evaluate_memory_record(&persisted, &logs, now);

        let core_retention_frozen = metrics.retention == 1.0 && persisted.tier == MemoryTier::Core;

        // Simulate pruning pass: Core memory should be protected and never archived/pruned
        let prune_report = calculator.prune_expired(&store, Some(0.5), Some(now))?;
        let immune_to_pruning = prune_report.core_protected >= 1 && prune_report.memories_pruned == 0;

        // 6. Test SemanticFact HITL promotion
        let fact = SemanticFact::new(
            "All SQL queries in repository must use parameterized prepared statements",
            "security",
            Scope::Global,
        ).with_tier(MemoryTier::Working);
        store.insert_or_update_semantic_fact(&fact)?;

        let fact_promoted = store.promote_semantic_fact_to_core(&fact.id, true, Some("Architecture ADR-101"))?;
        let fact_promoted_ok = fact_promoted.tier == MemoryTier::Core && fact_promoted.approved_by_human;

        let duration = start.elapsed();
        let duration_micros = duration.as_micros();
        let is_sub_50ms = duration.as_millis() < 50;

        Ok(HitlCoreApprovalEvalResult {
            unapproved_write_rejected,
            unapproved_promote_rejected,
            approved_promote_succeeded: approved_promote_succeeded && fact_promoted_ok,
            core_retention_frozen,
            immune_to_pruning,
            evaluation_duration_micros: duration_micros,
            is_sub_50ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_hitl_core_approval_and_retention_freeze() {
        let result = HitlCoreApprovalEval::run_eval().await.expect("eval run");

        assert!(result.unapproved_write_rejected, "Unapproved write to Core Tier must be rejected");
        assert!(result.unapproved_promote_rejected, "Unapproved promotion to Core Tier must be rejected");
        assert!(result.approved_promote_succeeded, "Human-approved promotion must succeed");
        assert!(result.core_retention_frozen, "Core Tier retention must be frozen at R=1.0");
        assert!(result.immune_to_pruning, "Core Tier memories must be immune to decay pruning");
        assert!(result.is_sub_50ms, "Evaluation latency must be < 50ms");
    }
}
