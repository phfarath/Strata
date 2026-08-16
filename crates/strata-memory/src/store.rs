use std::path::Path;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::events::{
    DataClassification, Event, EventId, EventPayload, Provenance, RetentionPolicy,
};
use strata_core::state::{
    FailurePattern, FailureSeverity, MemoryRecord, MemoryType, Scope,
};

use crate::embedding::{bytes_to_embedding, embedding_to_bytes};

pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StrataError> {
        let conn = Connection::open(path)
            .map_err(|e| StrataError::Database(format!("Failed to open SQLite database: {e}")))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, StrataError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| StrataError::Database(format!("Failed to open in-memory SQLite: {e}")))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn from_connection(conn: Connection) -> Result<Self, StrataError> {
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(None, "foreign_keys", "ON");
        let _ = conn.pragma_update(None, "busy_timeout", 5000);

        conn.execute_batch(
            "
            -- Canonical Events table
            CREATE TABLE IF NOT EXISTS events (
                sequence_num INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT UNIQUE NOT NULL,
                timestamp TEXT NOT NULL,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                organization_id TEXT,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                provenance_json TEXT NOT NULL,
                classification TEXT NOT NULL,
                retention TEXT NOT NULL,
                metadata_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, sequence_num);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);

            -- Memories table
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                memory_type TEXT NOT NULL,
                content TEXT NOT NULL,
                summary TEXT,
                scope TEXT NOT NULL,
                importance REAL NOT NULL DEFAULT 0.5,
                confidence REAL NOT NULL DEFAULT 1.0,
                tags_json TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                access_count INTEGER NOT NULL DEFAULT 0,
                last_accessed_at TEXT,
                evidence_ids_json TEXT NOT NULL DEFAULT '[]',
                embedding BLOB,
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope);
            CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);

            -- Full-text search virtual table for memories
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                id UNINDEXED,
                content,
                summary,
                tags,
                tokenize = 'porter unicode61'
            );

            -- FTS Triggers for memories
            CREATE TRIGGER IF NOT EXISTS trg_memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(id, content, summary, tags)
                VALUES (new.id, new.content, COALESCE(new.summary, ''), new.tags_json);
            END;

            CREATE TRIGGER IF NOT EXISTS trg_memories_ad AFTER DELETE ON memories BEGIN
                DELETE FROM memories_fts WHERE id = old.id;
            END;

            CREATE TRIGGER IF NOT EXISTS trg_memories_au AFTER UPDATE ON memories BEGIN
                DELETE FROM memories_fts WHERE id = old.id;
                INSERT INTO memories_fts(id, content, summary, tags)
                VALUES (new.id, new.content, COALESCE(new.summary, ''), new.tags_json);
            END;

            -- Failure Patterns table
            CREATE TABLE IF NOT EXISTS failure_patterns (
                id TEXT PRIMARY KEY,
                signature TEXT UNIQUE NOT NULL,
                pattern_name TEXT NOT NULL,
                description TEXT NOT NULL,
                trigger_condition TEXT NOT NULL,
                error_type TEXT NOT NULL,
                mitigation TEXT NOT NULL,
                occurrences INTEGER NOT NULL DEFAULT 1,
                first_seen TEXT NOT NULL,
                last_seen TEXT NOT NULL,
                severity TEXT NOT NULL DEFAULT 'medium',
                scope TEXT NOT NULL DEFAULT 'global',
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_failures_sig ON failure_patterns(signature);
            CREATE INDEX IF NOT EXISTS idx_failures_scope ON failure_patterns(scope);
            CREATE INDEX IF NOT EXISTS idx_failures_last_seen ON failure_patterns(last_seen);

            -- Failure patterns full text search
            CREATE VIRTUAL TABLE IF NOT EXISTS failure_patterns_fts USING fts5(
                signature UNINDEXED,
                pattern_name,
                description,
                trigger_condition,
                error_type,
                mitigation,
                tokenize = 'porter unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS trg_failure_ai AFTER INSERT ON failure_patterns BEGIN
                INSERT INTO failure_patterns_fts(signature, pattern_name, description, trigger_condition, error_type, mitigation)
                VALUES (new.signature, new.pattern_name, new.description, new.trigger_condition, new.error_type, new.mitigation);
            END;

            CREATE TRIGGER IF NOT EXISTS trg_failure_ad AFTER DELETE ON failure_patterns BEGIN
                DELETE FROM failure_patterns_fts WHERE signature = old.signature;
            END;

            CREATE TRIGGER IF NOT EXISTS trg_failure_au AFTER UPDATE ON failure_patterns BEGIN
                DELETE FROM failure_patterns_fts WHERE signature = old.signature;
                INSERT INTO failure_patterns_fts(signature, pattern_name, description, trigger_condition, error_type, mitigation)
                VALUES (new.signature, new.pattern_name, new.description, new.trigger_condition, new.error_type, new.mitigation);
            END;
            ",
        )
        .map_err(|e| StrataError::Database(format!("Failed to execute schema migration: {e}")))?;

        Ok(())
    }

    // ==========================================
    // Event Store Operations
    // ==========================================

    pub fn insert_event(&self, event: &Event) -> Result<EventId, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = event.id.to_string();
        let ts_str = event.timestamp.to_rfc3339();
        let payload_json = serde_json::to_string(&event.payload)?;
        let provenance_json = serde_json::to_string(&event.provenance)?;
        let classification_str = serde_json::to_string(&event.classification)?;
        let retention_str = serde_json::to_string(&event.retention)?;
        let metadata_json = serde_json::to_string(&event.metadata)?;
        let event_type = event.payload.event_type();

        conn.execute(
            "INSERT INTO events (
                id, timestamp, session_id, agent_id, organization_id,
                event_type, payload_json, provenance_json, classification,
                retention, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id_str,
                ts_str,
                event.session_id,
                event.agent_id,
                event.organization_id,
                event_type,
                payload_json,
                provenance_json,
                classification_str,
                retention_str,
                metadata_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert event: {e}")))?;

        Ok(event.id)
    }

    pub fn get_events(
        &self,
        session_id: &str,
        from_seq: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<Event>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let from_seq_val = from_seq.unwrap_or(0);
        let limit_val = limit.unwrap_or(1000) as i64;

        let mut stmt = conn
            .prepare(
                "SELECT id, sequence_num, timestamp, session_id, agent_id,
                        organization_id, payload_json, provenance_json,
                        classification, retention, metadata_json
                 FROM events
                 WHERE session_id = ?1 AND sequence_num >= ?2
                 ORDER BY sequence_num ASC
                 LIMIT ?3",
            )
            .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![session_id, from_seq_val, limit_val], |row| {
                let id_str: String = row.get(0)?;
                let seq_num: i64 = row.get(1)?;
                let ts_str: String = row.get(2)?;
                let session_id: String = row.get(3)?;
                let agent_id: String = row.get(4)?;
                let org_id: Option<String> = row.get(5)?;
                let payload_json: String = row.get(6)?;
                let prov_json: String = row.get(7)?;
                let class_json: String = row.get(8)?;
                let ret_json: String = row.get(9)?;
                let meta_json: String = row.get(10)?;

                Ok((
                    id_str,
                    seq_num,
                    ts_str,
                    session_id,
                    agent_id,
                    org_id,
                    payload_json,
                    prov_json,
                    class_json,
                    ret_json,
                    meta_json,
                ))
            })
            .map_err(|e| StrataError::Database(format!("Query failed: {e}")))?;

        let mut events = Vec::new();
        for row in rows {
            let (
                id_str,
                seq_num,
                ts_str,
                s_id,
                a_id,
                org_id,
                payload_json,
                prov_json,
                class_json,
                ret_json,
                meta_json,
            ) = row.map_err(|e| StrataError::Database(e.to_string()))?;

            let id = id_str.parse::<Uuid>().map(EventId::from_uuid).map_err(|e| {
                StrataError::Validation(format!("Invalid UUID in event id: {e}"))
            })?;
            let timestamp = DateTime::parse_from_rfc3339(&ts_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let payload: EventPayload = serde_json::from_str(&payload_json)?;
            let provenance: Provenance = serde_json::from_str(&prov_json)?;
            let classification: DataClassification = serde_json::from_str(&class_json)?;
            let retention: RetentionPolicy = serde_json::from_str(&ret_json)?;
            let metadata: serde_json::Value = serde_json::from_str(&meta_json)?;

            events.push(Event {
                id,
                sequence: Some(seq_num as u64),
                timestamp,
                session_id: s_id,
                agent_id: a_id,
                organization_id: org_id,
                provenance,
                classification,
                retention,
                payload,
                metadata,
            });
        }

        Ok(events)
    }

    // ==========================================
    // Memory Record Operations
    // ==========================================

    pub fn insert_or_update_memory(&self, memory: &MemoryRecord) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = memory.id.to_string();
        let mem_type_str = memory.memory_type.to_string();
        let scope_str = memory.scope.to_string();
        let tags_json = serde_json::to_string(&memory.tags)?;
        let evidence_json = serde_json::to_string(&memory.evidence_ids)?;
        let metadata_json = serde_json::to_string(&memory.metadata)?;
        let created_at_str = memory.created_at.to_rfc3339();
        let updated_at_str = memory.updated_at.to_rfc3339();
        let last_accessed_str = memory.last_accessed_at.map(|t| t.to_rfc3339());
        let embedding_blob = memory.embedding.as_ref().map(|e| embedding_to_bytes(e));

        conn.execute(
            "INSERT INTO memories (
                id, memory_type, content, summary, scope, importance, confidence,
                tags_json, created_at, updated_at, access_count, last_accessed_at,
                evidence_ids_json, embedding, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(id) DO UPDATE SET
                memory_type = excluded.memory_type,
                content = excluded.content,
                summary = excluded.summary,
                scope = excluded.scope,
                importance = excluded.importance,
                confidence = excluded.confidence,
                tags_json = excluded.tags_json,
                updated_at = excluded.updated_at,
                access_count = excluded.access_count,
                last_accessed_at = excluded.last_accessed_at,
                evidence_ids_json = excluded.evidence_ids_json,
                embedding = COALESCE(excluded.embedding, memories.embedding),
                metadata_json = excluded.metadata_json",
            params![
                id_str,
                mem_type_str,
                memory.content,
                memory.summary,
                scope_str,
                memory.importance,
                memory.confidence,
                tags_json,
                created_at_str,
                updated_at_str,
                memory.access_count as i64,
                last_accessed_str,
                evidence_json,
                embedding_blob,
                metadata_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to persist memory: {e}")))?;

        Ok(())
    }

    pub fn get_memory(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, content, summary, scope, importance,
                        confidence, tags_json, created_at, updated_at, access_count,
                        last_accessed_at, evidence_ids_json, embedding, metadata_json
                 FROM memories
                 WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![id.to_string()], |row| Self::row_to_memory(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(res)
    }

    pub fn delete_memory(&self, id: &Uuid) -> Result<bool, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let rows = conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id.to_string()])
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(rows > 0)
    }

    pub fn get_all_memories(
        &self,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut query = "SELECT id, memory_type, content, summary, scope, importance,
                                confidence, tags_json, created_at, updated_at, access_count,
                                last_accessed_at, evidence_ids_json, embedding, metadata_json
                         FROM memories WHERE 1=1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(sc) = scope {
            if *sc != Scope::Global {
                query.push_str(" AND (scope = ? OR scope = 'global')");
                params_vec.push(Box::new(sc.to_string()));
            }
        }

        if let Some(types) = memory_types {
            if !types.is_empty() {
                let type_strs: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                let placeholders = vec!["?"; type_strs.len()].join(",");
                query.push_str(&format!(" AND memory_type IN ({placeholders})"));
                for t in type_strs {
                    params_vec.push(Box::new(t));
                }
            }
        }

        query.push_str(" ORDER BY created_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| Self::row_to_memory(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Search memories using FTS5 Porter stemmer.
    /// Returns (MemoryRecord, raw BM25 score).
    pub fn search_fts(
        &self,
        query_text: &str,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<(MemoryRecord, f32)>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let sanitized = sanitize_fts5_query(query_text);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = "
            SELECT m.id, m.memory_type, m.content, m.summary, m.scope, m.importance,
                   m.confidence, m.tags_json, m.created_at, m.updated_at, m.access_count,
                   m.last_accessed_at, m.evidence_ids_json, m.embedding, m.metadata_json,
                   bm25(memories_fts) as rank
            FROM memories_fts f
            JOIN memories m ON m.id = f.id
            WHERE memories_fts MATCH ?1
        ".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(sanitized));

        if let Some(sc) = scope {
            if *sc != Scope::Global {
                sql.push_str(" AND (m.scope = ?2 OR m.scope = 'global')");
                params_vec.push(Box::new(sc.to_string()));
            }
        }

        sql.push_str(" ORDER BY rank ASC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| {
                let mem = Self::row_to_memory(row)?;
                let rank: f64 = row.get(15)?;
                Ok((mem, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    // ==========================================
    // Failure Patterns Operations
    // ==========================================

    pub fn upsert_failure_pattern(&self, failure: &FailurePattern) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = failure.id.to_string();
        let first_seen_str = failure.first_seen.to_rfc3339();
        let last_seen_str = failure.last_seen.to_rfc3339();
        let sev_str = failure.severity.to_string();
        let scope_str = failure.scope.to_string();
        let metadata_json = serde_json::to_string(&failure.metadata)?;

        conn.execute(
            "INSERT INTO failure_patterns (
                id, signature, pattern_name, description, trigger_condition,
                error_type, mitigation, occurrences, first_seen, last_seen,
                severity, scope, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(signature) DO UPDATE SET
                pattern_name = excluded.pattern_name,
                description = excluded.description,
                trigger_condition = excluded.trigger_condition,
                error_type = excluded.error_type,
                mitigation = excluded.mitigation,
                occurrences = failure_patterns.occurrences + excluded.occurrences,
                last_seen = excluded.last_seen,
                severity = excluded.severity,
                scope = excluded.scope,
                metadata_json = excluded.metadata_json",
            params![
                id_str,
                failure.signature,
                failure.pattern_name,
                failure.description,
                failure.trigger_condition,
                failure.error_type,
                failure.mitigation,
                failure.occurrences as i64,
                first_seen_str,
                last_seen_str,
                sev_str,
                scope_str,
                metadata_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to upsert failure pattern: {e}")))?;

        Ok(())
    }

    pub fn get_failure_pattern_by_signature(
        &self,
        signature: &str,
    ) -> Result<Option<FailurePattern>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, signature, pattern_name, description, trigger_condition,
                        error_type, mitigation, occurrences, first_seen, last_seen,
                        severity, scope, metadata_json
                 FROM failure_patterns
                 WHERE signature = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![signature], |row| Self::row_to_failure(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(res)
    }

    pub fn search_failures(
        &self,
        query_text: Option<&str>,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        if let Some(q) = query_text {
            let sanitized = sanitize_fts5_query(q);
            if !sanitized.is_empty() {
                let mut sql = "
                    SELECT f.id, f.signature, f.pattern_name, f.description, f.trigger_condition,
                           f.error_type, f.mitigation, f.occurrences, f.first_seen, f.last_seen,
                           f.severity, f.scope, f.metadata_json
                    FROM failure_patterns_fts s
                    JOIN failure_patterns f ON f.signature = s.signature
                    WHERE failure_patterns_fts MATCH ?1
                ".to_string();

                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
                params_vec.push(Box::new(sanitized));

                if let Some(sc) = scope {
                    if *sc != Scope::Global {
                        sql.push_str(" AND (f.scope = ?2 OR f.scope = 'global')");
                        params_vec.push(Box::new(sc.to_string()));
                    }
                }

                sql.push_str(" ORDER BY f.occurrences DESC, f.last_seen DESC LIMIT ?");
                params_vec.push(Box::new(limit as i64));

                let mut stmt = conn
                    .prepare(&sql)
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                let params_slice: Vec<&dyn rusqlite::ToSql> =
                    params_vec.iter().map(|b| b.as_ref()).collect();

                let rows = stmt
                    .query_map(params_slice.as_slice(), |row| Self::row_to_failure(row))
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                let mut results = Vec::new();
                for r in rows {
                    results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
                }
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }

        // Fallback or unfiltered retrieval
        let mut sql = "
            SELECT id, signature, pattern_name, description, trigger_condition,
                   error_type, mitigation, occurrences, first_seen, last_seen,
                   severity, scope, metadata_json
            FROM failure_patterns
            WHERE 1=1
        ".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(sc) = scope {
            if *sc != Scope::Global {
                sql.push_str(" AND (scope = ? OR scope = 'global')");
                params_vec.push(Box::new(sc.to_string()));
            }
        }

        sql.push_str(" ORDER BY occurrences DESC, last_seen DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| Self::row_to_failure(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    // ==========================================
    // Row Mapping Helpers
    // ==========================================

    fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<MemoryRecord> {
        let id_str: String = row.get(0)?;
        let mem_type_str: String = row.get(1)?;
        let content: String = row.get(2)?;
        let summary: Option<String> = row.get(3)?;
        let scope_str: String = row.get(4)?;
        let importance: f64 = row.get(5)?;
        let confidence: f64 = row.get(6)?;
        let tags_json: String = row.get(7)?;
        let created_at_str: String = row.get(8)?;
        let updated_at_str: String = row.get(9)?;
        let access_count: i64 = row.get(10)?;
        let last_accessed_str: Option<String> = row.get(11)?;
        let evidence_json: String = row.get(12)?;
        let embedding_bytes: Option<Vec<u8>> = row.get(13)?;
        let metadata_json: String = row.get(14)?;

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let memory_type = mem_type_str
            .parse::<MemoryType>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let scope = scope_str
            .parse::<Scope>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let evidence_ids: Vec<Uuid> = serde_json::from_str(&evidence_json).unwrap_or_default();
        let metadata: serde_json::Value =
            serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);

        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let last_accessed_at = last_accessed_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let embedding = embedding_bytes.and_then(|b| bytes_to_embedding(&b).ok());

        Ok(MemoryRecord {
            id,
            memory_type,
            content,
            summary,
            scope,
            importance: importance as f32,
            confidence: confidence as f32,
            tags,
            created_at,
            updated_at,
            access_count: access_count as u64,
            last_accessed_at,
            evidence_ids,
            embedding,
            metadata,
        })
    }

    fn row_to_failure(row: &rusqlite::Row) -> rusqlite::Result<FailurePattern> {
        let id_str: String = row.get(0)?;
        let signature: String = row.get(1)?;
        let pattern_name: String = row.get(2)?;
        let description: String = row.get(3)?;
        let trigger_condition: String = row.get(4)?;
        let error_type: String = row.get(5)?;
        let mitigation: String = row.get(6)?;
        let occurrences: i64 = row.get(7)?;
        let first_seen_str: String = row.get(8)?;
        let last_seen_str: String = row.get(9)?;
        let sev_str: String = row.get(10)?;
        let scope_str: String = row.get(11)?;
        let metadata_json: String = row.get(12)?;

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let first_seen = DateTime::parse_from_rfc3339(&first_seen_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let last_seen = DateTime::parse_from_rfc3339(&last_seen_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let severity = sev_str
            .parse::<FailureSeverity>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let scope = scope_str
            .parse::<Scope>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let metadata: serde_json::Value =
            serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);

        Ok(FailurePattern {
            id,
            signature,
            pattern_name,
            description,
            trigger_condition,
            error_type,
            mitigation,
            occurrences: occurrences as u64,
            first_seen,
            last_seen,
            severity,
            scope,
            metadata,
        })
    }
}

/// Helper to sanitize and prepare a query for SQLite FTS5 MATCH expressions.
pub fn sanitize_fts5_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let clean = s.replace('\"', "");
            format!("\"{clean}\"*")
        })
        .collect();

    tokens.join(" OR ")
}
