pub mod auth;
pub mod handlers;
pub mod models;
pub mod security;
pub mod storage;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use anyhow::{Context, Result};
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub use handlers::{AppState, WsBroadcastMsg};
pub use models::*;
pub use storage::ServerStorage;

/// Server configuration structure for standalone or embedded execution.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub db_path: Option<PathBuf>,
    pub jwt_secret: String,
    pub legacy_secret: Option<String>,
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

        let legacy_secret = std::env::var("STRATA_SERVER_SECRET")
            .or_else(|_| std::env::var("STRATA_SYNC_TOKEN"))
            .or_else(|_| std::env::var("STRATA_AUTH_TOKEN"))
            .ok()
            .filter(|s| !s.trim().is_empty());

        let jwt_secret = std::env::var("JWT_SECRET")
            .or_else(|_| std::env::var("STRATA_SERVER_SECRET"))
            .unwrap_or_else(|_| "strata-default-jwt-secret-key-change-in-prod".to_string());

        Self {
            host,
            port,
            db_path,
            jwt_secret,
            legacy_secret,
        }
    }
}

/// Create the Axum router with all SaaS authentication, workspace, API key, and CDC sync routes.
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
        // Self-Serve SaaS Auth endpoints
        .route("/api/v1/auth/signup", post(handlers::signup_handler))
        .route("/api/v1/auth/login", post(handlers::login_handler))
        .route("/api/v1/auth/me", get(handlers::me_handler))
        // Workspace management endpoints
        .route(
            "/api/v1/workspaces",
            post(handlers::create_workspace_handler).get(handlers::list_workspaces_handler),
        )
        // API Key management endpoints
        .route(
            "/api/v1/keys",
            post(handlers::create_key_handler).get(handlers::list_keys_handler),
        )
        .route("/api/v1/keys/{key_id}", delete(handlers::revoke_key_handler))
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
        jwt_secret: config.jwt_secret.clone(),
        legacy_secret: config.legacy_secret.clone(),
        ws_broadcast: ws_tx,
        start_time: Instant::now(),
    });

    let app = create_app(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind TCP listener on {}", addr))?;

    tracing::info!("🚀 Strata Cloud SaaS Server listening on http://{}", addr);
    tracing::info!("🔒 JWT Authentication is ENABLED.");
    if config.legacy_secret.is_some() {
        tracing::info!("🔑 Legacy static token authentication is ENABLED.");
    }

    axum::serve(listener, app)
        .await
        .context("Server error during execution")?;

    Ok(())
}
