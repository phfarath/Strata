use anyhow::{bail, Context, Result};
use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use clap::Args;
use rand::Rng;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::config::StrataConfig;

#[derive(Args, Debug, Clone)]
pub struct LoginArgs {
    #[arg(
        long,
        env = "STRATA_SYNC_ENDPOINT",
        help = "Strata Cloud Server URL (default: https://strata.pedrofarath.me)"
    )]
    pub endpoint: Option<String>,

    #[arg(
        long,
        help = "Do not open default browser automatically (prints URL for manual copy/paste)"
    )]
    pub no_browser: bool,

    #[arg(long, help = "Specific local loopback port for callback")]
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub token: String,
    pub state: String,
    pub workspace_id: Option<String>,
    pub workspace_slug: Option<String>,
    pub user_email: Option<String>,
}

#[derive(Debug)]
struct AuthCallbackData {
    token: String,
    workspace_id: Option<String>,
    workspace_slug: Option<String>,
    user_email: Option<String>,
}

pub async fn run_login(args: LoginArgs) -> Result<()> {
    let endpoint = StrataConfig::resolve_endpoint(args.endpoint.as_deref());
    let clean_endpoint = endpoint.trim_end_matches('/').to_string();

    // 1. Generate cryptographically random 32-char hex state for anti-CSRF
    let mut rng = rand::thread_rng();
    let state_bytes: [u8; 16] = rng.gen();
    let expected_state = hex::encode(state_bytes);

    // 2. Bind local loopback TCP listener on 127.0.0.1
    let bind_port = args.port.unwrap_or(0);
    let addr = SocketAddr::from(([127, 0, 0, 1], bind_port));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind local loopback server on {}", addr))?;

    let actual_port = listener.local_addr()?.port();

    // 3. Channel to receive callback data
    let (tx, rx) = oneshot::channel::<AuthCallbackData>();
    let tx_arc = Arc::new(tokio::sync::Mutex::new(Some(tx)));
    let expected_state_clone = expected_state.clone();

    // 4. Build local loopback Axum router
    let app = Router::new().route(
        "/callback",
        get(move |Query(query): Query<CallbackQuery>| {
            let tx_arc = tx_arc.clone();
            let exp_state = expected_state_clone.clone();
            async move {
                if query.state != exp_state {
                    return Html(
                        r#"<!DOCTYPE html><html><body style="background:#090d16;color:#ef4444;font-family:sans-serif;text-align:center;padding:3rem;"><h1>✗ Authentication Failed</h1><p>State token mismatch. Please retry login.</p></body></html>"#.to_string()
                    );
                }

                if let Some(sender) = tx_arc.lock().await.take() {
                    let _ = sender.send(AuthCallbackData {
                        token: query.token.clone(),
                        workspace_id: query.workspace_id,
                        workspace_slug: query.workspace_slug,
                        user_email: query.user_email,
                    });
                }

                Html(
                    r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>✓ Authenticated</title>
  <style>
    body { background: #090d16; color: #f8fafc; font-family: system-ui, sans-serif; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }
    .card { background: #131b2e; border: 1px solid #1e293b; padding: 2.5rem; border-radius: 1rem; text-align: center; max-width: 400px; box-shadow: 0 25px 50px -12px rgba(0,0,0,0.5); }
    h1 { color: #38bdf8; font-size: 1.5rem; margin-bottom: 0.5rem; }
    p { color: #94a3b8; font-size: 0.95rem; line-height: 1.5; margin-bottom: 1.5rem; }
    .badge { display: inline-block; padding: 0.35rem 0.85rem; background: rgba(56, 189, 248, 0.15); border: 1px solid #38bdf8; color: #38bdf8; border-radius: 9999px; font-size: 0.85rem; font-weight: 600; }
  </style>
</head>
<body>
  <div class="card">
    <h1>✓ Strata Connected</h1>
    <p>Your terminal has been successfully authenticated. You can safely close this browser window.</p>
    <div class="badge">Session Active</div>
  </div>
</body>
</html>"#
                        .to_string(),
                )
            }
        }),
    );

    // 5. Spawn background loopback HTTP server
    let server_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // 6. Form authentication URL
    let auth_url = format!("{clean_endpoint}/auth/cli?port={actual_port}&state={expected_state}");

    println!("\n⚡ [Strata Cloud Login]");
    println!("══════════════════════════════════════════════════════");
    println!("Server:    {clean_endpoint}");
    println!("Callback:  http://127.0.0.1:{actual_port}/callback");
    println!("══════════════════════════════════════════════════════");

    if !args.no_browser {
        println!("🌐 Opening your browser for authentication...");
        if let Err(e) = opener::open(&auth_url) {
            tracing::warn!("Could not open browser automatically: {e}");
            println!("\n👉 Please open this link manually in your browser:\n   {auth_url}\n");
        }
    } else {
        println!("\n👉 Open this link in your browser:\n   {auth_url}\n");
    }

    println!("⏳ Waiting for authentication in browser (timeout: 120s)...");

    // 7. Await callback with 120s timeout
    let callback_result = tokio::time::timeout(Duration::from_secs(120), rx).await;

    // Terminate local loopback server
    server_handle.abort();

    let data = match callback_result {
        Ok(Ok(d)) => d,
        Ok(Err(_)) => bail!("Authentication server closed unexpectedly"),
        Err(_) => {
            bail!("Authentication timed out after 120 seconds. Please run `strata login` again.")
        }
    };

    // 8. Save configuration to ~/.strata/config.toml
    let mut config = StrataConfig::load();
    config.endpoint = Some(clean_endpoint.clone());
    config.token = Some(data.token.clone());
    config.workspace_id = data.workspace_id.clone();
    config.workspace_slug = data.workspace_slug.clone();
    config.user_email = data.user_email.clone();
    config.save()?;

    let user_display = data.user_email.as_deref().unwrap_or("Developer");
    let ws_display = data.workspace_slug.as_deref().unwrap_or("default");

    println!("\n🎉 [Authentication Successful]");
    println!("══════════════════════════════════════════════════════");
    println!("User:       {user_display}");
    println!("Workspace:  {ws_display}");
    println!("Server:     {clean_endpoint}");
    println!("Config:     {}", StrataConfig::config_path()?.display());
    println!("══════════════════════════════════════════════════════");
    println!("✓ API Key generated and saved automatically.");
    println!("🚀 You can now run `strata sync push` and `strata sync pull` with no extra setup!\n");

    Ok(())
}

pub async fn run_logout() -> Result<()> {
    StrataConfig::clear()?;
    println!("✓ Successfully logged out from Strata Cloud. Saved credentials removed.");
    Ok(())
}
