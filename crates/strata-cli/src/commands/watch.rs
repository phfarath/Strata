use anyhow::{Context, Result};
use clap::Args;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

use strata_memory::{CodeAnchorEngine, SqliteStore, TruthMaintenanceSystem};
use strata_reasoning::WorldModel;

#[derive(Debug, Args, Clone)]
pub struct WatchArgs {
    /// Workspace root directory to watch for AST code drift
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,

    /// Debounce duration in milliseconds for file events
    #[arg(long, default_value_t = 500)]
    pub debounce_ms: u64,

    /// Single-pass mode: scan for drift once and exit without entering continuous watch loop
    #[arg(long)]
    pub once: bool,

    /// Output report in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftEventSummary {
    pub file_path: String,
    pub direct_stale_facts: Vec<Uuid>,
    pub cascaded_stale_facts: Vec<Uuid>,
    pub total_invalidated: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchSweepReport {
    pub workspace: String,
    pub total_facts_scanned: usize,
    pub merkle_root_after: Option<String>,
    pub events: Vec<DriftEventSummary>,
}

/// Returns true if the path points to or inside a noisy or build directory that should be ignored.
pub fn is_ignored_path(path: &Path) -> bool {
    for comp in path.components() {
        if let std::path::Component::Normal(os_str) = comp {
            let s = os_str.to_string_lossy();
            if s == ".git"
                || s == "target"
                || s == ".strata"
                || s == "node_modules"
                || s == "dist"
                || s == ".cargo"
                || s == "build"
                || s == "venv"
                || s == ".venv"
                || s.starts_with('.')
            {
                return true;
            }
        }
    }

    if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
        if file_name.starts_with('.')
            || file_name.ends_with(".tmp")
            || file_name.ends_with('~')
            || file_name.ends_with(".swp")
        {
            return true;
        }
    }

    false
}

/// Returns true if the path is a supported source file (.rs, .ts, .tsx, .js, .jsx, .py, .pyi)
/// and not inside an ignored directory.
pub fn is_supported_source_file(path: &Path) -> bool {
    if is_ignored_path(path) {
        return false;
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "pyi")
    } else {
        false
    }
}

/// Performs a full workspace AST reconciliation, checks for code drift against code anchors,
/// and propagates cascading invalidation in the JTMS for all downstream facts.
pub async fn check_and_reconcile_drift(
    workspace_root: &Path,
    store: &SqliteStore,
    world_model: &WorldModel,
    json: bool,
) -> Result<WatchSweepReport> {
    let anchor_engine = CodeAnchorEngine::new();
    let report = anchor_engine
        .reconcile_workspace_dir(store, workspace_root, None, Some(world_model))
        .await
        .context("Failed to reconcile workspace directory")?;

    let jtms = TruthMaintenanceSystem::with_default_threshold();
    let mut file_events: HashMap<String, DriftEventSummary> = HashMap::new();

    if !report.stale_facts.is_empty() {
        for stale_id in &report.stale_facts {
            let fact = store.get_semantic_fact(stale_id)?;
            let file_path = fact
                .as_ref()
                .and_then(|f| f.code_anchor.as_ref())
                .map(|a| a.file_path.replace('\\', "/"))
                .unwrap_or_else(|| "unknown".to_string());

            let cascaded_ids = jtms
                .propagate_downstream_invalidation(store, stale_id, "AST code drift watcher")
                .context("Failed to propagate JTMS downstream invalidation")?;

            let entry = file_events
                .entry(file_path.clone())
                .or_insert_with(|| DriftEventSummary {
                    file_path: file_path.clone(),
                    direct_stale_facts: Vec::new(),
                    cascaded_stale_facts: Vec::new(),
                    total_invalidated: 0,
                });

            if !entry.direct_stale_facts.contains(stale_id) {
                entry.direct_stale_facts.push(*stale_id);
            }
            for cid in cascaded_ids {
                if !entry.cascaded_stale_facts.contains(&cid) {
                    entry.cascaded_stale_facts.push(cid);
                }
            }
        }

        for entry in file_events.values_mut() {
            entry.total_invalidated =
                entry.direct_stale_facts.len() + entry.cascaded_stale_facts.len();
        }

        if !json {
            for (file, entry) in &file_events {
                println!(
                    "⚠️ [STRATA WATCH] Code drift detected in '{}': {} facts transitioned to Stale (JTMS invalidation cascade)",
                    file, entry.total_invalidated
                );
            }
        }
    }

    let sweep_report = WatchSweepReport {
        workspace: workspace_root.display().to_string(),
        total_facts_scanned: report.total_facts_scanned,
        merkle_root_after: report.merkle_root_after,
        events: file_events.into_values().collect(),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&sweep_report)?);
    }

    Ok(sweep_report)
}

/// Runs the real-time file watcher loop or single-pass drift scan.
pub async fn run_watch(args: WatchArgs, store: Arc<SqliteStore>) -> Result<()> {
    let workspace_root = if args.workspace.is_relative() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        cwd.join(&args.workspace)
    } else {
        args.workspace.clone()
    };
    let workspace_root = workspace_root.canonicalize().unwrap_or(workspace_root);

    if !workspace_root.exists() {
        anyhow::bail!(
            "Workspace directory does not exist: {}",
            workspace_root.display()
        );
    }

    let world_model = WorldModel::new();
    let _ = world_model.index_workspace(&workspace_root).await;

    // Run initial drift sweep
    let initial_report =
        check_and_reconcile_drift(&workspace_root, store.as_ref(), &world_model, args.json).await?;

    if args.once {
        if !args.json && initial_report.events.is_empty() {
            println!("✅ [STRATA WATCH] No code drift detected. All code anchors intact.");
        }
        return Ok(());
    }

    if !args.json {
        println!(
            "👀 [STRATA WATCH] Monitoring '{}' for AST code drift (debounce: {}ms)...",
            workspace_root.display(),
            args.debounce_ms
        );
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .context("Failed to initialize file watcher")?;

    watcher
        .watch(&workspace_root, RecursiveMode::Recursive)
        .with_context(|| {
            format!(
                "Failed to watch workspace root: {}",
                workspace_root.display()
            )
        })?;

    let debounce_duration = Duration::from_millis(args.debounce_ms);
    let mut pending_paths: HashSet<PathBuf> = HashSet::new();
    let sleep = tokio::time::sleep(debounce_duration);
    tokio::pin!(sleep);
    let mut timer_active = false;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C interrupt signal. Shutting down Strata drift watcher...");
                break;
            }
            Some(res) = rx.recv() => {
                match res {
                    Ok(event) => {
                        for path in event.paths {
                            let rel = path.strip_prefix(&workspace_root).unwrap_or(&path);
                            if is_supported_source_file(rel) {
                                debug!("Supported file changed: {}", path.display());
                                pending_paths.insert(path);
                                sleep.as_mut().reset(tokio::time::Instant::now() + debounce_duration);
                                timer_active = true;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Notify watcher event error: {e}");
                    }
                }
            }
            _ = &mut sleep, if timer_active => {
                timer_active = false;
                if !pending_paths.is_empty() {
                    pending_paths.clear();
                    let _ = check_and_reconcile_drift(
                        &workspace_root,
                        store.as_ref(),
                        &world_model,
                        args.json,
                    )
                    .await;
                }
            }
        }
    }

    Ok(())
}
