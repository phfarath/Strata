use anyhow::Result;
use clap::Args;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

use strata_core::schemas::SyncConfig;
use strata_memory::{EpisodicPyramidStore, SqliteStore, SyncEngine};

#[derive(Args, Debug, Clone)]
pub struct DaemonArgs {
    #[arg(
        long,
        default_value_t = 30,
        help = "Synchronization loop interval in seconds"
    )]
    pub interval: u64,

    #[arg(long, help = "Remote synchronization endpoint URL")]
    pub endpoint: Option<String>,

    #[arg(long, help = "Bearer authentication token for remote endpoint")]
    pub token: Option<String>,

    #[arg(long, default_value = "default", help = "Workspace identifier")]
    pub workspace: String,

    #[arg(
        long,
        help = "Enable real-time AST code drift watcher alongside sync daemon"
    )]
    pub watch: bool,

    #[arg(
        long,
        default_value_t = true,
        help = "Enable automatic background episodic pyramid rollups for idle episodes"
    )]
    pub auto_rollup: bool,
}

/// Processes pending unrolled episodes into Tier 2 milestone rollups during daemon cycles.
pub fn run_daemon_pyramid_rollup(
    pyramid_store: &EpisodicPyramidStore,
    session_id: Option<&str>,
) -> Result<usize> {
    let rollups = pyramid_store.process_idle_episodes(session_id)?;
    if !rollups.is_empty() {
        info!(
            "Episodic pyramid auto-rollup generated {} Tier 2 milestone rollups in background",
            rollups.len()
        );
        println!(
            "🏛️  Episodic pyramid auto-rollup: {} milestone rollups generated in background",
            rollups.len()
        );
    }
    Ok(rollups.len())
}

pub async fn run_daemon(args: DaemonArgs, store: Arc<SqliteStore>) -> Result<()> {
    let mut config = SyncConfig::new(&args.workspace);
    config.endpoint = args
        .endpoint
        .or_else(|| std::env::var("STRATA_SYNC_ENDPOINT").ok());
    config.token = args
        .token
        .or_else(|| std::env::var("STRATA_SYNC_TOKEN").ok());

    let interval_secs = args.interval.max(1);
    let sync_engine = SyncEngine::new(store.clone(), config.clone());
    let pyramid_store = EpisodicPyramidStore::new(store.clone());

    info!(
        "Starting Strata background sync daemon (interval: {}s, workspace: '{}', endpoint: {:?}, auto_rollup: {})",
        interval_secs, config.workspace_id, config.endpoint, args.auto_rollup
    );
    println!(
        "🚀 Strata background sync daemon running (< 10MB RAM footprint). Press Ctrl+C to stop."
    );
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let (ipc_endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
    let ipc_server = match strata_memory::IpcServer::bind(&ipc_endpoint).await {
        Ok(srv) => {
            info!("A2A Realtime Local IPC Server listening on {ipc_endpoint}");
            println!("🌐 A2A Realtime Local IPC Server active ({ipc_endpoint})");
            Some(srv)
        }
        Err(e) => {
            warn!("Could not bind A2A Realtime IPC Server ({e}). Continuing in sync-only mode.");
            None
        }
    };

    let watcher_task = if args.watch {
        let watch_store = store.clone();
        let watch_args = crate::commands::watch::WatchArgs {
            workspace: std::path::PathBuf::from("."),
            debounce_ms: 500,
            once: false,
            json: false,
        };
        info!("Starting background AST drift watcher alongside sync daemon");
        println!("👀 Real-time AST code drift watcher active (debounce: 500ms)");
        Some(tokio::spawn(async move {
            if let Err(e) = crate::commands::watch::run_watch(watch_args, watch_store).await {
                warn!("Background AST drift watcher stopped: {e}");
            }
        }))
    } else {
        None
    };

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C interrupt signal. Shutting down Strata daemon gracefully...");
                println!("\nGracefully shutting down Strata daemon...");
                if let Some(ref task) = watcher_task {
                    task.abort();
                }
                if let Some(srv) = ipc_server {
                    srv.stop();
                }
                if args.auto_rollup {
                    let _ = run_daemon_pyramid_rollup(&pyramid_store, None);
                }
                // Final push attempt before exit
                let _ = sync_engine.push_deltas().await;
                break;
            }
            _ = sleep(Duration::from_secs(interval_secs)) => {
                match sync_engine.sync_cycle().await {
                    Ok(report) => {
                        if report.pushed_count > 0 || report.pulled_count > 0 || report.conflicts_resolved > 0 {
                            info!(
                                "Sync cycle complete: pushed={}, pulled={}, conflicts_resolved={}",
                                report.pushed_count,
                                report.pulled_count,
                                report.conflicts_resolved
                            );
                        }
                        for err in report.errors {
                            warn!("Sync cycle warning: {err}");
                        }
                    }
                    Err(e) => {
                        error!("Sync cycle failed: {e}");
                    }
                }

                if args.auto_rollup {
                    if let Err(e) = run_daemon_pyramid_rollup(&pyramid_store, None) {
                        warn!("Episodic pyramid auto-rollup warning in daemon cycle: {e}");
                    }
                }
            }
        }
    }

    info!("Strata sync daemon stopped cleanly.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::schemas::{EpisodeRecord, EpisodeStep};

    #[test]
    fn test_daemon_auto_rollup_idle_episodes() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let pyramid_store = EpisodicPyramidStore::new(store.clone());

        // Initially 0 rollups
        let count = run_daemon_pyramid_rollup(&pyramid_store, Some("sess-daemon")).unwrap();
        assert_eq!(count, 0);

        // Ingest 2 pending Tier 1 episodes
        let ep1 = EpisodeRecord::new("sess-daemon", "agent-worker", "Worker step 1")
            .with_task_id("task-daemon-auto")
            .with_steps(vec![EpisodeStep::new(0, "Write test")]);
        let ep2 = EpisodeRecord::new("sess-daemon", "agent-worker", "Worker step 2")
            .with_task_id("task-daemon-auto")
            .with_steps(vec![EpisodeStep::new(0, "Run test")]);
        store.write_episode(&ep1).unwrap();
        store.write_episode(&ep2).unwrap();

        // Daemon background tick processes idle episodes
        let count = run_daemon_pyramid_rollup(&pyramid_store, Some("sess-daemon")).unwrap();
        assert_eq!(count, 1, "Should generate 1 milestone rollup during daemon cycle");

        // Verify tier 2 rollup exists and children were linked
        let rollups = store.list_episodes_by_tier(2, Some("sess-daemon"), 10).unwrap();
        assert_eq!(rollups.len(), 1);
        assert_eq!(rollups[0].child_ids.len(), 2);

        // Next tick: no pending unrolled episodes
        let count2 = run_daemon_pyramid_rollup(&pyramid_store, Some("sess-daemon")).unwrap();
        assert_eq!(count2, 0);
    }
}
