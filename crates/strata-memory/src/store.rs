use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::events::{
    DataClassification, Event, EventId, EventPayload, Provenance, RetentionPolicy,
};
use strata_core::schemas::{
    CodeAnchor, EpisodicMemory, EvidenceRef, FactDependency, FactStatus, FeedbackEvent,
    FeedbackRating, ImplicitSignal, JtmsAuditRow, MemoryFeedback, ParameterDef, PreferencePair,
    ProceduralExample, ProceduralSkill, ProceduralStep, SemanticFact, SignalKind, SignalScores,
    SyncDelta,
};

use strata_core::a2a::{AgentPresence, LeaseAcquireResult, ResourceLease};
use strata_core::state::{
    FailurePattern, FailureSeverity, MemoryRecord, MemoryTier, MemoryType, Scope,
};

use crate::call_graph::{CallEdge, CallType};
use crate::community::ArchitectureGraphSummary;
use crate::embedding::{bytes_to_embedding, embedding_to_bytes};

pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore").finish()
    }
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(None, "foreign_keys", "ON");
        let _ = conn.pragma_update(None, "busy_timeout", 5000);

        // Safe pre-migration for existing databases before indices are declared on newer columns
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN tier TEXT NOT NULL DEFAULT 'peripheral'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN approved_by_human INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN tier TEXT NOT NULL DEFAULT 'peripheral'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN approved_by_human INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN code_anchor_json TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN depends_on_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE cold_storage_memories ADD COLUMN tier TEXT NOT NULL DEFAULT 'peripheral'",
            [],
        );
        let _ = conn.execute("ALTER TABLE cold_storage_memories ADD COLUMN approved_by_human INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute(
            "ALTER TABLE preference_pairs ADD COLUMN oracle_verified INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE preference_pairs ADD COLUMN verification_source TEXT",
            [],
        );

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
                approved_by_human INTEGER NOT NULL DEFAULT 0,
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
            CREATE INDEX IF NOT EXISTS idx_memories_approved ON memories(approved_by_human);
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
                approved_by_human INTEGER NOT NULL DEFAULT 0,
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
                approved_by_human INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                last_updated_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                version INTEGER NOT NULL DEFAULT 1,
                replaced_by TEXT,
                tags_json TEXT NOT NULL DEFAULT '[]',
                code_anchor_json TEXT DEFAULT NULL,
                depends_on_json TEXT NOT NULL DEFAULT '[]',
                embedding BLOB
            );

            CREATE INDEX IF NOT EXISTS idx_facts_scope ON semantic_facts(scope);
            CREATE INDEX IF NOT EXISTS idx_facts_category ON semantic_facts(category);
            CREATE INDEX IF NOT EXISTS idx_facts_tier ON semantic_facts(tier);
            CREATE INDEX IF NOT EXISTS idx_facts_approved ON semantic_facts(approved_by_human);
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
                created_at TEXT NOT NULL,
                oracle_verified INTEGER DEFAULT 0,
                verification_source TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_pref_pairs_session ON preference_pairs(source_session_id);
            CREATE INDEX IF NOT EXISTS idx_pref_pairs_created ON preference_pairs(created_at);

            -- Native Call Graph Edges Table
            CREATE TABLE IF NOT EXISTS call_edges (
                id TEXT PRIMARY KEY,
                caller_file TEXT NOT NULL,
                caller_symbol TEXT NOT NULL,
                callee_symbol TEXT NOT NULL,
                callee_file_hint TEXT,
                line_number INTEGER NOT NULL,
                call_type TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_call_edges_caller ON call_edges(caller_file, caller_symbol);
            CREATE INDEX IF NOT EXISTS idx_call_edges_callee ON call_edges(callee_symbol);
            CREATE INDEX IF NOT EXISTS idx_call_edges_file ON call_edges(caller_file);
            CREATE INDEX IF NOT EXISTS idx_call_edges_type ON call_edges(call_type);

            -- Architecture Graph Summaries and Community Clusters cache table
            CREATE TABLE IF NOT EXISTS architecture_summaries (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                summary_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_arch_summaries_ws ON architecture_summaries(workspace_id);
            CREATE INDEX IF NOT EXISTS idx_arch_summaries_created ON architecture_summaries(created_at);

            -- JTMS Truth Maintenance Audits Table
            CREATE TABLE IF NOT EXISTS jtms_audits (
                id TEXT PRIMARY KEY,
                winning_fact_id TEXT NOT NULL,
                losing_fact_id TEXT NOT NULL,
                resolution_type TEXT NOT NULL,
                reason TEXT NOT NULL,
                contradiction_cues_json TEXT NOT NULL DEFAULT '[]',
                similarity REAL NOT NULL DEFAULT 0.0,
                timestamp TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_jtms_audits_winner ON jtms_audits(winning_fact_id);
            CREATE INDEX IF NOT EXISTS idx_jtms_audits_loser ON jtms_audits(losing_fact_id);
            CREATE INDEX IF NOT EXISTS idx_jtms_audits_ts ON jtms_audits(timestamp);
            CREATE INDEX IF NOT EXISTS idx_jtms_audits_type ON jtms_audits(resolution_type);

            -- Semantic Fact Dependencies Table
            CREATE TABLE IF NOT EXISTS fact_dependencies (
                id TEXT PRIMARY KEY,
                dependent_fact_id TEXT NOT NULL,
                prerequisite_fact_id TEXT NOT NULL,
                dependency_type TEXT NOT NULL DEFAULT 'supports',
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_fact_deps_dependent ON fact_dependencies(dependent_fact_id);
            CREATE INDEX IF NOT EXISTS idx_fact_deps_prereq ON fact_dependencies(prerequisite_fact_id);

            -- Agent Presence Table for Stigmergic Workspace Coordination
            CREATE TABLE IF NOT EXISTS agent_presence (
                agent_id TEXT PRIMARY KEY,
                host TEXT NOT NULL,
                pid INTEGER NOT NULL,
                active_task TEXT,
                heartbeat_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_agent_presence_hb ON agent_presence(heartbeat_at);

            -- Resource Leases Table for Cross-Agent Conflict Prevention
            CREATE TABLE IF NOT EXISTS resource_leases (
                resource_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                lease_expires_at INTEGER NOT NULL,
                metadata TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_resource_leases_expires ON resource_leases(lease_expires_at);
            CREATE INDEX IF NOT EXISTS idx_resource_leases_agent ON resource_leases(agent_id);
            ",
        )
        .map_err(|e| StrataError::Database(format!("Failed to execute schema migration: {e}")))?;

        // Safe migration for existing databases: add code_anchor_json, depends_on_json, tier, approved_by_human, oracle_verified if not exists
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN code_anchor_json TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN depends_on_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN tier TEXT NOT NULL DEFAULT 'peripheral'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN tier TEXT NOT NULL DEFAULT 'peripheral'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN approved_by_human INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN approved_by_human INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE cold_storage_memories ADD COLUMN approved_by_human INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute(
            "ALTER TABLE preference_pairs ADD COLUMN oracle_verified INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE preference_pairs ADD COLUMN verification_source TEXT",
            [],
        );

        Ok(())
    }

    // ==========================================
    // Event Store Operations
    // ==========================================

    pub fn insert_event(&self, event: &Event) -> Result<EventId, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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

            let id = id_str
                .parse::<Uuid>()
                .map(EventId::from_uuid)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in event id: {e}")))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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

            let id = id_str
                .parse::<Uuid>()
                .map(EventId::from_uuid)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in event id: {e}")))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let approved_int = if memory.approved_by_human { 1i64 } else { 0i64 };

        conn.execute(
            "INSERT INTO memories (
                id, memory_type, content, summary, scope, tier, approved_by_human, importance, confidence,
                tags_json, created_at, updated_at, access_count, last_accessed_at,
                evidence_ids_json, embedding, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                memory_type = excluded.memory_type,
                content = excluded.content,
                summary = excluded.summary,
                scope = excluded.scope,
                tier = excluded.tier,
                approved_by_human = excluded.approved_by_human,
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
                approved_int,
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let rows = conn
            .execute(
                "DELETE FROM memories WHERE id = ?1",
                params![id.to_string()],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(rows > 0)
    }

    pub fn get_all_memories(
        &self,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut query =
            "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
                                confidence, tags_json, created_at, updated_at, access_count,
                                last_accessed_at, evidence_ids_json, embedding, metadata_json
                         FROM memories WHERE 1=1"
                .to_string();

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let sanitized = sanitize_fts5_query(query_text);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = "
            SELECT m.id, m.memory_type, m.content, m.summary, m.scope, m.tier, m.approved_by_human, m.importance,
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
                let rank: f64 = row.get(17)?;
                Ok((mem, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Promotes a memory record to permanent Core Tier (frozen retention, R=1.0).
    /// Enforces strict requirement for explicit human approval (`approved_by_human == true`).
    pub fn promote_memory_to_core(
        &self,
        id: &Uuid,
        approved_by_human: bool,
        reason: Option<&str>,
    ) -> Result<MemoryRecord, StrataError> {
        if !approved_by_human {
            return Err(StrataError::Validation(
                "Cannot promote memory to Core Tier without explicit human approval (approved_by_human=true)".to_string(),
            ));
        }

        let mut mem = self.get_memory(id)?.ok_or_else(|| {
            StrataError::NotFound(format!("Memory record with ID '{id}' not found"))
        })?;

        mem.tier = MemoryTier::Core;
        mem.approved_by_human = true;
        mem.importance = 1.0;
        mem.updated_at = Utc::now();

        if let Some(r) = reason {
            if let serde_json::Value::Object(ref mut map) = mem.metadata {
                map.insert(
                    "promotion_reason".to_string(),
                    serde_json::Value::String(r.to_string()),
                );
                map.insert(
                    "promoted_at".to_string(),
                    serde_json::Value::String(Utc::now().to_rfc3339()),
                );
            } else {
                mem.metadata = serde_json::json!({
                    "promotion_reason": r,
                    "promoted_at": Utc::now().to_rfc3339(),
                });
            }
        }

        self.insert_or_update_memory(&mem)?;
        Ok(mem)
    }

    // ==========================================
    // Cold Storage Memory Operations
    // ==========================================

    /// Archives a memory record to cold storage, removing it from active memory and FTS index.
    pub fn archive_to_cold_storage(&self, record_id: &Uuid) -> Result<bool, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = record_id.to_string();
        let now_str = Utc::now().to_rfc3339();

        // 1. Fetch memory from active table
        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
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
                    row.get::<_, i64>(6).unwrap_or(0),
                    row.get::<_, f64>(7)?,
                    row.get::<_, f64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                ))
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let Some((
            id,
            m_type,
            content,
            summary,
            scope,
            tier,
            approved_by_human,
            imp,
            conf,
            tags,
            created,
            updated,
            count,
            last_acc,
            ev_ids,
            meta,
        )) = row_opt
        else {
            return Ok(false);
        };

        // 2. Insert into cold_storage_memories
        conn.execute(
            "INSERT INTO cold_storage_memories (
                id, memory_type, content, summary, scope, tier, approved_by_human, importance, confidence,
                tags_json, created_at, updated_at, archived_at, access_count,
                last_accessed_at, evidence_ids_json, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                archived_at = excluded.archived_at,
                updated_at = excluded.updated_at,
                access_count = excluded.access_count",
            params![
                id, m_type, content, summary, scope, tier, approved_by_human, imp, conf,
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        if let Some(q) = query {
            let sanitized = sanitize_fts5_query(q);
            if !sanitized.is_empty() {
                let sql = "
                    SELECT c.id, c.memory_type, c.content, c.summary, c.scope, c.tier, c.approved_by_human, c.importance,
                           c.confidence, c.tags_json, c.created_at, c.updated_at, c.access_count,
                           c.last_accessed_at, c.evidence_ids_json, NULL as embedding, c.metadata_json
                    FROM cold_storage_memories_fts f
                    JOIN cold_storage_memories c ON c.id = f.id
                    WHERE cold_storage_memories_fts MATCH ?1
                    LIMIT ?2
                ";
                let mut stmt = conn
                    .prepare(sql)
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = stmt
                    .query_map(params![sanitized, limit as i64], |row| {
                        Self::row_to_memory(row)
                    })
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                let mut results = Vec::new();
                for r in rows {
                    results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
                }
                return Ok(results);
            }
        }

        let mut sql =
            "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
                              confidence, tags_json, created_at, updated_at, access_count,
                              last_accessed_at, evidence_ids_json, NULL as embedding, metadata_json
                       FROM cold_storage_memories WHERE 1=1"
                .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(sc) = scope {
            if *sc != Scope::Global {
                sql.push_str(" AND (scope = ? OR scope = 'global')");
                params_vec.push(Box::new(sc.to_string()));
            }
        }

        sql.push_str(" ORDER BY archived_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn
            .prepare(&sql)
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

    /// Restores an archived memory from cold storage back into active memory with target tier.
    pub fn restore_from_cold_storage(
        &self,
        record_id: &Uuid,
        target_tier: Option<MemoryTier>,
    ) -> Result<bool, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = record_id.to_string();

        // 1. Fetch from cold storage
        let mut stmt = conn
            .prepare(
                "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
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
        conn.execute(
            "DELETE FROM cold_storage_memories WHERE id = ?1",
            params![id_str],
        )
        .map_err(|e| StrataError::Database(e.to_string()))?;

        // 4. Insert into active memories (unlocking conn before calling insert_or_update_memory)
        drop(conn);
        self.insert_or_update_memory(&mem)?;

        Ok(true)
    }

    /// Returns the total number of memories archived in cold storage.
    pub fn get_cold_storage_count(&self) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM cold_storage_memories", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);

        Ok(count as usize)
    }

    // ==========================================
    // Failure Patterns Operations
    // ==========================================

    pub fn upsert_failure_pattern(&self, failure: &FailurePattern) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
                "
                .to_string();

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
        "
        .to_string();

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, session_id, created_at, time_start, time_end, actor, project,
                              files_json, tools_used_json, summary, goals_json, obstacles_json,
                              outcomes_json, signals_json, tags_json, raw_event_ids_json
                       FROM episodic_memories WHERE 1=1"
            .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            params_vec.push(Box::new(p.to_string()));
        }

        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let rows = conn
            .execute(
                "DELETE FROM episodic_memories WHERE id = ?1",
                params![id.to_string()],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(rows > 0)
    }

    // ==========================================
    // ==========================================
    // Semantic Facts CRUD
    // ==========================================

    pub fn insert_or_update_semantic_fact(&self, fact: &SemanticFact) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = fact.id.to_string();
        let scope_str = fact.scope.to_string();
        let tier_str = fact.tier.to_string();
        let approved_int = if fact.approved_by_human { 1i64 } else { 0i64 };
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
        let depends_on_json = serde_json::to_string(&fact.depends_on)?;

        conn.execute(
            "INSERT INTO semantic_facts (
                id, project, scope, statement, category, evidence_json,
                importance, confidence, tier, approved_by_human, created_at, last_updated_at, status,
                version, replaced_by, tags_json, code_anchor_json, depends_on_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            ON CONFLICT(id) DO UPDATE SET
                project = excluded.project,
                scope = excluded.scope,
                statement = excluded.statement,
                category = excluded.category,
                evidence_json = excluded.evidence_json,
                importance = excluded.importance,
                confidence = excluded.confidence,
                tier = excluded.tier,
                approved_by_human = excluded.approved_by_human,
                last_updated_at = excluded.last_updated_at,
                status = excluded.status,
                version = excluded.version,
                replaced_by = excluded.replaced_by,
                tags_json = excluded.tags_json,
                code_anchor_json = excluded.code_anchor_json,
                depends_on_json = excluded.depends_on_json",
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
                approved_int,
                created_at_str,
                updated_at_str,
                status_str,
                fact.version as i64,
                replaced_str,
                tags_json,
                code_anchor_json,
                depends_on_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to persist semantic fact: {e}")))?;

        // Also record dependencies in fact_dependencies table
        for prereq_id in &fact.depends_on {
            let dep_id = format!("{}_{}", fact.id, prereq_id);
            let _ = conn.execute(
                "INSERT INTO fact_dependencies (id, dependent_fact_id, prerequisite_fact_id, dependency_type, created_at)
                 VALUES (?1, ?2, ?3, 'supports', ?4)
                 ON CONFLICT(id) DO NOTHING",
                params![dep_id, fact.id.to_string(), prereq_id.to_string(), created_at_str],
            );
        }

        Ok(())
    }

    pub fn get_semantic_fact(&self, id: &Uuid) -> Result<Option<SemanticFact>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, project, scope, statement, category, evidence_json,
                        importance, confidence, tier, approved_by_human, created_at, last_updated_at, status,
                        version, replaced_by, tags_json, code_anchor_json, depends_on_json
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

    pub fn update_semantic_fact_embedding(
        &self,
        id: &Uuid,
        embedding: &[f32],
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, project, scope, statement, category, evidence_json,
                              importance, confidence, tier, approved_by_human, created_at, last_updated_at, status,
                              version, replaced_by, tags_json, code_anchor_json, depends_on_json
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

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, project, scope, statement, category, evidence_json,
                              importance, confidence, tier, approved_by_human, created_at, last_updated_at, status,
                              version, replaced_by, tags_json, code_anchor_json, depends_on_json, embedding
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

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| {
                let fact = Self::row_to_fact(row)?;
                let blob_opt: Option<Vec<u8>> = row.get(18)?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let sanitized = sanitize_fts5_query(query_text);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "
            SELECT m.id, m.project, m.scope, m.statement, m.category, m.evidence_json,
                   m.importance, m.confidence, m.tier, m.approved_by_human, m.created_at, m.last_updated_at, m.status,
                   m.version, m.replaced_by, m.tags_json, m.code_anchor_json, m.depends_on_json,
                   bm25(semantic_facts_fts) as rank
            FROM semantic_facts_fts f
            JOIN semantic_facts m ON m.id = f.id
            WHERE semantic_facts_fts MATCH ?1
            ORDER BY rank ASC LIMIT ?2
        ";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![sanitized, limit as i64], |row| {
                let fact = Self::row_to_fact(row)?;
                let rank: f64 = row.get(18)?;
                Ok((fact, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    // ==========================================
    // JTMS Audits & Fact Dependencies
    // ==========================================

    pub fn insert_jtms_audit(&self, audit: &JtmsAuditRow) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = audit.id.to_string();
        let winning_str = audit.winning_fact_id.to_string();
        let losing_str = audit.losing_fact_id.to_string();
        let cues_json = serde_json::to_string(&audit.contradiction_cues)?;
        let ts_str = audit.timestamp.to_rfc3339();
        let metadata_json = serde_json::to_string(&audit.metadata)?;

        conn.execute(
            "INSERT INTO jtms_audits (
                id, winning_fact_id, losing_fact_id, resolution_type, reason,
                contradiction_cues_json, similarity, timestamp, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                winning_fact_id = excluded.winning_fact_id,
                losing_fact_id = excluded.losing_fact_id,
                resolution_type = excluded.resolution_type,
                reason = excluded.reason,
                contradiction_cues_json = excluded.contradiction_cues_json,
                similarity = excluded.similarity,
                timestamp = excluded.timestamp,
                metadata_json = excluded.metadata_json",
            params![
                id_str,
                winning_str,
                losing_str,
                audit.resolution_type,
                audit.reason,
                cues_json,
                audit.similarity,
                ts_str,
                metadata_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert JTMS audit row: {e}")))?;

        Ok(())
    }

    pub fn get_jtms_audits_for_fact(
        &self,
        fact_id: &Uuid,
    ) -> Result<Vec<JtmsAuditRow>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let fact_str = fact_id.to_string();
        let mut stmt = conn
            .prepare(
                "SELECT id, winning_fact_id, losing_fact_id, resolution_type, reason,
                        contradiction_cues_json, similarity, timestamp, metadata_json
                 FROM jtms_audits
                 WHERE winning_fact_id = ?1 OR losing_fact_id = ?1
                 ORDER BY timestamp DESC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![fact_str], |row| Self::row_to_jtms_audit(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_all_jtms_audits(&self, limit: usize) -> Result<Vec<JtmsAuditRow>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, winning_fact_id, losing_fact_id, resolution_type, reason,
                        contradiction_cues_json, similarity, timestamp, metadata_json
                 FROM jtms_audits
                 ORDER BY timestamp DESC
                 LIMIT ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit as i64], |row| Self::row_to_jtms_audit(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn add_fact_dependency(&self, dep: &FactDependency) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = dep.id.to_string();
        let dep_fact_str = dep.dependent_fact_id.to_string();
        let prereq_str = dep.prerequisite_fact_id.to_string();
        let ts_str = dep.created_at.to_rfc3339();

        conn.execute(
            "INSERT INTO fact_dependencies (id, dependent_fact_id, prerequisite_fact_id, dependency_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                dependent_fact_id = excluded.dependent_fact_id,
                prerequisite_fact_id = excluded.prerequisite_fact_id,
                dependency_type = excluded.dependency_type",
            params![id_str, dep_fact_str, prereq_str, dep.dependency_type, ts_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert fact dependency: {e}")))?;

        Ok(())
    }

    pub fn get_downstream_dependent_fact_ids(
        &self,
        prerequisite_id: &Uuid,
    ) -> Result<Vec<Uuid>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let prereq_str = prerequisite_id.to_string();
        let mut stmt = conn
            .prepare(
                "SELECT dependent_fact_id FROM fact_dependencies WHERE prerequisite_fact_id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![prereq_str], |row| {
                let id_str: String = row.get(0)?;
                Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_upstream_prerequisite_fact_ids(
        &self,
        dependent_id: &Uuid,
    ) -> Result<Vec<Uuid>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let dep_str = dependent_id.to_string();
        let mut stmt = conn
            .prepare(
                "SELECT prerequisite_fact_id FROM fact_dependencies WHERE dependent_fact_id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![dep_str], |row| {
                let id_str: String = row.get(0)?;
                Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Promotes a semantic fact to permanent Core Tier (frozen retention, R=1.0).
    /// Enforces strict requirement for explicit human approval (`approved_by_human == true`).
    pub fn promote_semantic_fact_to_core(
        &self,
        id: &Uuid,
        approved_by_human: bool,
        reason: Option<&str>,
    ) -> Result<SemanticFact, StrataError> {
        if !approved_by_human {
            return Err(StrataError::Validation(
                "Cannot promote semantic fact to Core Tier without explicit human approval (approved_by_human=true)".to_string(),
            ));
        }

        let mut fact = self.get_semantic_fact(id)?.ok_or_else(|| {
            StrataError::NotFound(format!("Semantic fact with ID '{id}' not found"))
        })?;

        fact.tier = MemoryTier::Core;
        fact.approved_by_human = true;
        fact.importance = 1.0;
        fact.last_updated_at = Utc::now();

        if let Some(r) = reason {
            fact.tags.push(format!("reason:{r}"));
        }

        self.insert_or_update_semantic_fact(&fact)?;
        Ok(fact)
    }

    pub fn get_facts_by_file_anchor(
        &self,
        file_path: &str,
    ) -> Result<Vec<SemanticFact>, StrataError> {
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let rows = conn
            .execute(
                "DELETE FROM semantic_facts WHERE id = ?1",
                params![id.to_string()],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(rows > 0)
    }

    // ==========================================
    // Procedural Skills CRUD
    // ==========================================

    pub fn insert_or_update_procedural_skill(
        &self,
        skill: &ProceduralSkill,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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

    pub fn get_procedural_skill_by_name(
        &self,
        name: &str,
    ) -> Result<Option<ProceduralSkill>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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

    pub fn update_procedural_skill_embedding(
        &self,
        id: &Uuid,
        embedding: &[f32],
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, name, project, description, preconditions_json,
                              postconditions_json, parameters_json, steps_json, examples_json,
                              success_rate, importance, created_at, last_used_at, usage_count,
                              tags_json
                       FROM procedural_skills WHERE 1=1"
            .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            params_vec.push(Box::new(p.to_string()));
        }

        sql.push_str(" ORDER BY usage_count DESC, created_at DESC LIMIT ?");
        params_vec.push(Box::new(limit as i64));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;
        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let rows = conn
            .execute(
                "DELETE FROM procedural_skills WHERE id = ?1",
                params![id.to_string()],
            )
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

    pub fn record_memory_access(
        &self,
        memory_id: &Uuid,
        accessed_at: DateTime<Utc>,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        self.record_memory_access_internal(&conn, memory_id, accessed_at)
    }

    pub fn get_memory_access_logs(
        &self,
        memory_id: &Uuid,
    ) -> Result<Vec<DateTime<Utc>>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let approved_by_human_int: i64 = row.get(6).unwrap_or(0);
        let importance: f64 = row.get(7)?;
        let confidence: f64 = row.get(8)?;
        let tags_json: String = row.get(9)?;
        let created_at_str: String = row.get(10)?;
        let updated_at_str: String = row.get(11)?;
        let access_count: i64 = row.get(12)?;
        let last_accessed_str: Option<String> = row.get(13)?;
        let evidence_json: String = row.get(14)?;
        let embedding_bytes: Option<Vec<u8>> = row.get(15)?;
        let metadata_json: String = row.get(16)?;

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
        let approved_by_human = approved_by_human_int != 0;
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
            approved_by_human,
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
        let approved_by_human_int: i64 = row.get(9).unwrap_or(0);
        let created_at_str: String = row.get(10)?;
        let updated_at_str: String = row.get(11)?;
        let status_str: String = row.get(12)?;
        let version: i64 = row.get(13)?;
        let replaced_str: Option<String> = row.get(14)?;
        let tags_json: String = row.get(15)?;
        let code_anchor_json: Option<String> = row.get(16).ok().flatten();

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let scope = scope_str
            .parse::<Scope>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let tier = tier_str
            .parse::<MemoryTier>()
            .unwrap_or(MemoryTier::Peripheral);
        let approved_by_human = approved_by_human_int != 0;
        let evidence: Vec<EvidenceRef> = serde_json::from_str(&evidence_json).unwrap_or_default();
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let last_updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let status = status_str
            .parse::<FactStatus>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let replaced_by = replaced_str.and_then(|s| Uuid::parse_str(&s).ok());
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let code_anchor: Option<CodeAnchor> =
            code_anchor_json.and_then(|s| serde_json::from_str(&s).ok());
        let depends_on_json: String = row.get(17).unwrap_or_else(|_| "[]".to_string());
        let depends_on: Vec<Uuid> = serde_json::from_str(&depends_on_json).unwrap_or_default();

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
            approved_by_human,
            created_at,
            last_updated_at,
            status,
            version: version as u32,
            replaced_by,
            tags,
            code_anchor,
            depends_on,
        })
    }

    fn row_to_jtms_audit(row: &rusqlite::Row) -> rusqlite::Result<JtmsAuditRow> {
        let id_str: String = row.get(0)?;
        let winning_str: String = row.get(1)?;
        let losing_str: String = row.get(2)?;
        let resolution_type: String = row.get(3)?;
        let reason: String = row.get(4)?;
        let cues_json: String = row.get(5)?;
        let similarity: f64 = row.get(6)?;
        let ts_str: String = row.get(7)?;
        let meta_json: String = row.get(8)?;

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let winning_fact_id =
            Uuid::parse_str(&winning_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let losing_fact_id =
            Uuid::parse_str(&losing_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let contradiction_cues: Vec<String> = serde_json::from_str(&cues_json).unwrap_or_default();
        let timestamp = DateTime::parse_from_rfc3339(&ts_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let metadata: serde_json::Value = serde_json::from_str(&meta_json).unwrap_or_default();

        Ok(JtmsAuditRow {
            id,
            winning_fact_id,
            losing_fact_id,
            resolution_type,
            reason,
            contradiction_cues,
            similarity: similarity as f32,
            timestamp,
            metadata,
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
        let preconditions: Vec<String> =
            serde_json::from_str(&preconditions_json).unwrap_or_default();
        let postconditions: Vec<String> =
            serde_json::from_str(&postconditions_json).unwrap_or_default();
        let parameters: Vec<ParameterDef> = serde_json::from_str(&params_json).unwrap_or_default();
        let steps: Vec<ProceduralStep> = serde_json::from_str(&steps_json).unwrap_or_default();
        let examples: Vec<ProceduralExample> =
            serde_json::from_str(&examples_json).unwrap_or_default();
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
    pub fn get_pending_deltas(
        &self,
        workspace_id: &str,
        limit: usize,
    ) -> Result<Vec<SyncDelta>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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

                Ok((
                    id_str,
                    ws_id,
                    seq,
                    ts_str,
                    kind,
                    payload_json,
                    version_hash,
                    synced_int,
                ))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut deltas = Vec::new();
        for r in rows {
            let (id_str, ws_id, seq, ts_str, kind, payload_json, version_hash, synced_int) =
                r.map_err(|e| StrataError::Database(e.to_string()))?;
            let id = Uuid::parse_str(&id_str).map_err(|e| {
                StrataError::Validation(format!("Invalid UUID in sync delta id: {e}"))
            })?;
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

        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to begin transaction: {e}")))?;

        {
            let mut stmt = tx
                .prepare("UPDATE sync_outbox SET synced = 1 WHERE id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;

            for id in delta_ids {
                stmt.execute(params![id.to_string()]).map_err(|e| {
                    StrataError::Database(format!("Failed to mark delta synced: {e}"))
                })?;
            }
        }

        tx.commit().map_err(|e| {
            StrataError::Database(format!("Failed to commit mark_deltas_synced: {e}"))
        })?;

        Ok(())
    }

    /// Record a failure for a batch of deltas, incrementing retry count and calculating exponential backoff.
    pub fn record_delta_failure(
        &self,
        delta_ids: &[Uuid],
        err_msg: &str,
    ) -> Result<(), StrataError> {
        if delta_ids.is_empty() {
            return Ok(());
        }

        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to begin transaction: {e}")))?;

        {
            let mut select_stmt = tx
                .prepare("SELECT retry_count FROM sync_outbox WHERE id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;

            let mut update_stmt = tx
                .prepare(
                    "UPDATE sync_outbox SET retry_count = ?1, next_retry_ts = ?2 WHERE id = ?3",
                )
                .map_err(|e| StrataError::Database(e.to_string()))?;

            for id in delta_ids {
                let id_str = id.to_string();
                let retry_count: i64 = select_stmt
                    .query_row(params![&id_str], |row| row.get(0))
                    .unwrap_or(0);

                let next_count = retry_count + 1;
                let backoff_secs = (0.5 * (2_f64.powi(next_count.min(10) as i32))).min(300.0);
                let next_retry_ts = (Utc::now()
                    + chrono::Duration::milliseconds((backoff_secs * 1000.0) as i64))
                .to_rfc3339();

                update_stmt
                    .execute(params![next_count, next_retry_ts, &id_str])
                    .map_err(|e| {
                        StrataError::Database(format!("Failed to update delta failure: {e}"))
                    })?;
            }
        }

        tx.commit().map_err(|e| {
            StrataError::Database(format!("Failed to commit record_delta_failure: {e}"))
        })?;

        tracing::warn!(
            "Recorded sync delta failure for {} deltas: {}",
            delta_ids.len(),
            err_msg
        );
        Ok(())
    }

    /// Retrieve sync status (pending unsynced deltas count, highest local sequence number).
    pub fn get_sync_status(&self, workspace_id: &str) -> Result<(usize, u64), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        .map_err(|e| {
            StrataError::Database(format!("Failed to update semantic fact feedback: {e}"))
        })?;

        // 5. Update procedural_skills table if present
        conn.execute(
            "UPDATE procedural_skills
             SET importance = MAX(0.0, MIN(1.0, importance + ?1)),
                 last_used_at = ?2
             WHERE id = ?3",
            params![adj_delta, now_str, mem_id_str],
        )
        .map_err(|e| {
            StrataError::Database(format!("Failed to update procedural skill feedback: {e}"))
        })?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "
            SELECT id, kind, session_id, agent_id, tool_name, file_path, extra, created_at
            FROM implicit_signals
        "
        .to_string();

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "
            SELECT f.id, f.memory_id, f.signal_id, f.rating, f.comment, f.created_at, f.source
            FROM feedback_events f
        "
        .to_string();

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
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = pair.id.to_string();
        let ts_str = pair.created_at.to_rfc3339();
        let oracle_verified_int = if pair.oracle_verified { 1 } else { 0 };

        conn.execute(
            "INSERT INTO preference_pairs (
                id, prompt, chosen, rejected, source_session_id, created_at, oracle_verified, verification_source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id_str,
                pair.prompt,
                pair.chosen,
                pair.rejected,
                pair.source_session_id,
                ts_str,
                oracle_verified_int,
                pair.verification_source,
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
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

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
            let oracle_verified_int: Option<i64> = row.get(6).ok();
            let oracle_verified = oracle_verified_int.map(|v| v != 0).unwrap_or(false);
            let verification_source: Option<String> = row.get(7).ok().flatten();

            Ok(PreferencePair {
                id,
                prompt,
                chosen,
                rejected,
                source_session_id,
                created_at,
                oracle_verified,
                verification_source,
            })
        };

        let mut results = Vec::new();
        if let Some(sid) = session_id {
            let mut stmt = conn
                .prepare(
                    "SELECT id, prompt, chosen, rejected, source_session_id, created_at, oracle_verified, verification_source
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
                    "SELECT id, prompt, chosen, rejected, source_session_id, created_at, oracle_verified, verification_source
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

    // ==========================================
    // Call Graph Edges CRUD Operations
    // ==========================================

    /// Inserts a batch of extracted CallGraph edges atomically into SQLite.
    pub fn insert_call_edges(&self, edges: &[CallEdge]) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        for edge in edges {
            let id_str = edge.id.to_string();
            let call_type_str = edge.call_type.to_string();
            let created_at_str = edge.created_at.to_rfc3339();

            conn.execute(
                "INSERT INTO call_edges (
                    id, caller_file, caller_symbol, callee_symbol,
                    callee_file_hint, line_number, call_type, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(id) DO UPDATE SET
                    caller_file = excluded.caller_file,
                    caller_symbol = excluded.caller_symbol,
                    callee_symbol = excluded.callee_symbol,
                    callee_file_hint = excluded.callee_file_hint,
                    line_number = excluded.line_number,
                    call_type = excluded.call_type",
                params![
                    id_str,
                    edge.caller_file,
                    edge.caller_symbol,
                    edge.callee_symbol,
                    edge.callee_file_hint,
                    edge.line_number as i64,
                    call_type_str,
                    created_at_str,
                ],
            )
            .map_err(|e| StrataError::Database(format!("Failed to persist call edge: {e}")))?;
        }

        Ok(())
    }

    /// Clears all call edges originating from a specific file (used prior to re-indexing).
    pub fn clear_call_edges_for_file(&self, file_path: &str) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let rows = conn
            .execute(
                "DELETE FROM call_edges WHERE caller_file = ?1",
                params![file_path],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(rows)
    }

    /// Retrieves all callers that invoke or import a given callee symbol.
    pub fn get_callers_of_symbol(
        &self,
        callee_symbol: &str,
        limit: usize,
    ) -> Result<Vec<CallEdge>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, caller_file, caller_symbol, callee_symbol,
                        callee_file_hint, line_number, call_type, created_at
                 FROM call_edges
                 WHERE callee_symbol = ?1 OR callee_symbol LIKE ?2
                 ORDER BY created_at DESC
                 LIMIT ?3",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let wildcard = format!("%::{callee_symbol}");
        let rows = stmt
            .query_map(params![callee_symbol, wildcard, limit as i64], |row| {
                Self::row_to_call_edge(row)
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Retrieves all callees invoked by a specific symbol inside a file.
    pub fn get_callees_of_symbol(
        &self,
        caller_file: &str,
        caller_symbol: &str,
    ) -> Result<Vec<CallEdge>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, caller_file, caller_symbol, callee_symbol,
                        callee_file_hint, line_number, call_type, created_at
                 FROM call_edges
                 WHERE caller_file = ?1 AND caller_symbol = ?2
                 ORDER BY line_number ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![caller_file, caller_symbol], |row| {
                Self::row_to_call_edge(row)
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Retrieves all call and import edges recorded for a given file.
    pub fn get_file_call_edges(&self, file_path: &str) -> Result<Vec<CallEdge>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, caller_file, caller_symbol, callee_symbol,
                        callee_file_hint, line_number, call_type, created_at
                 FROM call_edges
                 WHERE caller_file = ?1
                 ORDER BY line_number ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![file_path], |row| Self::row_to_call_edge(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Returns the total count of call edges indexed in SQLite.
    pub fn get_call_edges_count(&self) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM call_edges", [], |r| r.get(0))
            .unwrap_or(0);

        Ok(count as usize)
    }

    fn row_to_call_edge(row: &rusqlite::Row) -> rusqlite::Result<CallEdge> {
        let id_str: String = row.get(0)?;
        let caller_file: String = row.get(1)?;
        let caller_symbol: String = row.get(2)?;
        let callee_symbol: String = row.get(3)?;
        let callee_file_hint: Option<String> = row.get(4)?;
        let line_number: i64 = row.get(5)?;
        let call_type_str: String = row.get(6)?;
        let created_at_str: String = row.get(7)?;

        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let call_type = call_type_str
            .parse::<CallType>()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(CallEdge {
            id,
            caller_file,
            caller_symbol,
            callee_symbol,
            callee_file_hint,
            line_number: line_number as u32,
            call_type,
            created_at,
        })
    }

    // ==========================================
    // Architecture Graph Summary Cache
    // ==========================================

    /// Caches an ArchitectureGraphSummary into SQLite for fast recall.
    pub fn cache_architecture_summary(
        &self,
        summary: &ArchitectureGraphSummary,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = summary.id.to_string();
        let created_at_str = summary.created_at.to_rfc3339();
        let summary_json = serde_json::to_string(summary)?;

        conn.execute(
            "INSERT INTO architecture_summaries (id, workspace_id, summary_json, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                workspace_id = excluded.workspace_id,
                summary_json = excluded.summary_json,
                created_at = excluded.created_at",
            params![id_str, summary.workspace_id, summary_json, created_at_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to cache architecture summary: {e}")))?;

        Ok(())
    }

    /// Retrieves the most recent cached ArchitectureGraphSummary for a given workspace.
    pub fn get_cached_architecture_summary(
        &self,
        workspace_id: &str,
    ) -> Result<Option<ArchitectureGraphSummary>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT summary_json FROM architecture_summaries
                 WHERE workspace_id = ?1
                 ORDER BY created_at DESC
                 LIMIT 1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let summary_opt: Option<String> = stmt
            .query_row(params![workspace_id], |r| r.get(0))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if let Some(json_str) = summary_opt {
            let summary: ArchitectureGraphSummary = serde_json::from_str(&json_str)?;
            Ok(Some(summary))
        } else {
            Ok(None)
        }
    }

    // ==========================================
    // Agent-to-Agent (A2A) Stigmergic Leases & Presence Operations
    // ==========================================

    /// Upserts agent presence and heartbeat.
    pub fn record_agent_presence(&self, presence: &AgentPresence) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "INSERT INTO agent_presence (agent_id, host, pid, active_task, heartbeat_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(agent_id) DO UPDATE SET
                 host = excluded.host,
                 pid = excluded.pid,
                 active_task = excluded.active_task,
                 heartbeat_at = excluded.heartbeat_at",
            params![
                presence.agent_id,
                presence.host,
                presence.pid,
                presence.active_task,
                presence.heartbeat_at
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to record agent presence: {e}")))?;

        Ok(())
    }

    /// Retrieves all active agents whose last heartbeat is within `ttl_seconds`.
    pub fn get_active_agents(&self, ttl_seconds: i64) -> Result<Vec<AgentPresence>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let cutoff = now - ttl_seconds;

        let mut stmt = conn
            .prepare(
                "SELECT agent_id, host, pid, active_task, heartbeat_at
                 FROM agent_presence
                 WHERE heartbeat_at >= ?1
                 ORDER BY heartbeat_at DESC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![cutoff], |row| {
                Ok(AgentPresence {
                    agent_id: row.get(0)?,
                    host: row.get(1)?,
                    pid: row.get(2)?,
                    active_task: row.get(3)?,
                    heartbeat_at: row.get(4)?,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Removes an agent presence record (e.g. on clean agent exit).
    pub fn remove_agent_presence(&self, agent_id: &str) -> Result<bool, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let rows = conn
            .execute(
                "DELETE FROM agent_presence WHERE agent_id = ?1",
                params![agent_id],
            )
            .map_err(|e| StrataError::Database(format!("Failed to remove agent presence: {e}")))?;

        Ok(rows > 0)
    }

    /// Atomically acquires or renews a lease on a resource with a given TTL.
    ///
    /// If the resource is unleased or expired, or already held by the same agent,
    /// the lease is acquired/renewed and `LeaseAcquireResult::Acquired` is returned.
    ///
    /// If another agent holds an unexpired lease, `LeaseAcquireResult::Conflict` is returned.
    pub fn acquire_or_renew_lease(
        &self,
        resource_id: &str,
        agent_id: &str,
        ttl_seconds: i64,
        metadata: Option<&str>,
    ) -> Result<LeaseAcquireResult, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let expires_at = now + ttl_seconds;

        // Atomic acquire/renew statement
        let rows_affected = conn
            .execute(
                "INSERT INTO resource_leases (resource_id, agent_id, lease_expires_at, metadata)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(resource_id) DO UPDATE SET
                 agent_id = excluded.agent_id,
                 lease_expires_at = excluded.lease_expires_at,
                 metadata = excluded.metadata
             WHERE lease_expires_at <= ?5 OR agent_id = excluded.agent_id",
                params![resource_id, agent_id, expires_at, metadata, now],
            )
            .map_err(|e| {
                StrataError::Database(format!("Failed to execute atomic lease statement: {e}"))
            })?;

        if rows_affected > 0 {
            Ok(LeaseAcquireResult::Acquired {
                resource_id: resource_id.to_string(),
                expires_at,
            })
        } else {
            // Conflict: query who holds the unexpired lease
            let mut stmt = conn
                .prepare(
                    "SELECT agent_id, lease_expires_at FROM resource_leases WHERE resource_id = ?1",
                )
                .map_err(|e| StrataError::Database(e.to_string()))?;

            let conflict_info = stmt
                .query_row(params![resource_id], |row| {
                    let held_by: String = row.get(0)?;
                    let held_until: i64 = row.get(1)?;
                    Ok((held_by, held_until))
                })
                .optional()
                .map_err(|e| StrataError::Database(e.to_string()))?;

            if let Some((held_by, held_until)) = conflict_info {
                let remaining_seconds = (held_until - now).max(0);
                Ok(LeaseAcquireResult::Conflict {
                    resource_id: resource_id.to_string(),
                    held_by,
                    remaining_seconds,
                })
            } else {
                // Highly improbable edge case: row was deleted immediately after
                Ok(LeaseAcquireResult::Acquired {
                    resource_id: resource_id.to_string(),
                    expires_at,
                })
            }
        }
    }

    /// Releases a lease if held by the given agent.
    pub fn release_lease(&self, resource_id: &str, agent_id: &str) -> Result<bool, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let rows = conn
            .execute(
                "DELETE FROM resource_leases WHERE resource_id = ?1 AND agent_id = ?2",
                params![resource_id, agent_id],
            )
            .map_err(|e| StrataError::Database(format!("Failed to release lease: {e}")))?;

        Ok(rows > 0)
    }

    /// Fetches a lease for a resource.
    pub fn get_lease(&self, resource_id: &str) -> Result<Option<ResourceLease>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT resource_id, agent_id, lease_expires_at, metadata
             FROM resource_leases WHERE resource_id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let lease = stmt
            .query_row(params![resource_id], |row| {
                Ok(ResourceLease {
                    resource_id: row.get(0)?,
                    agent_id: row.get(1)?,
                    lease_expires_at: row.get(2)?,
                    metadata: row.get(3)?,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(lease)
    }

    /// Returns all unexpired leases.
    pub fn list_active_leases(&self) -> Result<Vec<ResourceLease>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();

        let mut stmt = conn
            .prepare(
                "SELECT resource_id, agent_id, lease_expires_at, metadata
             FROM resource_leases
             WHERE lease_expires_at > ?1
             ORDER BY lease_expires_at ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![now], |row| {
                Ok(ResourceLease {
                    resource_id: row.get(0)?,
                    agent_id: row.get(1)?,
                    lease_expires_at: row.get(2)?,
                    metadata: row.get(3)?,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Prunes expired leases from the database.
    pub fn prune_expired_leases(&self) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let rows = conn
            .execute(
                "DELETE FROM resource_leases WHERE lease_expires_at <= ?1",
                params![now],
            )
            .map_err(|e| StrataError::Database(format!("Failed to prune expired leases: {e}")))?;

        Ok(rows)
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
