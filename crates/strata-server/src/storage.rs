use std::path::Path;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use strata_core::errors::StrataError;
use strata_core::schemas::SyncDelta;
use uuid::Uuid;

use crate::models::{ApiKey, User, Workspace};

/// Server-side SQLite/Postgres storage for multi-tenant users, workspaces, API keys, and CDC deltas.
#[derive(Clone)]
pub struct ServerStorage {
    conn: Arc<Mutex<Connection>>,
}

impl ServerStorage {
    /// Open or create a persistent SQLite storage file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StrataError> {
        let conn = Connection::open(path)
            .map_err(|e| StrataError::Database(format!("Failed to open server SQLite database: {e}")))?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Open an in-memory SQLite storage (primarily for tests and ephemeral runs).
    pub fn in_memory() -> Result<Self, StrataError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| StrataError::Database(format!("Failed to open in-memory SQLite database: {e}")))?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Initialize SQLite schema with WAL mode, foreign keys, and indices.
    pub fn init_schema(&self) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        // Enable WAL mode and foreign keys for high performance
        let _ = conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        );

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                full_name TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'free',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS workspaces (
                id TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL REFERENCES users(id),
                slug TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                memory_quota_bytes INTEGER NOT NULL DEFAULT 104857600,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS api_keys (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id),
                user_id TEXT NOT NULL REFERENCES users(id),
                name TEXT NOT NULL,
                key_prefix TEXT NOT NULL,
                key_hash TEXT UNIQUE NOT NULL,
                scopes_json TEXT NOT NULL DEFAULT '[\"sync:read\",\"sync:write\"]',
                last_used_at TEXT,
                expires_at TEXT,
                revoked_at TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS workspace_sequences (
                workspace_id TEXT PRIMARY KEY,
                last_seq INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS server_deltas (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                client_seq INTEGER NOT NULL,
                ts TEXT NOT NULL,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                version_hash TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_server_deltas_ws_seq
                ON server_deltas(workspace_id, seq);
            CREATE INDEX IF NOT EXISTS idx_server_deltas_kind
                ON server_deltas(kind);
            CREATE INDEX IF NOT EXISTS idx_api_keys_hash
                ON api_keys(key_hash);
            CREATE INDEX IF NOT EXISTS idx_workspaces_owner
                ON workspaces(owner_id);",
        )
        .map_err(|e| StrataError::Database(format!("Failed to initialize server schema: {e}")))?;

        Ok(())
    }

    // -------------------------------------------------------------
    // User Accounts CRUD
    // -------------------------------------------------------------

    pub fn create_user(
        &self,
        email: &str,
        password_hash: &str,
        full_name: &str,
    ) -> Result<User, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let email_clean = email.trim().to_lowercase();

        conn.execute(
            "INSERT INTO users (id, email, password_hash, full_name, tier, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id.to_string(),
                email_clean,
                password_hash,
                full_name.trim(),
                "free",
                now_str,
                now_str,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to create user: {e}")))?;

        Ok(User {
            id,
            email: email_clean,
            password_hash: password_hash.to_string(),
            full_name: full_name.trim().to_string(),
            tier: "free".to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    pub fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let email_clean = email.trim().to_lowercase();
        let mut stmt = conn
            .prepare("SELECT id, email, password_hash, full_name, tier, created_at, updated_at FROM users WHERE email = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let user = stmt
            .query_row(params![email_clean], |row| {
                let id_str: String = row.get(0)?;
                let email: String = row.get(1)?;
                let password_hash: String = row.get(2)?;
                let full_name: String = row.get(3)?;
                let tier: String = row.get(4)?;
                let created_str: String = row.get(5)?;
                let updated_str: String = row.get(6)?;

                let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(User {
                    id,
                    email,
                    password_hash,
                    full_name,
                    tier,
                    created_at,
                    updated_at,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(user)
    }

    pub fn get_user_by_id(&self, id: &Uuid) -> Result<Option<User>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT id, email, password_hash, full_name, tier, created_at, updated_at FROM users WHERE id = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let user = stmt
            .query_row(params![id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let email: String = row.get(1)?;
                let password_hash: String = row.get(2)?;
                let full_name: String = row.get(3)?;
                let tier: String = row.get(4)?;
                let created_str: String = row.get(5)?;
                let updated_str: String = row.get(6)?;

                let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(User {
                    id,
                    email,
                    password_hash,
                    full_name,
                    tier,
                    created_at,
                    updated_at,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(user)
    }

    // -------------------------------------------------------------
    // Workspace Management
    // -------------------------------------------------------------

    pub fn create_workspace(
        &self,
        owner_id: &Uuid,
        name: &str,
        slug: &str,
    ) -> Result<Workspace, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let slug_clean = slug.trim().to_lowercase();

        conn.execute(
            "INSERT INTO workspaces (id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id.to_string(),
                owner_id.to_string(),
                slug_clean,
                name.trim(),
                104857600_i64, // 100 MB default quota
                now_str,
                now_str,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to create workspace: {e}")))?;

        Ok(Workspace {
            id,
            owner_id: *owner_id,
            slug: slug_clean,
            name: name.trim().to_string(),
            memory_quota_bytes: 104857600,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn get_workspaces_for_user(&self, user_id: &Uuid) -> Result<Vec<Workspace>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at FROM workspaces WHERE owner_id = ?1 ORDER BY created_at ASC")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![user_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let owner_str: String = row.get(1)?;
                let slug: String = row.get(2)?;
                let name: String = row.get(3)?;
                let quota: i64 = row.get(4)?;
                let created_str: String = row.get(5)?;
                let updated_str: String = row.get(6)?;

                let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let owner_id = Uuid::parse_str(&owner_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(Workspace {
                    id,
                    owner_id,
                    slug,
                    name,
                    memory_quota_bytes: quota,
                    created_at,
                    updated_at,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut workspaces = Vec::new();
        for r in rows {
            workspaces.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(workspaces)
    }

    pub fn get_workspace_by_id(&self, id: &Uuid) -> Result<Option<Workspace>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at FROM workspaces WHERE id = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let ws = stmt
            .query_row(params![id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let owner_str: String = row.get(1)?;
                let slug: String = row.get(2)?;
                let name: String = row.get(3)?;
                let quota: i64 = row.get(4)?;
                let created_str: String = row.get(5)?;
                let updated_str: String = row.get(6)?;

                let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let owner_id = Uuid::parse_str(&owner_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(Workspace {
                    id,
                    owner_id,
                    slug,
                    name,
                    memory_quota_bytes: quota,
                    created_at,
                    updated_at,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(ws)
    }

    pub fn get_workspace_by_slug(&self, slug: &str) -> Result<Option<Workspace>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let slug_clean = slug.trim().to_lowercase();
        let mut stmt = conn
            .prepare("SELECT id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at FROM workspaces WHERE slug = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let ws = stmt
            .query_row(params![slug_clean], |row| {
                let id_str: String = row.get(0)?;
                let owner_str: String = row.get(1)?;
                let slug: String = row.get(2)?;
                let name: String = row.get(3)?;
                let quota: i64 = row.get(4)?;
                let created_str: String = row.get(5)?;
                let updated_str: String = row.get(6)?;

                let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let owner_id = Uuid::parse_str(&owner_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(Workspace {
                    id,
                    owner_id,
                    slug,
                    name,
                    memory_quota_bytes: quota,
                    created_at,
                    updated_at,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(ws)
    }

    // -------------------------------------------------------------
    // API Keys Management
    // -------------------------------------------------------------

    pub fn create_api_key(
        &self,
        workspace_id: &Uuid,
        user_id: &Uuid,
        name: &str,
        prefix: &str,
        hash: &str,
        scopes: &[String],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiKey, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let expires_str = expires_at.map(|dt| dt.to_rfc3339());
        let scopes_json = serde_json::to_string(scopes)?;

        conn.execute(
            "INSERT INTO api_keys (id, workspace_id, user_id, name, key_prefix, key_hash, scopes_json, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id.to_string(),
                workspace_id.to_string(),
                user_id.to_string(),
                name.trim(),
                prefix,
                hash,
                scopes_json,
                expires_str,
                now_str,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert API key: {e}")))?;

        Ok(ApiKey {
            id,
            workspace_id: *workspace_id,
            user_id: *user_id,
            name: name.trim().to_string(),
            key_prefix: prefix.to_string(),
            key_hash: hash.to_string(),
            scopes: scopes.to_vec(),
            last_used_at: None,
            expires_at,
            revoked_at: None,
            created_at: now,
        })
    }

    pub fn list_api_keys_for_workspace(&self, workspace_id: &Uuid) -> Result<Vec<ApiKey>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, user_id, name, key_prefix, key_hash, scopes_json, last_used_at, expires_at, revoked_at, created_at
                 FROM api_keys
                 WHERE workspace_id = ?1 AND revoked_at IS NULL
                 ORDER BY created_at DESC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![workspace_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let ws_str: String = row.get(1)?;
                let user_str: String = row.get(2)?;
                let name: String = row.get(3)?;
                let prefix: String = row.get(4)?;
                let hash: String = row.get(5)?;
                let scopes_json: String = row.get(6)?;
                let last_used_str: Option<String> = row.get(7)?;
                let expires_str: Option<String> = row.get(8)?;
                let revoked_str: Option<String> = row.get(9)?;
                let created_str: String = row.get(10)?;

                let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let workspace_id = Uuid::parse_str(&ws_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let user_id = Uuid::parse_str(&user_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();

                let last_used_at = last_used_str
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let expires_at = expires_str
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let revoked_at = revoked_str
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(ApiKey {
                    id,
                    workspace_id,
                    user_id,
                    name,
                    key_prefix: prefix,
                    key_hash: hash,
                    scopes,
                    last_used_at,
                    expires_at,
                    revoked_at,
                    created_at,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut keys = Vec::new();
        for r in rows {
            keys.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(keys)
    }

    pub fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, user_id, name, key_prefix, key_hash, scopes_json, last_used_at, expires_at, revoked_at, created_at
                 FROM api_keys
                 WHERE key_hash = ?1 AND revoked_at IS NULL",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let key = stmt
            .query_row(params![key_hash], |row| {
                let id_str: String = row.get(0)?;
                let ws_str: String = row.get(1)?;
                let user_str: String = row.get(2)?;
                let name: String = row.get(3)?;
                let prefix: String = row.get(4)?;
                let hash: String = row.get(5)?;
                let scopes_json: String = row.get(6)?;
                let last_used_str: Option<String> = row.get(7)?;
                let expires_str: Option<String> = row.get(8)?;
                let revoked_str: Option<String> = row.get(9)?;
                let created_str: String = row.get(10)?;

                let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let workspace_id = Uuid::parse_str(&ws_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let user_id = Uuid::parse_str(&user_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();

                let last_used_at = last_used_str
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let expires_at = expires_str
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let revoked_at = revoked_str
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(ApiKey {
                    id,
                    workspace_id,
                    user_id,
                    name,
                    key_prefix: prefix,
                    key_hash: hash,
                    scopes,
                    last_used_at,
                    expires_at,
                    revoked_at,
                    created_at,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(key)
    }

    pub fn record_api_key_usage(&self, id: &Uuid) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let now_str = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
            params![now_str, id.to_string()],
        )
        .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(())
    }

    pub fn revoke_api_key(&self, id: &Uuid, user_id: &Uuid) -> Result<bool, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let now_str = Utc::now().to_rfc3339();
        let rows = conn
            .execute(
                "UPDATE api_keys SET revoked_at = ?1 WHERE id = ?2 AND user_id = ?3",
                params![now_str, id.to_string(), user_id.to_string()],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(rows > 0)
    }

    // -------------------------------------------------------------
    // CDC Deltas & Synchronization Storage
    // -------------------------------------------------------------

    /// Push a batch of incoming deltas for a workspace.
    /// Assigns monotonic sequential IDs on the server per workspace.
    pub fn push_deltas(&self, workspace_id: &str, deltas: Vec<SyncDelta>) -> Result<(usize, u64), StrataError> {
        if deltas.is_empty() {
            let (_, current_seq) = self.get_status(workspace_id)?;
            return Ok((0, current_seq));
        }

        let mut conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to begin transaction: {e}")))?;

        let now_str = Utc::now().to_rfc3339();

        // 1. Get or initialize current workspace sequence
        let mut last_seq: i64 = tx
            .query_row(
                "SELECT last_seq FROM workspace_sequences WHERE workspace_id = ?1",
                params![workspace_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| StrataError::Database(format!("Failed to query workspace sequence: {e}")))?
            .unwrap_or(0);

        let mut inserted_count = 0;

        {
            let mut check_stmt = tx
                .prepare("SELECT seq FROM server_deltas WHERE id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;

            let mut insert_stmt = tx
                .prepare(
                    "INSERT INTO server_deltas (
                        id, workspace_id, seq, client_seq, ts, kind, payload, version_hash, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .map_err(|e| StrataError::Database(e.to_string()))?;

            for delta in deltas {
                let id_str = delta.id.to_string();

                // Idempotency: if delta with same UUID already exists, skip
                let exists: bool = check_stmt
                    .query_row(params![&id_str], |_| Ok(()))
                    .optional()
                    .map_err(|e| StrataError::Database(format!("Failed to check delta existence: {e}")))?
                    .is_some();

                if exists {
                    continue;
                }

                last_seq += 1;
                let payload_json = serde_json::to_string(&delta.payload)?;

                insert_stmt
                    .execute(params![
                        id_str,
                        workspace_id,
                        last_seq,
                        delta.seq as i64,
                        delta.ts.to_rfc3339(),
                        delta.kind,
                        payload_json,
                        delta.version_hash,
                        now_str,
                    ])
                    .map_err(|e| StrataError::Database(format!("Failed to insert server delta: {e}")))?;

                inserted_count += 1;
            }
        }

        // 2. Update workspace sequence record
        tx.execute(
            "INSERT INTO workspace_sequences (workspace_id, last_seq, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(workspace_id) DO UPDATE SET
                 last_seq = excluded.last_seq,
                 updated_at = excluded.updated_at",
            params![workspace_id, last_seq, now_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update workspace sequence: {e}")))?;

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit delta push: {e}")))?;

        Ok((inserted_count, last_seq as u64))
    }

    /// Pull deltas for a workspace starting strictly after `since_seq` up to `limit`.
    pub fn pull_deltas(
        &self,
        workspace_id: &str,
        since_seq: u64,
        limit: usize,
    ) -> Result<Vec<SyncDelta>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let capped_limit = limit.clamp(1, 1000);

        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, seq, ts, kind, payload, version_hash
                 FROM server_deltas
                 WHERE workspace_id = ?1 AND seq > ?2
                 ORDER BY seq ASC
                 LIMIT ?3",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![workspace_id, since_seq as i64, capped_limit as i64], |row| {
                let id_str: String = row.get(0)?;
                let ws_id: String = row.get(1)?;
                let seq: i64 = row.get(2)?;
                let ts_str: String = row.get(3)?;
                let kind: String = row.get(4)?;
                let payload_json: String = row.get(5)?;
                let version_hash: String = row.get(6)?;

                Ok((id_str, ws_id, seq, ts_str, kind, payload_json, version_hash))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut deltas = Vec::new();
        for r in rows {
            let (id_str, ws_id, seq, ts_str, kind, payload_json, version_hash) =
                r.map_err(|e| StrataError::Database(e.to_string()))?;

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in server delta: {e}")))?;
            let ts = DateTime::parse_from_rfc3339(&ts_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let payload: serde_json::Value = serde_json::from_str(&payload_json)?;

            deltas.push(SyncDelta {
                id,
                workspace_id: ws_id,
                seq: seq as u64,
                ts,
                kind,
                payload,
                version_hash,
                synced: true,
            });
        }

        Ok(deltas)
    }

    /// Retrieve total deltas count and maximum sequence number for a workspace.
    pub fn get_status(&self, workspace_id: &str) -> Result<(usize, u64), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let total_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM server_deltas WHERE workspace_id = ?1",
                params![workspace_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let max_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(last_seq, 0) FROM workspace_sequences WHERE workspace_id = ?1",
                params![workspace_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        Ok((total_count as usize, max_seq as u64))
    }

    /// List all known workspaces.
    pub fn list_workspaces(&self) -> Result<Vec<String>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT slug FROM workspaces UNION SELECT workspace_id FROM workspace_sequences ORDER BY 1 ASC")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut workspaces = Vec::new();
        for r in rows {
            workspaces.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(workspaces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_user_and_workspace_and_api_keys_storage_flow() {
        let storage = ServerStorage::in_memory().expect("Failed to create storage");

        // 1. Create user
        let user = storage
            .create_user("pedro@strata.dev", "hashed_pwd", "Pedro Farath")
            .expect("Create user failed");
        assert_eq!(user.email, "pedro@strata.dev");

        // 2. Fetch user by email & id
        let fetched = storage.get_user_by_email("pedro@strata.dev").unwrap().unwrap();
        assert_eq!(fetched.id, user.id);

        // 3. Create workspace
        let ws = storage
            .create_workspace(&user.id, "Pedro Dev Workspace", "phfarath-dev")
            .expect("Create workspace failed");
        assert_eq!(ws.slug, "phfarath-dev");

        // 4. Create API Key
        let key = storage
            .create_api_key(
                &ws.id,
                &user.id,
                "MacBook Pro Cursor",
                "strata_live_1234",
                "hash_1234567890",
                &["sync:read".to_string(), "sync:write".to_string()],
                None,
            )
            .expect("Create API key failed");
        assert_eq!(key.name, "MacBook Pro Cursor");

        // 5. Lookup API Key by hash
        let lookup_key = storage.get_api_key_by_hash("hash_1234567890").unwrap().unwrap();
        assert_eq!(lookup_key.id, key.id);

        // 6. Revoke API Key
        assert!(storage.revoke_api_key(&key.id, &user.id).unwrap());
        assert!(storage.get_api_key_by_hash("hash_1234567890").unwrap().is_none());
    }

    #[test]
    fn test_server_storage_in_memory_crud_and_idempotency() {
        let storage = ServerStorage::in_memory().expect("Failed to create storage");
        let ws = "test-ws";

        // Initial status should be empty
        let (count, max_seq) = storage.get_status(ws).unwrap();
        assert_eq!(count, 0);
        assert_eq!(max_seq, 0);

        let delta1_id = Uuid::new_v4();
        let delta1 = SyncDelta::new(ws, 1, "fact", json!({"statement": "fact 1"}), "hash1")
            .with_id(delta1_id);
        let delta2 = SyncDelta::new(ws, 2, "fact", json!({"statement": "fact 2"}), "hash2");

        // Push 2 deltas
        let (pushed, seq) = storage.push_deltas(ws, vec![delta1.clone(), delta2.clone()]).unwrap();
        assert_eq!(pushed, 2);
        assert_eq!(seq, 2);

        // Idempotency: push delta1 again, should be skipped
        let (pushed_again, seq_again) = storage.push_deltas(ws, vec![delta1]).unwrap();
        assert_eq!(pushed_again, 0);
        assert_eq!(seq_again, 2);

        // Pull deltas since_seq 0
        let all_deltas = storage.pull_deltas(ws, 0, 100).unwrap();
        assert_eq!(all_deltas.len(), 2);
        assert_eq!(all_deltas[0].seq, 1);
        assert_eq!(all_deltas[1].seq, 2);

        // Pull deltas since_seq 1
        let since_1 = storage.pull_deltas(ws, 1, 100).unwrap();
        assert_eq!(since_1.len(), 1);
        assert_eq!(since_1[0].seq, 2);
    }
}
