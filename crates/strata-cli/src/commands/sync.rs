use std::sync::Arc;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use strata_core::schemas::SyncConfig;
use strata_memory::{SqliteStore, SyncEngine};

#[derive(Args, Debug, Clone)]
pub struct SyncArgs {
    #[command(subcommand)]
    pub action: Option<SyncAction>,

    #[arg(long, global = true, help = "Remote synchronization endpoint URL")]
    pub endpoint: Option<String>,

    #[arg(long, global = true, help = "Bearer authentication token for remote endpoint")]
    pub token: Option<String>,

    #[arg(long, global = true, default_value = "default", help = "Workspace identifier")]
    pub workspace: String,

    #[arg(long, global = true, help = "Output report as JSON")]
    pub json: bool,
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
    let workspace = if args.workspace != "default" {
        args.workspace.clone()
    } else {
        StrataConfig::resolve_workspace(Some(&args.workspace))
    };

    let mut config = SyncConfig::new(&workspace);
    config.endpoint = Some(StrataConfig::resolve_endpoint(args.endpoint.as_deref()));
    config.token = StrataConfig::resolve_token(args.token.as_deref());

    let sync_engine = SyncEngine::new(store.clone(), config.clone());

    match args.action.unwrap_or(SyncAction::Status) {
        SyncAction::Status => {
            let (pending_count, max_seq) = store.get_sync_status(&config.workspace_id)
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
                println!("Mode:             {}", if config.endpoint.is_some() { "Connected" } else { "Offline-First (Local)" });
                if let Some(ep) = &config.endpoint {
                    println!("Endpoint:         {ep}");
                }
                println!("Pending Deltas:   {pending_count}");
                println!("Max Local Seq:    {max_seq}");
                println!("Last Remote Seq:  {last_remote_seq}");
            }
        }
        SyncAction::Push => {
            let count = sync_engine.push_deltas().await
                .context("Failed to push sync deltas")?;

            if args.json {
                println!("{}", serde_json::json!({
                    "action": "push",
                    "workspace_id": config.workspace_id,
                    "pushed_count": count,
                    "status": "success"
                }));
            } else {
                println!("✓ Successfully pushed {count} delta(s) to remote sync endpoint.");
            }
        }
        SyncAction::Pull => {
            let remote_deltas = sync_engine.pull_remote().await
                .context("Failed to pull remote deltas")?;
            let count = remote_deltas.len();
            let applied = sync_engine.pull_deltas(remote_deltas).await
                .context("Failed to apply remote deltas")?;

            if args.json {
                println!("{}", serde_json::json!({
                    "action": "pull",
                    "workspace_id": config.workspace_id,
                    "pulled_count": count,
                    "applied_count": applied,
                    "status": "success"
                }));
            } else {
                println!("✓ Successfully pulled {count} delta(s), applied {applied} with JTMS conflict resolution.");
            }
        }
    }

    Ok(())
}
