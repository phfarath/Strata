use std::path::Path;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use strata_core::errors::StrataError;
use strata_core::schemas::SyncDelta;
use uuid::Uuid;

/// Server-side SQLite storage for synchronizing workspace CDC deltas.
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

    /// Initialize SQLite schema with WAL mode and indices.
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
            "CREATE TABLE IF NOT EXISTS workspace_sequences (
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
                ON server_deltas(kind);",
        )
        .map_err(|e| StrataError::Database(format!("Failed to initialize server schema: {e}")))?;

        Ok(())
    }

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
            .prepare("SELECT workspace_id FROM workspace_sequences ORDER BY workspace_id ASC")
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

        // List workspaces
        let workspaces = storage.list_workspaces().unwrap();
        assert_eq!(workspaces, vec!["test-ws".to_string()]);
    }
}

