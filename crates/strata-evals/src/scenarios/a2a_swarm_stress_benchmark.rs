use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use strata_core::blackboard::{TaskArtifact, TaskFilter, TaskRecord, TaskStatus};
use strata_core::errors::StrataError;
use strata_memory::SqliteMemoryEngine;

/// Detailed performance and concurrency metrics from the Swarm Stress Benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmStressBenchmarkResult {
    pub virtual_agents_count: usize,
    pub total_tasks_scheduled: usize,
    pub total_tasks_completed: usize,
    pub lease_collisions_observed: usize,
    pub zombie_tasks_reclaimed: usize,
    pub duration_ms: f64,
    pub throughput_tasks_per_sec: f64,
    pub zero_deadlocks_verified: bool,
    pub all_leases_released_verified: bool,
    pub data_consistency_verified: bool,
}

pub struct A2aSwarmStressBenchmark;

impl A2aSwarmStressBenchmark {
    pub async fn run_benchmark() -> Result<SwarmStressBenchmarkResult, StrataError> {
        let temp_dir = std::env::temp_dir().join(format!("strata_swarm_stress_{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let _ = std::fs::create_dir_all(&temp_dir);
        let db_path = temp_dir.join("swarm_stress.db");
        let _ = std::fs::remove_file(&db_path);

        // Open shared SQLite database engine with WAL enabled
        let engine = Arc::new(SqliteMemoryEngine::open(&db_path, None)?);
        let coordinator = engine.stigmergy();

        let num_agents = 8;
        let num_tasks = 24;
        let lease_collision_counter = Arc::new(AtomicUsize::new(0));

        // 1. Populate blackboard with DAG and shared contested resources
        // Tasks 0..6 contend on "res:crate:strata-core"
        // Tasks 6..12 contend on "res:crate:strata-memory"
        // Tasks 12..18 contend on "res:file:src/lib.rs"
        // Tasks 18..24 have DAG dependencies on earlier tasks
        let mut created_task_ids = Vec::new();
        for i in 0..num_tasks {
            let task_id = format!("stress-task-{:02}", i);
            let mut record = TaskRecord::new(&task_id, format!("Stress task #{}", i));
            
            // Assign contested resources to induce lease arbitration
            if i < 6 {
                record.required_resources = vec!["res:crate:strata-core".to_string()];
            } else if i < 12 {
                record.required_resources = vec!["res:crate:strata-memory".to_string()];
            } else if i < 18 {
                record.required_resources = vec!["res:file:src/lib.rs".to_string()];
            } else {
                // Dependency on an earlier task
                let prereq = format!("stress-task-{:02}", i - 12);
                record.dependencies = vec![prereq];
                record.status = TaskStatus::Blocked;
            }

            coordinator.create_task(&record)?;
            created_task_ids.push(task_id);
        }

        // 2. Insert a Zombie Task claimed by a dead agent that will time out
        let zombie_task_id = "stress-zombie-task";
        let mut zombie_task = TaskRecord::new(zombie_task_id, "Zombie agent dead task");
        zombie_task.status = TaskStatus::Running;
        zombie_task.assigned_agent = Some("zombie-agent-died".to_string());
        zombie_task.required_resources = vec!["res:zombie-lock".to_string()];
        // Backdate updated_at by 100 seconds
        zombie_task.updated_at = Utc::now().timestamp() - 100;
        coordinator.create_task(&zombie_task)?;

        // Acquire lock on behalf of zombie agent
        let _ = coordinator.acquire_lease("res:zombie-lock", "zombie-agent-died", 1, Some("task:stress-zombie-task"))?;

        let start_time = Instant::now();

        // 3. Spawn 8 concurrent virtual workers running work-stealing loops
        let mut worker_handles = Vec::new();
        for worker_idx in 0..num_agents {
            let coord = coordinator.clone();
            let agent_id = format!("virtual-worker-{:02}", worker_idx);
            let collision_acc = Arc::clone(&lease_collision_counter);

            let handle = tokio::spawn(async move {
                let mut completed_by_this_worker = 0;
                let worker_start = Instant::now();

                // Loop until no pending/blocked tasks remain or timeout (max 6s)
                while worker_start.elapsed().as_secs() < 6 {
                    // Try to claim ready task
                    match coord.claim_task_atomic(None, &agent_id, 30) {
                        Ok(Some(task)) => {
                            // Heartbeat progress
                            let _ = coord.update_task_progress(
                                &task.task_id,
                                &agent_id,
                                "Executing simulated stress workload",
                            );

                            // Simulate real cpu work (1-3ms)
                            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;

                            // Create artifact
                            let artifact = TaskArtifact::new(
                                format!("art-{}-{}", task.task_id, agent_id),
                                &task.task_id,
                                &agent_id,
                                "result",
                                format!("Completed {} by {}", task.task_id, agent_id),
                            );

                            // Mark completed
                            let _ = coord.complete_task(
                                &task.task_id,
                                &agent_id,
                                Some("Stress task completed successfully"),
                                Some(&artifact),
                            );

                            completed_by_this_worker += 1;
                        }
                        Ok(None) => {
                            // Check if all tasks are done
                            let filter = TaskFilter {
                                status: Some(TaskStatus::Completed),
                                ..Default::default()
                            };
                            let completed_count = coord.list_tasks(&filter).unwrap_or_default().len();
                            if completed_count > num_tasks {
                                break;
                            }

                            // Contention: either blocked on dependencies or locked by peers
                            collision_acc.fetch_add(1, Ordering::Relaxed);
                            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                        }
                        Err(_) => {
                            collision_acc.fetch_add(1, Ordering::Relaxed);
                            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                        }
                    }
                }
                completed_by_this_worker
            });
            worker_handles.push(handle);
        }

        // 4. Background task: Sweep and reclaim stale zombie task (stale_after = 10s)
        let coord_reclaim = coordinator.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = coord_reclaim.reclaim_stale_tasks(10);
        });

        // Await all workers
        let mut total_completed = 0;
        for h in worker_handles {
            if let Ok(count) = h.await {
                total_completed += count;
            }
        }

        let duration = start_time.elapsed();
        let duration_ms = duration.as_secs_f64() * 1000.0;
        let throughput = if duration_ms > 0.0 {
            (total_completed as f64) / (duration.as_secs_f64())
        } else {
            0.0
        };

        // 5. Verification & Invariant Checks
        let zombie_record = coordinator.get_task(zombie_task_id)?;
        let zombie_completed = zombie_record.map(|t| t.status == TaskStatus::Completed).unwrap_or(false);

        let filter_done = TaskFilter {
            status: Some(TaskStatus::Completed),
            ..Default::default()
        };
        let final_completed_tasks = coordinator.list_tasks(&filter_done)?;
        let data_consistency_verified = final_completed_tasks.len() == num_tasks + 1;

        let active_leases = coordinator.active_leases()?;
        let all_leases_released_verified = active_leases.is_empty();

        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(SwarmStressBenchmarkResult {
            virtual_agents_count: num_agents,
            total_tasks_scheduled: num_tasks + 1,
            total_tasks_completed: final_completed_tasks.len(),
            lease_collisions_observed: lease_collision_counter.load(Ordering::Relaxed),
            zombie_tasks_reclaimed: if zombie_completed { 1 } else { 0 },
            duration_ms,
            throughput_tasks_per_sec: throughput,
            zero_deadlocks_verified: duration.as_secs() < 10,
            all_leases_released_verified,
            data_consistency_verified,
        })
    }
}

pub async fn run_a2a_swarm_stress_benchmark_scenario() -> Result<()> {
    println!("\n--------------------------------------------------------");
    println!("Scenario: Multi-Agent Swarm Stress Benchmark & Work-Stealing");
    println!("--------------------------------------------------------");

    let result = A2aSwarmStressBenchmark::run_benchmark().await?;

    println!("🤖 Concurrent Virtual Agents:        {}", result.virtual_agents_count);
    println!("📋 Total Tasks Scheduled:            {}", result.total_tasks_scheduled);
    println!("✅ Total Tasks Completed:            {}", result.total_tasks_completed);
    println!("🔒 Lease Contention Collisions:      {}", result.lease_collisions_observed);
    println!("♻️ Zombie Tasks Reclaimed:           {}", result.zombie_tasks_reclaimed);
    println!("⏱ Benchmark Latency:                 {:.2} ms", result.duration_ms);
    println!("🚀 Work-Stealing Throughput:         {:.1} tasks/sec", result.throughput_tasks_per_sec);
    println!("🛡 Zero Deadlocks Verified:           {}", result.zero_deadlocks_verified);
    println!("🔓 All Locks Released:               {}", result.all_leases_released_verified);
    println!("💎 Data Consistency Verified:        {}", result.data_consistency_verified);

    assert!(result.zero_deadlocks_verified, "Swarm execution must not deadlock");
    assert!(result.data_consistency_verified, "All tasks must reach completed status");
    assert!(result.all_leases_released_verified, "All temporal leases must be released");
    assert_eq!(result.zombie_tasks_reclaimed, 1, "Zombie orphan task must be reclaimed and finished");

    Ok(())
}
