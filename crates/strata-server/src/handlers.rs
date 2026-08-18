use std::sync::Arc;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use strata_core::schemas::SyncDelta;
use uuid::Uuid;

use crate::auth::{require_user_session, resolve_auth, AuthQuery};
use crate::models::{
    ApiKeyCreated, AuthResponse, CreateApiKeyRequest, CreateWorkspaceRequest, LoginRequest,
    SignupRequest, UserPublic, Workspace,
};
use crate::security::{create_jwt, generate_api_key, hash_password, verify_password};
use crate::storage::ServerStorage;

#[derive(Clone)]
pub struct AppState {
    pub storage: ServerStorage,
    pub jwt_secret: String,
    pub legacy_secret: Option<String>,
    pub ws_broadcast: tokio::sync::broadcast::Sender<WsBroadcastMsg>,
    pub start_time: std::time::Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsBroadcastMsg {
    pub event: String,
    pub workspace_id: String,
    pub max_seq: u64,
    pub delta_count: usize,
}

#[derive(Debug, Deserialize)]
pub struct PushPayload {
    pub workspace_id: String,
    pub deltas: Vec<SyncDelta>,
}

#[derive(Debug, Serialize)]
pub struct PushResponse {
    pub status: String,
    pub workspace_id: String,
    pub pushed: usize,
    pub max_seq: u64,
}

#[derive(Debug, Deserialize)]
pub struct PullQuery {
    pub workspace_id: Option<String>,
    pub since_seq: Option<u64>,
    pub limit: Option<usize>,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    pub workspace_id: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub workspace_id: String,
    pub total_deltas: usize,
    pub max_seq: u64,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_secs: u64,
    pub workspaces_count: usize,
}

#[derive(Debug, Deserialize)]
pub struct ListKeysQuery {
    pub workspace_id: Uuid,
    pub token: Option<String>,
}

// -------------------------------------------------------------
// Public Health Check
// -------------------------------------------------------------

pub async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let workspaces = state.storage.list_workspaces().unwrap_or_default();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.start_time.elapsed().as_secs(),
        workspaces_count: workspaces.len(),
    })
}

// -------------------------------------------------------------
// User Auth & Signup Handlers (Self-Serve SaaS)
// -------------------------------------------------------------

pub async fn signup_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SignupRequest>,
) -> Result<Json<AuthResponse>, Response> {
    let email = payload.email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid email address" })),
        )
            .into_response());
    }

    if payload.password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Password must be at least 8 characters long" })),
        )
            .into_response());
    }

    // Check if user already exists
    if let Ok(Some(_)) = state.storage.get_user_by_email(email) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "User with this email already exists" })),
        )
            .into_response());
    }

    let password_hash = hash_password(&payload.password).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Hashing error: {e}") })),
        )
            .into_response()
    })?;

    let user = state
        .storage
        .create_user(email, &password_hash, &payload.full_name)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to create user: {e}") })),
            )
                .into_response()
        })?;

    // Automatically provision default workspace
    let ws_name = payload
        .workspace_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{}'s Workspace", user.full_name));

    let slug_prefix = email.split('@').next().unwrap_or("default");
    let slug = format!("{slug_prefix}-workspace");

    let workspace = state
        .storage
        .create_workspace(&user.id, &ws_name, &slug)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to provision workspace: {e}") })),
            )
                .into_response()
        })?;

    // Issue JWT token valid for 30 days
    let token = create_jwt(&user.id, &user.email, &state.jwt_secret, 30 * 86400).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("JWT error: {e}") })),
        )
            .into_response()
    })?;

    Ok(Json(AuthResponse {
        user: UserPublic::from(user),
        workspaces: vec![workspace],
        token,
    }))
}

pub async fn login_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, Response> {
    let email = payload.email.trim();
    let user = state
        .storage
        .get_user_by_email(email)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {e}") })),
            )
                .into_response()
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Invalid email or password" })),
            )
                .into_response()
        })?;

    let valid = verify_password(&payload.password, &user.password_hash).unwrap_or(false);
    if !valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid email or password" })),
        )
            .into_response());
    }

    let workspaces = state
        .storage
        .get_workspaces_for_user(&user.id)
        .unwrap_or_default();

    let token = create_jwt(&user.id, &user.email, &state.jwt_secret, 30 * 86400).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("JWT error: {e}") })),
        )
            .into_response()
    })?;

    Ok(Json(AuthResponse {
        user: UserPublic::from(user),
        workspaces,
        token,
    }))
}

pub async fn me_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Json<AuthResponse>, Response> {
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret)?;
    let workspaces = state
        .storage
        .get_workspaces_for_user(&user.id)
        .unwrap_or_default();

    let token = create_jwt(&user.id, &user.email, &state.jwt_secret, 30 * 86400).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("JWT error: {e}") })),
        )
            .into_response()
    })?;

    Ok(Json(AuthResponse {
        user: UserPublic::from(user),
        workspaces,
        token,
    }))
}

// -------------------------------------------------------------
// Workspace Management Handlers
// -------------------------------------------------------------

pub async fn create_workspace_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<CreateWorkspaceRequest>,
) -> Result<Json<Workspace>, Response> {
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret)?;

    let name = payload.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Workspace name cannot be empty" })),
        )
            .into_response());
    }

    let slug = payload.slug.unwrap_or_else(|| {
        let clean = name.to_lowercase().replace(' ', "-");
        format!("{clean}-{}", Uuid::new_v4().simple())
    });

    let ws = state
        .storage
        .create_workspace(&user.id, name, &slug)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to create workspace: {e}") })),
            )
                .into_response()
        })?;

    Ok(Json(ws))
}

pub async fn list_workspaces_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Json<Vec<Workspace>>, Response> {
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret)?;
    let workspaces = state
        .storage
        .get_workspaces_for_user(&user.id)
        .unwrap_or_default();
    Ok(Json(workspaces))
}

// -------------------------------------------------------------
// API Keys Management Handlers
// -------------------------------------------------------------

pub async fn create_key_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyCreated>, Response> {
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret)?;

    // Verify user owns the workspace
    let ws = state
        .storage
        .get_workspace_by_id(&payload.workspace_id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {e}") })),
            )
                .into_response()
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Workspace not found" })),
            )
                .into_response()
        })?;

    if ws.owner_id != user.id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "You do not own this workspace" })),
        )
            .into_response());
    }

    let (full_key, key_prefix, key_hash) = generate_api_key();
    let scopes = payload.scopes.unwrap_or_else(|| vec!["sync:read".to_string(), "sync:write".to_string()]);
    let expires_at = payload.expires_days.map(|d| Utc::now() + chrono::Duration::days(d as i64));

    let created = state
        .storage
        .create_api_key(
            &payload.workspace_id,
            &user.id,
            &payload.name,
            &key_prefix,
            &key_hash,
            &scopes,
            expires_at,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to create API key: {e}") })),
            )
                .into_response()
        })?;

    Ok(Json(ApiKeyCreated {
        id: created.id,
        workspace_id: created.workspace_id,
        name: created.name,
        key: full_key, // Returned once!
        key_prefix: created.key_prefix,
        scopes: created.scopes,
        created_at: created.created_at,
    }))
}

pub async fn list_keys_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<crate::models::ApiKey>>, Response> {
    let auth_q = AuthQuery {
        token: query.token.clone(),
    };
    let user = require_user_session(&headers, Some(&auth_q), &state.storage, &state.jwt_secret)?;

    let ws = state
        .storage
        .get_workspace_by_id(&query.workspace_id)
        .unwrap_or(None)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Workspace not found" })),
            )
                .into_response()
        })?;

    if ws.owner_id != user.id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Forbidden" })),
        )
            .into_response());
    }

    let keys = state
        .storage
        .list_api_keys_for_workspace(&query.workspace_id)
        .unwrap_or_default();

    Ok(Json(keys))
}

pub async fn revoke_key_handler(
    State(state): State<Arc<AppState>>,
    Path(key_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret)?;

    let revoked = state
        .storage
        .revoke_api_key(&key_id, &user.id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {e}") })),
            )
                .into_response()
        })?;

    if !revoked {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "API key not found or not owned by user" })),
        )
            .into_response());
    }

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": "API key revoked successfully"
    })))
}

// -------------------------------------------------------------
// CDC Memory Synchronization Handlers (Multi-Tenant)
// -------------------------------------------------------------

pub async fn push_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PushPayload>,
) -> Result<Json<PushResponse>, Response> {
    let _auth = resolve_auth(
        &headers,
        None,
        &state.storage,
        &state.jwt_secret,
        state.legacy_secret.as_deref(),
    )?;

    let workspace_id = payload.workspace_id.trim();
    if workspace_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "workspace_id cannot be empty" })),
        )
            .into_response());
    }

    let (pushed, max_seq) = state
        .storage
        .push_deltas(workspace_id, payload.deltas)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to push deltas: {e}") })),
            )
                .into_response()
        })?;

    // Broadcast update to WebSocket listeners
    if pushed > 0 {
        let _ = state.ws_broadcast.send(WsBroadcastMsg {
            event: "new_deltas".to_string(),
            workspace_id: workspace_id.to_string(),
            max_seq,
            delta_count: pushed,
        });
    }

    Ok(Json(PushResponse {
        status: "success".to_string(),
        workspace_id: workspace_id.to_string(),
        pushed,
        max_seq,
    }))
}

pub async fn pull_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PullQuery>,
) -> Result<Json<Vec<SyncDelta>>, Response> {
    let auth_q = AuthQuery {
        token: query.token.clone(),
    };
    let _auth = resolve_auth(
        &headers,
        Some(&auth_q),
        &state.storage,
        &state.jwt_secret,
        state.legacy_secret.as_deref(),
    )?;

    let workspace_id = query
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");

    let since_seq = query.since_seq.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);

    let deltas = state
        .storage
        .pull_deltas(workspace_id, since_seq, limit)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to pull deltas: {e}") })),
            )
                .into_response()
        })?;

    Ok(Json(deltas))
}

pub async fn status_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StatusQuery>,
) -> Result<Json<StatusResponse>, Response> {
    let auth_q = AuthQuery {
        token: query.token.clone(),
    };
    let _auth = resolve_auth(
        &headers,
        Some(&auth_q),
        &state.storage,
        &state.jwt_secret,
        state.legacy_secret.as_deref(),
    )?;

    let workspace_id = query
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");

    let (total_deltas, max_seq) = state.storage.get_status(workspace_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to get status: {e}") })),
        )
            .into_response()
    })?;

    Ok(Json(StatusResponse {
        workspace_id: workspace_id.to_string(),
        total_deltas,
        max_seq,
    }))
}

// -------------------------------------------------------------
// Realtime WebSocket Stream
// -------------------------------------------------------------

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Response, Response> {
    let _auth = resolve_auth(
        &headers,
        Some(&query),
        &state.storage,
        &state.jwt_secret,
        state.legacy_secret.as_deref(),
    )?;

    Ok(ws.on_upgrade(move |socket| handle_ws_socket(socket, state)))
}

async fn handle_ws_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.ws_broadcast.subscribe();

    let welcome = serde_json::json!({
        "event": "connected",
        "version": env!("CARGO_PKG_VERSION"),
        "message": "Connected to Strata Cloud Sync Realtime Stream"
    });
    if let Ok(msg_text) = serde_json::to_string(&welcome) {
        if socket.send(Message::Text(msg_text.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            Ok(broadcast_msg) = rx.recv() => {
                if let Ok(msg_text) = serde_json::to_string(&broadcast_msg) {
                    if socket.send(Message::Text(msg_text.into())).await.is_err() {
                        break;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
