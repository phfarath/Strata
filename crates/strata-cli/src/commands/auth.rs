use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use reqwest::Client;
use serde_json::Value;

#[derive(Args, Debug, Clone)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub action: AuthAction,

    #[arg(
        long,
        global = true,
        env = "STRATA_SYNC_ENDPOINT",
        default_value = "https://strata.pedrofarath.me",
        help = "Strata Cloud Server URL"
    )]
    pub endpoint: String,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthAction {
    /// Create a new Strata Cloud developer account
    Signup {
        #[arg(short, long, help = "Account email address")]
        email: String,

        #[arg(short, long, help = "Account password (min 8 characters)")]
        password: String,

        #[arg(short, long, help = "Developer full name")]
        name: String,

        #[arg(short, long, help = "Default workspace name")]
        workspace: Option<String>,
    },

    /// Log in to an existing Strata Cloud account
    Login {
        #[arg(short, long, help = "Account email address")]
        email: String,

        #[arg(short, long, help = "Account password")]
        password: String,
    },

    /// Check current authenticated account details and workspaces
    Whoami {
        #[arg(
            short,
            long,
            env = "STRATA_AUTH_TOKEN",
            help = "Bearer Session JWT Token"
        )]
        token: Option<String>,
    },
}

pub async fn run_auth(args: AuthArgs) -> Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    match args.action {
        AuthAction::Signup {
            email,
            password,
            name,
            workspace,
        } => {
            let payload = serde_json::json!({
                "email": email,
                "password": password,
                "full_name": name,
                "workspace_name": workspace
            });

            let resp = client
                .post(format!(
                    "{}/api/v1/auth/signup",
                    args.endpoint.trim_end_matches('/')
                ))
                .json(&payload)
                .send()
                .await
                .context("Failed to connect to Strata Cloud Auth server")?;

            if resp.status().is_success() {
                let data: Value = resp.json().await?;
                let token = data["token"].as_str().unwrap_or_default();
                let user_name = data["user"]["full_name"].as_str().unwrap_or_default();
                let ws_slug = data["workspaces"][0]["slug"].as_str().unwrap_or_default();
                let ws_id = data["workspaces"][0]["id"].as_str().unwrap_or_default();

                println!("\n🎉 [Strata Cloud Account Created]");
                println!("══════════════════════════════════════════════════════");
                println!("Developer:   {user_name} ({email})");
                println!("Workspace:   {ws_slug} (ID: {ws_id})");
                println!("Session JWT: {token}");
                println!("══════════════════════════════════════════════════════");
                println!("\n💡 Tip: You can now create an API Key with `strata key create --workspace-id {ws_id} --token {token}`");
            } else {
                let status = resp.status();
                let body: Value = resp.json().await.unwrap_or_default();
                let err_msg = body["error"].as_str().unwrap_or("Signup failed");
                anyhow::bail!("HTTP {status}: {err_msg}");
            }
        }
        AuthAction::Login { email, password } => {
            let payload = serde_json::json!({
                "email": email,
                "password": password
            });

            let resp = client
                .post(format!(
                    "{}/api/v1/auth/login",
                    args.endpoint.trim_end_matches('/')
                ))
                .json(&payload)
                .send()
                .await
                .context("Failed to connect to Strata Cloud Auth server")?;

            if resp.status().is_success() {
                let data: Value = resp.json().await?;
                let token = data["token"].as_str().unwrap_or_default();
                let user_name = data["user"]["full_name"].as_str().unwrap_or_default();

                println!("\n🔑 [Strata Cloud Login Successful]");
                println!("══════════════════════════════════════════════════════");
                println!("Welcome back, {user_name}!");
                println!("Session JWT: {token}");
                if let Some(workspaces) = data["workspaces"].as_array() {
                    println!("\nWorkspaces:");
                    for ws in workspaces {
                        let name = ws["name"].as_str().unwrap_or_default();
                        let slug = ws["slug"].as_str().unwrap_or_default();
                        let id = ws["id"].as_str().unwrap_or_default();
                        println!("  • {name} [{slug}] -> {id}");
                    }
                }
                println!("══════════════════════════════════════════════════════");
            } else {
                let status = resp.status();
                let body: Value = resp.json().await.unwrap_or_default();
                let err_msg = body["error"]
                    .as_str()
                    .unwrap_or("Invalid email or password");
                anyhow::bail!("HTTP {status}: {err_msg}");
            }
        }
        AuthAction::Whoami { token } => {
            let jwt = token
                .or_else(|| std::env::var("STRATA_AUTH_TOKEN").ok())
                .context("No session token provided. Pass --token or set STRATA_AUTH_TOKEN")?;

            let resp = client
                .get(format!(
                    "{}/api/v1/auth/me",
                    args.endpoint.trim_end_matches('/')
                ))
                .bearer_auth(&jwt)
                .send()
                .await
                .context("Failed to verify session with Strata Cloud server")?;

            if resp.status().is_success() {
                let data: Value = resp.json().await?;
                let user = &data["user"];
                println!("\n👤 [Strata Cloud Session]");
                println!("══════════════════════════════════════════════════════");
                println!(
                    "Name:    {}",
                    user["full_name"].as_str().unwrap_or_default()
                );
                println!("Email:   {}", user["email"].as_str().unwrap_or_default());
                println!("Tier:    {}", user["tier"].as_str().unwrap_or_default());
                if let Some(workspaces) = data["workspaces"].as_array() {
                    println!("\nWorkspaces ({}):", workspaces.len());
                    for ws in workspaces {
                        let name = ws["name"].as_str().unwrap_or_default();
                        let slug = ws["slug"].as_str().unwrap_or_default();
                        let id = ws["id"].as_str().unwrap_or_default();
                        println!("  • {name} ({slug}) -> ID: {id}");
                    }
                }
                println!("══════════════════════════════════════════════════════");
            } else {
                let status = resp.status();
                anyhow::bail!("Session invalid or expired (HTTP {status})");
            }
        }
    }

    Ok(())
}
