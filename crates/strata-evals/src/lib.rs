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
    run_cognitive_observability_scenario().await?;
    run_world_model_causal_scenario().await?;
    run_hierarchical_planning_dag_scenario().await?;
    run_lora_finetuning_pipeline_scenario().await?;
    NativeCallGraphEval::run_eval().await?;
    WorkspaceMonorepoEval::run_eval().await?;
    ArchitectureClusteringEval::run_eval().await?;
    HitlCoreApprovalEval::run_eval().await?;
    run_a2a_stigmergic_leases_scenario().await?;
    run_a2a_swarm_stress_benchmark_scenario().await?;
    run_a2a_transactive_directory_scenario().await?;
    run_pluggable_reasoning_cascade_scenario().await?;
    run_consolidation_ab_benchmark_scenario().await?;
    run_agent_loop_futility_baseline_scenario().await?;
    run_hierarchical_episodic_pyramid_eval().await?;
    run_community_scoped_recall_scenario().await?;
    run_context_rot_scenario().await?;
    run_long_mem_eval_scenario().await?;

    println!("\n========================================================");
    println!("🎉 ALL EVAL SCENARIOS PASSED (26/26)");
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

    #[tokio::test]
    async fn test_eval_cognitive_observability() {
        run_cognitive_observability_scenario()
            .await
            .expect("Cognitive observability scenario failed");
    }

    #[tokio::test]
    async fn test_eval_world_model_causal() {
        run_world_model_causal_scenario()
            .await
            .expect("World model causal scenario failed");
    }

    #[tokio::test]
    async fn test_eval_hierarchical_planning_dag() {
        run_hierarchical_planning_dag_scenario()
            .await
            .expect("Hierarchical planning DAG scenario failed");
    }

    #[tokio::test]
    async fn test_eval_lora_finetuning_pipeline() {
        run_lora_finetuning_pipeline_scenario()
            .await
            .expect("LoRA fine-tuning pipeline scenario failed");
    }

    #[tokio::test]
    async fn test_eval_native_call_graph() {
        let res = NativeCallGraphEval::run_eval()
            .await
            .expect("Native call graph eval failed");
        assert!(res.accuracy_passed);
        assert!(res.is_latency_sub_5ms);
    }

    #[tokio::test]
    async fn test_eval_workspace_monorepo() {
        let res = WorkspaceMonorepoEval::run_eval()
            .await
            .expect("Workspace monorepo eval failed");
        assert!(res.isolation_passed);
        assert!(res.is_sub_50ms);
    }

    #[tokio::test]
    async fn test_eval_architecture_clustering() {
        let res = ArchitectureClusteringEval::run_eval()
            .await
            .expect("Architecture clustering eval failed");
        assert!(res.accuracy_passed);
        assert!(res.cache_roundtrip_passed);
        assert!(res.is_latency_sub_20ms);
    }

    #[tokio::test]
    async fn test_eval_hitl_core_approval() {
        let res = HitlCoreApprovalEval::run_eval()
            .await
            .expect("HITL Core Approval eval failed");
        assert!(res.unapproved_write_rejected);
        assert!(res.unapproved_promote_rejected);
        assert!(res.approved_promote_succeeded);
        assert!(res.core_retention_frozen);
        assert!(res.immune_to_pruning);
        assert!(res.is_sub_50ms);
    }

    #[tokio::test]
    async fn test_eval_a2a_stigmergic_leases() {
        run_a2a_stigmergic_leases_scenario()
            .await
            .expect("A2A stigmergic leases scenario failed");
    }

    #[tokio::test]
    async fn test_eval_a2a_swarm_stress_benchmark() {
        run_a2a_swarm_stress_benchmark_scenario()
            .await
            .expect("A2A swarm stress benchmark scenario failed");
    }

    #[tokio::test]
    async fn test_eval_pluggable_reasoning_cascade() {
        run_pluggable_reasoning_cascade_scenario()
            .await
            .expect("Pluggable reasoning cascade scenario failed");
    }

    #[tokio::test]
    async fn test_eval_consolidation_ab_benchmark() {
        run_consolidation_ab_benchmark_scenario()
            .await
            .expect("Consolidation A/B benchmark scenario failed");
    }

    #[tokio::test]
    async fn test_eval_agent_loop_futility_baseline() {
        run_agent_loop_futility_baseline_scenario()
            .await
            .expect("Agent loop futility baseline scenario failed");
    }

    #[tokio::test]
    async fn test_eval_hierarchical_episodic_pyramid() {
        let res = run_hierarchical_episodic_pyramid_eval()
            .await
            .expect("Hierarchical episodic pyramid eval failed");
        assert!(res.compression_ratio >= 0.70);
        assert_eq!(res.drill_down_accuracy, 1.0);
        assert!(res.is_deterministic);
        assert_eq!(res.clusters_count, 3);
    }

    #[tokio::test]
    async fn test_eval_a2a_transactive_directory() {
        run_a2a_transactive_directory_scenario()
            .await
            .expect("A2A Transactive Directory eval scenario failed");
    }

    #[tokio::test]
    async fn test_eval_community_scoped_recall() {
        run_community_scoped_recall_scenario()
            .await
            .expect("Community scoped recall scenario failed");
    }

    #[tokio::test]
    async fn test_eval_context_rot_detection() {
        let report = run_context_rot_scenario()
            .await
            .expect("Context rot scenario failed");
        assert!(report.poisoned_memory_detected);
        assert!(report.suppression_verified);
        assert!(report.rejection_rate > 0.60);
    }

    #[tokio::test]
    async fn test_eval_long_mem_eval_coding_agents() {
        let report = LongMemEvalSuite::run_suite("coding-agents")
            .await
            .expect("LongMemEval failed");
        assert!(report.metrics_10_cycles.recall_accuracy >= 0.85);
        assert!(report.metrics_100_cycles.recall_accuracy >= 0.80);
        assert!(report.metrics_1000_cycles.recall_accuracy >= 0.75);
        assert!(report.metrics_10_cycles.mrr >= 0.80);
        assert!(report.metrics_10_cycles.recall_at_1 >= 0.80);
        assert!(report.metrics_10_cycles.recall_at_3 >= 0.85);
        assert!(report.contradiction_resolution_passed);
        assert!(report.cross_agent_transfer_passed);
        assert!(report.multi_session_continuity_passed);
        assert!(report.rule_invalidation_passed);
        assert!(report.baseline_comparison_passed);
        assert_eq!(report.baselines.len(), 4);
        assert!(!report.format_ascii_report().is_empty());
    }

    #[tokio::test]
    async fn test_eval_multi_session_continuity() {
        let passed = evaluate_multi_session_continuity()
            .await
            .expect("Multi-session continuity eval failed");
        assert!(passed);
    }

    #[tokio::test]
    async fn test_eval_rule_invalidation_clean_context() {
        let passed = evaluate_rule_invalidation_clean_context()
            .await
            .expect("Rule invalidation clean context eval failed");
        assert!(passed);
    }

    #[tokio::test]
    async fn test_eval_four_baselines_comparative() {
        let rules = get_target_code_rules();
        let baselines = evaluate_four_baselines_comparative(&rules)
            .await
            .expect("4-baseline comparative eval failed");
        assert_eq!(baselines.len(), 4);
        assert_eq!(baselines[0].name, "1. Zero-shot (No Memory)");
        assert_eq!(baselines[1].name, "2. Static AGENTS.md Stuffing");
        assert_eq!(baselines[2].name, "3. Lexical Search (FTS5)");
        assert_eq!(baselines[3].name, "4. Strata Full Engine");
        assert!(baselines[3].mrr >= 0.80);
        assert!(baselines[3].recall_at_3 >= 0.85);
        assert_eq!(baselines[3].stale_leakage_pct, 0.0);
        assert!(baselines[1].stale_leakage_pct > 0.0);
    }

    #[tokio::test]
    async fn test_eval_long_mem_eval_unknown_suite() {
        let err = LongMemEvalSuite::run_suite("unknown-suite").await;
        assert!(err.is_err());
    }
}
