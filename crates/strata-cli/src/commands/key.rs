use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use reqwest::Client;
use serde_json::Value;
use uuid::Uuid;

#[derive(Args, Debug, Clone)]
pub struct KeyArgs {
    #[command(subcommand)]
    pub action: KeyAction,

    #[arg(long, global = true, env = "STRATA_SYNC_ENDPOINT", default_value = "https://strata.pedrofarath.me", help = "Strata Cloud Server URL")]
    pub endpoint: String,

    #[arg(long, global = true, env = "STRATA_AUTH_TOKEN", help = "Bearer Session JWT Token")]
    pub token: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum KeyAction {
    /// Create a new machine API Key (e.g. for Cursor, Claude, Codex, Gemini)
    Create {
        #[arg(short, long, help = "Descriptive key name (e.g. 'MacBook Pro Cursor')")]
        name: String,

        #[arg(short, long, help = "Workspace UUID")]
        workspace_id: Uuid,

        #[arg(long, help = "Key expiration in days (optional)")]
        expires_days: Option<u32>,
    },

    /// List active API Keys for a workspace
    List {
        #[arg(short, long, help = "Workspace UUID")]
        workspace_id: Uuid,
    },

    /// Revoke an API Key
    Revoke {
        #[arg(help = "Key UUID to revoke")]
        key_id: Uuid,
    },
}

pub async fn run_key(args: KeyArgs) -> Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let jwt = args.token.or_else(|| std::env::var("STRATA_AUTH_TOKEN").ok())
        .context("Authentication required. Pass --token or set STRATA_AUTH_TOKEN (from `strata auth login`)")?;

    match args.action {
        KeyAction::Create { name, workspace_id, expires_days } => {
            let payload = serde_json::json!({
                "workspace_id": workspace_id,
                "name": name,
                "expires_days": expires_days
            });

            let resp = client
                .post(format!("{}/api/v1/keys", args.endpoint.trim_end_matches('/')))
                .bearer_auth(&jwt)
                .json(&payload)
                .send()
                .await
                .context("Failed to connect to Strata Cloud server")?;

            if resp.status().is_success() {
                let data: Value = resp.json().await?;
                let key_secret = data["key"].as_str().unwrap_or_default();
                let key_id = data["id"].as_str().unwrap_or_default();

                println!("\n🔑 [Strata API Key Generated]");
                println!("══════════════════════════════════════════════════════");
                println!("Name:       {}", data["name"].as_str().unwrap_or_default());
                println!("Key ID:     {key_id}");
                println!("API Secret: {key_secret}");
                println!("══════════════════════════════════════════════════════");
                println!("⚠️ IMPORTANT: Save this secret! It will not be shown again.");
                println!("\nTo configure on this machine:");
                println!("  export STRATA_SYNC_ENDPOINT=\"{}\"", args.endpoint);
                println!("  export STRATA_SYNC_TOKEN=\"{key_secret}\"");
            } else {
                let status = resp.status();
                let body: Value = resp.json().await.unwrap_or_default();
                let err_msg = body["error"].as_str().unwrap_or("Failed to create key");
                anyhow::bail!("HTTP {status}: {err_msg}");
            }
        }
        KeyAction::List { workspace_id } => {
            let resp = client
                .get(format!("{}/api/v1/keys?workspace_id={workspace_id}", args.endpoint.trim_end_matches('/')))
                .bearer_auth(&jwt)
                .send()
                .await
                .context("Failed to connect to Strata Cloud server")?;

            if resp.status().is_success() {
                let keys: Vec<Value> = resp.json().await?;
                println!("\n🔑 [Strata API Keys for Workspace {workspace_id}]");
                println!("══════════════════════════════════════════════════════");
                if keys.is_empty() {
                    println!("No active API keys found.");
                } else {
                    for k in keys {
                        let id = k["id"].as_str().unwrap_or_default();
                        let name = k["name"].as_str().unwrap_or_default();
                        let prefix = k["key_prefix"].as_str().unwrap_or_default();
                        let last_used = k["last_used_at"].as_str().unwrap_or("never");
                        println!("• {name} [{prefix}...] (ID: {id}) | Last used: {last_used}");
                    }
                }
                println!("══════════════════════════════════════════════════════");
            } else {
                let status = resp.status();
                anyhow::bail!("Failed to list keys (HTTP {status})");
            }
        }
        KeyAction::Revoke { key_id } => {
            let resp = client
                .delete(format!("{}/api/v1/keys/{key_id}", args.endpoint.trim_end_matches('/')))
                .bearer_auth(&jwt)
                .send()
                .await
                .context("Failed to connect to Strata Cloud server")?;

            if resp.status().is_success() {
                println!("✓ API Key {key_id} revoked successfully.");
            } else {
                let status = resp.status();
                anyhow::bail!("Failed to revoke key (HTTP {status})");
            }
        }
    }

    Ok(())
}
