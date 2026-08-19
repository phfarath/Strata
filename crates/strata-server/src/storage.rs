use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use deadpool_postgres::{Manager as PgManager, Pool as PgPool};
use pgvector::Vector;
use rusqlite::{params, Connection, OptionalExtension};
use strata_core::errors::StrataError;
use strata_core::schemas::SyncDelta;
use uuid::Uuid;

use crate::models::{ApiKey, User, VectorSearchResult, Workspace};

/// Internal representation of the storage engine backend.
#[derive(Clone)]
pub enum StorageBackend {
    Sqlite(Arc<Mutex<Connection>>),
    Postgres {
        pool: PgPool,
        has_pgvector: Arc<AtomicBool>,
    },
}

/// Server-side universal storage supporting Postgres (Supabase, Neon, Railway, RDS)
/// and SQLite (offline-first local files and in-memory).
#[derive(Clone)]
pub struct ServerStorage {
    backend: StorageBackend,
}

#[derive(Debug)]
struct AcceptAnyServerCertVerifier(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl ServerStorage {
    /// Open a persistent SQLite storage file.
    pub fn open_sqlite<P: AsRef<Path>>(path: P) -> Result<Self, StrataError> {
        let conn = Connection::open(path)
            .map_err(|e| StrataError::Database(format!("Failed to open server SQLite database: {e}")))?;
        let storage = Self {
            backend: StorageBackend::Sqlite(Arc::new(Mutex::new(conn))),
        };
        storage.init_sqlite_schema()?;
        Ok(storage)
    }

    /// Open an in-memory SQLite storage (primarily for tests and ephemeral runs).
    pub fn in_memory() -> Result<Self, StrataError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| StrataError::Database(format!("Failed to open in-memory SQLite database: {e}")))?;
        let storage = Self {
            backend: StorageBackend::Sqlite(Arc::new(Mutex::new(conn))),
        };
        storage.init_sqlite_schema()?;
        Ok(storage)
    }

    /// Open a PostgreSQL storage pool with automatic TLS (via pure Rust Rustls)
    /// and connection pooling. Fully compatible with Supabase, Neon, Railway, and RDS.
    pub async fn open_postgres(database_url: &str) -> Result<Self, StrataError> {
        let pg_config: tokio_postgres::Config = database_url
            .parse()
            .map_err(|e| StrataError::Database(format!("Invalid PostgreSQL connection URL: {e}")))?;

        let disable_ssl = database_url.contains("sslmode=disable")
            || database_url.contains("sslmode=allow")
            || database_url.contains("sslmode=prefer");

        let pool = if disable_ssl {
            let mgr = PgManager::new(pg_config, tokio_postgres::NoTls);
            PgPool::builder(mgr)
                .max_size(16)
                .runtime(deadpool_postgres::Runtime::Tokio1)
                .build()
                .map_err(|e| StrataError::Database(format!("Failed to build Postgres pool: {e}")))?
        } else {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let provider = Arc::new(rustls::crypto::ring::default_provider());

            let tls_config = if database_url.contains("sslmode=verify-full") {
                let mut root_store = rustls::RootCertStore::empty();
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                rustls::ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth()
            } else {
                rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCertVerifier(provider)))
                    .with_no_client_auth()
            };

            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);
            let mgr = PgManager::new(pg_config, tls);
            PgPool::builder(mgr)
                .max_size(16)
                .runtime(deadpool_postgres::Runtime::Tokio1)
                .build()
                .map_err(|e| StrataError::Database(format!("Failed to build TLS Postgres pool: {e}")))?
        };

        // Validate connectivity
        let client = pool.get().await.map_err(|e| {
            StrataError::Database(format!("Failed to acquire initial PostgreSQL connection: {e}"))
        })?;
        drop(client);

        let storage = Self {
            backend: StorageBackend::Postgres {
                pool,
                has_pgvector: Arc::new(AtomicBool::new(false)),
            },
        };

        storage.init_postgres_schema().await?;
        Ok(storage)
    }

    /// Check if current storage is connected to PostgreSQL.
    pub fn is_postgres(&self) -> bool {
        matches!(self.backend, StorageBackend::Postgres { .. })
    }

    /// Check if pgvector extension is available and enabled.
    pub fn has_pgvector(&self) -> bool {
        match &self.backend {
            StorageBackend::Postgres { has_pgvector, .. } => {
                has_pgvector.load(Ordering::Relaxed)
            }
            StorageBackend::Sqlite(_) => false,
        }
    }

    // -------------------------------------------------------------
    // Schema Migrations
    // -------------------------------------------------------------

    fn init_sqlite_schema(&self) -> Result<(), StrataError> {
        let StorageBackend::Sqlite(conn) = &self.backend else {
            return Ok(());
        };
        let conn = conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on server SQLite connection".to_string())
        })?;

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

            CREATE INDEX IF NOT EXISTS idx_server_deltas_ws_seq ON server_deltas(workspace_id, seq);
            CREATE INDEX IF NOT EXISTS idx_server_deltas_kind ON server_deltas(kind);
            CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
            CREATE INDEX IF NOT EXISTS idx_workspaces_owner ON workspaces(owner_id);",
        )
        .map_err(|e| StrataError::Database(format!("Failed to initialize SQLite schema: {e}")))?;

        Ok(())
    }

    async fn init_postgres_schema(&self) -> Result<(), StrataError> {
        let StorageBackend::Postgres { pool, has_pgvector } = &self.backend else {
            return Ok(());
        };

        let client = pool.get().await.map_err(|e| {
            StrataError::Database(format!("Failed to get Postgres connection for schema migration: {e}"))
        })?;

        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS users (
                    id UUID PRIMARY KEY,
                    email TEXT UNIQUE NOT NULL,
                    password_hash TEXT NOT NULL,
                    full_name TEXT NOT NULL,
                    tier TEXT NOT NULL DEFAULT 'free',
                    created_at TIMESTAMPTZ NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL
                );

                CREATE TABLE IF NOT EXISTS workspaces (
                    id UUID PRIMARY KEY,
                    owner_id UUID NOT NULL REFERENCES users(id),
                    slug TEXT UNIQUE NOT NULL,
                    name TEXT NOT NULL,
                    memory_quota_bytes BIGINT NOT NULL DEFAULT 104857600,
                    created_at TIMESTAMPTZ NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL
                );

                CREATE TABLE IF NOT EXISTS api_keys (
                    id UUID PRIMARY KEY,
                    workspace_id UUID NOT NULL REFERENCES workspaces(id),
                    user_id UUID NOT NULL REFERENCES users(id),
                    name TEXT NOT NULL,
                    key_prefix TEXT NOT NULL,
                    key_hash TEXT UNIQUE NOT NULL,
                    scopes_json JSONB NOT NULL DEFAULT '[\"sync:read\",\"sync:write\"]'::jsonb,
                    last_used_at TIMESTAMPTZ,
                    expires_at TIMESTAMPTZ,
                    revoked_at TIMESTAMPTZ,
                    created_at TIMESTAMPTZ NOT NULL
                );

                CREATE TABLE IF NOT EXISTS workspace_sequences (
                    workspace_id TEXT PRIMARY KEY,
                    last_seq BIGINT NOT NULL DEFAULT 0,
                    updated_at TIMESTAMPTZ NOT NULL
                );

                CREATE TABLE IF NOT EXISTS server_deltas (
                    id UUID PRIMARY KEY,
                    workspace_id TEXT NOT NULL,
                    seq BIGINT NOT NULL,
                    client_seq BIGINT NOT NULL,
                    ts TIMESTAMPTZ NOT NULL,
                    kind TEXT NOT NULL,
                    payload JSONB NOT NULL,
                    version_hash TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_server_deltas_ws_seq ON server_deltas(workspace_id, seq);
                CREATE INDEX IF NOT EXISTS idx_server_deltas_kind ON server_deltas(kind);
                CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
                CREATE INDEX IF NOT EXISTS idx_workspaces_owner ON workspaces(owner_id);",
            )
            .await
            .map_err(|e| StrataError::Database(format!("Failed to execute PostgreSQL schema migration: {e}")))?;

        // Try enabling pgvector extension gracefully
        let vector_res = client
            .batch_execute(
                "CREATE EXTENSION IF NOT EXISTS vector;
                 CREATE TABLE IF NOT EXISTS server_embeddings (
                     id UUID PRIMARY KEY,
                     workspace_id TEXT NOT NULL,
                     memory_id UUID NOT NULL,
                     embedding vector(384),
                     metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
                     created_at TIMESTAMPTZ NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_server_embeddings_ws ON server_embeddings(workspace_id);",
            )
            .await;

        match vector_res {
            Ok(_) => {
                tracing::info!("✨ pgvector extension & vector tables successfully initialized.");
                has_pgvector.store(true, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!("ℹ️ pgvector extension not enabled on this PostgreSQL instance ({e}). Vector search will operate in fallback mode.");
                has_pgvector.store(false, Ordering::Relaxed);
            }
        }

        Ok(())
    }

    // -------------------------------------------------------------
    // User Accounts CRUD
    // -------------------------------------------------------------

    pub async fn create_user(
        &self,
        email: &str,
        password_hash: &str,
        full_name: &str,
    ) -> Result<User, StrataError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let email_clean = email.trim().to_lowercase();
        let name_clean = full_name.trim().to_string();

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;
                let now_str = now.to_rfc3339();

                conn.execute(
                    "INSERT INTO users (id, email, password_hash, full_name, tier, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        id.to_string(),
                        email_clean,
                        password_hash,
                        name_clean,
                        "free",
                        now_str,
                        now_str,
                    ],
                )
                .map_err(|e| StrataError::Database(format!("Failed to create user: {e}")))?;
            }
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                client
                    .execute(
                        "INSERT INTO users (id, email, password_hash, full_name, tier, created_at, updated_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7)",
                        &[&id, &email_clean, &password_hash, &name_clean, &"free", &now, &now],
                    )
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to create user in Postgres: {e}")))?;
            }
        }

        Ok(User {
            id,
            email: email_clean,
            password_hash: password_hash.to_string(),
            full_name: name_clean,
            tier: "free".to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StrataError> {
        let email_clean = email.trim().to_lowercase();

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;

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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let row = client
                    .query_opt(
                        "SELECT id, email, password_hash, full_name, tier, created_at, updated_at FROM users WHERE email = $1",
                        &[&email_clean],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(row.map(|r| User {
                    id: r.get(0),
                    email: r.get(1),
                    password_hash: r.get(2),
                    full_name: r.get(3),
                    tier: r.get(4),
                    created_at: r.get(5),
                    updated_at: r.get(6),
                }))
            }
        }
    }

    pub async fn get_user_by_id(&self, id: &Uuid) -> Result<Option<User>, StrataError> {
        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let row = client
                    .query_opt(
                        "SELECT id, email, password_hash, full_name, tier, created_at, updated_at FROM users WHERE id = $1",
                        &[id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(row.map(|r| User {
                    id: r.get(0),
                    email: r.get(1),
                    password_hash: r.get(2),
                    full_name: r.get(3),
                    tier: r.get(4),
                    created_at: r.get(5),
                    updated_at: r.get(6),
                }))
            }
        }
    }

    // -------------------------------------------------------------
    // Workspace Management
    // -------------------------------------------------------------

    pub async fn create_workspace(
        &self,
        owner_id: &Uuid,
        name: &str,
        slug: &str,
    ) -> Result<Workspace, StrataError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let slug_clean = slug.trim().to_lowercase();
        let name_clean = name.trim().to_string();
        let quota = 104857600_i64; // 100 MB default

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;
                let now_str = now.to_rfc3339();

                conn.execute(
                    "INSERT INTO workspaces (id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        id.to_string(),
                        owner_id.to_string(),
                        slug_clean,
                        name_clean,
                        quota,
                        now_str,
                        now_str,
                    ],
                )
                .map_err(|e| StrataError::Database(format!("Failed to create workspace: {e}")))?;
            }
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                client
                    .execute(
                        "INSERT INTO workspaces (id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7)",
                        &[&id, owner_id, &slug_clean, &name_clean, &quota, &now, &now],
                    )
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to create workspace in Postgres: {e}")))?;
            }
        }

        Ok(Workspace {
            id,
            owner_id: *owner_id,
            slug: slug_clean,
            name: name_clean,
            memory_quota_bytes: quota,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get_workspaces_for_user(&self, user_id: &Uuid) -> Result<Vec<Workspace>, StrataError> {
        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = client
                    .query(
                        "SELECT id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at
                         FROM workspaces
                         WHERE owner_id = $1
                         ORDER BY created_at ASC",
                        &[user_id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(rows
                    .into_iter()
                    .map(|r| Workspace {
                        id: r.get(0),
                        owner_id: r.get(1),
                        slug: r.get(2),
                        name: r.get(3),
                        memory_quota_bytes: r.get(4),
                        created_at: r.get(5),
                        updated_at: r.get(6),
                    })
                    .collect())
            }
        }
    }

    pub async fn get_workspace_by_id(&self, id: &Uuid) -> Result<Option<Workspace>, StrataError> {
        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let row = client
                    .query_opt(
                        "SELECT id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at FROM workspaces WHERE id = $1",
                        &[id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(row.map(|r| Workspace {
                    id: r.get(0),
                    owner_id: r.get(1),
                    slug: r.get(2),
                    name: r.get(3),
                    memory_quota_bytes: r.get(4),
                    created_at: r.get(5),
                    updated_at: r.get(6),
                }))
            }
        }
    }

    pub async fn get_workspace_by_slug(&self, slug: &str) -> Result<Option<Workspace>, StrataError> {
        let slug_clean = slug.trim().to_lowercase();

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;

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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let row = client
                    .query_opt(
                        "SELECT id, owner_id, slug, name, memory_quota_bytes, created_at, updated_at FROM workspaces WHERE slug = $1",
                        &[&slug_clean],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(row.map(|r| Workspace {
                    id: r.get(0),
                    owner_id: r.get(1),
                    slug: r.get(2),
                    name: r.get(3),
                    memory_quota_bytes: r.get(4),
                    created_at: r.get(5),
                    updated_at: r.get(6),
                }))
            }
        }
    }

    // -------------------------------------------------------------
    // API Keys Management
    // -------------------------------------------------------------

    pub async fn create_api_key(
        &self,
        workspace_id: &Uuid,
        user_id: &Uuid,
        name: &str,
        prefix: &str,
        hash: &str,
        scopes: &[String],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiKey, StrataError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let name_clean = name.trim().to_string();

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;
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
                        name_clean,
                        prefix,
                        hash,
                        scopes_json,
                        expires_str,
                        now_str,
                    ],
                )
                .map_err(|e| StrataError::Database(format!("Failed to insert API key: {e}")))?;
            }
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let scopes_value = serde_json::to_value(scopes)?;

                client
                    .execute(
                        "INSERT INTO api_keys (id, workspace_id, user_id, name, key_prefix, key_hash, scopes_json, expires_at, created_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                        &[
                            &id,
                            workspace_id,
                            user_id,
                            &name_clean,
                            &prefix,
                            &hash,
                            &scopes_value,
                            &expires_at,
                            &now,
                        ],
                    )
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to insert API key in Postgres: {e}")))?;
            }
        }

        Ok(ApiKey {
            id,
            workspace_id: *workspace_id,
            user_id: *user_id,
            name: name_clean,
            key_prefix: prefix.to_string(),
            key_hash: hash.to_string(),
            scopes: scopes.to_vec(),
            last_used_at: None,
            expires_at,
            revoked_at: None,
            created_at: now,
        })
    }

    pub async fn list_api_keys_for_workspace(&self, workspace_id: &Uuid) -> Result<Vec<ApiKey>, StrataError> {
        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = client
                    .query(
                        "SELECT id, workspace_id, user_id, name, key_prefix, key_hash, scopes_json, last_used_at, expires_at, revoked_at, created_at
                         FROM api_keys
                         WHERE workspace_id = $1 AND revoked_at IS NULL
                         ORDER BY created_at DESC",
                        &[workspace_id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(rows
                    .into_iter()
                    .map(|r| {
                        let scopes_val: serde_json::Value = r.get(6);
                        let scopes: Vec<String> = serde_json::from_value(scopes_val).unwrap_or_default();
                        ApiKey {
                            id: r.get(0),
                            workspace_id: r.get(1),
                            user_id: r.get(2),
                            name: r.get(3),
                            key_prefix: r.get(4),
                            key_hash: r.get(5),
                            scopes,
                            last_used_at: r.get(7),
                            expires_at: r.get(8),
                            revoked_at: r.get(9),
                            created_at: r.get(10),
                        }
                    })
                    .collect())
            }
        }
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, StrataError> {
        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let row = client
                    .query_opt(
                        "SELECT id, workspace_id, user_id, name, key_prefix, key_hash, scopes_json, last_used_at, expires_at, revoked_at, created_at
                         FROM api_keys
                         WHERE key_hash = $1 AND revoked_at IS NULL",
                        &[&key_hash],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(row.map(|r| {
                    let scopes_val: serde_json::Value = r.get(6);
                    let scopes: Vec<String> = serde_json::from_value(scopes_val).unwrap_or_default();
                    ApiKey {
                        id: r.get(0),
                        workspace_id: r.get(1),
                        user_id: r.get(2),
                        name: r.get(3),
                        key_prefix: r.get(4),
                        key_hash: r.get(5),
                        scopes,
                        last_used_at: r.get(7),
                        expires_at: r.get(8),
                        revoked_at: r.get(9),
                        created_at: r.get(10),
                    }
                }))
            }
        }
    }

    pub async fn record_api_key_usage(&self, id: &Uuid) -> Result<(), StrataError> {
        let now = Utc::now();

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;
                let now_str = now.to_rfc3339();
                conn.execute(
                    "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
                    params![now_str, id.to_string()],
                )
                .map_err(|e| StrataError::Database(e.to_string()))?;
            }
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                client
                    .execute("UPDATE api_keys SET last_used_at = $1 WHERE id = $2", &[&now, id])
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;
            }
        }

        Ok(())
    }

    pub async fn revoke_api_key(&self, id: &Uuid, user_id: &Uuid) -> Result<bool, StrataError> {
        let now = Utc::now();

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;
                let now_str = now.to_rfc3339();
                let rows = conn
                    .execute(
                        "UPDATE api_keys SET revoked_at = ?1 WHERE id = ?2 AND user_id = ?3",
                        params![now_str, id.to_string(), user_id.to_string()],
                    )
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                Ok(rows > 0)
            }
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = client
                    .execute(
                        "UPDATE api_keys SET revoked_at = $1 WHERE id = $2 AND user_id = $3",
                        &[&now, id, user_id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                Ok(rows > 0)
            }
        }
    }

    // -------------------------------------------------------------
    // CDC Deltas & Synchronization Storage
    // -------------------------------------------------------------

    /// Push a batch of incoming deltas for a workspace.
    /// Assigns monotonic sequential IDs on the server per workspace.
    pub async fn push_deltas(&self, workspace_id: &str, deltas: Vec<SyncDelta>) -> Result<(usize, u64), StrataError> {
        if deltas.is_empty() {
            let (_, current_seq) = self.get_status(workspace_id).await?;
            return Ok((0, current_seq));
        }

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let mut conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;

                let tx = conn
                    .transaction()
                    .map_err(|e| StrataError::Database(format!("Failed to begin SQLite transaction: {e}")))?;

                let now_str = Utc::now().to_rfc3339();

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
            StorageBackend::Postgres { pool, .. } => {
                let mut client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let tx = client
                    .transaction()
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to begin PostgreSQL transaction: {e}")))?;

                let now = Utc::now();

                // 1. Get or initialize sequence with row lock for multi-instance safety
                let row_opt = tx
                    .query_opt(
                        "SELECT last_seq FROM workspace_sequences WHERE workspace_id = $1 FOR UPDATE",
                        &[&workspace_id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to query workspace sequence: {e}")))?;

                let mut last_seq: i64 = row_opt.map(|r| r.get(0)).unwrap_or(0);
                let mut inserted_count = 0;

                for delta in deltas {
                    let exists_row = tx
                        .query_opt("SELECT seq FROM server_deltas WHERE id = $1", &[&delta.id])
                        .await
                        .map_err(|e| StrataError::Database(format!("Failed to check delta existence: {e}")))?;

                    if exists_row.is_some() {
                        continue;
                    }

                    last_seq += 1;
                    let payload_value = serde_json::to_value(&delta.payload)?;

                    tx.execute(
                        "INSERT INTO server_deltas (
                            id, workspace_id, seq, client_seq, ts, kind, payload, version_hash, created_at
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                        &[
                            &delta.id,
                            &workspace_id,
                            &last_seq,
                            &(delta.seq as i64),
                            &delta.ts,
                            &delta.kind,
                            &payload_value,
                            &delta.version_hash,
                            &now,
                        ],
                    )
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to insert server delta in Postgres: {e}")))?;

                    inserted_count += 1;
                }

                tx.execute(
                    "INSERT INTO workspace_sequences (workspace_id, last_seq, updated_at)
                     VALUES ($1, $2, $3)
                     ON CONFLICT(workspace_id) DO UPDATE SET
                         last_seq = EXCLUDED.last_seq,
                         updated_at = EXCLUDED.updated_at",
                    &[&workspace_id, &last_seq, &now],
                )
                .await
                .map_err(|e| StrataError::Database(format!("Failed to update workspace sequence in Postgres: {e}")))?;

                tx.commit()
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to commit PostgreSQL delta push: {e}")))?;

                Ok((inserted_count, last_seq as u64))
            }
        }
    }

    /// Pull deltas for a workspace starting strictly after `since_seq` up to `limit`.
    pub async fn pull_deltas(
        &self,
        workspace_id: &str,
        since_seq: u64,
        limit: usize,
    ) -> Result<Vec<SyncDelta>, StrataError> {
        let capped_limit = limit.clamp(1, 1000);

        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
                    StrataError::Database("Lock poisoned on server SQLite connection".to_string())
                })?;

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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = client
                    .query(
                        "SELECT id, workspace_id, seq, ts, kind, payload, version_hash
                         FROM server_deltas
                         WHERE workspace_id = $1 AND seq > $2
                         ORDER BY seq ASC
                         LIMIT $3",
                        &[&workspace_id, &(since_seq as i64), &(capped_limit as i64)],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(rows
                    .into_iter()
                    .map(|r| SyncDelta {
                        id: r.get(0),
                        workspace_id: r.get(1),
                        seq: r.get::<_, i64>(2) as u64,
                        ts: r.get(3),
                        kind: r.get(4),
                        payload: r.get(5),
                        version_hash: r.get(6),
                        synced: true,
                    })
                    .collect())
            }
        }
    }

    /// Retrieve total deltas count and maximum sequence number for a workspace.
    pub async fn get_status(&self, workspace_id: &str) -> Result<(usize, u64), StrataError> {
        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;

                let count_row = client
                    .query_one(
                        "SELECT COUNT(*)::bigint FROM server_deltas WHERE workspace_id = $1",
                        &[&workspace_id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                let total_count: i64 = count_row.get(0);

                let seq_row = client
                    .query_opt(
                        "SELECT COALESCE(last_seq, 0) FROM workspace_sequences WHERE workspace_id = $1",
                        &[&workspace_id],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                let max_seq: i64 = seq_row.map(|r| r.get(0)).unwrap_or(0);

                Ok((total_count as usize, max_seq as u64))
            }
        }
    }

    /// List all known workspaces.
    pub async fn list_workspaces(&self) -> Result<Vec<String>, StrataError> {
        match &self.backend {
            StorageBackend::Sqlite(conn) => {
                let conn = conn.lock().map_err(|_| {
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
            StorageBackend::Postgres { pool, .. } => {
                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = client
                    .query(
                        "SELECT slug FROM workspaces UNION SELECT workspace_id FROM workspace_sequences ORDER BY 1 ASC",
                        &[],
                    )
                    .await
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                Ok(rows.into_iter().map(|r| r.get(0)).collect())
            }
        }
    }

    // -------------------------------------------------------------
    // Vector & Embeddings Storage (pgvector)
    // -------------------------------------------------------------

    /// Upsert an embedding into the central cloud vector store.
    pub async fn upsert_embedding(
        &self,
        workspace_id: &str,
        memory_id: &Uuid,
        embedding: &[f32],
        metadata: &serde_json::Value,
    ) -> Result<(), StrataError> {
        match &self.backend {
            StorageBackend::Postgres { pool, has_pgvector } => {
                if !has_pgvector.load(Ordering::Relaxed) {
                    tracing::debug!("pgvector not active; skipping cloud vector indexing.");
                    return Ok(());
                }

                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let id = Uuid::new_v4();
                let now = Utc::now();
                let vec = Vector::from(embedding.to_vec());

                client
                    .execute(
                        "INSERT INTO server_embeddings (id, workspace_id, memory_id, embedding, metadata, created_at)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT (id) DO UPDATE SET
                             embedding = EXCLUDED.embedding,
                             metadata = EXCLUDED.metadata",
                        &[&id, &workspace_id, memory_id, &vec, metadata, &now],
                    )
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to upsert embedding: {e}")))?;

                Ok(())
            }
            StorageBackend::Sqlite(_) => {
                // SQLite uses local strata-memory vector indexing
                Ok(())
            }
        }
    }

    /// Search embeddings by cosine similarity in the central cloud vector store.
    pub async fn search_embeddings(
        &self,
        workspace_id: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorSearchResult>, StrataError> {
        let capped_limit = limit.clamp(1, 100);

        match &self.backend {
            StorageBackend::Postgres { pool, has_pgvector } => {
                if !has_pgvector.load(Ordering::Relaxed) {
                    return Ok(Vec::new());
                }

                let client = pool.get().await.map_err(|e| StrataError::Database(e.to_string()))?;
                let vec = Vector::from(query_embedding.to_vec());

                let rows = client
                    .query(
                        "SELECT memory_id, metadata, (embedding <=> $1) as distance
                         FROM server_embeddings
                         WHERE workspace_id = $2
                         ORDER BY embedding <=> $1 ASC
                         LIMIT $3",
                        &[&vec, &workspace_id, &(capped_limit as i64)],
                    )
                    .await
                    .map_err(|e| StrataError::Database(format!("Failed to search embeddings: {e}")))?;

                Ok(rows
                    .into_iter()
                    .map(|r| {
                        let memory_id: Uuid = r.get(0);
                        let metadata: serde_json::Value = r.get(1);
                        let distance: f64 = r.get(2);
                        // Convert cosine distance [0, 2] to similarity score [0, 1]
                        let score = (1.0 - (distance as f32)).clamp(0.0, 1.0);
                        VectorSearchResult {
                            memory_id,
                            score,
                            metadata,
                        }
                    })
                    .collect())
            }
            StorageBackend::Sqlite(_) => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_user_and_workspace_and_api_keys_storage_flow() {
        let storage = ServerStorage::in_memory().expect("Failed to create storage");

        // 1. Create user
        let user = storage
            .create_user("pedro@strata.dev", "hashed_pwd", "Pedro Farath")
            .await
            .expect("Create user failed");
        assert_eq!(user.email, "pedro@strata.dev");

        // 2. Fetch user by email & id
        let fetched = storage.get_user_by_email("pedro@strata.dev").await.unwrap().unwrap();
        assert_eq!(fetched.id, user.id);

        // 3. Create workspace
        let ws = storage
            .create_workspace(&user.id, "Pedro Dev Workspace", "phfarath-dev")
            .await
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
            .await
            .expect("Create API key failed");
        assert_eq!(key.name, "MacBook Pro Cursor");

        // 5. Lookup API Key by hash
        let lookup_key = storage.get_api_key_by_hash("hash_1234567890").await.unwrap().unwrap();
        assert_eq!(lookup_key.id, key.id);

        // 6. Revoke API Key
        assert!(storage.revoke_api_key(&key.id, &user.id).await.unwrap());
        assert!(storage.get_api_key_by_hash("hash_1234567890").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_server_storage_in_memory_crud_and_idempotency() {
        let storage = ServerStorage::in_memory().expect("Failed to create storage");
        let ws = "test-ws";

        // Initial status should be empty
        let (count, max_seq) = storage.get_status(ws).await.unwrap();
        assert_eq!(count, 0);
        assert_eq!(max_seq, 0);

        let delta1_id = Uuid::new_v4();
        let delta1 = SyncDelta::new(ws, 1, "fact", json!({"statement": "fact 1"}), "hash1")
            .with_id(delta1_id);
        let delta2 = SyncDelta::new(ws, 2, "fact", json!({"statement": "fact 2"}), "hash2");

        // Push 2 deltas
        let (pushed, seq) = storage.push_deltas(ws, vec![delta1.clone(), delta2.clone()]).await.unwrap();
        assert_eq!(pushed, 2);
        assert_eq!(seq, 2);

        // Idempotency: push delta1 again, should be skipped
        let (pushed_again, seq_again) = storage.push_deltas(ws, vec![delta1]).await.unwrap();
        assert_eq!(pushed_again, 0);
        assert_eq!(seq_again, 2);

        // Pull deltas since_seq 0
        let all_deltas = storage.pull_deltas(ws, 0, 100).await.unwrap();
        assert_eq!(all_deltas.len(), 2);
        assert_eq!(all_deltas[0].seq, 1);
        assert_eq!(all_deltas[1].seq, 2);

        // Pull deltas since_seq 1
        let since_1 = storage.pull_deltas(ws, 1, 100).await.unwrap();
        assert_eq!(since_1.len(), 1);
        assert_eq!(since_1[0].seq, 2);
    }
}
