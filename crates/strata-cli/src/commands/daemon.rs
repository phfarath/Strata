use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

use strata_core::schemas::SyncConfig;
use strata_memory::{
    parse_dream_duration, DreamConfig, DreamEngine, DreamReport, EpisodicPyramidStore, SqliteStore,
    SyncEngine,
};

#[derive(Args, Debug, Clone)]
pub struct DaemonArgs {
    #[arg(
        long,
        default_value_t = 30,
        help = "Synchronization loop interval in seconds"
    )]
    pub interval: u64,

    #[arg(
        long = "idle-timeout",
        alias = "idle_timeout",
        default_value = "60s",
        help = "Inactivity timeout before triggering background KAIROS auto-dreaming cycle (e.g. '30s', '5m', '500ms')"
    )]
    pub idle_timeout: String,

    #[arg(
        long = "dream-sessions",
        alias = "dream_sessions",
        alias = "sessions-threshold",
        default_value_t = 3,
        help = "Trigger background auto-dreaming cycle after every N new sessions"
    )]
    pub dream_sessions: usize,

    #[arg(
        long = "auto-dream",
        alias = "auto_dream",
        default_value_t = true,
        help = "Enable KAIROS-style background auto-dreaming memory consolidation (< 5MB RAM)"
    )]
    pub auto_dream: bool,

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

/// Evaluates inactivity and session count thresholds and runs a KAIROS auto-dreaming cycle if triggered.
pub fn run_daemon_dream_cycle(
    dream_engine: &mut DreamEngine,
    now: chrono::DateTime<Utc>,
) -> Result<Option<DreamReport>> {
    if let Some(report) = dream_engine.dream_if_needed(now)? {
        if report.has_changes() || report.sessions_processed > 0 {
            info!(
                "KAIROS auto-dream cycle completed (trigger={:?}): merged={}, contradictions_resolved={}, episodes_consolidated={}, rollups={}, pruned={}",
                report.trigger,
                report.memories_merged,
                report.contradictions_resolved,
                report.episodes_consolidated,
                report.rollups_created,
                report.memories_pruned
            );
            println!(
                "🌙 Auto-dream cycle ({:?}): merged={}, contradictions_resolved={}, episodes={}, pruned={}",
                report.trigger,
                report.memories_merged,
                report.contradictions_resolved,
                report.episodes_consolidated,
                report.memories_pruned
            );
        }
        return Ok(Some(report));
    }
    Ok(None)
}

pub async fn run_daemon(args: DaemonArgs, store: Arc<SqliteStore>) -> Result<()> {
    let idle_duration = parse_dream_duration(&args.idle_timeout)
        .with_context(|| format!("Invalid --idle-timeout value '{}'", args.idle_timeout))?;

    let mut config = SyncConfig::new(&args.workspace);
    config.endpoint = args
        .endpoint
        .or_else(|| std::env::var("STRATA_SYNC_ENDPOINT").ok());
    config.token = args
        .token
        .or_else(|| std::env::var("STRATA_SYNC_TOKEN").ok());

    let interval_secs = args.interval.max(1);
    let sync_interval = Duration::from_secs(interval_secs);
    let tick_duration = if args.auto_dream {
        sync_interval.min(idle_duration.max(Duration::from_millis(50)))
    } else {
        sync_interval
    };

    let sync_engine = SyncEngine::new(store.clone(), config.clone());
    let pyramid_store = EpisodicPyramidStore::new(store.clone());
    let dream_config = DreamConfig::new(idle_duration, args.dream_sessions);
    let mut dream_engine = DreamEngine::new(store.clone(), dream_config);

    info!(
        "Starting Strata background daemon (interval: {}s, idle_timeout: {:?}, dream_sessions: {}, workspace: '{}', endpoint: {:?}, auto_rollup: {}, auto_dream: {})",
        interval_secs, idle_duration, args.dream_sessions, config.workspace_id, config.endpoint, args.auto_rollup, args.auto_dream
    );
    println!(
        "🚀 Strata background daemon running (< 5MB RAM footprint, KAIROS auto-dreaming, idle-timeout: {}). Press Ctrl+C to stop.",
        args.idle_timeout
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
                if args.auto_dream {
                    let _ = dream_engine.run_dream_cycle();
                }
                // Final push attempt before exit
                let _ = sync_engine.push_deltas().await;
                break;
            }
            _ = sleep(tick_duration) => {
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

                if args.auto_dream {
                    if let Err(e) = run_daemon_dream_cycle(&mut dream_engine, Utc::now()) {
                        warn!("Auto-dreaming cycle warning in daemon loop: {e}");
                    }
                }

                // Run context rot detection and decay poisoned memories in background
                let rot_detector = strata_memory::ContextRotDetector::with_default_config();
                if let Ok(decayed) = rot_detector.apply_penalty_decay(&store) {
                    if decayed > 0 {
                        info!("Context rot detector quarantined and decayed {decayed} poisoned memories in daemon cycle");
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
    use clap::Parser;
    use strata_core::schemas::{EpisodeRecord, EpisodeStep, FactStatus, SemanticFact};
    use strata_core::state::{MemoryRecord, MemoryType, Scope};
    use strata_memory::DreamTriggerReason;

    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(flatten)]
        daemon: DaemonArgs,
    }

    #[test]
    fn test_daemon_cli_args_idle_timeout_parsing() {
        let parsed = TestCli::try_parse_from([
            "strata-daemon",
            "--idle-timeout",
            "45s",
            "--dream-sessions",
            "5",
        ])
        .unwrap();
        assert_eq!(parsed.daemon.idle_timeout, "45s");
        assert_eq!(parsed.daemon.dream_sessions, 5);
        assert!(parsed.daemon.auto_dream);
        assert_eq!(
            parse_dream_duration(&parsed.daemon.idle_timeout).unwrap(),
            Duration::from_secs(45)
        );
    }

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

    #[test]
    fn test_daemon_dream_cycle_merges_and_resolves_contradictions_on_idle_and_sessions() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let t0 = Utc::now();

        // 1. Insert duplicate memories and contradicting facts
        let mut dup1 = MemoryRecord::new(
            MemoryType::Semantic,
            "Cache layer uses Redis cluster for session tokens",
            Scope::Project("strata".to_string()),
        );
        dup1.created_at = t0;
        dup1.updated_at = t0;

        let mut dup2 = MemoryRecord::new(
            MemoryType::Semantic,
            "Cache layer uses Redis cluster for session tokens",
            Scope::Project("strata".to_string()),
        );
        dup2.created_at = t0;
        dup2.updated_at = t0;

        store.insert_or_update_memory(&dup1).unwrap();
        store.insert_or_update_memory(&dup2).unwrap();

        let mut old_fact = SemanticFact::new(
            "Database storage engine uses SQLite",
            "architecture",
            Scope::Project("strata".to_string()),
        );
        old_fact.created_at = t0 - chrono::Duration::minutes(5);
        old_fact.last_updated_at = old_fact.created_at;

        let mut new_fact = SemanticFact::new(
            "Database storage engine migrated to PostgreSQL instead of SQLite",
            "architecture",
            Scope::Project("strata".to_string()),
        );
        new_fact.created_at = t0;
        new_fact.last_updated_at = t0;

        store.insert_or_update_semantic_fact(&old_fact).unwrap();
        store.insert_or_update_semantic_fact(&new_fact).unwrap();

        let config = DreamConfig::new(Duration::from_secs(15), 2);
        let mut dream_engine = DreamEngine::new(store.clone(), config);

        // Before idle timeout (5s < 15s): no dream triggered
        let res_early =
            run_daemon_dream_cycle(&mut dream_engine, t0 + chrono::Duration::seconds(5)).unwrap();
        assert!(res_early.is_none());

        // After idle timeout (20s >= 15s): dream cycle triggers and consolidates!
        let report = run_daemon_dream_cycle(&mut dream_engine, t0 + chrono::Duration::seconds(20))
            .unwrap()
            .expect("Expected dream cycle to fire after idle timeout");

        assert!(matches!(
            report.trigger,
            DreamTriggerReason::IdleTimeout { .. }
        ));
        assert_eq!(report.memories_merged, 1);
        assert_eq!(report.contradictions_resolved, 1);

        // Verify duplicate was merged into 1 record
        let memories = store.get_all_memories(None, None, 10).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].access_count, 2);

        // Verify contradiction was resolved (old_fact deprecated, new_fact active)
        let old_f = store.get_semantic_fact(&old_fact.id).unwrap().unwrap();
        assert_eq!(old_f.status, FactStatus::Deprecated);
        assert_eq!(old_f.replaced_by, Some(new_fact.id));

        let new_f = store.get_semantic_fact(&new_fact.id).unwrap().unwrap();
        assert_eq!(new_f.status, FactStatus::Active);

        // 2. Now add 2 new sessions ("s1", "s2") and verify SessionThreshold triggers immediately
        let t1 = t0 + chrono::Duration::seconds(25);
        for sid in ["s1", "s2"] {
            let mut m = MemoryRecord::new(
                MemoryType::Episodic,
                format!("Session {sid} trace"),
                Scope::Session(sid.to_string()),
            );
            m.created_at = t1;
            m.updated_at = t1;
            store.insert_or_update_memory(&m).unwrap();
        }

        let sess_report =
            run_daemon_dream_cycle(&mut dream_engine, t1 + chrono::Duration::seconds(1))
                .unwrap()
                .expect("Expected dream cycle to fire after 2 new sessions");
        assert_eq!(
            sess_report.trigger,
            DreamTriggerReason::SessionThreshold {
                new_sessions: 2,
                threshold: 2
            }
        );
    }
}

