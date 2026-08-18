pub mod scenarios;

pub use scenarios::*;

use anyhow::Result;

/// Runs the complete evaluation suite deterministically
pub async fn run_all_scenarios() -> Result<()> {
    println!("\n========================================================");
    println!("🧪 STRATA COGNITIVE RUNTIME — DETERMINISTIC EVAL SUITE");
    println!("========================================================");

    run_cross_host_transfer_scenario().await?;
    run_silent_failure_avoidance_scenario().await?;
    run_decay_curve_simulation_scenario().await?;
    run_jtms_belief_revision_scenario().await?;
    run_procedural_skill_distillation_scenario().await?;
    run_mcp_protocol_multi_version_scenario().await?;
    run_offline_first_cdc_sync_scenario().await?;
    run_cognitive_feedback_and_alignment_scenario().await?;

    println!("\n========================================================");
    println!("🎉 ALL EVAL SCENARIOS PASSED (8/8)");
    println!("========================================================\n");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_cross_host_transfer() {
        run_cross_host_transfer_scenario()
            .await
            .expect("Cross-host transfer scenario failed");
    }

    #[tokio::test]
    async fn test_eval_silent_failure_avoidance() {
        run_silent_failure_avoidance_scenario()
            .await
            .expect("Silent failure avoidance scenario failed");
    }

    #[tokio::test]
    async fn test_eval_decay_curve_simulation() {
        run_decay_curve_simulation_scenario()
            .await
            .expect("Decay curve simulation scenario failed");
    }

    #[tokio::test]
    async fn test_eval_jtms_belief_revision() {
        run_jtms_belief_revision_scenario()
            .await
            .expect("JTMS belief revision scenario failed");
    }

    #[tokio::test]
    async fn test_eval_procedural_skill_distillation() {
        run_procedural_skill_distillation_scenario()
            .await
            .expect("Procedural skill distillation scenario failed");
    }

    #[tokio::test]
    async fn test_eval_mcp_protocol_multi_version() {
        run_mcp_protocol_multi_version_scenario()
            .await
            .expect("MCP multi-version scenario failed");
    }

    #[tokio::test]
    async fn test_eval_offline_first_cdc_sync() {
        run_offline_first_cdc_sync_scenario()
            .await
            .expect("Offline-first CDC sync scenario failed");
    }

    #[tokio::test]
    async fn test_eval_cognitive_feedback_and_alignment() {
        run_cognitive_feedback_and_alignment_scenario()
            .await
            .expect("Cognitive feedback and alignment scenario failed");
    }
}

