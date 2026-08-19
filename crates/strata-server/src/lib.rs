pub mod auth;
pub mod handlers;
pub mod models;
pub mod security;
pub mod storage;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use anyhow::{Context, Result};
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
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
    pub database_url: Option<String>,
    pub db_path: Option<PathBuf>,
    pub custom_domain: Option<String>,
    pub cors_allowed_origins: Vec<String>,
    pub enable_security_headers: bool,
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

        let database_url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("POSTGRES_URL"))
            .or_else(|_| std::env::var("SUPABASE_DB_URL"))
            .ok()
            .filter(|s| !s.trim().is_empty());

        let db_path = std::env::var("DATABASE_PATH")
            .or_else(|_| std::env::var("DATA_DIR").map(|d| format!("{d}/strata_sync.db")))
            .ok()
            .map(PathBuf::from);

        let custom_domain = std::env::var("CUSTOM_DOMAIN")
            .or_else(|_| std::env::var("STRATA_DOMAIN"))
            .or_else(|_| std::env::var("RAILWAY_PUBLIC_DOMAIN"))
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| Some("strata.pedrofarath.me".to_string()));

        let cors_allowed_origins = std::env::var("CORS_ALLOWED_ORIGINS")
            .map(|s| {
                s.split(',')
                    .map(|o| o.trim().to_string())
                    .filter(|o| !o.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let enable_security_headers = std::env::var("ENABLE_SECURITY_HEADERS")
            .map(|s| s.to_lowercase() != "false" && s != "0")
            .unwrap_or(true);

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
            database_url,
            db_path,
            custom_domain,
            cors_allowed_origins,
            enable_security_headers,
            jwt_secret,
            legacy_secret,
        }
    }
}

/// Security headers middleware that injects HSTS, CSP, X-Frame-Options, and nosniff.
async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();

    // HSTS (HTTP Strict Transport Security) - 2 years + subdomains + preload
    headers.insert(
        axum::http::header::STRICT_TRANSPORT_SECURITY,
        axum::http::HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );

    // Prevent MIME-sniffing
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );

    // Clickjacking protection (X-Frame-Options: DENY)
    headers.insert(
        axum::http::header::X_FRAME_OPTIONS,
        axum::http::HeaderValue::from_static("DENY"),
    );

    // Referrer Policy
    headers.insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Content Security Policy (CSP)
    headers.insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data:; connect-src 'self' https: wss: ws: http:; frame-ancestors 'none';",
        ),
    );

    // Permissions Policy
    if let Ok(header_name) = axum::http::HeaderName::from_lowercase(b"permissions-policy") {
        headers.insert(
            header_name,
            axum::http::HeaderValue::from_static(
                "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()",
            ),
        );
    }

    resp
}

/// Create the Axum router with all SaaS authentication, workspace, API key, CDC sync, and pgvector routes.
pub fn create_app(state: Arc<AppState>) -> Router {
    create_app_with_cors(state, CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any), true)
}

/// Create the Axum router with custom CORS and security header options.
pub fn create_app_with_cors(state: Arc<AppState>, cors: CorsLayer, enable_security_headers: bool) -> Router {
    let mut router = Router::new()
        // Health check & latency ping endpoints
        .route("/health", get(handlers::health_handler))
        .route("/api/health", get(handlers::health_handler))
        .route("/api/v1/health", get(handlers::health_handler))
        .route("/ping", get(handlers::ping_handler))
        .route("/api/ping", get(handlers::ping_handler))
        .route("/api/v1/ping", get(handlers::ping_handler))
        // Browser CLI Auth endpoints
        .route("/auth/cli", get(handlers::cli_auth_page_handler))
        .route(
            "/api/v1/auth/cli/authorize",
            post(handlers::cli_authorize_handler),
        )
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
        // Vector Embeddings endpoints (pgvector)
        .route("/api/v1/embeddings/upsert", post(handlers::upsert_embedding_handler))
        .route("/api/v1/embeddings/search", post(handlers::search_embedding_handler))
        // Realtime WebSocket stream
        .route("/ws", get(handlers::ws_handler))
        .route("/sync/ws", get(handlers::ws_handler))
        .route("/api/v1/sync/ws", get(handlers::ws_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    if enable_security_headers {
        router = router.layer(middleware::from_fn(security_headers_middleware));
    }

    router.with_state(state)
}

/// Initialize state and launch the Axum server.
pub async fn run_server(config: ServerConfig) -> Result<()> {
    let storage = if let Some(ref db_url) = config.database_url {
        if db_url.starts_with("postgres://") || db_url.starts_with("postgresql://") {
            tracing::info!("🐘 Connecting to PostgreSQL production database (Supabase/Neon/Railway)...");
            ServerStorage::open_postgres(db_url)
                .await
                .with_context(|| "Failed to connect to PostgreSQL database")?
        } else if db_url.starts_with("sqlite://") {
            let path = db_url.strip_prefix("sqlite://").unwrap();
            ServerStorage::open_sqlite(path)
                .with_context(|| format!("Failed to open SQLite database at {path}"))?
        } else {
            anyhow::bail!("Unsupported database URL scheme: {db_url}");
        }
    } else if let Some(ref path) = config.db_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent directory for {:?}", path))?;
        }
        ServerStorage::open_sqlite(path)
            .with_context(|| format!("Failed to open SQLite database at {:?}", path))?
    } else {
        tracing::warn!("No DATABASE_URL or DATABASE_PATH specified; using ephemeral in-memory storage.");
        ServerStorage::in_memory().context("Failed to initialize in-memory storage")?
    };

    let (ws_tx, _) = tokio::sync::broadcast::channel(256);

    let state = Arc::new(AppState {
        storage,
        jwt_secret: config.jwt_secret.clone(),
        legacy_secret: config.legacy_secret.clone(),
        custom_domain: config.custom_domain.clone(),
        ws_broadcast: ws_tx,
        start_time: Instant::now(),
    });

    let cors = if config.cors_allowed_origins.is_empty() || config.cors_allowed_origins.contains(&"*".to_string()) {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        let origins: Vec<axum::http::HeaderValue> = config
            .cors_allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let app = create_app_with_cors(state, cors, config.enable_security_headers);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind TCP listener on {}", addr))?;

    tracing::info!("🚀 Strata Cloud SaaS Server listening on http://{}", addr);
    if let Some(ref domain) = config.custom_domain {
        tracing::info!("🌐 Custom Domain configured: https://{}", domain);
    }
    tracing::info!("🔒 Security Headers (HSTS, CSP, X-Frame-Options) enabled: {}", config.enable_security_headers);
    tracing::info!("🔒 JWT Authentication is ENABLED.");
    if config.legacy_secret.is_some() {
        tracing::info!("🔑 Legacy static token authentication is ENABLED.");
    }

    axum::serve(listener, app)
        .await
        .context("Server error during execution")?;

    Ok(())
}
