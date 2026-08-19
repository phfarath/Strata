use std::sync::Arc;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use strata_core::schemas::SyncDelta;
use uuid::Uuid;

use crate::auth::{require_user_session, resolve_auth, AuthQuery};
use crate::models::{
    ApiKeyCreated, AuthResponse, CreateApiKeyRequest, CreateWorkspaceRequest, LoginRequest,
    SearchEmbeddingRequest, SearchEmbeddingResponse, SignupRequest, UpsertEmbeddingRequest,
    UserPublic, Workspace,
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
    pub is_postgres: bool,
    pub has_pgvector: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_secs: u64,
    pub workspaces_count: usize,
    pub is_postgres: bool,
    pub has_pgvector: bool,
}

#[derive(Debug, Deserialize)]
pub struct ListKeysQuery {
    pub workspace_id: Uuid,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CliAuthPageQuery {
    pub port: Option<u16>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CliAuthorizeRequest {
    pub email: String,
    pub password: String,
    pub port: u16,
    pub state: String,
    pub machine_name: Option<String>,
    pub is_signup: Option<bool>,
    pub full_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CliAuthorizeResponse {
    pub redirect_url: String,
    pub token: String,
    pub workspace_id: Uuid,
    pub workspace_slug: String,
    pub user_email: String,
}

// -------------------------------------------------------------
// Public Health Check
// -------------------------------------------------------------

pub async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let workspaces = state.storage.list_workspaces().await.unwrap_or_default();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.start_time.elapsed().as_secs(),
        workspaces_count: workspaces.len(),
        is_postgres: state.storage.is_postgres(),
        has_pgvector: state.storage.has_pgvector(),
    })
}

// -------------------------------------------------------------
// Browser CLI Authentication UI & Authorization
// -------------------------------------------------------------

pub async fn cli_auth_page_handler(Query(query): Query<CliAuthPageQuery>) -> Html<String> {
    let port = query.port.unwrap_or(54321);
    let state = query.state.unwrap_or_default();

    let html = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Authorize Strata CLI</title>
  <style>
    :root {{
      --bg: #090d16;
      --card-bg: rgba(19, 27, 46, 0.85);
      --border: #1e293b;
      --primary: #38bdf8;
      --primary-hover: #0284c7;
      --text: #f8fafc;
      --text-muted: #94a3b8;
      --danger: #ef4444;
      --success: #10b981;
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; }}
    body {{
      background: var(--bg);
      background-image: radial-gradient(circle at 50% 10%, #1e293b 0%, var(--bg) 80%);
      color: var(--text);
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      padding: 1rem;
    }}
    .card {{
      background: var(--card-bg);
      border: 1px solid var(--border);
      backdrop-filter: blur(12px);
      padding: 2.5rem;
      border-radius: 1.25rem;
      width: 100%;
      max-width: 440px;
      box-shadow: 0 25px 50px -12px rgba(0,0,0,0.6);
      text-align: center;
    }}
    .logo-badge {{
      display: inline-flex;
      align-items: center;
      gap: 0.5rem;
      font-size: 1.25rem;
      font-weight: 700;
      letter-spacing: -0.025em;
      color: var(--primary);
      margin-bottom: 1.5rem;
    }}
    h1 {{ font-size: 1.5rem; font-weight: 700; margin-bottom: 0.5rem; color: var(--text); }}
    p.subtitle {{ color: var(--text-muted); font-size: 0.9rem; margin-bottom: 2rem; }}
    .tabs {{
      display: flex;
      background: #0f172a;
      padding: 0.25rem;
      border-radius: 0.5rem;
      margin-bottom: 1.5rem;
      border: 1px solid var(--border);
    }}
    .tab-btn {{
      flex: 1;
      background: none;
      border: none;
      color: var(--text-muted);
      padding: 0.5rem;
      font-size: 0.875rem;
      font-weight: 600;
      border-radius: 0.375rem;
      cursor: pointer;
      transition: all 0.2s;
    }}
    .tab-btn.active {{
      background: var(--primary);
      color: #04101e;
    }}
    form {{ display: flex; flex-direction: column; gap: 1rem; text-align: left; }}
    label {{ font-size: 0.8rem; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }}
    input {{
      width: 100%;
      padding: 0.75rem 1rem;
      background: #0b1120;
      border: 1px solid var(--border);
      border-radius: 0.5rem;
      color: var(--text);
      font-size: 0.95rem;
      outline: none;
      transition: border-color 0.2s;
    }}
    input:focus {{ border-color: var(--primary); }}
    .submit-btn {{
      margin-top: 0.5rem;
      width: 100%;
      padding: 0.85rem;
      background: var(--primary);
      color: #04101e;
      border: none;
      border-radius: 0.5rem;
      font-size: 0.95rem;
      font-weight: 700;
      cursor: pointer;
      transition: background 0.2s, transform 0.1s;
    }}
    .submit-btn:hover {{ background: var(--primary-hover); }}
    .submit-btn:active {{ transform: scale(0.98); }}
    .alert {{
      padding: 0.75rem;
      border-radius: 0.5rem;
      font-size: 0.85rem;
      margin-bottom: 1rem;
      display: none;
      text-align: left;
    }}
    .alert.error {{ background: rgba(239, 68, 68, 0.15); border: 1px solid var(--danger); color: #fca5a5; display: block; }}
    .alert.success {{ background: rgba(16, 185, 129, 0.15); border: 1px solid var(--success); color: #6ee7b7; display: block; }}
    .meta-tag {{
      margin-top: 1.5rem;
      font-size: 0.75rem;
      color: #64748b;
    }}
  </style>
</head>
<body>
  <div class="card">
    <div class="logo-badge">
      <span>⚡</span> Strata Cloud
    </div>
    <h1>Authorize Machine</h1>
    <p class="subtitle">Connect your terminal to persistent cloud cognitive runtime</p>

    <div id="alert-box" class="alert"></div>

    <div class="tabs">
      <button type="button" class="tab-btn active" id="tab-login" onclick="switchTab('login')">Sign In</button>
      <button type="button" class="tab-btn" id="tab-signup" onclick="switchTab('signup')">Create Account</button>
    </div>

    <form id="auth-form" onsubmit="handleAuth(event)">
      <div id="name-group" style="display: none;">
        <label for="full_name">Full Name</label>
        <input type="text" id="full_name" placeholder="Pedro Dev">
      </div>

      <div>
        <label for="email">Email</label>
        <input type="email" id="email" required placeholder="developer@strata.dev">
      </div>

      <div>
        <label for="password">Password</label>
        <input type="password" id="password" required placeholder="••••••••••••">
      </div>

      <div>
        <label for="machine_name">Device Name</label>
        <input type="text" id="machine_name" value="CLI Device ({port})">
      </div>

      <button type="submit" id="submit-btn" class="submit-btn">Authorize Terminal</button>
    </form>

    <div class="meta-tag">
      Callback Port: <code>127.0.0.1:{port}</code>
    </div>
  </div>

  <script>
    let isSignup = false;
    const port = {port};
    const state = "{state}";

    function switchTab(mode) {{
      isSignup = (mode === 'signup');
      document.getElementById('tab-login').classList.toggle('active', !isSignup);
      document.getElementById('tab-signup').classList.toggle('active', isSignup);
      document.getElementById('name-group').style.display = isSignup ? 'block' : 'none';
      document.getElementById('submit-btn').innerText = isSignup ? 'Create Account & Authorize' : 'Authorize Terminal';
      document.getElementById('alert-box').className = 'alert';
    }}

    async function handleAuth(e) {{
      e.preventDefault();
      const btn = document.getElementById('submit-btn');
      const alertBox = document.getElementById('alert-box');
      btn.disabled = true;
      btn.innerText = 'Connecting...';

      const email = document.getElementById('email').value.trim();
      const password = document.getElementById('password').value;
      const full_name = document.getElementById('full_name').value.trim();
      const machine_name = document.getElementById('machine_name').value.trim();

      try {{
        const resp = await fetch('/api/v1/auth/cli/authorize', {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify({{
            email,
            password,
            port,
            state,
            machine_name,
            is_signup: isSignup,
            full_name: isSignup ? full_name : undefined
          }})
        }});

        const data = await resp.json();
        if (resp.ok) {{
          alertBox.className = 'alert success';
          alertBox.innerText = '✓ Authorized! Redirecting back to your terminal...';
          setTimeout(() => {{
            window.location.href = data.redirect_url;
          }}, 500);
        }} else {{
          alertBox.className = 'alert error';
          alertBox.innerText = data.error || 'Authentication failed';
          btn.disabled = false;
          btn.innerText = isSignup ? 'Create Account & Authorize' : 'Authorize Terminal';
        }}
      }} catch (err) {{
        alertBox.className = 'alert error';
        alertBox.innerText = 'Network error: ' + err.message;
        btn.disabled = false;
        btn.innerText = isSignup ? 'Create Account & Authorize' : 'Authorize Terminal';
      }}
    }}
  </script>
</body>
</html>"#);

    Html(html)
}

pub async fn cli_authorize_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CliAuthorizeRequest>,
) -> Result<Json<CliAuthorizeResponse>, Response> {
    let email = payload.email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid email address" })),
        )
            .into_response());
    }

    let user = if payload.is_signup.unwrap_or(false) {
        if payload.password.len() < 8 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Password must be at least 8 characters long" })),
            )
                .into_response());
        }

        if let Ok(Some(_)) = state.storage.get_user_by_email(email).await {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "User with this email already exists. Please sign in." })),
            )
                .into_response());
        }

        let full_name = payload.full_name.unwrap_or_else(|| email.split('@').next().unwrap_or("Developer").to_string());
        let password_hash = hash_password(&payload.password).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Hash error: {e}") })),
            )
                .into_response()
        })?;

        let created_user = state
            .storage
            .create_user(email, &password_hash, &full_name)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to create user: {e}") })),
                )
                    .into_response()
            })?;

        let slug = format!("{}-workspace", email.split('@').next().unwrap_or("dev"));
        let _ = state.storage.create_workspace(&created_user.id, &format!("{full_name}'s Workspace"), &slug).await;
        created_user
    } else {
        let existing = state
            .storage
            .get_user_by_email(email)
            .await
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

        let valid = verify_password(&payload.password, &existing.password_hash).unwrap_or(false);
        if !valid {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Invalid email or password" })),
            )
                .into_response());
        }
        existing
    };

    // Get user's active workspaces (or ensure at least 1 exists)
    let workspaces = state
        .storage
        .get_workspaces_for_user(&user.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Storage error: {e}") })),
            )
                .into_response()
        })?;

    let workspace = if let Some(ws) = workspaces.into_iter().next() {
        ws
    } else {
        let slug = format!("{}-ws", user.id.simple());
        state
            .storage
            .create_workspace(&user.id, &format!("{}'s Workspace", user.full_name), &slug)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Workspace provision error: {e}") })),
                )
                    .into_response()
            })?
    };

    // Generate machine API Key
    let (full_key, key_prefix, key_hash) = generate_api_key();
    let key_name = payload
        .machine_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "CLI Developer Device".to_string());

    let _ = state.storage.create_api_key(
        &workspace.id,
        &user.id,
        &key_name,
        &key_prefix,
        &key_hash,
        &["sync:read".to_string(), "sync:write".to_string()],
        None,
    ).await;

    let redirect_url = format!(
        "http://127.0.0.1:{}/callback?token={}&state={}&workspace_id={}&workspace_slug={}&user_email={}",
        payload.port,
        full_key,
        payload.state,
        workspace.id,
        workspace.slug,
        urlencoding_encode(&user.email)
    );

    Ok(Json(CliAuthorizeResponse {
        redirect_url,
        token: full_key,
        workspace_id: workspace.id,
        workspace_slug: workspace.slug,
        user_email: user.email,
    }))
}

fn urlencoding_encode(s: &str) -> String {
    s.replace('@', "%40").replace('+', "%2B")
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
    if let Ok(Some(_)) = state.storage.get_user_by_email(email).await {
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
        .await
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
        .await
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
        .await
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
        .await
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
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret).await?;
    let workspaces = state
        .storage
        .get_workspaces_for_user(&user.id)
        .await
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
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret).await?;

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
        .await
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
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret).await?;
    let workspaces = state
        .storage
        .get_workspaces_for_user(&user.id)
        .await
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
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret).await?;

    // Verify user owns the workspace
    let ws = state
        .storage
        .get_workspace_by_id(&payload.workspace_id)
        .await
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
        .await
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
    let user = require_user_session(&headers, Some(&auth_q), &state.storage, &state.jwt_secret).await?;

    let ws = state
        .storage
        .get_workspace_by_id(&query.workspace_id)
        .await
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
        .await
        .unwrap_or_default();

    Ok(Json(keys))
}

pub async fn revoke_key_handler(
    State(state): State<Arc<AppState>>,
    Path(key_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    let user = require_user_session(&headers, Some(&query), &state.storage, &state.jwt_secret).await?;

    let revoked = state
        .storage
        .revoke_api_key(&key_id, &user.id)
        .await
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
    )
    .await?;

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
        .await
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
    )
    .await?;

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
        .await
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
    )
    .await?;

    let workspace_id = query
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");

    let (total_deltas, max_seq) = state.storage.get_status(workspace_id).await.map_err(|e| {
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
        is_postgres: state.storage.is_postgres(),
        has_pgvector: state.storage.has_pgvector(),
    }))
}

// -------------------------------------------------------------
// Vector Embeddings Handlers (pgvector Cloud Vector Store)
// -------------------------------------------------------------

pub async fn upsert_embedding_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpsertEmbeddingRequest>,
) -> Result<Json<serde_json::Value>, Response> {
    let _auth = resolve_auth(
        &headers,
        None,
        &state.storage,
        &state.jwt_secret,
        state.legacy_secret.as_deref(),
    )
    .await?;

    let metadata = payload.metadata.unwrap_or_else(|| serde_json::json!({}));
    state
        .storage
        .upsert_embedding(&payload.workspace_id, &payload.memory_id, &payload.embedding, &metadata)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to upsert embedding: {e}") })),
            )
                .into_response()
        })?;

    Ok(Json(serde_json::json!({
        "status": "success",
        "memory_id": payload.memory_id
    })))
}

pub async fn search_embedding_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<SearchEmbeddingRequest>,
) -> Result<Json<SearchEmbeddingResponse>, Response> {
    let _auth = resolve_auth(
        &headers,
        None,
        &state.storage,
        &state.jwt_secret,
        state.legacy_secret.as_deref(),
    )
    .await?;

    let limit = payload.limit.unwrap_or(10);
    let results = state
        .storage
        .search_embeddings(&payload.workspace_id, &payload.query_embedding, limit)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to search embeddings: {e}") })),
            )
                .into_response()
        })?;

    let total = results.len();
    Ok(Json(SearchEmbeddingResponse {
        workspace_id: payload.workspace_id,
        results,
        total,
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
    )
    .await?;

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
