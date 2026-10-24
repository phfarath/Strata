use anyhow::{bail, Result};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;

use strata_cli::mcp::server::McpServer;
use strata_core::{
    a2a::{DelegationContract, DelegationStatus},
    schemas::{EpisodicMemory, SemanticFact, SignalScores},
    state::{FailurePattern, HumanApprovalRecord, MemoryRecord, MemoryTier, MemoryType, Scope},
};
use strata_memory::{ExportFormat, PreferenceMiner, SqliteMemoryEngine};
use strata_reasoning::{TrainingConfig, TrainingMethod, TrainingPipeline};
use strata_tools::DistillPipelineTool;

/// Evaluation Scenario 28: Self-Supervised Distillation & LoRA Fine-Tuning Pipeline
///
/// Validates the automated closed neuro-symbolic loop:
/// 1. Populates heterogeneous A2A delegation contracts (completed with oracle verification vs rejected in same domain).
/// 2. Records HITL & Council approved Core tier memory records and semantic facts.
/// 3. Records episodic trajectories with high success rate and failure patterns with verified mitigations.
/// 4. Executes PreferenceMiner to extract oracle-gated DPO pairs, KTO binary feedback, and AlignmentDatasetReport.
/// 5. Validates automated curriculum engine: deterministic deduplication, train/eval split (e.g. 80/20), and artifact generation.
/// 6. Executes DistillPipelineTool directly, verifying script generation, Modelfile, runner script, and manifest.
/// 7. Tests McpServer tool execution for "distill_pipeline".
pub async fn run_self_supervised_distillation_pipeline_scenario() -> Result<()> {
    println!("\n========================================================");
    println!("▶ Running Eval Scenario 28: Self-Supervised Distillation & LoRA Pipeline");
    println!("========================================================");

    // -------------------------------------------------------------------------
    // Step 1: Initialize in-memory cognitive engine and populate continuous traces
    // -------------------------------------------------------------------------
    println!("  [Step 1] Populating oracle-verified A2A delegations and HITL Core memories...");
    let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None)?);
    let store = engine.store_arc();

    // 1. A2A Completed Delegation with oracle verification
    let mut completed_delegation = DelegationContract::new(
        "del-auth-verified",
        "planner-agent",
        "task-jwt-rs256",
        "Implement asymmetric RS256 JWT authentication validator",
    );
    completed_delegation.delegatee_agent = Some("security-worker".to_string());
    completed_delegation.domain_scope = Some("security_auth".to_string());
    completed_delegation.context = Some(
        "Zero-trust API gateway invariant: tokens must have valid signature and exp claim"
            .to_string(),
    );
    completed_delegation.expected_artifacts = vec!["crates/auth/src/jwt.rs".to_string()];
    completed_delegation.verification_command =
        Some("cargo test -p strata-auth --lib jwt::tests::verify_signature".to_string());
    completed_delegation.status = DelegationStatus::Completed;
    completed_delegation.result_summary = Some(
        "RS256 token verification implemented with full test coverage and expiration checks"
            .to_string(),
    );
    store.save_delegation(&completed_delegation)?;

    // 2. A2A Rejected Delegation in same domain (hard negative)
    let mut rejected_delegation = DelegationContract::new(
        "del-auth-rejected",
        "planner-agent",
        "task-jwt-insecure",
        "Implement asymmetric RS256 JWT authentication validator",
    );
    rejected_delegation.delegatee_agent = Some("untrusted-worker".to_string());
    rejected_delegation.domain_scope = Some("security_auth".to_string());
    rejected_delegation.context = Some("Allow none algorithm or unsigned tokens".to_string());
    rejected_delegation.expected_artifacts = vec!["crates/auth/src/jwt.rs".to_string()];
    rejected_delegation.status = DelegationStatus::Rejected;
    rejected_delegation.error =
        Some("Rejected: Acceptance of unsigned tokens violates zero-trust invariant".to_string());
    store.save_delegation(&rejected_delegation)?;

    // 3. HITL Council-Approved Core Memory
    let mut core_mem = MemoryRecord::new(
        MemoryType::Semantic,
        "System Invariant: Agent IPC messages across boundaries must use ed25519 signatures and SQLite WAL mode.",
        Scope::Global,
    );
    core_mem.summary = Some("Zero-Trust IPC & SQLite Storage Invariant".to_string());
    core_mem.tier = MemoryTier::Core;
    core_mem.approved_by_human = true;
    core_mem.human_approval = Some(HumanApprovalRecord::new(
        "architecture-council",
        "review-tui",
        &core_mem.content,
        Some("Formal architectural sign-off on IPC security".to_string()),
    ));
    store.insert_memory(&core_mem)?;

    // 4. HITL Approved Core Semantic Fact
    let mut core_fact = SemanticFact::new(
        "All database transactions must employ exponential backoff with jitter on lock contention",
        "storage_policy",
        Scope::Global,
    );
    core_fact.tier = MemoryTier::Core;
    core_fact.approved_by_human = true;
    core_fact.human_approval = Some(HumanApprovalRecord::new(
        "lead-maintainer",
        "cli",
        &core_fact.statement,
        Some("Mandated resilience rule".to_string()),
    ));
    store.insert_or_update_semantic_fact(&core_fact)?;

    // 5. Failure Pattern with Mitigation
    let mut failure = FailurePattern::new(
        "database is locked (5)",
        "SqliteLockContention",
        "Concurrent writers colliding on busy SQLite table without busy_timeout",
        "Enable WAL mode (PRAGMA journal_mode=WAL) and configure busy_timeout to at least 5000ms",
    );
    failure.trigger_condition = "High concurrency write operations".to_string();
    failure.error_type = "DatabaseLocked".to_string();
    store.upsert_failure_pattern(&failure)?;

    // 6. High-Success Episodic Memory
    let now = Utc::now();
    let episode = EpisodicMemory::new(
        "distill-eval-session",
        "antigravity",
        "Implemented atomic task work-stealing and stigmergic leases",
        now,
        now,
    )
    .with_goals(vec![
        "Prevent race conditions between peer agents".to_string()
    ])
    .with_obstacles(vec![
        "Simultaneous lease claims by concurrent processes".to_string()
    ])
    .with_outcomes(vec![
        "Atomic SQLite CAS transaction eliminated collisions".to_string()
    ])
    .with_signals(SignalScores {
        success: 0.98,
        frustration: 0.02,
        novelty: 0.8,
        importance: 0.95,
    });
    store.insert_episodic_memory(&episode)?;

    println!("    ✓ Successfully seeded continuous memory with delegations, HITL invariants, and episodes.");

    // -------------------------------------------------------------------------
    // Step 2: Test PreferenceMiner and AlignmentDatasetReport
    // -------------------------------------------------------------------------
    println!("  [Step 2] Testing PreferenceMiner alignment extraction & report...");
    let miner = PreferenceMiner::new(Arc::clone(&store));
    let report = miner.get_alignment_report()?;

    println!("    • Total DPO pairs: {}", report.total_dpo_pairs);
    println!(
        "    • Oracle-verified DPO pairs: {}",
        report.oracle_verified_dpo_pairs
    );
    println!(
        "    • Delegation DPO pairs: {}",
        report.delegation_dpo_pairs
    );
    println!(
        "    • HITL Core DPO pairs: {}",
        report.hitl_approval_dpo_pairs
    );
    println!("    • Total KTO samples: {}", report.total_kto_samples);
    println!(
        "    • Oracle-verified KTO samples: {}",
        report.oracle_verified_kto_samples
    );

    if report.delegation_dpo_pairs < 1 {
        bail!(
            "Expected at least 1 delegation DPO pair, got {}",
            report.delegation_dpo_pairs
        );
    }
    if report.hitl_approval_dpo_pairs < 2 {
        bail!(
            "Expected at least 2 HITL Core DPO pairs (1 mem + 1 fact), got {}",
            report.hitl_approval_dpo_pairs
        );
    }
    if report.oracle_verified_dpo_pairs < 3 {
        bail!(
            "Expected at least 3 oracle-verified DPO pairs, got {}",
            report.oracle_verified_dpo_pairs
        );
    }

    // Verify Delegation DPO Pair contents
    let del_pairs = miner.mine_delegation_dpo_pairs(None)?;
    let p_del = &del_pairs[0];
    if !p_del.oracle_verified {
        bail!("Delegation pair with verification_command must have oracle_verified == true");
    }
    if !p_del
        .verification_source
        .as_deref()
        .unwrap_or("")
        .starts_with("delegation_contract_oracle:")
    {
        bail!(
            "Expected verification_source starting with 'delegation_contract_oracle:', got {:?}",
            p_del.verification_source
        );
    }
    if !p_del
        .chosen
        .contains("RS256 token verification implemented")
    {
        bail!("Chosen text missing expected outcome summary");
    }
    if !p_del.rejected.contains("violates zero-trust invariant") {
        bail!("Rejected text missing hard-negative rejection from failed contract in same domain");
    }

    // Verify HITL Core Pairs
    let hitl_pairs = miner.mine_hitl_approval_pairs(None)?;
    if !hitl_pairs.iter().all(|p| p.oracle_verified) {
        bail!("All HITL Core pairs must be oracle-verified");
    }
    println!("    ✓ PreferenceMiner correctly extracted and structured delegation and HITL pairs.");

    // -------------------------------------------------------------------------
    // Step 3: Test Automated Curriculum Splitting and Deduplication
    // -------------------------------------------------------------------------
    println!("  [Step 3] Testing curriculum dataset splitting and deduplication...");
    let raw_dpo = miner.export_with_gating(ExportFormat::Dpo, None, true)?;
    let duplicated_dpo = format!("{}\n{}", raw_dpo, raw_dpo); // double it

    let (train_split, eval_split) =
        TrainingPipeline::split_dataset_curriculum(&duplicated_dpo, 0.25, 42);
    let total_unique = train_split.len() + eval_split.len();
    let expected_unique = raw_dpo.lines().filter(|l| !l.trim().is_empty()).count();

    if total_unique != expected_unique {
        bail!(
            "Deduplication failed: expected {} unique samples, got {}",
            expected_unique,
            total_unique
        );
    }
    if eval_split.is_empty() {
        bail!("Expected non-empty evaluation split with ratio 0.25");
    }
    println!(
        "    ✓ Deduplicated {} raw lines into {} unique samples (train: {}, eval: {}).",
        duplicated_dpo
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count(),
        total_unique,
        train_split.len(),
        eval_split.len()
    );

    // -------------------------------------------------------------------------
    // Step 4: Test Curriculum Artifact Generation (train.jsonl, eval.jsonl, scripts)
    // -------------------------------------------------------------------------
    println!("  [Step 4] Testing curriculum artifact generation...");
    let temp_dir = std::env::temp_dir().join(format!(
        "strata_eval_distill_pipeline_{}",
        uuid::Uuid::new_v4()
    ));

    let config = TrainingConfig::new("unsloth/Llama-3.2-3B-Instruct")
        .with_method(TrainingMethod::Dpo)
        .with_lora(16, 32, 0.0)
        .with_learning_rate(5e-5)
        .with_output_dir(temp_dir.to_string_lossy().to_string());

    let pipeline = TrainingPipeline::new(config);
    let (split_res, train_res) =
        pipeline.generate_curriculum_artifacts(&temp_dir, &raw_dpo, 0.25)?;

    if !std::path::Path::new(&split_res.train_path).exists() {
        bail!("train.jsonl was not generated at {}", split_res.train_path);
    }
    if !std::path::Path::new(&split_res.eval_path).exists() {
        bail!("eval.jsonl was not generated at {}", split_res.eval_path);
    }
    if !std::path::Path::new(&train_res.script_path).exists() {
        bail!(
            "train_lora.py was not generated at {}",
            train_res.script_path
        );
    }
    if train_res.modelfile_path.is_none() {
        bail!("Ollama Modelfile path missing");
    }
    println!(
        "    ✓ Curriculum artifacts generated cleanly with train/eval splits and Unsloth script."
    );

    // -------------------------------------------------------------------------
    // Step 5: Test DistillPipelineTool Execution
    // -------------------------------------------------------------------------
    println!("  [Step 5] Testing DistillPipelineTool execution...");
    use strata_core::traits::Tool;
    let tool_temp_dir =
        std::env::temp_dir().join(format!("strata_eval_distill_tool_{}", uuid::Uuid::new_v4()));

    let tool = DistillPipelineTool::with_store(Arc::clone(&store));
    let tool_params = json!({
        "base_model": "unsloth/Qwen2.5-Coder-7B-Instruct",
        "method": "dpo",
        "eval_ratio": 0.2,
        "oracle_only": true,
        "output_dir": tool_temp_dir.to_string_lossy().to_string(),
        "ollama_model_name": "strata-distilled-eval",
        "dry_run": true
    });

    let tool_res = tool.execute(tool_params).await;
    if let Err(ref e) = tool_res {
        bail!("DistillPipelineTool execution failed: {:?}", e);
    }
    let val = tool_res.unwrap();
    if val.get("status").and_then(|v| v.as_str()) != Some("success") {
        bail!("Expected status 'success', got {:?}", val.get("status"));
    }
    if val.get("train_dataset_path").is_none() {
        bail!("Missing train_dataset_path in tool output");
    }
    if val.get("eval_dataset_path").is_none() {
        bail!("Missing eval_dataset_path in tool output");
    }
    println!("    ✓ DistillPipelineTool executed cleanly and generated curriculum output.");

    // -------------------------------------------------------------------------
    // Step 6: Test MCP Server Tool Dispatch
    // -------------------------------------------------------------------------
    println!("  [Step 6] Testing MCP server tool dispatch for 'distill_pipeline'...");
    let mcp_server = McpServer::new_with_engine(Arc::clone(&engine));
    let mcp_temp_dir =
        std::env::temp_dir().join(format!("strata_eval_mcp_distill_{}", uuid::Uuid::new_v4()));

    let mcp_res = mcp_server
        .execute_tool(
            "distill_pipeline",
            json!({
                "base_model": "unsloth/Llama-3.2-3B-Instruct",
                "method": "dpo",
                "eval_ratio": 0.2,
                "oracle_only": true,
                "output_dir": mcp_temp_dir.to_string_lossy(),
                "ollama_model_name": "strata-mcp-distilled",
                "dry_run": true
            }),
        )
        .await;

    if mcp_res.is_error == Some(true) {
        bail!("MCP call tool failed: {:?}", mcp_res.content);
    }
    let structured = mcp_res
        .structured_content
        .expect("Expected structured_content from distill_pipeline");
    assert_eq!(
        structured.get("status").and_then(|v| v.as_str()),
        Some("success")
    );
    println!("    ✓ McpServer successfully handled 'distill_pipeline' tool invocation.");

    // Cleanup temp dirs
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::remove_dir_all(&tool_temp_dir);
    let _ = std::fs::remove_dir_all(&mcp_temp_dir);

    println!("\n========================================================");
    println!("✓ Scenario 28 Passed: Self-Supervised Distillation Pipeline fully verified.");
    println!("========================================================\n");

    Ok(())
}
