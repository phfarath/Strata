pub mod auth;
pub mod handlers;
pub mod storage;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use anyhow::{Context, Result};
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub use handlers::{AppState, WsBroadcastMsg};
pub use storage::ServerStorage;

/// Server configuration structure for standalone or embedded execution.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub db_path: Option<PathBuf>,
    pub auth_token: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(8080);

        let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let db_path = std::env::var("DATABASE_PATH")
            .or_else(|_| std::env::var("DATA_DIR").map(|d| format!("{d}/strata_sync.db")))
            .ok()
            .map(PathBuf::from);

        let auth_token = std::env::var("STRATA_SERVER_SECRET")
            .or_else(|_| std::env::var("STRATA_SYNC_TOKEN"))
            .or_else(|_| std::env::var("STRATA_AUTH_TOKEN"))
            .ok()
            .filter(|s| !s.trim().is_empty());

        Self {
            host,
            port,
            db_path,
            auth_token,
        }
    }
}

/// Create the Axum router with all sync, status, health, and WebSocket routes.
pub fn create_app(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health check endpoints
        .route("/health", get(handlers::health_handler))
        .route("/api/health", get(handlers::health_handler))
        .route("/api/v1/health", get(handlers::health_handler))
        // Push deltas endpoints
        .route("/", post(handlers::push_handler))
        .route("/sync", post(handlers::push_handler))
        .route("/sync/push", post(handlers::push_handler))
        .route("/api/v1/sync/push", post(handlers::push_handler))
        // Pull deltas endpoints
        .route("/pull", get(handlers::pull_handler))
        .route("/sync/pull", get(handlers::pull_handler))
        .route("/api/v1/sync/pull", get(handlers::pull_handler))
        // Status endpoints
        .route("/status", get(handlers::status_handler))
        .route("/sync/status", get(handlers::status_handler))
        .route("/api/v1/sync/status", get(handlers::status_handler))
        // Realtime WebSocket stream
        .route("/ws", get(handlers::ws_handler))
        .route("/sync/ws", get(handlers::ws_handler))
        .route("/api/v1/sync/ws", get(handlers::ws_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Initialize state and launch the Axum server.
pub async fn run_server(config: ServerConfig) -> Result<()> {
    let storage = match config.db_path {
        Some(ref path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create parent directory for {:?}", path))?;
            }
            ServerStorage::open(path)
                .with_context(|| format!("Failed to open SQLite database at {:?}", path))?
        }
        None => {
            tracing::warn!("No DATABASE_PATH specified; using ephemeral in-memory storage.");
            ServerStorage::in_memory().context("Failed to initialize in-memory storage")?
        }
    };

    let (ws_tx, _) = tokio::sync::broadcast::channel(256);

    let state = Arc::new(AppState {
        storage,
        auth_token: config.auth_token.clone(),
        ws_broadcast: ws_tx,
        start_time: Instant::now(),
    });

    let app = create_app(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind TCP listener on {}", addr))?;

    tracing::info!("🚀 Strata Cloud Sync Server listening on http://{}", addr);
    if config.auth_token.is_some() {
        tracing::info!("🔒 Bearer authentication is ENABLED.");
    } else {
        tracing::info!("🔓 Authentication is DISABLED (open mode).");
    }

    axum::serve(listener, app)
        .await
        .context("Server error during execution")?;

    Ok(())
}
