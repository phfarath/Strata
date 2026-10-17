use anyhow::Result;
use clap::Args;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

use strata_core::a2a::AgentPresence;
use strata_core::blackboard::{TaskArtifact, TaskFilter, TaskStatus};
use strata_memory::StigmergyCoordinator;

use crate::agent_env::{resolve_agent_host, resolve_agent_id};

#[derive(Args, Debug, Clone)]
pub struct WorkerArgs {
    /// Agent identity (defaults to STRATA_AGENT_ID, or host-pid auto-generated ID)
    #[arg(long, help = "Agent identity for claiming tasks and resource locks")]
    pub agent: Option<String>,

    /// Host runner label (e.g. 'antigravity', 'claude-code', 'daemon')
    #[arg(long, help = "Host runner label")]
    pub host: Option<String>,

    /// Filter work-stealing by plan ID
    #[arg(long, help = "Specific plan ID to pull tasks from")]
    pub plan: Option<String>,

    /// Polling interval in seconds when blackboard is idle
    #[arg(long, default_value_t = 2, help = "Polling interval in seconds when idle")]
    pub poll_interval: u64,

    /// Progress heartbeat interval in seconds while executing
    #[arg(long, default_value_t = 5, help = "Progress heartbeat interval in seconds")]
    pub heartbeat_interval: u64,

    /// Lease TTL in seconds when claiming resources
    #[arg(long, default_value_t = 180, help = "Resource lease TTL in seconds")]
    pub lease_ttl: i64,

    /// Stale timeout threshold in seconds to auto-reclaim orphaned tasks
    #[arg(long, default_value_t = 600, help = "Stale timeout threshold in seconds for reclaim")]
    pub stale_timeout: i64,

    /// Automatically run stale orphan reclamation check periodically
    #[arg(long, default_value_t = true, help = "Automatically reclaim stale orphaned tasks")]
    pub auto_reclaim: bool,

    /// Maximum number of tasks to execute before stopping (omit to run indefinitely)
    #[arg(long, help = "Maximum tasks to execute before exiting")]
    pub max_tasks: Option<usize>,

    /// Exit immediately when no ready tasks remain (non-blocking batch execution mode)
    #[arg(long, help = "Exit when no ready/unblocked tasks remain on blackboard")]
    pub exit_when_idle: bool,

    /// Execute command dry-run (simulation mode without running subprocesses)
    #[arg(long, help = "Simulate task execution without running shell subprocesses")]
    pub dry_run: bool,

    /// Output raw JSON events/logs
    #[arg(long, help = "Output as raw JSON")]
    pub json: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkerStats {
    pub tasks_executed: usize,
    pub tasks_succeeded: usize,
    pub tasks_failed: usize,
    pub orphans_reclaimed: usize,
    pub agent_id: String,
}

pub async fn run_worker(args: WorkerArgs, coordinator: StigmergyCoordinator) -> Result<WorkerStats> {
    let agent_id = resolve_agent_id(args.agent.as_deref());
    let host = resolve_agent_host(args.host.as_deref());

    let mut stats = WorkerStats {
        agent_id: agent_id.clone(),
        ..Default::default()
    };

    let pid = std::process::id();
    let presence = AgentPresence::new(&agent_id, &host, pid);
    let _ = coordinator.heartbeat(&presence);

    if !args.json {
        println!("\n🤖 [Strata Autonomous Blackboard Worker Daemon]");
        println!("══════════════════════════════════════════════════");
        println!("Agent ID:           {agent_id}");
        println!("Host Runner:        {host} (PID: {pid})");
        println!("Polling Interval:   {}s", args.poll_interval);
        println!("Heartbeat Interval: {}s", args.heartbeat_interval);
        println!("Resource Lease TTL: {}s", args.lease_ttl);
        if let Some(m) = args.max_tasks {
            println!("Max Tasks:          {m}");
        }
        if args.exit_when_idle {
            println!("Execution Mode:     Batch (exit when idle)");
        } else {
            println!("Execution Mode:     Continuous daemon (press Ctrl+C to stop)");
        }
    } else {
        println!(
            "{}",
            serde_json::json!({
                "event": "worker_started",
                "agent_id": agent_id,
                "host": host,
                "pid": pid,
            })
        );
    }

    let running = Arc::new(AtomicBool::new(true));
    let r_cancel = Arc::clone(&running);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            r_cancel.store(false, Ordering::SeqCst);
        }
    });

    // Main execution loop
    while running.load(Ordering::SeqCst) {
        // 1. Optional stale task reclamation
        if args.auto_reclaim {
            if let Ok(reclaimed) = coordinator.reclaim_stale_tasks(args.stale_timeout) {
                if !reclaimed.is_empty() {
                    stats.orphans_reclaimed += reclaimed.len();
                    if !args.json {
                        println!("♻️ Reclaimed {} orphaned tasks back to pool", reclaimed.len());
                    } else {
                        println!(
                            "{}",
                            serde_json::json!({
                                "event": "orphans_reclaimed",
                                "count": reclaimed.len(),
                                "tasks": reclaimed
                            })
                        );
                    }
                }
            }
        }

        // 2. Atomic Work-Stealing: Claim next ready task
        let claimed_task = match coordinator.claim_task_atomic(None, &agent_id, args.lease_ttl) {
            Ok(opt) => opt,
            Err(e) => {
                warn!("Worker encountered error while claiming task: {e}");
                sleep(Duration::from_secs(args.poll_interval.max(1))).await;
                continue;
            }
        };

        match claimed_task {
            Some(task) => {
                stats.tasks_executed += 1;
                let task_id = task.task_id.clone();
                let title = task.title.clone();

                if !args.json {
                    println!("\n⚙️ Claimed task [{}]: \"{}\"", task_id, title);
                } else {
                    println!(
                        "{}",
                        serde_json::json!({
                            "event": "task_claimed",
                            "task_id": task_id,
                            "title": title,
                            "required_resources": task.required_resources
                        })
                    );
                }

                // Update active presence
                let _ = coordinator.heartbeat(&presence.clone().with_active_task(&title));

                // Spawn background heartbeat task for liveness and lease renewal while running
                let lease_lost = Arc::new(AtomicBool::new(false));
                let lease_lost_clone = Arc::clone(&lease_lost);
                let heartbeat_stop = Arc::new(AtomicBool::new(false));
                let hb_stop_clone = Arc::clone(&heartbeat_stop);
                let coord_hb = coordinator.clone();
                let tid_hb = task_id.clone();
                let aid_hb = agent_id.clone();
                let hb_interval = args.heartbeat_interval.max(1);
                let lease_ttl = args.lease_ttl.max(10) as i64;
                let task_gen = task.claim_generation;

                let hb_handle = tokio::spawn(async move {
                    while !hb_stop_clone.load(Ordering::SeqCst) {
                        sleep(Duration::from_secs(hb_interval)).await;
                        if hb_stop_clone.load(Ordering::SeqCst) {
                            break;
                        }

                        // Renew task leases and check ownership/fencing token
                        let renewed = coord_hb.renew_task_leases_with_generation(
                            &tid_hb,
                            &aid_hb,
                            lease_ttl,
                            Some(task_gen),
                        ).unwrap_or(false);

                        if !renewed {
                            lease_lost_clone.store(true, Ordering::SeqCst);
                            break;
                        }

                        let _ = coord_hb.update_task_progress_with_generation(
                            &tid_hb,
                            &aid_hb,
                            "Worker executing active step",
                            Some(task_gen),
                        );
                    }
                });

                // Execute the task with race detection against lease loss
                let exec_future = execute_task_payload(&task, args.dry_run);
                tokio::pin!(exec_future);

                let exec_result = tokio::select! {
                    res = &mut exec_future => res,
                    _ = async {
                        while !lease_lost.load(Ordering::SeqCst) && !heartbeat_stop.load(Ordering::SeqCst) {
                            sleep(Duration::from_millis(50)).await;
                        }
                    } => {
                        if lease_lost.load(Ordering::SeqCst) {
                            Err(anyhow::anyhow!("Task lease was lost or revoked concurrently during execution"))
                        } else {
                            exec_future.await
                        }
                    }
                };

                heartbeat_stop.store(true, Ordering::SeqCst);
                let _ = hb_handle.await;

                match exec_result {
                    Ok(artifact_content) => {
                        stats.tasks_succeeded += 1;
                        let artifact = TaskArtifact::new(
                            format!("art-{}", Uuid::new_v4()),
                            &task_id,
                            &agent_id,
                            "output",
                            &artifact_content,
                        );

                        let completed = coordinator.complete_task_with_generation(
                            &task_id,
                            &agent_id,
                            Some(&format!("Task '{}' executed successfully", title)),
                            Some(&artifact),
                            Some(task.claim_generation),
                        ).unwrap_or(false);

                        if !completed {
                            stats.tasks_succeeded -= 1;
                            stats.tasks_failed += 1;
                            let err_msg = "Task completion rejected (lease expired or task revoked concurrently)";
                            if !args.json {
                                println!("❌ Failed to complete task [{}]: {}", task_id, err_msg);
                            } else {
                                println!(
                                    "{}",
                                    serde_json::json!({
                                        "event": "task_failed",
                                        "task_id": task_id,
                                        "title": title,
                                        "error": err_msg
                                    })
                                );
                            }
                        } else if !args.json {
                            println!("✅ Completed task [{}]: \"{}\"", task_id, title);
                        } else {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "event": "task_completed",
                                    "task_id": task_id,
                                    "title": title,
                                    "artifact_length": artifact_content.len()
                                })
                            );
                        }
                    }
                    Err(err) => {
                        stats.tasks_failed += 1;
                        let err_msg = format!("{err}");
                        let _ = coordinator.fail_task_with_generation(
                            &task_id,
                            &agent_id,
                            &err_msg,
                            Some(task.claim_generation),
                        );

                        if !args.json {
                            println!("❌ Failed task [{}]: {}", task_id, err_msg);
                        } else {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "event": "task_failed",
                                    "task_id": task_id,
                                    "error": err_msg
                                })
                            );
                        }
                    }
                }

                // Check max tasks budget
                if let Some(max) = args.max_tasks {
                    if stats.tasks_executed >= max {
                        if !args.json {
                            println!("🏁 Reached maximum task budget ({max}). Worker exiting.");
                        }
                        break;
                    }
                }
            }
            None => {
                // No task was ready right now
                if args.exit_when_idle {
                    let active_filter = TaskFilter {
                        plan_id: args.plan.clone(),
                        ..Default::default()
                    };
                    let all_tasks = coordinator.list_tasks(&active_filter).unwrap_or_default();
                    let has_in_progress = all_tasks.iter().any(|t| {
                        matches!(t.status, TaskStatus::Claimed | TaskStatus::Running)
                    });
                    let has_uncompleted = all_tasks.iter().any(|t| {
                        matches!(t.status, TaskStatus::Pending | TaskStatus::Blocked)
                    });

                    if has_in_progress && has_uncompleted {
                        // Peer workers are executing tasks that might unblock pending/blocked tasks
                        sleep(Duration::from_millis(100)).await;
                    } else {
                        if !args.json {
                            println!("💤 No ready tasks remaining on blackboard. Worker stopping.");
                        }
                        break;
                    }
                } else {
                    sleep(Duration::from_secs(args.poll_interval.max(1))).await;
                }
            }
        }
    }

    // Cleanup presence
    let _ = coordinator.deregister_agent(&agent_id);

    if !args.json {
        println!("\n📊 [Strata Worker Summary]");
        println!("══════════════════════════════════════════════════");
        println!("Tasks Executed:     {}", stats.tasks_executed);
        println!("Tasks Succeeded:    {}", stats.tasks_succeeded);
        println!("Tasks Failed:       {}", stats.tasks_failed);
        println!("Orphans Reclaimed:  {}", stats.orphans_reclaimed);
    } else {
        println!(
            "{}",
            serde_json::json!({
                "event": "worker_stopped",
                "stats": stats
            })
        );
    }

    Ok(stats)
}

/// Executes the payload associated with a task.
/// Supports deterministic commands embedded in description (e.g. "cmd: <command>" or "run: <command>"),
/// or marks normal completion with structured artifacts.
async fn execute_task_payload(task: &strata_core::blackboard::TaskRecord, dry_run: bool) -> Result<String> {
    if dry_run {
        sleep(Duration::from_millis(50)).await;
        return Ok(format!("Simulated dry-run execution of task '{}'", task.title));
    }

    let cmd_opt = task
        .description
        .as_deref()
        .and_then(|desc| {
            for line in desc.lines() {
                let trimmed = line.trim();
                if let Some(cmd) = trimmed.strip_prefix("cmd:") {
                    return Some(cmd.trim().to_string());
                }
                if let Some(cmd) = trimmed.strip_prefix("run:") {
                    return Some(cmd.trim().to_string());
                }
                if let Some(cmd) = trimmed.strip_prefix("$ ") {
                    return Some(cmd.trim().to_string());
                }
            }
            None
        });

    if let Some(command_str) = cmd_opt {
        info!("Worker executing shell command: {command_str}");
        #[cfg(target_os = "windows")]
        let output = tokio::process::Command::new("powershell")
            .kill_on_drop(true)
            .args(["-NoProfile", "-Command", &command_str])
            .output()
            .await?;

        #[cfg(not(target_os = "windows"))]
        let output = tokio::process::Command::new("sh")
            .kill_on_drop(true)
            .args(["-c", &command_str])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            anyhow::bail!("Command '{command_str}' exited with code {code}: {stderr}");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(format!("Command '{command_str}' succeeded:\n{stdout}"))
    } else {
        // Default deterministic execution: success
        sleep(Duration::from_millis(20)).await;
        Ok(format!("Executed task '{}' [ID: {}]", task.title, task.task_id))
    }
}
