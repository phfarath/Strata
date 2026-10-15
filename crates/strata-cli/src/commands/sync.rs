use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;
use strata_core::schemas::SyncConfig;
use strata_memory::{SqliteStore, SyncEngine};

#[derive(Args, Debug, Clone)]
pub struct SyncArgs {
    #[command(subcommand)]
    pub action: Option<SyncAction>,

    #[arg(long, global = true, help = "Remote synchronization endpoint URL")]
    pub endpoint: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Bearer authentication token for remote endpoint"
    )]
    pub token: Option<String>,

    #[arg(long, global = true, help = "Workspace identifier")]
    pub workspace: Option<String>,

    #[arg(long, global = true, help = "Output report as JSON")]
    pub json: bool,

    #[arg(
        long = "merge-branch",
        help = "Merge and promote memories from a feature branch into target branch (default: main)"
    )]
    pub merge_branch: Option<String>,

    #[arg(
        long = "target-branch",
        default_value = "main",
        help = "Target branch to promote memories into (default: main)"
    )]
    pub target_branch: String,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SyncAction {
    /// Push pending local deltas to remote endpoint
    Push,
    /// Pull remote deltas and resolve conflicts
    Pull,
    /// Display current synchronization status and outbox metrics
    Status,
}

use crate::config::StrataConfig;

pub async fn run_sync(args: SyncArgs, store: Arc<SqliteStore>) -> Result<()> {
    // If --merge-branch is passed, perform atomic memory branch merge and promotion
    if let Some(ref source_branch) = args.merge_branch {
        let report = store
            .merge_branch_memories(source_branch, &args.target_branch)
            .context("Failed to merge branch memories")?;

        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!("\n🔀 [Strata Memory Branch Merge]");
            println!("═════════════════════════════════════════");
            println!("Source Branch:            {}", report.source_branch);
            println!("Target Branch:            {}", report.target_branch);
            println!("Memories Merged:          {}", report.memories_merged);
            println!("Semantic Facts Merged:    {}", report.semantic_facts_merged);
            println!("Events Merged:            {}", report.events_merged);
            println!("Episodic Memories Merged: {}", report.episodic_memories_merged);
            println!("Episodes Merged:          {}", report.episodes_merged);
            println!("─────────────────────────────────────────");
            println!("Total Records Promoted:   {}", report.total_merged());
            println!("✓ Successfully unified memories into '{}'.", report.target_branch);
        }
        return Ok(());
    }

    let workspace = StrataConfig::resolve_workspace(args.workspace.as_deref());

    let mut config = SyncConfig::new(&workspace);
    config.endpoint = Some(StrataConfig::resolve_endpoint(args.endpoint.as_deref()));
    config.token = StrataConfig::resolve_token(args.token.as_deref());

    let sync_engine = SyncEngine::new(store.clone(), config.clone());

    match args.action.unwrap_or(SyncAction::Status) {
        SyncAction::Status => {
            let (pending_count, max_seq) = store
                .get_sync_status(&config.workspace_id)
                .context("Failed to query sync status")?;
            let last_remote_seq = store
                .get_sync_metadata("last_remote_seq")
                .unwrap_or(None)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            if args.json {
                let status_json = serde_json::json!({
                    "workspace_id": config.workspace_id,
                    "endpoint": config.endpoint,
                    "pending_deltas": pending_count,
                    "max_local_seq": max_seq,
                    "last_remote_seq": last_remote_seq,
                    "mode": if config.endpoint.is_some() { "connected" } else { "offline_first" }
                });
                println!("{}", serde_json::to_string_pretty(&status_json)?);
            } else {
                println!("\n🔄 [Strata Sync Status]");
                println!("═════════════════════════════════════════");
                println!("Workspace ID:     {}", config.workspace_id);
                println!(
                    "Mode:             {}",
                    if config.endpoint.is_some() {
                        "Connected"
                    } else {
                        "Offline-First (Local)"
                    }
                );
                if let Some(ep) = &config.endpoint {
                    println!("Endpoint:         {ep}");
                }
                println!("Pending Deltas:   {pending_count}");
                println!("Max Local Seq:    {max_seq}");
                println!("Last Remote Seq:  {last_remote_seq}");
            }
        }
        SyncAction::Push => {
            let count = sync_engine
                .push_deltas()
                .await
                .context("Failed to push sync deltas")?;

            if args.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "action": "push",
                        "workspace_id": config.workspace_id,
                        "pushed_count": count,
                        "status": "success"
                    })
                );
            } else {
                println!("✓ Successfully pushed {count} delta(s) to remote sync endpoint.");
            }
        }
        SyncAction::Pull => {
            let remote_deltas = sync_engine
                .pull_remote()
                .await
                .context("Failed to pull remote deltas")?;
            let count = remote_deltas.len();
            let applied = sync_engine
                .pull_deltas(remote_deltas)
                .await
                .context("Failed to apply remote deltas")?;

            if args.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "action": "pull",
                        "workspace_id": config.workspace_id,
                        "pulled_count": count,
                        "applied_count": applied,
                        "status": "success"
                    })
                );
            } else {
                println!("✓ Successfully pulled {count} delta(s), applied {applied} with JTMS conflict resolution.");
            }
        }
    }

    Ok(())
}
