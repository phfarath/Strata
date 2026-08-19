use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Internal user model stored in the database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub full_name: String,
    pub tier: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Publicly exposed user profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPublic {
    pub id: Uuid,
    pub email: String,
    pub full_name: String,
    pub tier: String,
    pub created_at: DateTime<Utc>,
}

impl From<User> for UserPublic {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            email: u.email,
            full_name: u.full_name,
            tier: u.tier,
            created_at: u.created_at,
        }
    }
}

/// Workspace representing an isolated cognitive memory space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub slug: String,
    pub name: String,
    pub memory_quota_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// API Key for machine-to-machine authentication (CLI, Cursor, Claude, Codex, Gemini).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub scopes: Vec<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Response returned when a new API key is created (secret shown once).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiKeyCreated {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub key: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
}

// -------------------------------------------------------------
// DTOs & Request / Response Schemas
// -------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    pub full_name: String,
    pub workspace_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthResponse {
    pub user: UserPublic,
    pub workspaces: Vec<Workspace>,
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiKeyRequest {
    pub workspace_id: Uuid,
    pub name: String,
    pub expires_days: Option<u32>,
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user_id
    pub email: String,
    pub exp: usize,
    pub iat: usize,
}
