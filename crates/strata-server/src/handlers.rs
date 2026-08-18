use std::sync::Arc;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use strata_core::schemas::SyncDelta;

use crate::auth::{validate_auth, AuthQuery};
use crate::storage::ServerStorage;

#[derive(Clone)]
pub struct AppState {
    pub storage: ServerStorage,
    pub auth_token: Option<String>,
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

/// Handler for health check endpoint (used by Railway liveness probes).
pub async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let workspaces = state.storage.list_workspaces().unwrap_or_default();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.start_time.elapsed().as_secs(),
        workspaces_count: workspaces.len(),
    })
}

/// Handler for pushing CDC deltas from client to server.
pub async fn push_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PushPayload>,
) -> Result<Json<PushResponse>, Response> {
    // Validate authentication
    validate_auth(&headers, None, state.auth_token.as_deref())?;

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

/// Handler for pulling remote deltas from server to client.
pub async fn pull_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PullQuery>,
) -> Result<Json<Vec<SyncDelta>>, Response> {
    let auth_q = AuthQuery {
        token: query.token.clone(),
    };
    validate_auth(&headers, Some(&auth_q), state.auth_token.as_deref())?;

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

/// Handler for checking sync status of a workspace.
pub async fn status_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StatusQuery>,
) -> Result<Json<StatusResponse>, Response> {
    let auth_q = AuthQuery {
        token: query.token.clone(),
    };
    validate_auth(&headers, Some(&auth_q), state.auth_token.as_deref())?;

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

/// WebSocket upgrade handler for realtime synchronization notifications.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Response, Response> {
    validate_auth(&headers, Some(&query), state.auth_token.as_deref())?;

    Ok(ws.on_upgrade(move |socket| handle_ws_socket(socket, state)))
}

async fn handle_ws_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.ws_broadcast.subscribe();

    // Send initial connected acknowledgement
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
            // Receive broadcast message from server state
            Ok(broadcast_msg) = rx.recv() => {
                if let Ok(msg_text) = serde_json::to_string(&broadcast_msg) {
                    if socket.send(Message::Text(msg_text.into())).await.is_err() {
                        break;
                    }
                }
            }
            // Receive incoming message from client (e.g. ping/pong)
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
