use std::path::Path;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::events::{
    DataClassification, Event, EventId, EventPayload, Provenance, RetentionPolicy,
};
use strata_core::schemas::{
    CodeAnchor, EpisodicMemory, EvidenceRef, FactStatus, FeedbackEvent, FeedbackRating, ImplicitSignal,
    MemoryFeedback, ParameterDef, PreferencePair, ProceduralExample,
    ProceduralSkill, ProceduralStep, SemanticFact, SignalKind, SignalScores, SyncDelta,
};

use strata_core::state::{
    FailurePattern, FailureSeverity, MemoryRecord, MemoryTier, MemoryType, Scope,
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
                tier TEXT NOT NULL DEFAULT 'peripheral',
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
            CREATE INDEX IF NOT EXISTS idx_memories_tier ON memories(tier);
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

            -- Cold Storage Memories table for archived peripheral memories
            CREATE TABLE IF NOT EXISTS cold_storage_memories (
                id TEXT PRIMARY KEY,
                memory_type TEXT NOT NULL,
                content TEXT NOT NULL,
                summary TEXT,
                scope TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'peripheral',
                importance REAL NOT NULL DEFAULT 0.5,
                confidence REAL NOT NULL DEFAULT 1.0,
                tags_json TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                archived_at TEXT NOT NULL,
                access_count INTEGER NOT NULL DEFAULT 0,
                last_accessed_at TEXT,
                evidence_ids_json TEXT NOT NULL DEFAULT '[]',
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_cold_scope ON cold_storage_memories(scope);
            CREATE INDEX IF NOT EXISTS idx_cold_archived ON cold_storage_memories(archived_at);

            CREATE VIRTUAL TABLE IF NOT EXISTS cold_storage_memories_fts USING fts5(
                id UNINDEXED,
                content,
                summary,
                tags,
                tokenize = 'porter unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS trg_cold_ai AFTER INSERT ON cold_storage_memories BEGIN
                INSERT INTO cold_storage_memories_fts(id, content, summary, tags)
                VALUES (new.id, new.content, COALESCE(new.summary, ''), new.tags_json);
            END;

            CREATE TRIGGER IF NOT EXISTS trg_cold_ad AFTER DELETE ON cold_storage_memories BEGIN
                DELETE FROM cold_storage_memories_fts WHERE id = old.id;
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

            -- Episodic Memories table
            CREATE TABLE IF NOT EXISTS episodic_memories (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                time_start TEXT NOT NULL,
                time_end TEXT NOT NULL,
                actor TEXT NOT NULL,
                project TEXT,
                files_json TEXT NOT NULL DEFAULT '[]',
                tools_used_json TEXT NOT NULL DEFAULT '[]',
                summary TEXT NOT NULL,
                goals_json TEXT NOT NULL DEFAULT '[]',
                obstacles_json TEXT NOT NULL DEFAULT '[]',
                outcomes_json TEXT NOT NULL DEFAULT '[]',
                signals_json TEXT NOT NULL,
                tags_json TEXT NOT NULL DEFAULT '[]',
                raw_event_ids_json TEXT NOT NULL DEFAULT '[]',
                embedding BLOB
            );

            CREATE INDEX IF NOT EXISTS idx_episodic_session ON episodic_memories(session_id);
            CREATE INDEX IF NOT EXISTS idx_episodic_created ON episodic_memories(created_at);
            CREATE INDEX IF NOT EXISTS idx_episodic_project ON episodic_memories(project);

            -- Full-text search virtual table for episodic memories
            CREATE VIRTUAL TABLE IF NOT EXISTS episodic_memories_fts USING fts5(
                id UNINDEXED,
                session_id,
                actor,
                summary,
                goals,
                obstacles,
                outcomes,
                tags,
                tokenize = 'porter unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS trg_episodic_ai AFTER INSERT ON episodic_memories BEGIN
                INSERT INTO episodic_memories_fts(id, session_id, actor, summary, goals, obstacles, outcomes, tags)
                VALUES (new.id, new.session_id, new.actor, new.summary, new.goals_json, new.obstacles_json, new.outcomes_json, new.tags_json);
            END;

            CREATE TRIGGER IF NOT EXISTS trg_episodic_ad AFTER DELETE ON episodic_memories BEGIN
                DELETE FROM episodic_memories_fts WHERE id = old.id;
            END;

            CREATE TRIGGER IF NOT EXISTS trg_episodic_au AFTER UPDATE ON episodic_memories BEGIN
                DELETE FROM episodic_memories_fts WHERE id = old.id;
                INSERT INTO episodic_memories_fts(id, session_id, actor, summary, goals, obstacles, outcomes, tags)
                VALUES (new.id, new.session_id, new.actor, new.summary, new.goals_json, new.obstacles_json, new.outcomes_json, new.tags_json);
            END;

            -- Semantic Facts table
            CREATE TABLE IF NOT EXISTS semantic_facts (
                id TEXT PRIMARY KEY,
                project TEXT,
                scope TEXT NOT NULL,
                statement TEXT NOT NULL,
                category TEXT NOT NULL,
                evidence_json TEXT NOT NULL DEFAULT '[]',
                importance REAL NOT NULL DEFAULT 0.5,
                confidence REAL NOT NULL DEFAULT 1.0,
                tier TEXT NOT NULL DEFAULT 'peripheral',
                created_at TEXT NOT NULL,
                last_updated_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                version INTEGER NOT NULL DEFAULT 1,
                replaced_by TEXT,
                tags_json TEXT NOT NULL DEFAULT '[]',
                code_anchor_json TEXT DEFAULT NULL,
                embedding BLOB
            );

            CREATE INDEX IF NOT EXISTS idx_facts_scope ON semantic_facts(scope);
            CREATE INDEX IF NOT EXISTS idx_facts_category ON semantic_facts(category);
            CREATE INDEX IF NOT EXISTS idx_facts_tier ON semantic_facts(tier);
            CREATE INDEX IF NOT EXISTS idx_facts_status ON semantic_facts(status);
            CREATE INDEX IF NOT EXISTS idx_facts_project ON semantic_facts(project);

            -- Full-text search virtual table for semantic facts
            CREATE VIRTUAL TABLE IF NOT EXISTS semantic_facts_fts USING fts5(
                id UNINDEXED,
                statement,
                category,
                tags,
                tokenize = 'porter unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS trg_facts_ai AFTER INSERT ON semantic_facts BEGIN
                INSERT INTO semantic_facts_fts(id, statement, category, tags)
                VALUES (new.id, new.statement, new.category, new.tags_json);
            END;

            CREATE TRIGGER IF NOT EXISTS trg_facts_ad AFTER DELETE ON semantic_facts BEGIN
                DELETE FROM semantic_facts_fts WHERE id = old.id;
            END;

            CREATE TRIGGER IF NOT EXISTS trg_facts_au AFTER UPDATE ON semantic_facts BEGIN
                DELETE FROM semantic_facts_fts WHERE id = old.id;
                INSERT INTO semantic_facts_fts(id, statement, category, tags)
                VALUES (new.id, new.statement, new.category, new.tags_json);
            END;

            -- Procedural Skills table
            CREATE TABLE IF NOT EXISTS procedural_skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                project TEXT,
                description TEXT NOT NULL,
                preconditions_json TEXT NOT NULL DEFAULT '[]',
                postconditions_json TEXT NOT NULL DEFAULT '[]',
                parameters_json TEXT NOT NULL DEFAULT '[]',
                steps_json TEXT NOT NULL DEFAULT '[]',
                examples_json TEXT NOT NULL DEFAULT '[]',
                success_rate REAL NOT NULL DEFAULT 1.0,
                importance REAL NOT NULL DEFAULT 0.5,
                created_at TEXT NOT NULL,
                last_used_at TEXT,
                usage_count INTEGER NOT NULL DEFAULT 0,
                tags_json TEXT NOT NULL DEFAULT '[]',
                embedding BLOB
            );

            CREATE INDEX IF NOT EXISTS idx_skills_name ON procedural_skills(name);
            CREATE INDEX IF NOT EXISTS idx_skills_project ON procedural_skills(project);

            -- Full-text search virtual table for procedural skills
            CREATE VIRTUAL TABLE IF NOT EXISTS procedural_skills_fts USING fts5(
                id UNINDEXED,
                name,
                description,
                tags,
                tokenize = 'porter unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS trg_skills_ai AFTER INSERT ON procedural_skills BEGIN
                INSERT INTO procedural_skills_fts(id, name, description, tags)
                VALUES (new.id, new.name, new.description, new.tags_json);
            END;

            CREATE TRIGGER IF NOT EXISTS trg_skills_ad AFTER DELETE ON procedural_skills BEGIN
                DELETE FROM procedural_skills_fts WHERE id = old.id;
            END;

            CREATE TRIGGER IF NOT EXISTS trg_skills_au AFTER UPDATE ON procedural_skills BEGIN
                DELETE FROM procedural_skills_fts WHERE id = old.id;
                INSERT INTO procedural_skills_fts(id, name, description, tags)
                VALUES (new.id, new.name, new.description, new.tags_json);
            END;

            -- Memory Access Logs table
            CREATE TABLE IF NOT EXISTS memory_access_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id TEXT NOT NULL,
                accessed_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_access_logs_memory ON memory_access_logs(memory_id, accessed_at);

            -- CDC Sync Outbox table for offline-first delta synchronization
            CREATE TABLE IF NOT EXISTS sync_outbox (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                ts TEXT NOT NULL,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                version_hash TEXT NOT NULL,
                synced INTEGER NOT NULL DEFAULT 0,
                retry_count INTEGER NOT NULL DEFAULT 0,
                next_retry_ts TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_outbox_pending ON sync_outbox(workspace_id, synced, next_retry_ts);
            CREATE INDEX IF NOT EXISTS idx_outbox_seq ON sync_outbox(workspace_id, seq);

            -- Sync Metadata table
            CREATE TABLE IF NOT EXISTS sync_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- Track 3 Tables: Implicit Signals, Feedback Events, Preference Pairs
            CREATE TABLE IF NOT EXISTS implicit_signals (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                tool_name TEXT,
                file_path TEXT,
                extra TEXT,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_signals_session ON implicit_signals(session_id);
            CREATE INDEX IF NOT EXISTS idx_signals_kind ON implicit_signals(kind);
            CREATE INDEX IF NOT EXISTS idx_signals_created ON implicit_signals(created_at);

            CREATE TABLE IF NOT EXISTS feedback_events (
                id TEXT PRIMARY KEY,
                memory_id TEXT,
                signal_id TEXT,
                rating TEXT NOT NULL,
                comment TEXT,
                created_at TEXT NOT NULL,
                source TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_feedback_memory ON feedback_events(memory_id);
            CREATE INDEX IF NOT EXISTS idx_feedback_signal ON feedback_events(signal_id);
            CREATE INDEX IF NOT EXISTS idx_feedback_created ON feedback_events(created_at);

            CREATE TABLE IF NOT EXISTS preference_pairs (
                id TEXT PRIMARY KEY,
                prompt TEXT NOT NULL,
                chosen TEXT NOT NULL,
                rejected TEXT NOT NULL,
                source_session_id TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_pref_pairs_session ON preference_pairs(source_session_id);
            CREATE INDEX IF NOT EXISTS idx_pref_pairs_created ON preference_pairs(created_at);
            ",
        )
        .map_err(|e| StrataError::Database(format!("Failed to execute schema migration: {e}")))?;

        // Safe migration for existing databases: add code_anchor_json and tier if not exists
        let _ = conn.execute("ALTER TABLE semantic_facts ADD COLUMN code_anchor_json TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE memories ADD COLUMN tier TEXT NOT NULL DEFAULT 'peripheral'", []);
        let _ = conn.execute("ALTER TABLE semantic_facts ADD COLUMN tier TEXT NOT NULL DEFAULT 'peripheral'", []);

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

    pub fn get_session_ids(&self) -> Result<Vec<String>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT DISTINCT session_id FROM events ORDER BY sequence_num DESC")
            .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| StrataError::Database(format!("Query failed: {e}")))?;

        let mut session_ids = Vec::new();
        for r in rows {
            if let Ok(id) = r {
                session_ids.push(id);
            }
        }
        Ok(session_ids)
    }

    pub fn get_all_events(&self) -> Result<Vec<Event>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, sequence_num, timestamp, session_id, agent_id,
                        organization_id, payload_json, provenance_json,
                        classification, retention, metadata_json
                 FROM events
                 ORDER BY sequence_num ASC",
            )
            .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map([], |row| {
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
        let tier_str = memory.tier.to_string();
        let tags_json = serde_json::to_string(&memory.tags)?;
        let evidence_json = serde_json::to_string(&memory.evidence_ids)?;
        let metadata_json = serde_json::to_string(&memory.metadata)?;
        let created_at_str = memory.created_at.to_rfc3339();
        let updated_at_str = memory.updated_at.to_rfc3339();
        let last_accessed_str = memory.last_accessed_at.map(|t| t.to_rfc3339());
        let embedding_blob = memory.embedding.as_ref().map(|e| embedding_to_bytes(e));

        conn.execute(
            "INSERT INTO memories (
                id, memory_type, content, summary, scope, tier, importance, confidence,
                tags_json, created_at, updated_at, access_count, last_accessed_at,
                evidence_ids_json, embedding, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(id) DO UPDATE SET
                memory_type = excluded.memory_type,
                content = excluded.content,
                summary = excluded.summary,
                scope = excluded.scope,
                tier = excluded.tier,
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
                tier_str,
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
                "SELECT id, memory_type, content, summary, scope, tier, importance,
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

        if res.is_some() {
            let _ = self.record_memory_access_internal(&conn, id, Utc::now());
        }

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

        let mut query = "SELECT id, memory_type, content, summary, scope, tier, importance,
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
            SELECT m.id, m.memory_type, m.content, m.summary, m.scope, m.tier, m.importance,
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
                let rank: f64 = row.get(16)?;
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
    // Cold Storage Memory Operations
    // ==========================================

    /// Archives a memory record to cold storage, removing it from active memory and FTS index.
    pub fn archive_to_cold_storage(&self, record_id: &Uuid) -> Result<bool, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = record_id.to_string();
        let now_str = Utc::now().to_rfc3339();

        // 1. Fetch memory from active table
        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, content, summary, scope, tier, importance,
                        confidence, tags_json, created_at, updated_at, access_count,
                        last_accessed_at, evidence_ids_json, metadata_json
                 FROM memories WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let row_opt = stmt
            .query_row(params![id_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, f64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                ))
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let Some((id, m_type, content, summary, scope, tier, imp, conf, tags, created, updated, count, last_acc, ev_ids, meta)) = row_opt else {
            return Ok(false);
        };

        // 2. Insert into cold_storage_memories
        conn.execute(
            "INSERT INTO cold_storage_memories (
                id, memory_type, content, summary, scope, tier, importance, confidence,
                tags_json, created_at, updated_at, archived_at, access_count,
                last_accessed_at, evidence_ids_json, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(id) DO UPDATE SET
                archived_at = excluded.archived_at,
                updated_at = excluded.updated_at,
                access_count = excluded.access_count",
            params![
                id, m_type, content, summary, scope, tier, imp, conf,
                tags, created, updated, now_str, count, last_acc, ev_ids, meta
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to archive memory to cold storage: {e}")))?;

        // 3. Delete from active memories (FTS trigger handles deletion from index)
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id_str])
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(true)
    }

    /// Queries cold storage memories directly (deep historical search).
    pub fn get_cold_storage_memories(
        &self,
        query: Option<&str>,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        if let Some(q) = query {
            let sanitized = sanitize_fts5_query(q);
            if !sanitized.is_empty() {
                let sql = "
                    SELECT c.id, c.memory_type, c.content, c.summary, c.scope, c.tier, c.importance,
                           c.confidence, c.tags_json, c.created_at, c.updated_at, c.access_count,
                           c.last_accessed_at, c.evidence_ids_json, NULL as embedding, c.metadata_json
                    FROM cold_storage_memories_fts f
                    JOIN cold_storage_memories c ON c.id = f.id
                    WHERE cold_storage_memories_fts MATCH ?1
                    LIMIT ?2
                ";
                let mut stmt = conn.prepare(sql).map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = stmt
                    .query_map(params![sanitized, limit as i64], |row| Self::row_to_memory(row))
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                let mut results = Vec::new();
                for r in rows {
                    results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
                }
                return Ok(results);
            }
        }

        let mut sql = "SELECT id, memory_type, content, summary, scope, tier, importance,
                              confidence, tags_json, created_at, updated_at, access_count,
                              last_accessed_at, evidence_ids_json, NULL as embedding, metadata_json
                       FROM cold_storage_memories WHERE 1=1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(sc) = scope {
            if *sc != Scope::Global {
                sql.push_str(" AND (scope = ? OR scope = 'global')");
                params_vec.push(Box::new(sc.to_string()));
            }
        }

        sql.push_str(" ORDER BY archived_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| Self::row_to_memory(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Restores an archived memory from cold storage back into active memory with target tier.
    pub fn restore_from_cold_storage(
        &self,
        record_id: &Uuid,
        target_tier: Option<MemoryTier>,
    ) -> Result<bool, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = record_id.to_string();

        // 1. Fetch from cold storage
        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, content, summary, scope, tier, importance,
                        confidence, tags_json, created_at, updated_at, access_count,
                        last_accessed_at, evidence_ids_json, NULL as embedding, metadata_json
                 FROM cold_storage_memories WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mem_opt = stmt
            .query_row(params![id_str], |row| Self::row_to_memory(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let Some(mut mem) = mem_opt else {
            return Ok(false);
        };

        // 2. Adjust tier and access timestamp
        mem.tier = target_tier.unwrap_or(MemoryTier::Working);
        mem.mark_accessed();

        // 3. Delete from cold storage
        drop(stmt);
        conn.execute("DELETE FROM cold_storage_memories WHERE id = ?1", params![id_str])
            .map_err(|e| StrataError::Database(e.to_string()))?;

        // 4. Insert into active memories (unlocking conn before calling insert_or_update_memory)
        drop(conn);
        self.insert_or_update_memory(&mem)?;

        Ok(true)
    }

    /// Returns the total number of memories archived in cold storage.
    pub fn get_cold_storage_count(&self) -> Result<usize, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM cold_storage_memories", [], |r| r.get(0))
            .unwrap_or(0);

        Ok(count as usize)
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
    // Episodic Memories CRUD
    // ==========================================

    pub fn insert_episodic_memory(&self, memory: &EpisodicMemory) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = memory.id.to_string();
        let created_at_str = memory.created_at.to_rfc3339();
        let time_start_str = memory.time_start.to_rfc3339();
        let time_end_str = memory.time_end.to_rfc3339();
        let files_json = serde_json::to_string(&memory.files)?;
        let tools_json = serde_json::to_string(&memory.tools_used)?;
        let goals_json = serde_json::to_string(&memory.goals)?;
        let obstacles_json = serde_json::to_string(&memory.obstacles)?;
        let outcomes_json = serde_json::to_string(&memory.outcomes)?;
        let signals_json = serde_json::to_string(&memory.signals)?;
        let tags_json = serde_json::to_string(&memory.tags)?;
        let raw_events_json = serde_json::to_string(&memory.raw_event_ids)?;

        conn.execute(
            "INSERT INTO episodic_memories (
                id, session_id, created_at, time_start, time_end, actor, project,
                files_json, tools_used_json, summary, goals_json, obstacles_json,
                outcomes_json, signals_json, tags_json, raw_event_ids_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(id) DO UPDATE SET
                session_id = excluded.session_id,
                time_start = excluded.time_start,
                time_end = excluded.time_end,
                actor = excluded.actor,
                project = excluded.project,
                files_json = excluded.files_json,
                tools_used_json = excluded.tools_used_json,
                summary = excluded.summary,
                goals_json = excluded.goals_json,
                obstacles_json = excluded.obstacles_json,
                outcomes_json = excluded.outcomes_json,
                signals_json = excluded.signals_json,
                tags_json = excluded.tags_json,
                raw_event_ids_json = excluded.raw_event_ids_json",
            params![
                id_str,
                memory.session_id,
                created_at_str,
                time_start_str,
                time_end_str,
                memory.actor,
                memory.project,
                files_json,
                tools_json,
                memory.summary,
                goals_json,
                obstacles_json,
                outcomes_json,
                signals_json,
                tags_json,
                raw_events_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert episodic memory: {e}")))?;

        Ok(())
    }

    pub fn get_episodic_memory(&self, id: &Uuid) -> Result<Option<EpisodicMemory>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, created_at, time_start, time_end, actor, project,
                        files_json, tools_used_json, summary, goals_json, obstacles_json,
                        outcomes_json, signals_json, tags_json, raw_event_ids_json
                 FROM episodic_memories
                 WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![id.to_string()], |row| Self::row_to_episodic(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if res.is_some() {
            let _ = self.record_memory_access_internal(&conn, id, Utc::now());
        }

        Ok(res)
    }

    pub fn get_episodic_memories_by_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<EpisodicMemory>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, created_at, time_start, time_end, actor, project,
                        files_json, tools_used_json, summary, goals_json, obstacles_json,
                        outcomes_json, signals_json, tags_json, raw_event_ids_json
                 FROM episodic_memories
                 WHERE session_id = ?1
                 ORDER BY created_at ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![session_id], |row| Self::row_to_episodic(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_all_episodic_memories(
        &self,
        project: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodicMemory>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut sql = "SELECT id, session_id, created_at, time_start, time_end, actor, project,
                              files_json, tools_used_json, summary, goals_json, obstacles_json,
                              outcomes_json, signals_json, tags_json, raw_event_ids_json
                       FROM episodic_memories WHERE 1=1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            params_vec.push(Box::new(p.to_string()));
        }

        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| Self::row_to_episodic(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn search_episodic_memories_fts(
        &self,
        query_text: &str,
        limit: usize,
    ) -> Result<Vec<(EpisodicMemory, f32)>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let sanitized = sanitize_fts5_query(query_text);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "
            SELECT m.id, m.session_id, m.created_at, m.time_start, m.time_end, m.actor, m.project,
                   m.files_json, m.tools_used_json, m.summary, m.goals_json, m.obstacles_json,
                   m.outcomes_json, m.signals_json, m.tags_json, m.raw_event_ids_json,
                   bm25(episodic_memories_fts) as rank
            FROM episodic_memories_fts f
            JOIN episodic_memories m ON m.id = f.id
            WHERE episodic_memories_fts MATCH ?1
            ORDER BY rank ASC LIMIT ?2
        ";

        let mut stmt = conn.prepare(sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![sanitized, limit as i64], |row| {
                let ep = Self::row_to_episodic(row)?;
                let rank: f64 = row.get(16)?;
                Ok((ep, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn delete_episodic_memory(&self, id: &Uuid) -> Result<bool, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;
        let rows = conn
            .execute("DELETE FROM episodic_memories WHERE id = ?1", params![id.to_string()])
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(rows > 0)
    }

    // ==========================================
    // Semantic Facts CRUD
    // ==========================================

    pub fn insert_or_update_semantic_fact(&self, fact: &SemanticFact) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = fact.id.to_string();
        let scope_str = fact.scope.to_string();
        let tier_str = fact.tier.to_string();
        let evidence_json = serde_json::to_string(&fact.evidence)?;
        let created_at_str = fact.created_at.to_rfc3339();
        let updated_at_str = fact.last_updated_at.to_rfc3339();
        let status_str = fact.status.to_string();
        let replaced_str = fact.replaced_by.map(|u| u.to_string());
        let tags_json = serde_json::to_string(&fact.tags)?;
        let code_anchor_json = match &fact.code_anchor {
            Some(anchor) => Some(serde_json::to_string(anchor)?),
            None => None,
        };

        conn.execute(
            "INSERT INTO semantic_facts (
                id, project, scope, statement, category, evidence_json,
                importance, confidence, tier, created_at, last_updated_at, status,
                version, replaced_by, tags_json, code_anchor_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(id) DO UPDATE SET
                project = excluded.project,
                scope = excluded.scope,
                statement = excluded.statement,
                category = excluded.category,
                evidence_json = excluded.evidence_json,
                importance = excluded.importance,
                confidence = excluded.confidence,
                tier = excluded.tier,
                last_updated_at = excluded.last_updated_at,
                status = excluded.status,
                version = excluded.version,
                replaced_by = excluded.replaced_by,
                tags_json = excluded.tags_json,
                code_anchor_json = excluded.code_anchor_json",
            params![
                id_str,
                fact.project,
                scope_str,
                fact.statement,
                fact.category,
                evidence_json,
                fact.importance,
                fact.confidence,
                tier_str,
                created_at_str,
                updated_at_str,
                status_str,
                fact.version as i64,
                replaced_str,
                tags_json,
                code_anchor_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to persist semantic fact: {e}")))?;

        Ok(())
    }


    pub fn get_semantic_fact(&self, id: &Uuid) -> Result<Option<SemanticFact>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, project, scope, statement, category, evidence_json,
                        importance, confidence, tier, created_at, last_updated_at, status,
                        version, replaced_by, tags_json, code_anchor_json
                 FROM semantic_facts
                 WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![id.to_string()], |row| Self::row_to_fact(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if res.is_some() {
            let _ = self.record_memory_access_internal(&conn, id, Utc::now());
        }

        Ok(res)
    }

    pub fn update_semantic_fact_embedding(&self, id: &Uuid, embedding: &[f32]) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        conn.execute(
            "UPDATE semantic_facts SET embedding = ?1 WHERE id = ?2",
            params![embedding_to_bytes(embedding), id.to_string()],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update fact embedding: {e}")))?;

        Ok(())
    }

    pub fn get_all_semantic_facts(
        &self,
        project: Option<&str>,
        status: Option<FactStatus>,
        limit: usize,
    ) -> Result<Vec<SemanticFact>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut sql = "SELECT id, project, scope, statement, category, evidence_json,
                              importance, confidence, tier, created_at, last_updated_at, status,
                              version, replaced_by, tags_json, code_anchor_json
                       FROM semantic_facts WHERE 1=1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            params_vec.push(Box::new(p.to_string()));
        }

        if let Some(st) = status {
            sql.push_str(" AND status = ?");
            params_vec.push(Box::new(st.to_string()));
        }

        sql.push_str(" ORDER BY last_updated_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| Self::row_to_fact(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_semantic_facts_with_embeddings(
        &self,
        project: Option<&str>,
        status: Option<FactStatus>,
    ) -> Result<Vec<(SemanticFact, Option<Vec<f32>>)>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut sql = "SELECT id, project, scope, statement, category, evidence_json,
                              importance, confidence, tier, created_at, last_updated_at, status,
                              version, replaced_by, tags_json, code_anchor_json, embedding
                       FROM semantic_facts WHERE 1=1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            params_vec.push(Box::new(p.to_string()));
        }

        if let Some(st) = status {
            sql.push_str(" AND status = ?");
            params_vec.push(Box::new(st.to_string()));
        }

        let mut stmt = conn.prepare(&sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| {
                let fact = Self::row_to_fact(row)?;
                let blob_opt: Option<Vec<u8>> = row.get(16)?;
                let emb = blob_opt.and_then(|b| bytes_to_embedding(&b).ok());
                Ok((fact, emb))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn search_semantic_facts_fts(
        &self,
        query_text: &str,
        limit: usize,
    ) -> Result<Vec<(SemanticFact, f32)>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let sanitized = sanitize_fts5_query(query_text);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "
            SELECT m.id, m.project, m.scope, m.statement, m.category, m.evidence_json,
                   m.importance, m.confidence, m.tier, m.created_at, m.last_updated_at, m.status,
                   m.version, m.replaced_by, m.tags_json, m.code_anchor_json,
                   bm25(semantic_facts_fts) as rank
            FROM semantic_facts_fts f
            JOIN semantic_facts m ON m.id = f.id
            WHERE semantic_facts_fts MATCH ?1
            ORDER BY rank ASC LIMIT ?2
        ";

        let mut stmt = conn.prepare(sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![sanitized, limit as i64], |row| {
                let fact = Self::row_to_fact(row)?;
                let rank: f64 = row.get(16)?;
                Ok((fact, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_facts_by_file_anchor(&self, file_path: &str) -> Result<Vec<SemanticFact>, StrataError> {
        let all_facts = self.get_all_semantic_facts(None, None, 1000)?;
        Ok(all_facts
            .into_iter()
            .filter(|f| {
                f.code_anchor
                    .as_ref()
                    .map(|a| a.file_path == file_path)
                    .unwrap_or(false)
            })
            .collect())
    }

    pub fn delete_semantic_fact(&self, id: &Uuid) -> Result<bool, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;
        let rows = conn
            .execute("DELETE FROM semantic_facts WHERE id = ?1", params![id.to_string()])
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(rows > 0)
    }

    // ==========================================
    // Procedural Skills CRUD
    // ==========================================

    pub fn insert_or_update_procedural_skill(&self, skill: &ProceduralSkill) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = skill.id.to_string();
        let preconditions_json = serde_json::to_string(&skill.preconditions)?;
        let postconditions_json = serde_json::to_string(&skill.postconditions)?;
        let params_json = serde_json::to_string(&skill.parameters)?;
        let steps_json = serde_json::to_string(&skill.steps)?;
        let examples_json = serde_json::to_string(&skill.examples)?;
        let created_at_str = skill.created_at.to_rfc3339();
        let last_used_str = skill.last_used_at.map(|t| t.to_rfc3339());
        let tags_json = serde_json::to_string(&skill.tags)?;

        conn.execute(
            "INSERT INTO procedural_skills (
                id, name, project, description, preconditions_json,
                postconditions_json, parameters_json, steps_json, examples_json,
                success_rate, importance, created_at, last_used_at, usage_count,
                tags_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                project = excluded.project,
                description = excluded.description,
                preconditions_json = excluded.preconditions_json,
                postconditions_json = excluded.postconditions_json,
                parameters_json = excluded.parameters_json,
                steps_json = excluded.steps_json,
                examples_json = excluded.examples_json,
                success_rate = excluded.success_rate,
                importance = excluded.importance,
                last_used_at = excluded.last_used_at,
                usage_count = excluded.usage_count,
                tags_json = excluded.tags_json",
            params![
                id_str,
                skill.name,
                skill.project,
                skill.description,
                preconditions_json,
                postconditions_json,
                params_json,
                steps_json,
                examples_json,
                skill.success_rate,
                skill.importance,
                created_at_str,
                last_used_str,
                skill.usage_count as i64,
                tags_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to persist procedural skill: {e}")))?;

        Ok(())
    }

    pub fn get_procedural_skill(&self, id: &Uuid) -> Result<Option<ProceduralSkill>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, name, project, description, preconditions_json,
                        postconditions_json, parameters_json, steps_json, examples_json,
                        success_rate, importance, created_at, last_used_at, usage_count,
                        tags_json
                 FROM procedural_skills
                 WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![id.to_string()], |row| Self::row_to_skill(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if res.is_some() {
            let _ = self.record_memory_access_internal(&conn, id, Utc::now());
        }

        Ok(res)
    }

    pub fn get_procedural_skill_by_name(&self, name: &str) -> Result<Option<ProceduralSkill>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, name, project, description, preconditions_json,
                        postconditions_json, parameters_json, steps_json, examples_json,
                        success_rate, importance, created_at, last_used_at, usage_count,
                        tags_json
                 FROM procedural_skills
                 WHERE name = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![name], |row| Self::row_to_skill(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if let Some(ref skill) = res {
            let _ = self.record_memory_access_internal(&conn, &skill.id, Utc::now());
        }

        Ok(res)
    }

    pub fn update_procedural_skill_embedding(&self, id: &Uuid, embedding: &[f32]) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;
        let blob = embedding_to_bytes(embedding);
        conn.execute(
            "UPDATE procedural_skills SET embedding = ?1 WHERE id = ?2",
            params![blob, id.to_string()],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update embedding: {e}")))?;
        Ok(())
    }

    pub fn get_all_procedural_skills(
        &self,
        project: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ProceduralSkill>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut sql = "SELECT id, name, project, description, preconditions_json,
                              postconditions_json, parameters_json, steps_json, examples_json,
                              success_rate, importance, created_at, last_used_at, usage_count,
                              tags_json
                       FROM procedural_skills WHERE 1=1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            params_vec.push(Box::new(p.to_string()));
        }

        sql.push_str(" ORDER BY usage_count DESC, created_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| Self::row_to_skill(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn search_procedural_skills_fts(
        &self,
        query_text: &str,
        limit: usize,
    ) -> Result<Vec<(ProceduralSkill, f32)>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let sanitized = sanitize_fts5_query(query_text);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "
            SELECT m.id, m.name, m.project, m.description, m.preconditions_json,
                   m.postconditions_json, m.parameters_json, m.steps_json, m.examples_json,
                   m.success_rate, m.importance, m.created_at, m.last_used_at, m.usage_count,
                   m.tags_json,
                   bm25(procedural_skills_fts) as rank
            FROM procedural_skills_fts f
            JOIN procedural_skills m ON m.id = f.id
            WHERE procedural_skills_fts MATCH ?1
            ORDER BY rank ASC LIMIT ?2
        ";

        let mut stmt = conn.prepare(sql).map_err(|e| StrataError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![sanitized, limit as i64], |row| {
                let skill = Self::row_to_skill(row)?;
                let rank: f64 = row.get(15)?;
                Ok((skill, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn delete_procedural_skill(&self, id: &Uuid) -> Result<bool, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;
        let rows = conn
            .execute("DELETE FROM procedural_skills WHERE id = ?1", params![id.to_string()])
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(rows > 0)
    }

    // ==========================================
    // Memory Access Logs
    // ==========================================

    fn record_memory_access_internal(
        &self,
        conn: &rusqlite::Connection,
        memory_id: &Uuid,
        accessed_at: DateTime<Utc>,
    ) -> Result<(), StrataError> {
        let ts_str = accessed_at.to_rfc3339();
        conn.execute(
            "INSERT INTO memory_access_logs (memory_id, accessed_at) VALUES (?1, ?2)",
            params![memory_id.to_string(), ts_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to record access log: {e}")))?;
        Ok(())
    }

    pub fn record_memory_access(&self, memory_id: &Uuid, accessed_at: DateTime<Utc>) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;
        self.record_memory_access_internal(&conn, memory_id, accessed_at)
    }

    pub fn get_memory_access_logs(&self, memory_id: &Uuid) -> Result<Vec<DateTime<Utc>>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT accessed_at FROM memory_access_logs WHERE memory_id = ?1 ORDER BY accessed_at ASC")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![memory_id.to_string()], |row| {
                let ts_str: String = row.get(0)?;
                Ok(ts_str)
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut logs = Vec::new();
        for r in rows {
            let ts_str = r.map_err(|e| StrataError::Database(e.to_string()))?;
            if let Ok(dt) = DateTime::parse_from_rfc3339(&ts_str) {
                logs.push(dt.with_timezone(&Utc));
            }
        }
        Ok(logs)
    }

    pub fn get_memory_access_count(&self, memory_id: &Uuid) -> Result<u64, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM memory_access_logs WHERE memory_id = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let count: i64 = stmt
            .query_row(params![memory_id.to_string()], |row| row.get(0))
            .unwrap_or(0);

        Ok(count as u64)
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
        let tier_str: String = row.get(5).unwrap_or_else(|_| "peripheral".to_string());
        let importance: f64 = row.get(6)?;
        let confidence: f64 = row.get(7)?;
        let tags_json: String = row.get(8)?;
        let created_at_str: String = row.get(9)?;
        let updated_at_str: String = row.get(10)?;
        let access_count: i64 = row.get(11)?;
        let last_accessed_str: Option<String> = row.get(12)?;
        let evidence_json: String = row.get(13)?;
        let embedding_bytes: Option<Vec<u8>> = row.get(14)?;
        let metadata_json: String = row.get(15)?;

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let memory_type = mem_type_str
            .parse::<MemoryType>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let scope = scope_str
            .parse::<Scope>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let tier = tier_str
            .parse::<MemoryTier>()
            .unwrap_or(MemoryTier::Peripheral);
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
            tier,
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

    fn row_to_episodic(row: &rusqlite::Row) -> rusqlite::Result<EpisodicMemory> {
        let id_str: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let created_at_str: String = row.get(2)?;
        let time_start_str: String = row.get(3)?;
        let time_end_str: String = row.get(4)?;
        let actor: String = row.get(5)?;
        let project: Option<String> = row.get(6)?;
        let files_json: String = row.get(7)?;
        let tools_json: String = row.get(8)?;
        let summary: String = row.get(9)?;
        let goals_json: String = row.get(10)?;
        let obstacles_json: String = row.get(11)?;
        let outcomes_json: String = row.get(12)?;
        let signals_json: String = row.get(13)?;
        let tags_json: String = row.get(14)?;
        let raw_events_json: String = row.get(15)?;

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let time_start = DateTime::parse_from_rfc3339(&time_start_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let time_end = DateTime::parse_from_rfc3339(&time_end_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();
        let tools_used: Vec<String> = serde_json::from_str(&tools_json).unwrap_or_default();
        let goals: Vec<String> = serde_json::from_str(&goals_json).unwrap_or_default();
        let obstacles: Vec<String> = serde_json::from_str(&obstacles_json).unwrap_or_default();
        let outcomes: Vec<String> = serde_json::from_str(&outcomes_json).unwrap_or_default();
        let signals: SignalScores = serde_json::from_str(&signals_json).unwrap_or_default();
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let raw_event_ids: Vec<i64> = serde_json::from_str(&raw_events_json).unwrap_or_default();

        Ok(EpisodicMemory {
            id,
            session_id,
            created_at,
            time_start,
            time_end,
            actor,
            project,
            files,
            tools_used,
            summary,
            goals,
            obstacles,
            outcomes,
            signals,
            tags,
            raw_event_ids,
        })
    }

    fn row_to_fact(row: &rusqlite::Row) -> rusqlite::Result<SemanticFact> {
        let id_str: String = row.get(0)?;
        let project: Option<String> = row.get(1)?;
        let scope_str: String = row.get(2)?;
        let statement: String = row.get(3)?;
        let category: String = row.get(4)?;
        let evidence_json: String = row.get(5)?;
        let importance: f64 = row.get(6)?;
        let confidence: f64 = row.get(7)?;
        let tier_str: String = row.get(8).unwrap_or_else(|_| "peripheral".to_string());
        let created_at_str: String = row.get(9)?;
        let updated_at_str: String = row.get(10)?;
        let status_str: String = row.get(11)?;
        let version: i64 = row.get(12)?;
        let replaced_str: Option<String> = row.get(13)?;
        let tags_json: String = row.get(14)?;
        let code_anchor_json: Option<String> = row.get(15).ok().flatten();

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let scope = scope_str.parse::<Scope>().map_err(|_| rusqlite::Error::InvalidQuery)?;
        let tier = tier_str.parse::<MemoryTier>().unwrap_or(MemoryTier::Peripheral);
        let evidence: Vec<EvidenceRef> = serde_json::from_str(&evidence_json).unwrap_or_default();
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let last_updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let status = status_str.parse::<FactStatus>().map_err(|_| rusqlite::Error::InvalidQuery)?;
        let replaced_by = replaced_str.and_then(|s| Uuid::parse_str(&s).ok());
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let code_anchor: Option<CodeAnchor> = code_anchor_json.and_then(|s| serde_json::from_str(&s).ok());

        Ok(SemanticFact {
            id,
            project,
            scope,
            statement,
            category,
            evidence,
            importance: importance as f32,
            confidence: confidence as f32,
            tier,
            created_at,
            last_updated_at,
            status,
            version: version as u32,
            replaced_by,
            tags,
            code_anchor,
        })
    }

    fn row_to_skill(row: &rusqlite::Row) -> rusqlite::Result<ProceduralSkill> {
        let id_str: String = row.get(0)?;
        let name: String = row.get(1)?;
        let project: Option<String> = row.get(2)?;
        let description: String = row.get(3)?;
        let preconditions_json: String = row.get(4)?;
        let postconditions_json: String = row.get(5)?;
        let params_json: String = row.get(6)?;
        let steps_json: String = row.get(7)?;
        let examples_json: String = row.get(8)?;
        let success_rate: f64 = row.get(9)?;
        let importance: f64 = row.get(10)?;
        let created_at_str: String = row.get(11)?;
        let last_used_str: Option<String> = row.get(12)?;
        let usage_count: i64 = row.get(13)?;
        let tags_json: String = row.get(14)?;

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let preconditions: Vec<String> = serde_json::from_str(&preconditions_json).unwrap_or_default();
        let postconditions: Vec<String> = serde_json::from_str(&postconditions_json).unwrap_or_default();
        let parameters: Vec<ParameterDef> = serde_json::from_str(&params_json).unwrap_or_default();
        let steps: Vec<ProceduralStep> = serde_json::from_str(&steps_json).unwrap_or_default();
        let examples: Vec<ProceduralExample> = serde_json::from_str(&examples_json).unwrap_or_default();
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let last_used_at = last_used_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

        Ok(ProceduralSkill {
            id,
            name,
            project,
            description,
            preconditions,
            postconditions,
            parameters,
            steps,
            examples,
            success_rate: success_rate as f32,
            importance: importance as f32,
            created_at,
            last_used_at,
            usage_count: usage_count as u32,
        tags,
        })
    }

    // ==========================================
    // CDC Outbox & Synchronization Operations
    // ==========================================

    /// Enqueue a change data capture (CDC) delta into the sync outbox.
    pub fn enqueue_delta(&self, delta: &SyncDelta) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = delta.id.to_string();
        let ts_str = delta.ts.to_rfc3339();
        let payload_json = serde_json::to_string(&delta.payload)?;
        let synced_int = if delta.synced { 1 } else { 0 };

        conn.execute(
            "INSERT INTO sync_outbox (
                id, workspace_id, seq, ts, kind, payload_json, version_hash, synced, retry_count, next_retry_ts
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, NULL)
            ON CONFLICT(id) DO UPDATE SET
                workspace_id = excluded.workspace_id,
                seq = excluded.seq,
                ts = excluded.ts,
                kind = excluded.kind,
                payload_json = excluded.payload_json,
                version_hash = excluded.version_hash,
                synced = excluded.synced",
            params![
                id_str,
                delta.workspace_id,
                delta.seq as i64,
                ts_str,
                delta.kind,
                payload_json,
                delta.version_hash,
                synced_int,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to enqueue sync delta: {e}")))?;

        Ok(())
    }

    /// Retrieve pending (unsynced) deltas eligible for transmission.
    pub fn get_pending_deltas(&self, workspace_id: &str, limit: usize) -> Result<Vec<SyncDelta>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let now_str = Utc::now().to_rfc3339();
        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, seq, ts, kind, payload_json, version_hash, synced
                 FROM sync_outbox
                 WHERE workspace_id = ?1 AND synced = 0 AND (next_retry_ts IS NULL OR next_retry_ts <= ?2)
                 ORDER BY seq ASC, ts ASC
                 LIMIT ?3",
            )
            .map_err(|e| StrataError::Database(format!("Failed to prepare get_pending_deltas query: {e}")))?;

        let rows = stmt
            .query_map(params![workspace_id, now_str, limit as i64], |row| {
                let id_str: String = row.get(0)?;
                let ws_id: String = row.get(1)?;
                let seq: i64 = row.get(2)?;
                let ts_str: String = row.get(3)?;
                let kind: String = row.get(4)?;
                let payload_json: String = row.get(5)?;
                let version_hash: String = row.get(6)?;
                let synced_int: i64 = row.get(7)?;

                Ok((id_str, ws_id, seq, ts_str, kind, payload_json, version_hash, synced_int))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut deltas = Vec::new();
        for r in rows {
            let (id_str, ws_id, seq, ts_str, kind, payload_json, version_hash, synced_int) =
                r.map_err(|e| StrataError::Database(e.to_string()))?;
            let id = Uuid::parse_str(&id_str)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in sync delta id: {e}")))?;
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
                synced: synced_int != 0,
            });
        }

        Ok(deltas)
    }

    /// Mark a list of deltas as successfully synchronized.
    pub fn mark_deltas_synced(&self, delta_ids: &[Uuid]) -> Result<(), StrataError> {
        if delta_ids.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to begin transaction: {e}")))?;

        {
            let mut stmt = tx
                .prepare("UPDATE sync_outbox SET synced = 1 WHERE id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;

            for id in delta_ids {
                stmt.execute(params![id.to_string()])
                    .map_err(|e| StrataError::Database(format!("Failed to mark delta synced: {e}")))?;
            }
        }

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit mark_deltas_synced: {e}")))?;

        Ok(())
    }

    /// Record a failure for a batch of deltas, incrementing retry count and calculating exponential backoff.
    pub fn record_delta_failure(&self, delta_ids: &[Uuid], err_msg: &str) -> Result<(), StrataError> {
        if delta_ids.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to begin transaction: {e}")))?;

        {
            let mut select_stmt = tx
                .prepare("SELECT retry_count FROM sync_outbox WHERE id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;

            let mut update_stmt = tx
                .prepare("UPDATE sync_outbox SET retry_count = ?1, next_retry_ts = ?2 WHERE id = ?3")
                .map_err(|e| StrataError::Database(e.to_string()))?;

            for id in delta_ids {
                let id_str = id.to_string();
                let retry_count: i64 = select_stmt
                    .query_row(params![&id_str], |row| row.get(0))
                    .unwrap_or(0);

                let next_count = retry_count + 1;
                let backoff_secs = (0.5 * (2_f64.powi(next_count.min(10) as i32))).min(300.0);
                let next_retry_ts = (Utc::now() + chrono::Duration::milliseconds((backoff_secs * 1000.0) as i64)).to_rfc3339();

                update_stmt
                    .execute(params![next_count, next_retry_ts, &id_str])
                    .map_err(|e| StrataError::Database(format!("Failed to update delta failure: {e}")))?;
            }
        }

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit record_delta_failure: {e}")))?;

        tracing::warn!("Recorded sync delta failure for {} deltas: {}", delta_ids.len(), err_msg);
        Ok(())
    }

    /// Retrieve sync status (pending unsynced deltas count, highest local sequence number).
    pub fn get_sync_status(&self, workspace_id: &str) -> Result<(usize, u64), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let pending_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_outbox WHERE workspace_id = ?1 AND synced = 0",
                params![workspace_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let max_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM sync_outbox WHERE workspace_id = ?1",
                params![workspace_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        Ok((pending_count as usize, max_seq as u64))
    }

    /// Record explicit feedback on a memory, adjusting importance and confidence, and recording an access log.
    pub fn record_memory_feedback(&self, feedback: &MemoryFeedback) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        // 1. Record access log
        self.record_memory_access_internal(&conn, &feedback.memory_id, feedback.created_at)?;

        // 2. Compute adjustment delta
        let score_val = feedback.score.unwrap_or(1.0);
        let effective_score = if score_val == 0.0 { 1.0 } else { score_val };
        let adj_delta = match feedback.rating.as_str() {
            "positive" => 0.1 * effective_score,
            "negative" => -0.1 * effective_score,
            _ => 0.0,
        };


        let now_str = feedback.created_at.to_rfc3339();
        let mem_id_str = feedback.memory_id.to_string();

        // 3. Update memories table if present
        conn.execute(
            "UPDATE memories
             SET importance = MAX(0.0, MIN(1.0, importance + ?1)),
                 confidence = MAX(0.0, MIN(1.0, confidence + ?1)),
                 access_count = access_count + 1,
                 last_accessed_at = ?2,
                 updated_at = ?2
             WHERE id = ?3",
            params![adj_delta, now_str, mem_id_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update memory feedback: {e}")))?;

        // 4. Update semantic_facts table if present
        conn.execute(
            "UPDATE semantic_facts
             SET importance = MAX(0.0, MIN(1.0, importance + ?1)),
                 confidence = MAX(0.0, MIN(1.0, confidence + ?1)),
                 last_updated_at = ?2
             WHERE id = ?3",
            params![adj_delta, now_str, mem_id_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update semantic fact feedback: {e}")))?;

        // 5. Update procedural_skills table if present
        conn.execute(
            "UPDATE procedural_skills
             SET importance = MAX(0.0, MIN(1.0, importance + ?1)),
                 last_used_at = ?2
             WHERE id = ?3",
            params![adj_delta, now_str, mem_id_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update procedural skill feedback: {e}")))?;

        // 6. Record FeedbackEvent in feedback_events table
        let rating_enum = match feedback.rating.as_str() {
            "positive" => FeedbackRating::Positive,
            _ => FeedbackRating::Negative,
        };
        let sig_id_str: Option<String> = None;
        let _ = conn.execute(
            "INSERT INTO feedback_events (
                id, memory_id, signal_id, rating, comment, created_at, source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                Uuid::new_v4().to_string(),
                mem_id_str,
                sig_id_str,
                rating_enum.to_string(),
                feedback.comment,
                now_str,
                "memory_feedback",
            ],
        );

        Ok(())
    }

    /// Retrieve value from sync_metadata table.
    pub fn get_sync_metadata(&self, key: &str) -> Result<Option<String>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare("SELECT value FROM sync_metadata WHERE key = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![key], |row| row.get(0))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(res)
    }

    /// Store key-value pair in sync_metadata table.
    pub fn set_sync_metadata(&self, key: &str, value: &str) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        conn.execute(
            "INSERT INTO sync_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| StrataError::Database(format!("Failed to set sync metadata: {e}")))?;

        Ok(())
    }

    // ==========================================
    // Track 3: Feedback, Signals & Preference Pairs
    // ==========================================

    /// Record an implicit behavioural signal.
    pub fn record_implicit_signal(&self, signal: &ImplicitSignal) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = signal.id.to_string();
        let kind_str = signal.kind.to_string();
        let ts_str = signal.timestamp.to_rfc3339();

        conn.execute(
            "INSERT INTO implicit_signals (
                id, kind, session_id, agent_id, tool_name, file_path, extra, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id_str,
                kind_str,
                signal.session_id,
                signal.agent_id,
                signal.tool_name,
                signal.file_path,
                signal.extra,
                ts_str,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert implicit signal: {e}")))?;

        Ok(())
    }

    /// Retrieve implicit signals, optionally filtered by session ID.
    pub fn get_implicit_signals(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<ImplicitSignal>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut sql = "
            SELECT id, kind, session_id, agent_id, tool_name, file_path, extra, created_at
            FROM implicit_signals
        ".to_string();

        if session_id.is_some() {
            sql.push_str(" WHERE session_id = ?1 ORDER BY created_at ASC");
        } else {
            sql.push_str(" ORDER BY created_at ASC");
        }

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let map_row = |row: &rusqlite::Row| -> Result<ImplicitSignal, rusqlite::Error> {
            let id_str: String = row.get(0)?;
            let id = Uuid::parse_str(&id_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let kind_str: String = row.get(1)?;
            let kind = kind_str
                .parse::<SignalKind>()
                .unwrap_or(SignalKind::ToolLoop);
            let session_id: String = row.get(2)?;
            let agent_id: String = row.get(3)?;
            let tool_name: Option<String> = row.get(4)?;
            let file_path: Option<String> = row.get(5)?;
            let extra: Option<String> = row.get(6)?;
            let created_at_str: String = row.get(7)?;
            let timestamp = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(ImplicitSignal {
                id,
                kind,
                timestamp,
                session_id,
                agent_id,
                tool_name,
                file_path,
                extra,
            })
        };

        let rows = if let Some(sid) = session_id {
            stmt.query_map(params![sid], map_row)
        } else {
            stmt.query_map([], map_row)
        }
        .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Record a feedback event and adjust associated memory weights if applicable.
    pub fn record_feedback_event(&self, feedback: &FeedbackEvent) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = feedback.id.to_string();
        let mem_id_str = feedback.memory_id.map(|u| u.to_string());
        let sig_id_str = feedback.signal_id.map(|u| u.to_string());
        let rating_str = feedback.rating.to_string();
        let ts_str = feedback.timestamp.to_rfc3339();

        conn.execute(
            "INSERT INTO feedback_events (
                id, memory_id, signal_id, rating, comment, created_at, source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id_str,
                mem_id_str,
                sig_id_str,
                rating_str,
                feedback.comment,
                ts_str,
                feedback.source,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert feedback event: {e}")))?;

        // If feedback is attached to a memory, adjust memory importance/confidence and log access
        if let Some(mem_id) = feedback.memory_id {
            let adj_delta = match feedback.rating {
                FeedbackRating::Positive => 0.1,
                FeedbackRating::Negative => -0.1,
            };

            let mem_str = mem_id.to_string();
            let _ = conn.execute(
                "UPDATE memories
                 SET importance = MAX(0.0, MIN(1.0, importance + ?1)),
                     confidence = MAX(0.0, MIN(1.0, confidence + ?1)),
                     access_count = access_count + 1,
                     last_accessed_at = ?2,
                     updated_at = ?2
                 WHERE id = ?3",
                params![adj_delta, ts_str, mem_str],
            );
            let _ = conn.execute(
                "UPDATE semantic_facts
                 SET importance = MAX(0.0, MIN(1.0, importance + ?1)),
                     confidence = MAX(0.0, MIN(1.0, confidence + ?1)),
                     last_updated_at = ?2
                 WHERE id = ?3",
                params![adj_delta, ts_str, mem_str],
            );
            let _ = conn.execute(
                "UPDATE procedural_skills
                 SET importance = MAX(0.0, MIN(1.0, importance + ?1)),
                     last_used_at = ?2
                 WHERE id = ?3",
                params![adj_delta, ts_str, mem_str],
            );
        }

        Ok(())
    }

    /// Retrieve feedback events recorded for a specific memory.
    pub fn get_feedback_events_for_memory(
        &self,
        memory_id: &Uuid,
    ) -> Result<Vec<FeedbackEvent>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut stmt = conn
            .prepare(
                "SELECT id, memory_id, signal_id, rating, comment, created_at, source
                 FROM feedback_events
                 WHERE memory_id = ?1
                 ORDER BY created_at DESC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mem_str = memory_id.to_string();
        let rows = stmt
            .query_map(params![mem_str], |row| Self::row_to_feedback_event(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Retrieve feedback events, optionally filtered by session ID.
    pub fn get_feedback_events(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<FeedbackEvent>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let mut sql = "
            SELECT f.id, f.memory_id, f.signal_id, f.rating, f.comment, f.created_at, f.source
            FROM feedback_events f
        ".to_string();

        if let Some(sid) = session_id {
            sql.push_str(" LEFT JOIN implicit_signals s ON f.signal_id = s.id WHERE s.session_id = ?1 ORDER BY f.created_at ASC");
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows = stmt
                .query_map(params![sid], |row| Self::row_to_feedback_event(row))
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
            }
            Ok(results)
        } else {
            sql.push_str(" ORDER BY f.created_at ASC");
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| Self::row_to_feedback_event(row))
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
            }
            Ok(results)
        }
    }

    fn row_to_feedback_event(row: &rusqlite::Row) -> Result<FeedbackEvent, rusqlite::Error> {
        let id_str: String = row.get(0)?;
        let id = Uuid::parse_str(&id_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?;
        let mem_id_str: Option<String> = row.get(1)?;
        let memory_id = mem_id_str.and_then(|s| Uuid::parse_str(&s).ok());
        let sig_id_str: Option<String> = row.get(2)?;
        let signal_id = sig_id_str.and_then(|s| Uuid::parse_str(&s).ok());
        let rating_str: String = row.get(3)?;
        let rating = rating_str
            .parse::<FeedbackRating>()
            .unwrap_or(FeedbackRating::Positive);
        let comment: Option<String> = row.get(4)?;
        let created_at_str: String = row.get(5)?;
        let timestamp = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let source: String = row.get(6)?;

        Ok(FeedbackEvent {
            id,
            memory_id,
            signal_id,
            rating,
            comment,
            timestamp,
            source,
        })
    }

    /// Record a DPO preference pair.
    pub fn record_preference_pair(&self, pair: &PreferencePair) -> Result<(), StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let id_str = pair.id.to_string();
        let ts_str = pair.created_at.to_rfc3339();

        conn.execute(
            "INSERT INTO preference_pairs (
                id, prompt, chosen, rejected, source_session_id, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id_str,
                pair.prompt,
                pair.chosen,
                pair.rejected,
                pair.source_session_id,
                ts_str,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert preference pair: {e}")))?;

        Ok(())
    }

    /// Retrieve preference pairs, optionally filtered by session ID.
    pub fn get_preference_pairs(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<PreferencePair>, StrataError> {
        let conn = self.conn.lock().map_err(|_| {
            StrataError::Database("Lock poisoned on SQLite connection".to_string())
        })?;

        let map_row = |row: &rusqlite::Row| -> Result<PreferencePair, rusqlite::Error> {
            let id_str: String = row.get(0)?;
            let id = Uuid::parse_str(&id_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let prompt: String = row.get(1)?;
            let chosen: String = row.get(2)?;
            let rejected: String = row.get(3)?;
            let source_session_id: String = row.get(4)?;
            let created_at_str: String = row.get(5)?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(PreferencePair {
                id,
                prompt,
                chosen,
                rejected,
                source_session_id,
                created_at,
            })
        };

        let mut results = Vec::new();
        if let Some(sid) = session_id {
            let mut stmt = conn
                .prepare(
                    "SELECT id, prompt, chosen, rejected, source_session_id, created_at
                     FROM preference_pairs
                     WHERE source_session_id = ?1
                     ORDER BY created_at DESC",
                )
                .map_err(|e| StrataError::Database(e.to_string()))?;

            let rows = stmt
                .query_map(params![sid], map_row)
                .map_err(|e| StrataError::Database(e.to_string()))?;

            for r in rows {
                results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, prompt, chosen, rejected, source_session_id, created_at
                     FROM preference_pairs
                     ORDER BY created_at DESC",
                )
                .map_err(|e| StrataError::Database(e.to_string()))?;

            let rows = stmt
                .query_map([], map_row)
                .map_err(|e| StrataError::Database(e.to_string()))?;

            for r in rows {
                results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
            }
        }

        Ok(results)
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
