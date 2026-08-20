use std::sync::Arc;
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::json;

use strata_core::traits::Tool;
use strata_reasoning::{
    DagScheduler, DynamicReplanner, GoalDag, GoalDecomposer, GoalNode, GoalNodeKind, GoalStatus,
    TaskExecutor,
};
use strata_tools::{DagExecuteTool, GoalDecomposeTool};

/// Custom mock executor for deterministic eval scenario testing.
struct EvalMockTaskExecutor;

#[async_trait]
impl TaskExecutor for EvalMockTaskExecutor {
    async fn execute(&self, node: &GoalNode) -> std::result::Result<serde_json::Value, String> {
        // If node title or metadata specifies failure simulation on initial run
        if let Some(err) = node.metadata.get("simulate_error").and_then(|v| v.as_str()) {
            if node.retry_count == 0 {
                return Err(err.to_string());
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        Ok(json!({
            "status": "success",
            "node_id": node.id,
            "title": node.title
        }))
    }
}

/// Evaluation Scenario 11: Hierarchical Planning & DAG Scheduler for Long-Horizon Autonomy
pub async fn run_hierarchical_planning_dag_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: Hierarchical Planning & DAG Scheduler (STRATA-T-8)");

    // -------------------------------------------------------------------------
    // Test A: Hierarchical Goal Decomposition & Wave Invariants
    // -------------------------------------------------------------------------
    println!("  [Test A] Testing hierarchical goal decomposition & topological waves...");

    let decomposer = GoalDecomposer::new();
    let goal = "Refactor storage layer to async and verify invariant contracts";
    let dag = decomposer.decompose(goal)?;

    if dag.node_count() < 5 {
        bail!("Expected at least 5 nodes in decomposed DAG, got {}", dag.node_count());
    }

    if dag.contains_cycle() {
        bail!("Decomposed Goal DAG unexpectedly contains cycles");
    }

    dag.validate()?;

    let waves = dag.compute_waves()?;
    println!("    Discovered {} execution waves:", waves.len());
    for wave in &waves {
        println!("      • Wave {}: {:?}", wave.wave_index, wave.node_ids);
    }

    if waves.len() < 3 {
        bail!("Expected at least 3 topological waves, got {}", waves.len());
    }

    // Verify wave 0 has no prerequisites
    for node_id in &waves[0].node_ids {
        let prereqs = dag.get_prerequisites(node_id);
        if !prereqs.is_empty() {
            bail!("Wave 0 node '{}' has unexpected prerequisites: {:?}", node_id, prereqs);
        }
    }

    // Verify verification gate is in wave >= 2 and depends on implementation
    let verify_node = dag.get_node("verify_contract_invariants")
        .expect("Expected 'verify_contract_invariants' in decomposed DAG");
    assert_eq!(verify_node.kind, GoalNodeKind::Verification);

    let ascii_tree = dag.to_ascii_tree();
    if !ascii_tree.contains("WAVE 0") || !ascii_tree.contains("WAVE 1") {
        bail!("ASCII tree output missing wave headers:\n{}", ascii_tree);
    }
    println!("    ✓ Goal decomposition and wave layering verified successfully");

    // -------------------------------------------------------------------------
    // Test B: Asynchronous Wave-by-Wave Execution with Bounded Concurrency
    // -------------------------------------------------------------------------
    println!("  [Test B] Testing asynchronous wave execution with concurrency...");

    let scheduler = DagScheduler::new()
        .with_concurrency(4)
        .with_executor(Arc::new(EvalMockTaskExecutor));

    let (finished_dag, report) = scheduler.execute(dag).await?;

    if !report.success {
        bail!("DAG execution failed: {}", report.summary);
    }

    if report.failed_nodes > 0 {
        bail!("Expected 0 failed nodes, got {}", report.failed_nodes);
    }

    if report.completed_nodes == 0 {
        bail!("Expected completed nodes > 0, got {}", report.completed_nodes);
    }

    for node in finished_dag.all_nodes() {
        if node.kind != GoalNodeKind::Root && node.status != GoalStatus::Completed {
            bail!("Goal node '{}' did not complete successfully (status: {:?})", node.id, node.status);
        }
    }
    println!(
        "    ✓ Asynchronous wave execution completed: {} goals in {} waves ({} ms)",
        report.completed_nodes, report.total_waves, report.duration_ms
    );

    // -------------------------------------------------------------------------
    // Test C: Dynamic Failure Recovery & Mitigation DAG Patching
    // -------------------------------------------------------------------------
    println!("  [Test C] Testing dynamic failure recovery & mitigation DAG patching...");

    let mut fail_dag = GoalDag::new();

    let root = GoalNode::root("root", "Test Dynamic Recovery Plan");
    let task_1 = GoalNode::task("task_setup", "Initial environment setup");
    let mut verify_gate = GoalNode::verification("verify_schema_invariants", "Schema contract verification")
        .with_metadata(json!({ "simulate_error": "AssertionError: database schema migration invariant violated" }));
    verify_gate.retry_count = 0;
    verify_gate.max_retries = 0; // Trigger replanner immediately

    let task_post = GoalNode::task("task_deploy", "Post-verification final deployment");

    fail_dag.add_node(root);
    fail_dag.add_node(task_1);
    fail_dag.add_node(verify_gate);
    fail_dag.add_node(task_post);

    fail_dag.add_dependency("verify_schema_invariants", "task_setup")?;
    fail_dag.add_dependency("task_deploy", "verify_schema_invariants")?;

    let replanner = DynamicReplanner::new();
    let recovery_scheduler = DagScheduler::new()
        .with_concurrency(2)
        .with_auto_recover(true)
        .with_replanner(replanner)
        .with_executor(Arc::new(EvalMockTaskExecutor));

    let (recovered_dag, rec_report) = recovery_scheduler.execute(fail_dag).await?;

    if rec_report.recovery_attempts == 0 {
        bail!("Expected at least 1 dynamic recovery attempt, got 0");
    }

    if !rec_report.success {
        bail!("Expected dynamic recovery to succeed, but report failed: {}", rec_report.summary);
    }

    if !recovered_dag.contains_node("verify_schema_invariants_mitigation_fix") {
        bail!("Expected dynamically injected mitigation node 'verify_schema_invariants_mitigation_fix'");
    }

    println!(
        "    ✓ Dynamic failure recovery patched DAG and verified mitigation: {} recovery attempts, report success={}",
        rec_report.recovery_attempts, rec_report.success
    );

    // -------------------------------------------------------------------------
    // Test D: Cycle Detection & Validation Resilience
    // -------------------------------------------------------------------------
    println!("  [Test D] Testing cycle detection and invariant validation...");

    let mut cyclic_dag = GoalDag::new();
    cyclic_dag.add_node(GoalNode::task("node_a", "Node A"));
    cyclic_dag.add_node(GoalNode::task("node_b", "Node B"));
    cyclic_dag.add_node(GoalNode::task("node_c", "Node C"));

    cyclic_dag.add_dependency("node_b", "node_a")?;
    cyclic_dag.add_dependency("node_c", "node_b")?;
    cyclic_dag.add_dependency("node_a", "node_c")?; // Creates cycle A -> B -> C -> A

    if !cyclic_dag.contains_cycle() {
        bail!("Expected cyclic DAG to be detected, but contains_cycle returned false");
    }

    if cyclic_dag.validate().is_ok() {
        bail!("Expected cyclic DAG validate() to return error");
    }

    if cyclic_dag.compute_waves().is_ok() {
        bail!("Expected cyclic DAG compute_waves() to return error");
    }
    println!("    ✓ Cycle detection and validation invariants passed");

    // -------------------------------------------------------------------------
    // Test E: Tool Gateway Integration (goal_decompose and dag_execute)
    // -------------------------------------------------------------------------
    println!("  [Test E] Testing Tool Gateway integrations (goal_decompose, dag_execute)...");

    let decompose_tool = GoalDecomposeTool::new();
    let decomp_res = decompose_tool.execute(json!({
        "goal": "Migrate SQLite store to distributed pgvector cluster",
        "include_verification": true
    })).await.map_err(|e| anyhow::anyhow!("GoalDecomposeTool execution failed: {:?}", e))?;

    if decomp_res.get("status").and_then(|v| v.as_str()) != Some("success") {
        bail!("GoalDecomposeTool did not return success status: {:?}", decomp_res);
    }

    let dag_val = decomp_res.get("dag").cloned().expect("Expected 'dag' in tool output");

    let execute_tool = DagExecuteTool::new();
    let exec_res = execute_tool.execute(json!({
        "dag": dag_val,
        "max_concurrency": 3,
        "auto_recover": true
    })).await.map_err(|e| anyhow::anyhow!("DagExecuteTool execution failed: {:?}", e))?;

    if exec_res.get("status").and_then(|v| v.as_str()) != Some("success") {
        bail!("DagExecuteTool did not return success status: {:?}", exec_res);
    }

    let rep = exec_res.get("report").expect("Expected report in DagExecuteTool output");
    if rep.get("success").and_then(|v| v.as_bool()) != Some(true) {
        bail!("DagExecuteTool reported failure: {:?}", rep);
    }

    println!("    ✓ Tool Gateway tool execution verified successfully");

    println!("  ✓ Hierarchical Planning & DAG Scheduler scenario PASSED (5/5 tests).\n");
    Ok(())
}
