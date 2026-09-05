use anyhow::{bail, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::dag::GoalDag;
use super::recovery::DynamicReplanner;
use super::types::{DagExecutionReport, ExecutionWave, GoalNode, GoalStatus, RecoveryAction};

/// Abstraction for executing concrete goal tasks and verification checks.
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(&self, node: &GoalNode) -> std::result::Result<serde_json::Value, String>;
}

/// Default task executor: simulates execution or executes custom action parameters.
pub struct DefaultTaskExecutor;

#[async_trait]
impl TaskExecutor for DefaultTaskExecutor {
    async fn execute(&self, node: &GoalNode) -> std::result::Result<serde_json::Value, String> {
        // If metadata specifies a forced error for testing, return error
        if let Some(err) = node.metadata.get("simulate_error").and_then(|v| v.as_str()) {
            return Err(err.to_string());
        }

        // Simulate lightweight async workload
        let duration_ms = (node.estimated_duration_ms / 10).clamp(5, 50);
        tokio::time::sleep(tokio::time::Duration::from_millis(duration_ms)).await;

        Ok(serde_json::json!({
            "status": "completed",
            "node_id": node.id,
            "title": node.title,
            "action": node.action.as_deref().unwrap_or("none")
        }))
    }
}

/// Asynchronous Scheduler that executes Goal DAGs wave-by-wave with bounded concurrency and recovery gates.
pub struct DagScheduler {
    max_concurrency: usize,
    auto_recover: bool,
    replanner: DynamicReplanner,
    executor: Arc<dyn TaskExecutor>,
}

impl Default for DagScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl DagScheduler {
    pub fn new() -> Self {
        Self {
            max_concurrency: 4,
            auto_recover: true,
            replanner: DynamicReplanner::new(),
            executor: Arc::new(DefaultTaskExecutor),
        }
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.max_concurrency = concurrency.max(1);
        self
    }

    pub fn with_auto_recover(mut self, enabled: bool) -> Self {
        self.auto_recover = enabled;
        self
    }

    pub fn with_replanner(mut self, replanner: DynamicReplanner) -> Self {
        self.replanner = replanner;
        self
    }

    pub fn with_executor(mut self, executor: Arc<dyn TaskExecutor>) -> Self {
        self.executor = executor;
        self
    }

    /// Asynchronously execute the Goal DAG wave-by-wave.
    pub async fn execute(&self, mut dag: GoalDag) -> Result<(GoalDag, DagExecutionReport)> {
        dag.validate()?;
        let start_time = Instant::now();

        let plan_id = format!("plan_{}", uuid::Uuid::new_v4().simple());
        let root_goal = dag
            .get_node("root_goal")
            .map(|n| n.title.clone())
            .unwrap_or_else(|| "Hierarchical Plan".to_string());

        let mut executed_waves: Vec<ExecutionWave> = Vec::new();
        let mut recovery_attempts = 0;

        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));

        // Execution loop: process waves dynamically
        let mut loop_count = 0;
        let max_loops = 50; // Safety guard against infinite replanning loops

        while loop_count < max_loops {
            loop_count += 1;

            // Compute current waves based on live DAG state
            let current_waves = match dag.compute_waves() {
                Ok(w) => w,
                Err(e) => bail!("Wave computation error during execution: {e}"),
            };

            // Find first wave that contains pending or running nodes
            let next_wave_opt = current_waves.into_iter().find(|w| {
                w.node_ids.iter().any(|id| {
                    dag.get_node(id)
                        .map(|n| n.status == GoalStatus::Pending || n.status == GoalStatus::Running)
                        .unwrap_or(false)
                })
            });

            let current_wave = match next_wave_opt {
                Some(w) => w,
                None => break, // All waves completed or finalized
            };

            let wave_start = Instant::now();
            let mut runnable_nodes = Vec::new();

            // Evaluate prerequisites for nodes in current wave
            for node_id in &current_wave.node_ids {
                let prereqs = dag.get_prerequisites(node_id);
                let mut prereqs_satisfied = true;
                let mut prereq_failed = false;

                for p in &prereqs {
                    if let Some(p_node) = dag.get_node(p) {
                        match p_node.status {
                            GoalStatus::Completed | GoalStatus::Skipped => {}
                            GoalStatus::Failed => {
                                prereq_failed = true;
                                prereqs_satisfied = false;
                            }
                            _ => {
                                prereqs_satisfied = false;
                            }
                        }
                    }
                }

                if prereq_failed {
                    if let Some(node) = dag.get_node_mut(node_id) {
                        node.mark_skipped("Prerequisite dependency failed");
                    }
                } else if prereqs_satisfied {
                    if let Some(node) = dag.get_node_mut(node_id) {
                        if node.status == GoalStatus::Pending {
                            node.mark_running();
                            runnable_nodes.push(node.clone());
                        }
                    }
                }
            }

            if runnable_nodes.is_empty() {
                // If nothing was runnable in this wave, check if we're stuck
                let any_pending = dag
                    .all_nodes()
                    .iter()
                    .any(|n| n.status == GoalStatus::Pending || n.status == GoalStatus::Running);
                if !any_pending {
                    break;
                }
                // Mark remaining blocked nodes skipped to avoid deadlock
                for node in dag.all_nodes_mut() {
                    if node.status == GoalStatus::Pending || node.status == GoalStatus::Blocked {
                        node.mark_skipped("Unsatisfiable dependency cycle or prerequisite failure");
                    }
                }
                break;
            }

            // Launch runnable goals in parallel via JoinSet
            let mut join_set = JoinSet::new();

            for node in runnable_nodes {
                let sem = semaphore.clone();
                let exec = self.executor.clone();

                join_set.spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    let node_start = Instant::now();
                    let result = exec.execute(&node).await;
                    let duration_ms = node_start.elapsed().as_millis() as u64;
                    (node.id, result, duration_ms)
                });
            }

            let mut wave_has_failure = false;
            let mut should_replan = false;

            while let Some(res) = join_set.join_next().await {
                match res {
                    Ok((node_id, exec_result, duration_ms)) => match exec_result {
                        Ok(output) => {
                            if let Some(n) = dag.get_node_mut(&node_id) {
                                n.mark_completed(Some(output), duration_ms);
                            }
                        }
                        Err(err_msg) => {
                            wave_has_failure = true;
                            if let Some(n) = dag.get_node_mut(&node_id) {
                                n.mark_failed(&err_msg, duration_ms);
                            }

                            let failed_node_clone = dag.get_node(&node_id).cloned();

                            if self.auto_recover {
                                if let Some(failed_node) = failed_node_clone {
                                    if let Ok(action) =
                                        self.replanner.handle_failure(&dag, &failed_node)
                                    {
                                        recovery_attempts += 1;
                                        match &action {
                                            RecoveryAction::RetryNode { node_id, attempt } => {
                                                if let Some(n) = dag.get_node_mut(node_id) {
                                                    n.retry_count = *attempt;
                                                    n.status = GoalStatus::Pending;
                                                    n.error = None;
                                                }
                                                should_replan = true;
                                            }
                                            RecoveryAction::InjectMitigation { .. }
                                            | RecoveryAction::SubstituteNode { .. }
                                            | RecoveryAction::BypassNode { .. } => {
                                                if self
                                                    .replanner
                                                    .apply_recovery(&mut dag, &action)
                                                    .is_ok()
                                                {
                                                    should_replan = true;
                                                }
                                            }
                                            RecoveryAction::Abort { reason } => {
                                                // Abort remaining execution
                                                for n in dag.all_nodes_mut() {
                                                    if n.status == GoalStatus::Pending
                                                        || n.status == GoalStatus::Running
                                                    {
                                                        n.mark_skipped(format!(
                                                            "Aborted: {reason}"
                                                        ));
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Err(join_err) => {
                        wave_has_failure = true;
                        tracing::error!("Goal execution task join error: {join_err}");
                    }
                }
            }

            let mut recorded_wave = current_wave.clone();
            recorded_wave.duration_ms = Some(wave_start.elapsed().as_millis() as u64);
            recorded_wave.status = if wave_has_failure && !should_replan {
                GoalStatus::Failed
            } else {
                GoalStatus::Completed
            };
            executed_waves.push(recorded_wave);

            if !self.auto_recover && wave_has_failure {
                // Without auto-recover, halt on failure
                for n in dag.all_nodes_mut() {
                    if n.status == GoalStatus::Pending {
                        n.mark_skipped("Halted due to preceding wave failure");
                    }
                }
                break;
            }
        }

        let total_duration_ms = start_time.elapsed().as_millis() as u64;

        // Compile final metrics
        let all_nodes = dag.all_nodes();
        let total_nodes = all_nodes.len();
        let completed_nodes = all_nodes
            .iter()
            .filter(|n| n.status == GoalStatus::Completed)
            .count();
        let failed_nodes = all_nodes
            .iter()
            .filter(|n| n.status == GoalStatus::Failed)
            .count();
        let skipped_nodes = all_nodes
            .iter()
            .filter(|n| n.status == GoalStatus::Skipped)
            .count();

        let success = failed_nodes == 0 && completed_nodes > 0;

        let mut node_results = HashMap::new();
        for node in all_nodes {
            node_results.insert(node.id.clone(), node.clone());
        }

        let summary = if success {
            format!(
                "Successfully executed Goal DAG across {} waves ({} goals completed, {} recovery attempts, {} ms)",
                executed_waves.len(),
                completed_nodes,
                recovery_attempts,
                total_duration_ms
            )
        } else {
            format!(
                "Goal DAG execution finished with {} failures, {} skipped, {} completed across {} waves ({} ms)",
                failed_nodes,
                skipped_nodes,
                completed_nodes,
                executed_waves.len(),
                total_duration_ms
            )
        };

        let report = DagExecutionReport {
            plan_id,
            root_goal,
            total_nodes,
            completed_nodes,
            failed_nodes,
            skipped_nodes,
            total_waves: executed_waves.len(),
            waves: executed_waves,
            duration_ms: total_duration_ms,
            success,
            node_results,
            recovery_attempts,
            summary,
        };

        Ok((dag, report))
    }
}
