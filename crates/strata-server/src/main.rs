use std::path::PathBuf;
use anyhow::Result;
use clap::Parser;
use strata_server::{run_server, ServerConfig};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(
    name = "strata-server",
    about = "Strata Cloud Sync: Lightweight Axum server for multi-device cognitive memory synchronization",
    version
)]
struct CliArgs {
    /// Port to listen on (default from PORT env var or 8080)
    #[arg(short, long, env = "PORT", default_value = "8080")]
    port: u16,

    /// Host IP address to bind to (default from HOST env var or 0.0.0.0)
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,

    /// Path to persistent SQLite database file
    #[arg(short, long, env = "DATABASE_PATH")]
    db_path: Option<PathBuf>,

    /// Database connection URL (PostgreSQL postgres://... or SQLite sqlite://...)
    #[arg(short = 'u', long, env = "DATABASE_URL")]
    database_url: Option<String>,

    /// Public custom domain (e.g. strata.pedrofarath.me)
    #[arg(short = 'd', long, env = "CUSTOM_DOMAIN")]
    custom_domain: Option<String>,

    /// Bearer authentication token for client verification
    #[arg(short, long, env = "STRATA_SERVER_SECRET")]
    auth_token: Option<String>,

    /// Secret key for signing and verifying JWT session tokens
    #[arg(long, env = "JWT_SECRET")]
    jwt_secret: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing subscriber
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "strata_server=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Install default crypto provider for Rustls (TLS for Supabase/Neon/Railway Postgres)
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = CliArgs::parse();

    let mut config = ServerConfig::default();
    config.port = args.port;
    config.host = args.host;
    if args.database_url.is_some() {
        config.database_url = args.database_url;
    }
    if args.custom_domain.is_some() {
        config.custom_domain = args.custom_domain;
    }
    if args.db_path.is_some() {
        config.db_path = args.db_path;
    }
    if let Some(jwt) = args.jwt_secret {
        config.jwt_secret = jwt;
    }
    if args.auth_token.is_some() {
        config.legacy_secret = args.auth_token;
    }

    run_server(config).await
}
