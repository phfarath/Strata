use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::models::{ApiKey, User, Workspace};
use crate::security::{hash_api_key, verify_jwt};
use crate::storage::ServerStorage;

#[derive(Debug, Deserialize)]
pub struct AuthQuery {
    pub token: Option<String>,
}

/// Represents the resolved authentication context of a request.
#[derive(Debug, Clone)]
pub enum AuthContext {
    /// Authenticated via User Session JWT (e.g. Dashboard web app).
    User(User),
    /// Authenticated via Machine API Key (e.g. CLI, IDE extension, AI Agent).
    ApiKey {
        key: ApiKey,
        workspace: Option<Workspace>,
    },
    /// Authenticated via Server Global Secret (legacy / single-tenant).
    LegacySecret,
    /// Unauthenticated / Open development mode.
    Open,
}

impl AuthContext {
    pub fn user_id(&self) -> Option<Uuid> {
        match self {
            AuthContext::User(u) => Some(u.id),
            AuthContext::ApiKey { key, .. } => Some(key.user_id),
            _ => None,
        }
    }

    pub fn workspace_id(&self) -> Option<Uuid> {
        match self {
            AuthContext::ApiKey { key, .. } => Some(key.workspace_id),
            _ => None,
        }
    }
}

/// Extract Bearer token string from headers or query params.
pub fn extract_token_str<'a>(
    headers: &'a HeaderMap,
    query: Option<&'a AuthQuery>,
) -> Option<&'a str> {
    // 1. Authorization header: `Bearer <token>`
    if let Some(auth_header) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ").or_else(|| auth_str.strip_prefix("bearer ")) {
                return Some(token.trim());
            }
        }
    }

    // 2. Custom headers
    if let Some(x_token) = headers.get("x-strata-token").or_else(|| headers.get("x-cortex-key")) {
        if let Ok(token_str) = x_token.to_str() {
            return Some(token_str.trim());
        }
    }

    // 3. Query string fallback `?token=...`
    if let Some(q) = query {
        if let Some(ref t) = q.token {
            return Some(t.trim());
        }
    }

    None
}

/// Validate authentication token and resolve context.
pub fn resolve_auth(
    headers: &HeaderMap,
    query: Option<&AuthQuery>,
    storage: &ServerStorage,
    jwt_secret: &str,
    legacy_secret: Option<&str>,
) -> Result<AuthContext, Response> {
    let token = match extract_token_str(headers, query) {
        Some(t) => t,
        None => {
            // If no token provided: check if open mode allowed
            if legacy_secret.is_none() || legacy_secret.unwrap().trim().is_empty() {
                // In open development mode, allow
                return Ok(AuthContext::Open);
            }
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Unauthorized: missing authentication token",
                    "status": 401
                })),
            )
                .into_response());
        }
    };

    // 1. Check if token is an API Key (starts with `strata_live_`)
    if token.starts_with("strata_live_") {
        let key_hash = hash_api_key(token);
        if let Ok(Some(api_key)) = storage.get_api_key_by_hash(&key_hash) {
            let _ = storage.record_api_key_usage(&api_key.id);
            let workspace = storage.get_workspace_by_id(&api_key.workspace_id).ok().flatten();
            return Ok(AuthContext::ApiKey {
                key: api_key,
                workspace,
            });
        }
    }

    // 2. Check if token matches legacy static secret
    if let Some(secret) = legacy_secret {
        if !secret.trim().is_empty() && token == secret.trim() {
            return Ok(AuthContext::LegacySecret);
        }
    }

    // 3. Check if token is a valid User Session JWT
    if let Ok(claims) = verify_jwt(token, jwt_secret) {
        if let Ok(user_id) = Uuid::parse_str(&claims.sub) {
            if let Ok(Some(user)) = storage.get_user_by_id(&user_id) {
                return Ok(AuthContext::User(user));
            }
        }
    }

    // Unauthorized response
    Err((
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": "Unauthorized: invalid or expired authentication token",
            "status": 401
        })),
    )
        .into_response())
}

/// Require valid user session JWT for protected user/workspace/key management routes.
pub fn require_user_session(
    headers: &HeaderMap,
    query: Option<&AuthQuery>,
    storage: &ServerStorage,
    jwt_secret: &str,
) -> Result<User, Response> {
    let token = match extract_token_str(headers, query) {
        Some(t) => t,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Unauthorized: session token required",
                    "status": 401
                })),
            )
                .into_response());
        }
    };

    let claims = verify_jwt(token, jwt_secret).map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": format!("Invalid session token: {e}"),
                "status": 401
            })),
        )
            .into_response()
    })?;

    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid user ID in token claims" })),
        )
            .into_response()
    })?;

    let user = storage
        .get_user_by_id(&user_id)
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
                Json(serde_json::json!({ "error": "User account no longer exists" })),
            )
                .into_response()
        })?;

    Ok(user)
}
