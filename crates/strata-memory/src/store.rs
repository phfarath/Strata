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
    BranchMergeReport, CodeAnchor, EpisodeRecord, EpisodeReplay, EpisodeStep, EpisodicMemory,
    EvidenceRef, FactDependency, FactStatus, FeedbackEvent, FeedbackRating,
    HippocampalConsolidationResult, ImplicitSignal, JtmsAuditRow, MemoryFeedback, ParameterDef,
    PreferencePair, ProceduralExample, ProceduralSkill, ProceduralStep, SemanticFact, SignalKind,
    SignalScores, SyncDelta,
};

use strata_core::a2a::{
    A2aMessage, AgentCard, AgentPresence, DomainAuthority, LeaseAcquireResult, ResourceLease,
};
use strata_core::blackboard::{TaskArtifact, TaskFilter, TaskPayload, TaskRecord, TaskStatus};
use strata_core::state::{
    FailurePattern, FailureSeverity, MemoryRecord, MemoryTier, MemoryType, Scope,
};

use crate::call_graph::{CallEdge, CallType};
use crate::community::ArchitectureGraphSummary;
use crate::embedding::{bytes_to_embedding, embedding_to_bytes};
use strata_reasoning::{ProceduralRule, ProceduralRuleVersion, TextGradientFeedback};

/// A modulated synaptic edge between two graph nodes in the Knowledge Graph.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryEdgeRecord {
    pub source_id: String,
    pub target_id: String,
    pub edge_kind: String,
    pub weight: f32,
    pub reinforcements: u32,
    pub depressions: u32,
    pub last_feedback_at: DateTime<Utc>,
}

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
        let _ = conn.execute(
            "ALTER TABLE episodes ADD COLUMN tier INTEGER NOT NULL DEFAULT 1",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE episodes ADD COLUMN parent_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE episodes ADD COLUMN child_ids_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN valid_from TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN valid_until TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN git_commit_hash TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN valid_from TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN valid_until TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN git_commit_hash TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE events ADD COLUMN git_branch TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE memories ADD COLUMN git_branch TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE semantic_facts ADD COLUMN git_branch TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE episodic_memories ADD COLUMN git_branch TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE episodes ADD COLUMN git_branch TEXT DEFAULT NULL",
            [],
        );
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_events_branch ON events(git_branch)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_memories_branch ON memories(git_branch)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_facts_branch ON semantic_facts(git_branch)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_episodic_branch ON episodic_memories(git_branch)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_episodes_branch ON episodes(git_branch)", []);
        let _ = conn.execute("ALTER TABLE tasks ADD COLUMN claim_generation INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE tasks ADD COLUMN payload_json TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE memories ADD COLUMN embedding_model TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE memories ADD COLUMN embedding_version TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE episodic_memories ADD COLUMN embedding_model TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE episodic_memories ADD COLUMN embedding_version TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE semantic_facts ADD COLUMN embedding_model TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE semantic_facts ADD COLUMN embedding_version TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE procedural_skills ADD COLUMN embedding_model TEXT DEFAULT NULL", []);
        let _ = conn.execute("ALTER TABLE procedural_skills ADD COLUMN embedding_version TEXT DEFAULT NULL", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_memories_embedding_model ON memories(embedding_model)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_facts_embedding_model ON semantic_facts(embedding_model)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_skills_embedding_model ON procedural_skills(embedding_model)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_episodic_embedding_model ON episodic_memories(embedding_model)", []);
        let _ = conn.execute("DROP TRIGGER IF EXISTS trg_memories_au", []);

        // Idempotently backfill valid_until for legacy deprecated records
        let _ = conn.execute(
            "UPDATE memories SET valid_until = updated_at WHERE valid_until IS NULL AND json_extract(metadata_json, '$.status') = 'deprecated'",
            [],
        );
        let _ = conn.execute(
            "UPDATE semantic_facts SET valid_until = last_updated_at WHERE valid_until IS NULL AND status = 'deprecated'",
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
                metadata_json TEXT NOT NULL,
                git_branch TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, sequence_num);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_events_branch ON events(git_branch);

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
                metadata_json TEXT NOT NULL DEFAULT '{}',
                valid_from TEXT,
                valid_until TEXT,
                git_commit_hash TEXT,
                git_branch TEXT,
                embedding_model TEXT,
                embedding_version TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope);
            CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_memories_tier ON memories(tier);
            CREATE INDEX IF NOT EXISTS idx_memories_approved ON memories(approved_by_human);
            CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);
            CREATE INDEX IF NOT EXISTS idx_memories_valid_from ON memories(valid_from);
            CREATE INDEX IF NOT EXISTS idx_memories_valid_until ON memories(valid_until);
            CREATE INDEX IF NOT EXISTS idx_memories_commit ON memories(git_commit_hash);
            CREATE INDEX IF NOT EXISTS idx_memories_branch ON memories(git_branch);
            CREATE INDEX IF NOT EXISTS idx_memories_embedding_model ON memories(embedding_model);

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

            CREATE TRIGGER IF NOT EXISTS trg_memories_au AFTER UPDATE OF content, summary, tags_json ON memories BEGIN
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
            CREATE INDEX IF NOT EXISTS idx_failures_file_symbol ON failure_patterns(
                json_extract(metadata_json, '$.file'),
                json_extract(metadata_json, '$.symbol')
            );

            -- Futility accumulator state, one row per (agent, task)
            CREATE TABLE IF NOT EXISTS futility_state (
                agent_id TEXT NOT NULL,
                task_key TEXT NOT NULL,
                state_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (agent_id, task_key)
            );

            -- Raw signal/verdict history, used to calibrate the futility accumulator
            CREATE TABLE IF NOT EXISTS futility_trace (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                task_key TEXT NOT NULL,
                signal TEXT NOT NULL,
                tool TEXT NOT NULL,
                signature TEXT NOT NULL,
                verdict TEXT NOT NULL,
                evidence INTEGER NOT NULL,
                ts INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_futility_trace_key ON futility_trace(agent_id, task_key, id);

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
                embedding BLOB,
                git_branch TEXT,
                embedding_model TEXT,
                embedding_version TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_episodic_session ON episodic_memories(session_id);
            CREATE INDEX IF NOT EXISTS idx_episodic_created ON episodic_memories(created_at);
            CREATE INDEX IF NOT EXISTS idx_episodic_project ON episodic_memories(project);
            CREATE INDEX IF NOT EXISTS idx_episodic_branch ON episodic_memories(git_branch);
            CREATE INDEX IF NOT EXISTS idx_episodic_embedding_model ON episodic_memories(embedding_model);

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
                embedding BLOB,
                valid_from TEXT,
                valid_until TEXT,
                git_commit_hash TEXT,
                git_branch TEXT,
                embedding_model TEXT,
                embedding_version TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_facts_scope ON semantic_facts(scope);
            CREATE INDEX IF NOT EXISTS idx_facts_category ON semantic_facts(category);
            CREATE INDEX IF NOT EXISTS idx_facts_tier ON semantic_facts(tier);
            CREATE INDEX IF NOT EXISTS idx_facts_approved ON semantic_facts(approved_by_human);
            CREATE INDEX IF NOT EXISTS idx_facts_status ON semantic_facts(status);
            CREATE INDEX IF NOT EXISTS idx_facts_project ON semantic_facts(project);
            CREATE INDEX IF NOT EXISTS idx_facts_valid_from ON semantic_facts(valid_from);
            CREATE INDEX IF NOT EXISTS idx_facts_valid_until ON semantic_facts(valid_until);
            CREATE INDEX IF NOT EXISTS idx_facts_commit ON semantic_facts(git_commit_hash);
            CREATE INDEX IF NOT EXISTS idx_facts_branch ON semantic_facts(git_branch);
            CREATE INDEX IF NOT EXISTS idx_facts_embedding_model ON semantic_facts(embedding_model);

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
                embedding BLOB,
                embedding_model TEXT,
                embedding_version TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_skills_name ON procedural_skills(name);
            CREATE INDEX IF NOT EXISTS idx_skills_project ON procedural_skills(project);
            CREATE INDEX IF NOT EXISTS idx_skills_embedding_model ON procedural_skills(embedding_model);

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

            -- Multi-Agent Blackboard Tasks Table
            CREATE TABLE IF NOT EXISTS tasks (
                task_id TEXT PRIMARY KEY,
                plan_id TEXT,
                parent_id TEXT,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL,
                assigned_agent TEXT,
                required_resources_json TEXT NOT NULL DEFAULT '[]',
                dependencies_json TEXT NOT NULL DEFAULT '[]',
                outputs_json TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                claim_generation INTEGER NOT NULL DEFAULT 0,
                payload_json TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_tasks_plan ON tasks(plan_id);
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_agent ON tasks(assigned_agent);

            -- Task Artifacts Table for Cross-Agent Handoff
            CREATE TABLE IF NOT EXISTS task_artifacts (
                artifact_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_task_artifacts_task ON task_artifacts(task_id);

            -- A2A Local Mailbox Messages Table
            CREATE TABLE IF NOT EXISTS a2a_messages (
                message_id TEXT PRIMARY KEY,
                from_agent TEXT NOT NULL,
                to_agent TEXT,
                task_id TEXT,
                kind TEXT NOT NULL DEFAULT 'direct',
                content TEXT NOT NULL,
                read_at INTEGER,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_a2a_messages_to ON a2a_messages(to_agent);
            CREATE INDEX IF NOT EXISTS idx_a2a_messages_task ON a2a_messages(task_id);
            CREATE INDEX IF NOT EXISTS idx_a2a_messages_read ON a2a_messages(read_at);

            -- Transactive Memory Directory: Agent Capability Cards
            CREATE TABLE IF NOT EXISTS agent_cards (
                agent_id TEXT PRIMARY KEY,
                host TEXT NOT NULL,
                memory_capabilities_json TEXT NOT NULL,
                domain_scopes_json TEXT NOT NULL,
                memory_persistence TEXT NOT NULL,
                tenant_scopes_json TEXT NOT NULL,
                memory_privacy_mode TEXT NOT NULL,
                confidence_score REAL NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_agent_cards_updated ON agent_cards(updated_at);

            -- Transactive Memory Directory: Domain Authorities
            CREATE TABLE IF NOT EXISTS domain_authorities (
                domain TEXT PRIMARY KEY,
                authoritative_agent TEXT NOT NULL,
                confidence_score REAL NOT NULL,
                lease_held INTEGER NOT NULL DEFAULT 0,
                active_tasks_count INTEGER NOT NULL DEFAULT 0,
                last_active_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_domain_authorities_agent ON domain_authorities(authoritative_agent);
            CREATE INDEX IF NOT EXISTS idx_domain_authorities_conf ON domain_authorities(confidence_score);

            -- Hippocampal Episodic Traces Table
            CREATE TABLE IF NOT EXISTS episodes (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                task_id TEXT,
                summary TEXT NOT NULL,
                event_trace_json TEXT NOT NULL,
                outcomes_json TEXT NOT NULL,
                salience_score REAL NOT NULL,
                state_snapshot_json TEXT,
                causal_provenance TEXT,
                consolidated INTEGER NOT NULL DEFAULT 0,
                tier INTEGER NOT NULL DEFAULT 1,
                parent_id TEXT,
                child_ids_json TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                git_branch TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_episodes_session ON episodes(session_id);
            CREATE INDEX IF NOT EXISTS idx_episodes_agent ON episodes(agent_id);
            CREATE INDEX IF NOT EXISTS idx_episodes_consolidated ON episodes(consolidated);
            CREATE INDEX IF NOT EXISTS idx_episodes_tier ON episodes(tier);
            CREATE INDEX IF NOT EXISTS idx_episodes_parent ON episodes(parent_id);
            CREATE INDEX IF NOT EXISTS idx_episodes_branch ON episodes(git_branch);

            -- Tripartite Synapse Modulated Edges Table
            CREATE TABLE IF NOT EXISTS memory_edges (
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                edge_kind TEXT NOT NULL,
                weight REAL NOT NULL,
                reinforcements INTEGER NOT NULL DEFAULT 0,
                depressions INTEGER NOT NULL DEFAULT 0,
                last_feedback_at TEXT NOT NULL,
                PRIMARY KEY (source_id, target_id, edge_kind)
            );

            CREATE INDEX IF NOT EXISTS idx_memory_edges_source ON memory_edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_memory_edges_target ON memory_edges(target_id);

            -- Procedural Rules Table
            CREATE TABLE IF NOT EXISTS procedural_rules (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'general',
                current_version INTEGER NOT NULL DEFAULT 1,
                current_content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Procedural Rule Versions Table
            CREATE TABLE IF NOT EXISTS procedural_rule_versions (
                id TEXT PRIMARY KEY,
                rule_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                content TEXT NOT NULL,
                rationale TEXT NOT NULL,
                feedback_json TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(rule_id) REFERENCES procedural_rules(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_rule_versions_rule ON procedural_rule_versions(rule_id, version);
            CREATE INDEX IF NOT EXISTS idx_procedural_rules_name ON procedural_rules(name);

            -- Memory Injection Outcomes Table for Context Rot & Poisoning Detection
            CREATE TABLE IF NOT EXISTS memory_injection_outcomes (
                id TEXT PRIMARY KEY,
                memory_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                task_id TEXT,
                command TEXT,
                outcome TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_injection_memory ON memory_injection_outcomes(memory_id);
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
        let _ = conn.execute(
            "ALTER TABLE episodes ADD COLUMN tier INTEGER NOT NULL DEFAULT 1",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE episodes ADD COLUMN parent_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE episodes ADD COLUMN child_ids_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_episodes_tier ON episodes(tier)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_episodes_parent ON episodes(parent_id)",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE tasks ADD COLUMN claim_generation INTEGER NOT NULL DEFAULT 0",
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
                retention, metadata_json, git_branch
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                event.git_branch,
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
                        classification, retention, metadata_json, git_branch
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
                let git_branch: Option<String> = row.get(11)?;

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
                    git_branch,
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
                git_branch,
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
                git_branch,
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
                        classification, retention, metadata_json, git_branch
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
                let git_branch: Option<String> = row.get(11)?;

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
                    git_branch,
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
                git_branch,
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
                git_branch,
            });
        }

        Ok(events)
    }

    // ==========================================
    // Memory Record Operations
    // ==========================================

    pub fn insert_memory(&self, memory: &MemoryRecord) -> Result<(), StrataError> {
        self.insert_or_update_memory(memory)
    }

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

        let valid_from_str = memory
            .metadata
            .get("valid_from")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| memory.created_at.to_rfc3339());

        let valid_until_str = memory
            .metadata
            .get("valid_until")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                if memory
                    .metadata
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map_or(false, |s| s.eq_ignore_ascii_case("deprecated"))
                {
                    Some(memory.updated_at.to_rfc3339())
                } else {
                    None
                }
            });

        let git_commit_str = memory
            .metadata
            .get("git_commit_hash")
            .or_else(|| memory.metadata.get("git_commit"))
            .or_else(|| memory.metadata.get("commit"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                memory
                    .metadata
                    .get("code_anchor")
                    .and_then(|ca| ca.get("git_commit_hash"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

        let git_branch_str = memory
            .git_branch()
            .map(|s| s.to_string())
            .or_else(|| {
                memory
                    .metadata
                    .get("git_branch")
                    .or_else(|| memory.metadata.get("branch"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                memory
                    .metadata
                    .get("code_anchor")
                    .and_then(|ca| ca.get("git_branch"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

        conn.execute(
            "INSERT INTO memories (
                id, memory_type, content, summary, scope, tier, approved_by_human, importance, confidence,
                tags_json, created_at, updated_at, access_count, last_accessed_at,
                evidence_ids_json, embedding, metadata_json, valid_from, valid_until, git_commit_hash, git_branch,
                embedding_model, embedding_version
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
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
                embedding = CASE WHEN excluded.embedding IS NOT NULL THEN excluded.embedding ELSE memories.embedding END,
                embedding_model = CASE WHEN excluded.embedding IS NOT NULL THEN excluded.embedding_model ELSE memories.embedding_model END,
                embedding_version = CASE WHEN excluded.embedding IS NOT NULL THEN excluded.embedding_version ELSE memories.embedding_version END,
                metadata_json = excluded.metadata_json,
                valid_from = excluded.valid_from,
                valid_until = excluded.valid_until,
                git_commit_hash = excluded.git_commit_hash,
                git_branch = COALESCE(excluded.git_branch, memories.git_branch)",
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
                valid_from_str,
                valid_until_str,
                git_commit_str,
                git_branch_str,
                memory.embedding_model.as_deref(),
                memory.embedding_version.as_deref(),
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to persist memory: {e}")))?;

        Ok(())
    }

    /// Atomically updates access metrics (`access_count` and `last_accessed_at`) for a memory record
    /// and logs the access in `memory_access_logs`, without rewriting content, metadata, importance,
    /// or quarantine state.
    pub fn update_access_metrics(
        &self,
        id: &Uuid,
        accessed_at: DateTime<Utc>,
    ) -> Result<bool, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = id.to_string();
        let ts_str = accessed_at.to_rfc3339();

        let rows_affected = conn
            .execute(
                "UPDATE memories
                 SET access_count = access_count + 1,
                     last_accessed_at = MAX(COALESCE(last_accessed_at, ?1), ?1)
                 WHERE id = ?2",
                params![ts_str, id_str],
            )
            .map_err(|e| StrataError::Database(format!("Failed to update access metrics: {e}")))?;

        if rows_affected > 0 {
            let _ = self.record_memory_access_internal(&conn, id, accessed_at);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn get_memory(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError> {
        self.get_memory_as_of(id, None, None, None)
    }

    pub fn get_memory_as_of(
        &self,
        id: &Uuid,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
    ) -> Result<Option<MemoryRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
                              confidence, tags_json, created_at, updated_at, access_count,
                              last_accessed_at, evidence_ids_json, embedding, metadata_json,
                              embedding_model, embedding_version
                       FROM memories
                       WHERE id = ?1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(id.to_string())];

        if let Some(t) = as_of_time {
            let t_str = t.to_rfc3339();
            sql.push_str(" AND COALESCE(valid_from, created_at) <= ? AND (valid_until IS NULL OR valid_until > ?)");
            params_vec.push(Box::new(t_str.clone()));
            params_vec.push(Box::new(t_str));
        }

        if let Some(c) = as_of_commit {
            sql.push_str(" AND (git_commit_hash = ? OR json_extract(metadata_json, '$.git_commit_hash') = ? OR json_extract(metadata_json, '$.git_commit') = ? OR json_extract(metadata_json, '$.commit') = ?)");
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master')");
            } else if b != "*" && b != "all" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master' OR git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
            }
        }

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let res = stmt
            .query_row(params_slice.as_slice(), |row| Self::row_to_memory(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

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
        self.get_all_memories_as_of(scope, memory_types, None, None, None, limit)
    }

    pub fn get_all_memories_as_of(
        &self,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut query =
            "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
                    confidence, tags_json, created_at, updated_at, access_count,
                    last_accessed_at, evidence_ids_json, embedding, metadata_json,
                    embedding_model, embedding_version
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

        if let Some(t) = as_of_time {
            let t_str = t.to_rfc3339();
            query.push_str(" AND COALESCE(valid_from, created_at) <= ? AND (valid_until IS NULL OR valid_until > ?)");
            params_vec.push(Box::new(t_str.clone()));
            params_vec.push(Box::new(t_str));
        }

        if let Some(c) = as_of_commit {
            query.push_str(" AND (git_commit_hash = ? OR json_extract(metadata_json, '$.git_commit_hash') = ? OR json_extract(metadata_json, '$.git_commit') = ? OR json_extract(metadata_json, '$.commit') = ?)");
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                query.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master')");
            } else if b != "*" && b != "all" {
                query.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master' OR git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
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

    /// Stream vector candidate memories that have non-null embeddings within bi-temporal and scope bounds.
    /// Eliminates premature chronological truncation (`ORDER BY created_at DESC LIMIT N`) so cosine similarity
    /// can be computed across the eligible corpus.
    pub fn for_each_vector_candidate_as_of<F>(
        &self,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
        mut callback: F,
    ) -> Result<(), StrataError>
    where
        F: FnMut(MemoryRecord) -> Result<bool, StrataError>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut query =
            "SELECT id, memory_type, content, summary, scope, tier, approved_by_human, importance,
                    confidence, tags_json, created_at, updated_at, access_count,
                    last_accessed_at, evidence_ids_json, embedding, metadata_json,
                    embedding_model, embedding_version
             FROM memories WHERE embedding IS NOT NULL"
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

        if let Some(t) = as_of_time {
            let t_str = t.to_rfc3339();
            query.push_str(" AND COALESCE(valid_from, created_at) <= ? AND (valid_until IS NULL OR valid_until > ?)");
            params_vec.push(Box::new(t_str.clone()));
            params_vec.push(Box::new(t_str));
        }

        if let Some(c) = as_of_commit {
            query.push_str(" AND (git_commit_hash = ? OR json_extract(metadata_json, '$.git_commit_hash') = ? OR json_extract(metadata_json, '$.git_commit') = ? OR json_extract(metadata_json, '$.commit') = ?)");
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                query.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master')");
            } else if b != "*" && b != "all" {
                query.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master' OR git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
            }
        }

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_slice.as_slice(), |row| Self::row_to_memory(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        for r in rows {
            let record = r.map_err(|e| StrataError::Database(e.to_string()))?;
            let keep_going = callback(record)?;
            if !keep_going {
                break;
            }
        }
        Ok(())
    }

    /// Retrieve all vector candidates as a collection within bi-temporal and scope bounds.
    pub fn get_vector_candidates_as_of(
        &self,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let mut results = Vec::new();
        self.for_each_vector_candidate_as_of(
            scope,
            memory_types,
            as_of_time,
            as_of_commit,
            as_of_branch,
            |rec| {
                results.push(rec);
                Ok(true)
            },
        )?;
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
        self.search_fts_as_of(query_text, scope, None, None, None, limit)
    }

    /// Search memories using FTS5 Porter stemmer with bi-temporal filters (as_of_time, as_of_commit, as_of_branch).
    pub fn search_fts_as_of(
        &self,
        query_text: &str,
        scope: Option<&Scope>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
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
                   m.embedding_model, m.embedding_version,
                   bm25(memories_fts) as rank
            FROM memories_fts f
            JOIN memories m ON m.id = f.id
            WHERE memories_fts MATCH ?1
        ".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(sanitized));

        if let Some(sc) = scope {
            if *sc != Scope::Global {
                sql.push_str(" AND (m.scope = ? OR m.scope = 'global')");
                params_vec.push(Box::new(sc.to_string()));
            }
        }

        if let Some(t) = as_of_time {
            let t_str = t.to_rfc3339();
            sql.push_str(" AND COALESCE(m.valid_from, m.created_at) <= ? AND (m.valid_until IS NULL OR m.valid_until > ?)");
            params_vec.push(Box::new(t_str.clone()));
            params_vec.push(Box::new(t_str));
        }

        if let Some(c) = as_of_commit {
            sql.push_str(" AND (m.git_commit_hash = ? OR json_extract(m.metadata_json, '$.git_commit_hash') = ? OR json_extract(m.metadata_json, '$.git_commit') = ? OR json_extract(m.metadata_json, '$.commit') = ?)");
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                sql.push_str(" AND (m.git_branch IS NULL OR m.git_branch = 'main' OR m.git_branch = 'master')");
            } else if b != "*" && b != "all" {
                sql.push_str(" AND (m.git_branch IS NULL OR m.git_branch = 'main' OR m.git_branch = 'master' OR m.git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
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
                let rank: f64 = row.get(19)?;
                Ok((mem, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Search memories using FTS5 Porter stemmer across multiple allowed scopes (e.g. Community + Bridge + Global).
    pub fn search_fts_scopes(
        &self,
        query_text: &str,
        scopes: &[Scope],
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
                   m.embedding_model, m.embedding_version,
                   bm25(memories_fts) as rank
            FROM memories_fts f
            JOIN memories m ON m.id = f.id
            WHERE memories_fts MATCH ?1
        ".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(sanitized));

        let non_global_scopes: Vec<String> = scopes
            .iter()
            .filter(|s| **s != Scope::Global)
            .map(|s| s.to_string())
            .collect();

        if !non_global_scopes.is_empty() {
            let placeholders: Vec<String> = (0..non_global_scopes.len())
                .map(|i| format!("?{}", i + 2))
                .collect();
            let placeholders_str = placeholders.join(",");
            sql.push_str(&format!(" AND (m.scope IN ({placeholders_str}) OR m.scope = 'global')"));
            for s in non_global_scopes {
                params_vec.push(Box::new(s));
            }
        } else if !scopes.is_empty() && scopes.iter().all(|s| *s == Scope::Global) {
            sql.push_str(" AND m.scope = 'global'");
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
                let rank: f64 = row.get(19)?;
                Ok((mem, rank as f32))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Retrieve memories across multiple allowed scopes (e.g. Community + Bridge clusters + Global).
    pub fn get_memories_by_scopes(
        &self,
        scopes: &[Scope],
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
                                last_accessed_at, evidence_ids_json, embedding, metadata_json,
                                embedding_model, embedding_version
                         FROM memories WHERE 1=1"
                .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        let non_global_scopes: Vec<String> = scopes
            .iter()
            .filter(|s| **s != Scope::Global)
            .map(|s| s.to_string())
            .collect();

        if !non_global_scopes.is_empty() {
            let placeholders = vec!["?"; non_global_scopes.len()].join(",");
            query.push_str(&format!(" AND (scope IN ({placeholders}) OR scope = 'global')"));
            for s in non_global_scopes {
                params_vec.push(Box::new(s));
            }
        } else if !scopes.is_empty() && scopes.iter().all(|s| *s == Scope::Global) {
            query.push_str(" AND scope = 'global'");
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

    /// Count memories belonging to a specific scope.
    pub fn count_memories_by_scope(&self, scope: &Scope) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE scope = ?1",
                rusqlite::params![scope.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(count as usize)
    }

    /// Count all active memories in the store.
    pub fn count_all_memories(&self) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(count as usize)
    }

    /// Update scope of a specific memory record.
    pub fn update_memory_scope(&self, id: &Uuid, new_scope: &Scope) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        conn.execute(
            "UPDATE memories SET scope = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![new_scope.to_string(), Utc::now().to_rfc3339(), id.to_string()],
        )
        .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(())
    }

    /// Batch migrate all memories from an old community cluster ID to a new cluster ID.
    pub fn migrate_community_scope(
        &self,
        old_cluster_id: &str,
        new_cluster_id: &str,
    ) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let old_scope_str = format!("community:{}", old_cluster_id);
        let new_scope_str = format!("community:{}", new_cluster_id);
        let changed = conn
            .execute(
                "UPDATE memories SET scope = ?1, updated_at = ?2 WHERE scope = ?3",
                rusqlite::params![new_scope_str, Utc::now().to_rfc3339(), old_scope_str],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(changed)
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

    /// Loads the futility accumulator for (agent, task). Returns a fresh one when no row
    /// exists or the row has been idle for more than `idle_ttl_secs`.
    pub fn load_futility_state(
        &self,
        agent_id: &str,
        task_key: &str,
        now: i64,
        idle_ttl_secs: i64,
    ) -> Result<crate::futility::FutilityAccumulator, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT state_json, updated_at FROM futility_state
                 WHERE agent_id = ?1 AND task_key = ?2",
                params![agent_id, task_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        match row {
            Some((json, updated_at)) if now - updated_at <= idle_ttl_secs => {
                Ok(serde_json::from_str(&json)?)
            }
            _ => Ok(crate::futility::FutilityAccumulator::new()),
        }
    }

    /// Persists the futility accumulator for (agent, task).
    pub fn save_futility_state(
        &self,
        agent_id: &str,
        task_key: &str,
        state: &crate::futility::FutilityAccumulator,
        now: i64,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "INSERT INTO futility_state (agent_id, task_key, state_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, task_key) DO UPDATE SET
                state_json = excluded.state_json,
                updated_at = excluded.updated_at",
            params![agent_id, task_key, serde_json::to_string(state)?, now],
        )
        .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(())
    }
    /// Appends one row to the futility trace.
    pub fn append_futility_trace(
        &self,
        agent_id: &str,
        task_key: &str,
        row: &crate::futility::FutilityTraceRow,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        conn.execute(
            "INSERT INTO futility_trace
                (agent_id, task_key, signal, tool, signature, verdict, evidence, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                agent_id,
                task_key,
                row.signal,
                row.tool,
                row.signature,
                row.verdict,
                row.evidence,
                row.ts
            ],
        )
        .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(())
    }

    /// Returns the most recent `limit` trace rows for (agent, task), oldest first.
    pub fn list_futility_trace(
        &self,
        agent_id: &str,
        task_key: &str,
        limit: usize,
    ) -> Result<Vec<crate::futility::FutilityTraceRow>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT signal, tool, signature, verdict, evidence, ts FROM futility_trace
                 WHERE agent_id = ?1 AND task_key = ?2 ORDER BY id DESC LIMIT ?3",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;
        let mut rows = stmt
            .query_map(params![agent_id, task_key, limit as i64], |r| {
                Ok(crate::futility::FutilityTraceRow {
                    signal: r.get(0)?,
                    tool: r.get(1)?,
                    signature: r.get(2)?,
                    verdict: r.get(3)?,
                    evidence: r.get(4)?,
                    ts: r.get(5)?,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| StrataError::Database(e.to_string()))?;
        rows.reverse();
        Ok(rows)
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

    pub fn get_failure_pattern(&self, id: &Uuid) -> Result<Option<FailurePattern>, StrataError> {
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
                 WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![id.to_string()], |row| Self::row_to_failure(row))
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

    /// Finds failure anti-patterns anchored to a specific file and optional symbol.
    /// Returns exact symbol matches first, followed by file-level failure patterns.
    pub fn find_symbol_anti_patterns(
        &self,
        file_path: &str,
        symbol: Option<&str>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let norm_file = file_path.replace('\\', "/");
        let norm_file = norm_file.trim_start_matches("./");

        let mut sql = "
            SELECT id, signature, pattern_name, description, trigger_condition,
                   error_type, mitigation, occurrences, first_seen, last_seen,
                   severity, scope, metadata_json
            FROM failure_patterns
            WHERE (
                json_extract(metadata_json, '$.file') = ?1
                OR ?1 LIKE '%' || json_extract(metadata_json, '$.file')
                OR json_extract(metadata_json, '$.file') LIKE '%' || ?1
            )
        "
        .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(norm_file.to_string()));

        if let Some(s) = symbol {
            sql.push_str(" AND (json_extract(metadata_json, '$.symbol') = ?2 OR json_extract(metadata_json, '$.symbol') IS NULL)");
            params_vec.push(Box::new(s.to_string()));
            sql.push_str(" ORDER BY (CASE WHEN json_extract(metadata_json, '$.symbol') = ?2 THEN 0 ELSE 1 END), occurrences DESC, last_seen DESC LIMIT ?3");
            params_vec.push(Box::new(limit as i64));
        } else {
            sql.push_str(" ORDER BY (CASE WHEN json_extract(metadata_json, '$.symbol') IS NULL THEN 0 ELSE 1 END), occurrences DESC, last_seen DESC LIMIT ?2");
            params_vec.push(Box::new(limit as i64));
        }

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

    /// Attaches a verified correction to a known anti-pattern by its unique signature (Dual-Polarity).
    pub fn attach_verified_correction(
        &self,
        signature: &str,
        correction: &str,
    ) -> Result<bool, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let rows_affected = conn
            .execute(
                "UPDATE failure_patterns
                 SET metadata_json = json_set(metadata_json, '$.verified_correction', ?1)
                 WHERE signature = ?2",
                rusqlite::params![correction, signature],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(rows_affected > 0)
    }

    /// Attaches a verified correction to all anti-patterns anchored to a file and optional symbol.
    pub fn attach_symbol_correction(
        &self,
        file: &str,
        symbol: Option<&str>,
        correction: &str,
    ) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let norm_file = file.replace('\\', "/");
        let norm_file = norm_file.trim_start_matches("./");

        let rows = if let Some(s) = symbol {
            conn.execute(
                "UPDATE failure_patterns
                 SET metadata_json = json_set(metadata_json, '$.verified_correction', ?1)
                 WHERE (json_extract(metadata_json, '$.file') = ?2
                        OR ?2 LIKE '%' || json_extract(metadata_json, '$.file')
                        OR json_extract(metadata_json, '$.file') LIKE '%' || ?2)
                   AND json_extract(metadata_json, '$.symbol') = ?3",
                rusqlite::params![correction, norm_file, s],
            )
        } else {
            conn.execute(
                "UPDATE failure_patterns
                 SET metadata_json = json_set(metadata_json, '$.verified_correction', ?1)
                 WHERE (json_extract(metadata_json, '$.file') = ?2
                        OR ?2 LIKE '%' || json_extract(metadata_json, '$.file')
                        OR json_extract(metadata_json, '$.file') LIKE '%' || ?2)",
                rusqlite::params![correction, norm_file],
            )
        }
        .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(rows)
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
                outcomes_json, signals_json, tags_json, raw_event_ids_json, git_branch
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
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
                raw_event_ids_json = excluded.raw_event_ids_json,
                git_branch = COALESCE(excluded.git_branch, episodic_memories.git_branch)",
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
                memory.git_branch,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert episodic memory: {e}")))?;

        Ok(())
    }

    pub fn update_episodic_memory_embedding_with_model(
        &self,
        id: &Uuid,
        embedding: &[f32],
        model: Option<&str>,
        version: Option<&str>,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "UPDATE episodic_memories SET embedding = ?1, embedding_model = ?2, embedding_version = ?3 WHERE id = ?4",
            params![embedding_to_bytes(embedding), model, version, id.to_string()],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update episodic memory embedding: {e}")))?;

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
                        outcomes_json, signals_json, tags_json, raw_event_ids_json, git_branch
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
                        outcomes_json, signals_json, tags_json, raw_event_ids_json, git_branch
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
        self.get_all_episodic_memories_as_of(project, None, limit)
    }

    pub fn get_all_episodic_memories_as_of(
        &self,
        project: Option<&str>,
        as_of_branch: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodicMemory>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, session_id, created_at, time_start, time_end, actor, project,
                              files_json, tools_used_json, summary, goals_json, obstacles_json,
                              outcomes_json, signals_json, tags_json, raw_event_ids_json, git_branch
                       FROM episodic_memories WHERE 1=1"
            .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(p) = project {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            params_vec.push(Box::new(p.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master')");
            } else if b != "*" && b != "all" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master' OR git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
            }
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
        self.search_episodic_memories_fts_as_of(query_text, None, limit)
    }

    pub fn search_episodic_memories_fts_as_of(
        &self,
        query_text: &str,
        as_of_branch: Option<&str>,
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

        let mut sql = "
            SELECT m.id, m.session_id, m.created_at, m.time_start, m.time_end, m.actor, m.project,
                   m.files_json, m.tools_used_json, m.summary, m.goals_json, m.obstacles_json,
                   m.outcomes_json, m.signals_json, m.tags_json, m.raw_event_ids_json, m.git_branch,
                   bm25(episodic_memories_fts) as rank
            FROM episodic_memories_fts f
            JOIN episodic_memories m ON m.id = f.id
            WHERE episodic_memories_fts MATCH ?1
        ".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(sanitized));

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                sql.push_str(" AND (m.git_branch IS NULL OR m.git_branch = 'main' OR m.git_branch = 'master')");
            } else if b != "*" && b != "all" {
                sql.push_str(" AND (m.git_branch IS NULL OR m.git_branch = 'main' OR m.git_branch = 'master' OR m.git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
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
                let ep = Self::row_to_episodic(row)?;
                let rank: f64 = row.get(17)?;
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

        let (valid_from_str, valid_until_str, git_commit_str) =
            if let Some(ref anchor) = fact.code_anchor {
                (
                    anchor.valid_from.to_rfc3339(),
                    anchor.valid_until.map(|u| u.to_rfc3339()).or_else(|| {
                        if fact.status == FactStatus::Deprecated {
                            Some(fact.last_updated_at.to_rfc3339())
                        } else {
                            None
                        }
                    }),
                    anchor.git_commit_hash.clone(),
                )
            } else {
                (
                    fact.created_at.to_rfc3339(),
                    if fact.status == FactStatus::Deprecated {
                        Some(fact.last_updated_at.to_rfc3339())
                    } else {
                        None
                    },
                    None,
                )
            };

        let git_branch_str = fact.git_branch().map(|s| s.to_string());

        conn.execute(
            "INSERT INTO semantic_facts (
                id, project, scope, statement, category, evidence_json,
                importance, confidence, tier, approved_by_human, created_at, last_updated_at, status,
                version, replaced_by, tags_json, code_anchor_json, depends_on_json,
                valid_from, valid_until, git_commit_hash, git_branch
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
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
                depends_on_json = excluded.depends_on_json,
                valid_from = excluded.valid_from,
                valid_until = excluded.valid_until,
                git_commit_hash = excluded.git_commit_hash,
                git_branch = COALESCE(excluded.git_branch, semantic_facts.git_branch)",
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
                valid_from_str,
                valid_until_str,
                git_commit_str,
                git_branch_str,
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
        self.get_semantic_fact_as_of(id, None, None, None)
    }

    pub fn get_semantic_fact_as_of(
        &self,
        id: &Uuid,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
    ) -> Result<Option<SemanticFact>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, project, scope, statement, category, evidence_json,
                              importance, confidence, tier, approved_by_human, created_at, last_updated_at, status,
                              version, replaced_by, tags_json, code_anchor_json, depends_on_json
                       FROM semantic_facts
                       WHERE id = ?1".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(id.to_string())];

        if let Some(t) = as_of_time {
            let t_str = t.to_rfc3339();
            sql.push_str(" AND COALESCE(valid_from, created_at) <= ? AND (valid_until IS NULL OR valid_until > ?)");
            params_vec.push(Box::new(t_str.clone()));
            params_vec.push(Box::new(t_str));
        }

        if let Some(c) = as_of_commit {
            sql.push_str(" AND (git_commit_hash = ? OR json_extract(code_anchor_json, '$.git_commit_hash') = ?)");
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master')");
            } else if b != "*" && b != "all" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master' OR git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
            }
        }

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let res = stmt
            .query_row(params_slice.as_slice(), |row| Self::row_to_fact(row))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if res.is_some() {
            let _ = self.record_memory_access_internal(&conn, id, Utc::now());
        }

        Ok(res)
    }

    pub fn update_memory_embedding_with_model(
        &self,
        id: &Uuid,
        embedding: &[f32],
        model: Option<&str>,
        version: Option<&str>,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "UPDATE memories SET embedding = ?1, embedding_model = ?2, embedding_version = ?3, updated_at = ?4 WHERE id = ?5",
            params![embedding_to_bytes(embedding), model, version, Utc::now().to_rfc3339(), id.to_string()],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update memory embedding: {e}")))?;

        Ok(())
    }

    pub fn update_semantic_fact_embedding(
        &self,
        id: &Uuid,
        embedding: &[f32],
    ) -> Result<(), StrataError> {
        self.update_semantic_fact_embedding_with_model(id, embedding, None, None)
    }

    pub fn update_semantic_fact_embedding_with_model(
        &self,
        id: &Uuid,
        embedding: &[f32],
        model: Option<&str>,
        version: Option<&str>,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "UPDATE semantic_facts SET embedding = ?1, embedding_model = ?2, embedding_version = ?3 WHERE id = ?4",
            params![embedding_to_bytes(embedding), model, version, id.to_string()],
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
        self.get_all_semantic_facts_as_of(project, status, None, None, None, limit)
    }

    pub fn get_all_semantic_facts_as_of(
        &self,
        project: Option<&str>,
        status: Option<FactStatus>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
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

        if let Some(t) = as_of_time {
            let t_str = t.to_rfc3339();
            sql.push_str(" AND COALESCE(valid_from, created_at) <= ? AND (valid_until IS NULL OR valid_until > ?)");
            params_vec.push(Box::new(t_str.clone()));
            params_vec.push(Box::new(t_str));
        }

        if let Some(c) = as_of_commit {
            sql.push_str(" AND (git_commit_hash = ? OR json_extract(code_anchor_json, '$.git_commit_hash') = ?)");
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master')");
            } else if b != "*" && b != "all" {
                sql.push_str(" AND (git_branch IS NULL OR git_branch = 'main' OR git_branch = 'master' OR git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
            }
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

    pub fn get_semantic_facts_with_embeddings_and_models(
        &self,
        project: Option<&str>,
        status: Option<FactStatus>,
    ) -> Result<Vec<(SemanticFact, Option<Vec<f32>>, Option<String>)>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = "SELECT id, project, scope, statement, category, evidence_json,
                              importance, confidence, tier, approved_by_human, created_at, last_updated_at, status,
                              version, replaced_by, tags_json, code_anchor_json, depends_on_json, embedding, embedding_model
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
                let model: Option<String> = row.get(19).ok();
                Ok((fact, emb, model))
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
        self.search_semantic_facts_as_of(query_text, None, None, None, limit)
    }

    /// Search semantic facts using FTS5 Porter stemmer with bi-temporal and branch filters (as_of_time, as_of_commit, as_of_branch).
    pub fn search_semantic_facts_as_of(
        &self,
        query_text: &str,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
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

        let mut sql = "
            SELECT m.id, m.project, m.scope, m.statement, m.category, m.evidence_json,
                   m.importance, m.confidence, m.tier, m.approved_by_human, m.created_at, m.last_updated_at, m.status,
                   m.version, m.replaced_by, m.tags_json, m.code_anchor_json, m.depends_on_json,
                   bm25(semantic_facts_fts) as rank
            FROM semantic_facts_fts f
            JOIN semantic_facts m ON m.id = f.id
            WHERE semantic_facts_fts MATCH ?1
        ".to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(sanitized));

        if let Some(t) = as_of_time {
            let t_str = t.to_rfc3339();
            sql.push_str(" AND COALESCE(m.valid_from, m.created_at) <= ? AND (m.valid_until IS NULL OR m.valid_until > ?)");
            params_vec.push(Box::new(t_str.clone()));
            params_vec.push(Box::new(t_str));
        }

        if let Some(c) = as_of_commit {
            sql.push_str(" AND (m.git_commit_hash = ? OR json_extract(m.code_anchor_json, '$.git_commit_hash') = ?)");
            params_vec.push(Box::new(c.to_string()));
            params_vec.push(Box::new(c.to_string()));
        }

        if let Some(b) = as_of_branch {
            if b == "main" || b == "master" {
                sql.push_str(" AND (m.git_branch IS NULL OR m.git_branch = 'main' OR m.git_branch = 'master')");
            } else if b != "*" && b != "all" {
                sql.push_str(" AND (m.git_branch IS NULL OR m.git_branch = 'main' OR m.git_branch = 'master' OR m.git_branch = ?)");
                params_vec.push(Box::new(b.to_string()));
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

    /// Retrieves all recorded semantic fact dependencies across the workspace.
    pub fn get_all_fact_dependencies(&self) -> Result<Vec<(Uuid, Uuid, String)>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT dependent_fact_id, prerequisite_fact_id, dependency_type FROM fact_dependencies",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let dep_str: String = row.get(0)?;
                let prereq_str: String = row.get(1)?;
                let dep_type: String = row.get(2)?;
                let dep_id =
                    Uuid::parse_str(&dep_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let prereq_id =
                    Uuid::parse_str(&prereq_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok((dep_id, prereq_id, dep_type))
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
        self.update_procedural_skill_embedding_with_model(id, embedding, None, None)
    }

    pub fn update_procedural_skill_embedding_with_model(
        &self,
        id: &Uuid,
        embedding: &[f32],
        model: Option<&str>,
        version: Option<&str>,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let blob = embedding_to_bytes(embedding);
        conn.execute(
            "UPDATE procedural_skills SET embedding = ?1, embedding_model = ?2, embedding_version = ?3 WHERE id = ?4",
            params![blob, model, version, id.to_string()],
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
            embedding_model: row.get::<_, Option<String>>(17).ok().flatten(),
            embedding_version: row.get::<_, Option<String>>(18).ok().flatten(),
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
        let git_branch: Option<String> = row.get(16).ok().flatten();

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
            git_branch,
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

        drop(conn);

        // 7. Modulate incident synaptic edges (Tripartite Synapse outcome plasticity)
        let is_positive = feedback.rating.as_str() == "positive";
        let _ = self.modulate_incident_edges_for_memory(
            &feedback.memory_id,
            is_positive,
            effective_score as f32,
        );

        Ok(())
    }

    /// Records feedback on a directed associative edge (source_id -> target_id) with soft-saturation LTP or exponential LTD.
    pub fn record_edge_feedback(
        &self,
        source_id: &str,
        target_id: &str,
        edge_kind: &str,
        positive: bool,
        score: f32,
    ) -> Result<f32, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let now_str = Utc::now().to_rfc3339();

        let existing: Option<(f32, u32, u32)> = conn
            .query_row(
                "SELECT weight, reinforcements, depressions FROM memory_edges WHERE source_id = ?1 AND target_id = ?2 AND edge_kind = ?3",
                params![source_id, target_id, edge_kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let (current_w, mut r_cnt, mut d_cnt) = match existing {
            Some((w, r, d)) => (w, r, d),
            None => {
                let default_w = crate::spreading_activation::EdgeKind::from_str_kind(edge_kind)
                    .map(|k| k.default_weight())
                    .unwrap_or(0.70);
                (default_w, 0, 0)
            }
        };

        let effective_score = if score <= 0.0 { 1.0 } else { score.min(1.0) };
        let new_w = if positive {
            r_cnt += 1;
            (current_w + 0.15 * effective_score * (1.0 - current_w)).clamp(0.05, 1.0)
        } else {
            d_cnt += 1;
            (current_w - 0.20 * effective_score * current_w).clamp(0.05, 1.0)
        };

        conn.execute(
            "INSERT INTO memory_edges (source_id, target_id, edge_kind, weight, reinforcements, depressions, last_feedback_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(source_id, target_id, edge_kind) DO UPDATE SET
                 weight = excluded.weight,
                 reinforcements = excluded.reinforcements,
                 depressions = excluded.depressions,
                 last_feedback_at = excluded.last_feedback_at",
            params![source_id, target_id, edge_kind, new_w, r_cnt, d_cnt, now_str],
        )
        .map_err(|e| StrataError::Database(format!("Failed to record edge feedback: {e}")))?;

        Ok(new_w)
    }

    /// Persists a batch of modulated Knowledge Graph memory edges idempotently into SQLite.
    pub fn save_memory_edges(&self, edges: &[MemoryEdgeRecord]) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        for edge in edges {
            let now_str = edge.last_feedback_at.to_rfc3339();
            conn.execute(
                "INSERT INTO memory_edges (source_id, target_id, edge_kind, weight, reinforcements, depressions, last_feedback_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(source_id, target_id, edge_kind) DO UPDATE SET
                     weight = excluded.weight,
                     last_feedback_at = excluded.last_feedback_at",
                params![
                    edge.source_id,
                    edge.target_id,
                    edge.edge_kind,
                    edge.weight,
                    edge.reinforcements as i64,
                    edge.depressions as i64,
                    now_str,
                ],
            )
            .map_err(|e| StrataError::Database(format!("Failed to persist memory edge: {e}")))?;
        }

        Ok(())
    }

    /// Returns the total count of memory edges stored in SQLite.
    pub fn get_memory_edges_count(&self) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_edges", [], |r| r.get(0))
            .unwrap_or(0);
        Ok(count as usize)
    }

    /// Clears all memory edges in SQLite.
    pub fn clear_memory_edges(&self) -> Result<usize, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
        let deleted = conn
            .execute("DELETE FROM memory_edges", [])
            .map_err(|e| StrataError::Database(e.to_string()))?;
        Ok(deleted)
    }

    /// Retrieves all modulated memory edges stored in SQLite.
    pub fn get_all_memory_edges(&self) -> Result<Vec<MemoryEdgeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT source_id, target_id, edge_kind, weight, reinforcements, depressions, last_feedback_at
                 FROM memory_edges",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let last_feedback_str: String = row.get(6)?;
                let last_feedback_at = DateTime::parse_from_rfc3339(&last_feedback_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(MemoryEdgeRecord {
                    source_id: row.get(0)?,
                    target_id: row.get(1)?,
                    edge_kind: row.get(2)?,
                    weight: row.get(3)?,
                    reinforcements: row.get(4)?,
                    depressions: row.get(5)?,
                    last_feedback_at,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Modulates incident edges (incoming from tags, symbols, files) for a memory upon feedback.
    pub fn modulate_incident_edges_for_memory(
        &self,
        memory_id: &Uuid,
        positive: bool,
        score: f32,
    ) -> Result<usize, StrataError> {
        let mem = match self.get_memory(memory_id)? {
            Some(m) => m,
            None => return Ok(0),
        };
        let target_key = crate::spreading_activation::GraphNode::memory(*memory_id).canonical_id();
        let mut count = 0;

        // Modulate tags / concepts
        for tag in &mem.tags {
            let concept_key = crate::spreading_activation::GraphNode::concept(tag).canonical_id();
            let _ = self.record_edge_feedback(
                &concept_key,
                &target_key,
                crate::spreading_activation::EdgeKind::SharedTag.as_str(),
                positive,
                score,
            )?;
            let _ = self.record_edge_feedback(
                &target_key,
                &concept_key,
                crate::spreading_activation::EdgeKind::SharedTag.as_str(),
                positive,
                score,
            )?;
            count += 2;
        }

        // Modulate metadata files & symbols
        if let Some(obj) = mem.metadata.as_object() {
            if let Some(fps) = obj.get("file_paths").and_then(|v| v.as_array()) {
                for fp in fps.iter().filter_map(|v| v.as_str()) {
                    let file_key = crate::spreading_activation::GraphNode::file(fp).canonical_id();
                    let _ = self.record_edge_feedback(
                        &file_key,
                        &target_key,
                        crate::spreading_activation::EdgeKind::MentionsFile.as_str(),
                        positive,
                        score,
                    )?;
                    let _ = self.record_edge_feedback(
                        &target_key,
                        &file_key,
                        crate::spreading_activation::EdgeKind::MentionsFile.as_str(),
                        positive,
                        score,
                    )?;
                    count += 2;
                }
            }

            if let Some(syms) = obj.get("symbols").and_then(|v| v.as_array()) {
                for sym in syms.iter().filter_map(|v| v.as_str()) {
                    let sym_key = crate::spreading_activation::GraphNode::symbol(sym).canonical_id();
                    let _ = self.record_edge_feedback(
                        &sym_key,
                        &target_key,
                        crate::spreading_activation::EdgeKind::ReferencesSymbol.as_str(),
                        positive,
                        score,
                    )?;
                    let _ = self.record_edge_feedback(
                        &target_key,
                        &sym_key,
                        crate::spreading_activation::EdgeKind::ReferencesSymbol.as_str(),
                        positive,
                        score,
                    )?;
                    count += 2;
                }
            }
        }

        Ok(count)
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

    /// Persists call edges idempotently: clears existing call edges for the files in the batch,
    /// then inserts the updated edges.
    pub fn save_call_edges(&self, edges: &[CallEdge]) -> Result<(), StrataError> {
        let mut files = std::collections::HashSet::new();
        for edge in edges {
            files.insert(edge.caller_file.clone());
        }
        for file in files {
            self.clear_call_edges_for_file(&file)?;
        }
        self.insert_call_edges(edges)?;
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

    /// Retrieves all call and import edges recorded for a given file (alias for get_file_call_edges).
    pub fn get_call_edges_for_file(&self, file_path: &str) -> Result<Vec<CallEdge>, StrataError> {
        self.get_file_call_edges(file_path)
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

    /// Retrieves all recorded call edges up to the specified limit.
    pub fn get_all_call_edges(&self, limit: usize) -> Result<Vec<CallEdge>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, caller_file, caller_symbol, callee_symbol,
                        callee_file_hint, line_number, call_type, created_at
                 FROM call_edges
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit as i64], |row| Self::row_to_call_edge(row))
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }
        Ok(results)
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
        let canonical_id = strata_core::paths::canonical_resource_id(resource_id);
        let resource_id = canonical_id.as_str();

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
            let domain_key = canonical_id.strip_prefix("file:").unwrap_or(&canonical_id);
            let _ = conn.execute(
                "INSERT INTO domain_authorities (
                    domain, authoritative_agent, confidence_score, lease_held, active_tasks_count, last_active_at
                 ) VALUES (?1, ?2, 0.95, 1, 1, ?3)
                 ON CONFLICT(domain) DO UPDATE SET
                     authoritative_agent = excluded.authoritative_agent,
                     lease_held = 1,
                     last_active_at = excluded.last_active_at",
                params![domain_key, agent_id, now],
            );

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
        let canonical_id = strata_core::paths::canonical_resource_id(resource_id);

        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let rows = conn
            .execute(
                "DELETE FROM resource_leases WHERE (resource_id = ?1 OR resource_id = ?2) AND agent_id = ?3",
                params![canonical_id, resource_id, agent_id],
            )
            .map_err(|e| StrataError::Database(format!("Failed to release lease: {e}")))?;

        if rows > 0 {
            let domain_key = canonical_id.strip_prefix("file:").unwrap_or(&canonical_id);
            let _ = conn.execute(
                "UPDATE domain_authorities SET lease_held = 0 WHERE (domain = ?1 OR domain = ?2 OR domain = ?3) AND authoritative_agent = ?4",
                params![domain_key, canonical_id, resource_id, agent_id],
            );
        }

        Ok(rows > 0)
    }

    /// Fetches a lease for a resource.
    pub fn get_lease(&self, resource_id: &str) -> Result<Option<ResourceLease>, StrataError> {
        let canonical_id = strata_core::paths::canonical_resource_id(resource_id);

        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT resource_id, agent_id, lease_expires_at, metadata
             FROM resource_leases WHERE resource_id = ?1 OR resource_id = ?2
             ORDER BY CASE WHEN resource_id = ?1 THEN 0 ELSE 1 END
             LIMIT 1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let lease = stmt
            .query_row(params![canonical_id, resource_id], |row| {
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

    // =========================================================================
    // Multi-Agent Blackboard & Task Coordination
    // =========================================================================

    /// Inserts a new task into the blackboard.
    pub fn create_task(&self, task: &TaskRecord) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let req_res = serde_json::to_string(&task.required_resources).unwrap_or_else(|_| "[]".to_string());
        let deps = serde_json::to_string(&task.dependencies).unwrap_or_else(|_| "[]".to_string());
        let outputs = task.outputs.as_ref().map(|o| serde_json::to_string(o).unwrap_or_default());
        let payload_json = task.payload.as_ref().map(|p| serde_json::to_string(p).unwrap_or_default());

        conn.execute(
            "INSERT INTO tasks (
                task_id, plan_id, parent_id, title, description, status,
                assigned_agent, required_resources_json, dependencies_json,
                outputs_json, created_at, updated_at, claim_generation, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(task_id) DO UPDATE SET
                plan_id = excluded.plan_id,
                parent_id = excluded.parent_id,
                title = excluded.title,
                description = excluded.description,
                status = excluded.status,
                assigned_agent = excluded.assigned_agent,
                required_resources_json = excluded.required_resources_json,
                dependencies_json = excluded.dependencies_json,
                outputs_json = excluded.outputs_json,
                updated_at = excluded.updated_at,
                claim_generation = excluded.claim_generation,
                payload_json = excluded.payload_json",
            params![
                task.task_id,
                task.plan_id,
                task.parent_id,
                task.title,
                task.description,
                task.status.as_str(),
                task.assigned_agent,
                req_res,
                deps,
                outputs,
                task.created_at,
                task.updated_at,
                task.claim_generation,
                payload_json,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to create task: {e}")))?;

        Ok(())
    }

    /// Inserts a batch of tasks into the blackboard in a single transaction.
    pub fn create_tasks_batch(&self, tasks: &[TaskRecord]) -> Result<(), StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to start transaction: {e}")))?;

        for task in tasks {
            let req_res = serde_json::to_string(&task.required_resources).unwrap_or_else(|_| "[]".to_string());
            let deps = serde_json::to_string(&task.dependencies).unwrap_or_else(|_| "[]".to_string());
            let outputs = task.outputs.as_ref().map(|o| serde_json::to_string(o).unwrap_or_default());
            let payload_json = task.payload.as_ref().map(|p| serde_json::to_string(p).unwrap_or_default());

            tx.execute(
                "INSERT INTO tasks (
                    task_id, plan_id, parent_id, title, description, status,
                    assigned_agent, required_resources_json, dependencies_json,
                    outputs_json, created_at, updated_at, claim_generation, payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ON CONFLICT(task_id) DO UPDATE SET
                    plan_id = excluded.plan_id,
                    parent_id = excluded.parent_id,
                    title = excluded.title,
                    description = excluded.description,
                    status = excluded.status,
                    assigned_agent = excluded.assigned_agent,
                    required_resources_json = excluded.required_resources_json,
                    dependencies_json = excluded.dependencies_json,
                    outputs_json = excluded.outputs_json,
                    updated_at = excluded.updated_at,
                    claim_generation = excluded.claim_generation,
                    payload_json = excluded.payload_json",
                params![
                    task.task_id,
                    task.plan_id,
                    task.parent_id,
                    task.title,
                    task.description,
                    task.status.as_str(),
                    task.assigned_agent,
                    req_res,
                    deps,
                    outputs,
                    task.created_at,
                    task.updated_at,
                    task.claim_generation,
                    payload_json,
                ],
            )
            .map_err(|e| StrataError::Database(format!("Failed to insert batch task: {e}")))?;
        }

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit batch tasks: {e}")))?;

        Ok(())
    }

    /// Fetches a task by its unique ID.
    pub fn get_task(&self, task_id: &str) -> Result<Option<TaskRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT task_id, plan_id, parent_id, title, description, status,
                        assigned_agent, required_resources_json, dependencies_json,
                        outputs_json, created_at, updated_at, claim_generation, payload_json
                 FROM tasks WHERE task_id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let task = stmt
            .query_row(params![task_id], map_task_row)
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(task)
    }

    /// Lists tasks matching an optional filter.
    pub fn list_tasks(&self, filter: &TaskFilter) -> Result<Vec<TaskRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut sql = String::from(
            "SELECT task_id, plan_id, parent_id, title, description, status,
                    assigned_agent, required_resources_json, dependencies_json,
                    outputs_json, created_at, updated_at, claim_generation, payload_json
             FROM tasks WHERE 1=1",
        );

        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(plan_id) = &filter.plan_id {
            sql.push_str(" AND plan_id = ?");
            param_values.push(Box::new(plan_id.clone()));
        }

        if let Some(status) = &filter.status {
            sql.push_str(" AND status = ?");
            param_values.push(Box::new(status.as_str().to_string()));
        }

        if let Some(agent) = &filter.assigned_agent {
            sql.push_str(" AND assigned_agent = ?");
            param_values.push(Box::new(agent.clone()));
        }

        sql.push_str(" ORDER BY created_at ASC");

        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut stmt = conn.prepare(&sql).map_err(|e| StrataError::Database(e.to_string()))?;

        let params_slice: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(&params_slice[..], map_task_row)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Atomically claims a task (or finds the next unblocked task) and acquires
    /// exclusive leases for all of its required resources.
    ///
    /// Uses an IMMEDIATE transaction to prevent concurrent race conditions across
    /// multiple processes (e.g. multiple agents or workers), ensures candidate
    /// dependencies and resource leases are verified atomically, bumps the task's
    /// fencing token (`claim_generation`), and commits the claim.
    pub fn claim_task_atomic(
        &self,
        task_id_opt: Option<&str>,
        agent_id: &str,
        lease_ttl_secs: i64,
    ) -> Result<Option<TaskRecord>, StrataError> {
        self.claim_task_atomic_typed(task_id_opt, agent_id, lease_ttl_secs, None)
    }

    /// Atomically claims a task (or finds the next unblocked task matching the optional
    /// supported payload types) and acquires exclusive leases for all of its required resources.
    pub fn claim_task_atomic_typed(
        &self,
        task_id_opt: Option<&str>,
        agent_id: &str,
        lease_ttl_secs: i64,
        supported_types: Option<&[&str]>,
    ) -> Result<Option<TaskRecord>, StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let lease_expires_at = now + lease_ttl_secs;

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StrataError::Database(format!("Failed to start transaction: {e}")))?;

        // Fetch completed task IDs in the workspace to evaluate dependencies
        let completed_ids: Vec<String> = {
            let mut completed_stmt = tx
                .prepare("SELECT task_id FROM tasks WHERE status = 'completed'")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let completed_rows = completed_stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| StrataError::Database(e.to_string()))?;
            completed_rows.filter_map(|r| r.ok()).collect()
        };

        // Query candidate tasks
        let candidate_tasks: Vec<TaskRecord> = match task_id_opt {
            Some(id) => {
                let mut stmt = tx
                    .prepare(
                        "SELECT task_id, plan_id, parent_id, title, description, status,
                                assigned_agent, required_resources_json, dependencies_json,
                                outputs_json, created_at, updated_at, claim_generation, payload_json
                         FROM tasks WHERE task_id = ?1",
                    )
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                let row_opt = stmt
                    .query_row(params![id], map_task_row)
                    .optional()
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                match row_opt {
                    Some(t) if t.status != TaskStatus::Completed => {
                        // Prevent peer agents from stealing an active task; owner renewal is allowed
                        if (t.status == TaskStatus::Claimed || t.status == TaskStatus::Running)
                            && t.assigned_agent.as_deref() != Some(agent_id)
                        {
                            vec![]
                        } else {
                            vec![t]
                        }
                    }
                    _ => vec![],
                }
            }
            None => {
                let mut stmt = tx
                    .prepare(
                        "SELECT task_id, plan_id, parent_id, title, description, status,
                                assigned_agent, required_resources_json, dependencies_json,
                                outputs_json, created_at, updated_at, claim_generation, payload_json
                         FROM tasks
                         WHERE status IN ('pending', 'blocked')
                         ORDER BY created_at ASC",
                    )
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                let rows = stmt
                    .query_map([], map_task_row)
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };

        for candidate in candidate_tasks {
            // Check 0: Does candidate match supported task types?
            if let Some(types) = supported_types {
                let ptype = candidate.payload_type();
                if !types.iter().any(|&t| t.eq_ignore_ascii_case(ptype)) {
                    continue;
                }
            }

            // Check 1: Are all prerequisite dependencies completed?
            if !candidate.dependencies.iter().all(|dep| completed_ids.contains(dep)) {
                continue;
            }

            // Check 2: Are all required resources unleased or owned by us?
            let mut resources_available = true;
            for res in &candidate.required_resources {
                let canon = strata_core::paths::canonical_resource_id(res);
                let mut lease_stmt = tx
                    .prepare("SELECT agent_id, lease_expires_at FROM resource_leases WHERE resource_id = ?1 OR resource_id = ?2")
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                let active_lease = lease_stmt
                    .query_row(params![canon, res], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })
                    .optional()
                    .map_err(|e| StrataError::Database(e.to_string()))?;

                if let Some((held_by, exp)) = active_lease {
                    if exp > now && held_by != agent_id {
                        resources_available = false;
                        break;
                    }
                }
            }

            if !resources_available {
                continue;
            }

            // Lock candidate resources
            for res in &candidate.required_resources {
                let canon = strata_core::paths::canonical_resource_id(res);
                tx.execute(
                    "INSERT INTO resource_leases (resource_id, agent_id, lease_expires_at, metadata)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(resource_id) DO UPDATE SET
                         agent_id = excluded.agent_id,
                         lease_expires_at = excluded.lease_expires_at,
                         metadata = excluded.metadata",
                    params![canon, agent_id, lease_expires_at, format!("task:{}", candidate.task_id)],
                )
                .map_err(|e| StrataError::Database(format!("Failed to acquire resource lease {res}: {e}")))?;
            }

            // Atomically update task to claimed and increment claim_generation
            let new_generation = candidate.claim_generation + 1;
            let rows_updated = tx.execute(
                "UPDATE tasks
                 SET status = 'claimed',
                     assigned_agent = ?1,
                     updated_at = ?2,
                     claim_generation = ?3
                 WHERE task_id = ?4
                   AND (
                       status IN ('pending', 'blocked')
                       OR (status IN ('claimed', 'running') AND assigned_agent = ?1)
                   )
                   AND claim_generation = ?5",
                params![agent_id, now, new_generation, candidate.task_id, candidate.claim_generation],
            )
            .map_err(|e| StrataError::Database(format!("Failed to update task to claimed: {e}")))?;

            if rows_updated == 0 {
                continue;
            }

            tx.commit()
                .map_err(|e| StrataError::Database(format!("Failed to commit claim transaction: {e}")))?;

            let mut claimed_task = candidate;
            claimed_task.status = TaskStatus::Claimed;
            claimed_task.assigned_agent = Some(agent_id.to_string());
            claimed_task.updated_at = now;
            claimed_task.claim_generation = new_generation;

            return Ok(Some(claimed_task));
        }

        Ok(None)
    }

    /// Marks a task as completed, registers optional summary/artifact, releases leases,
    /// and unblocks downstream tasks whose dependencies are now fully satisfied.
    pub fn complete_task(
        &self,
        task_id: &str,
        agent_id: &str,
        summary: Option<&str>,
        artifact: Option<&TaskArtifact>,
    ) -> Result<bool, StrataError> {
        self.complete_task_with_generation(task_id, agent_id, summary, artifact, None)
    }

    /// Marks a task as completed with optional fencing token (`claim_generation`) verification.
    ///
    /// Rejects the completion if:
    /// - The task does not exist
    /// - The task is not currently in 'claimed' or 'running' status
    /// - The assigned agent does not match `agent_id`
    /// - An `expected_generation` is provided and does not match the stored `claim_generation`
    pub fn complete_task_with_generation(
        &self,
        task_id: &str,
        agent_id: &str,
        summary: Option<&str>,
        artifact: Option<&TaskArtifact>,
        expected_generation: Option<i64>,
    ) -> Result<bool, StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StrataError::Database(format!("Failed to start transaction: {e}")))?;

        // Fetch status, assigned_agent, required_resources, claim_generation
        let task_meta: Option<(String, Option<String>, String, i64)> = {
            let mut stmt = tx
                .prepare("SELECT status, assigned_agent, required_resources_json, claim_generation FROM tasks WHERE task_id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            stmt
                .query_row(params![task_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .optional()
                .map_err(|e| StrataError::Database(e.to_string()))?
        };

        let (status_str, assigned_agent, res_json, current_gen) = match task_meta {
            Some(m) => m,
            None => return Ok(false),
        };

        // Enforce active status
        if status_str != "claimed" && status_str != "running" {
            return Ok(false);
        }

        // Enforce assigned agent ownership
        if assigned_agent.as_deref() != Some(agent_id) {
            return Ok(false);
        }

        // Enforce fencing token if provided
        if let Some(expected) = expected_generation {
            if current_gen != expected {
                return Ok(false);
            }
        }

        let outputs_val = summary.map(|s| serde_json::json!({ "summary": s, "completed_by": agent_id }));
        let outputs_str = outputs_val.map(|v| serde_json::to_string(&v).unwrap_or_default());

        let rows = tx.execute(
            "UPDATE tasks
             SET status = 'completed',
                 outputs_json = COALESCE(?1, outputs_json),
                 updated_at = ?2
             WHERE task_id = ?3
               AND assigned_agent = ?4
               AND status IN ('claimed', 'running')",
            params![outputs_str, now, task_id, agent_id],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update task to completed: {e}")))?;

        if rows == 0 {
            return Ok(false);
        }

        // If artifact provided, insert into task_artifacts
        if let Some(art) = artifact {
            tx.execute(
                "INSERT INTO task_artifacts (artifact_id, task_id, agent_id, kind, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(artifact_id) DO UPDATE SET payload = excluded.payload",
                params![art.artifact_id, art.task_id, art.agent_id, art.kind, art.payload, art.created_at],
            )
            .map_err(|e| StrataError::Database(format!("Failed to record task artifact: {e}")))?;
        }

        // Release required_resources leases
        let resources: Vec<String> = serde_json::from_str(&res_json).unwrap_or_default();
        for res in resources {
            let canon = strata_core::paths::canonical_resource_id(&res);
            let _ = tx.execute(
                "DELETE FROM resource_leases WHERE (resource_id = ?1 OR resource_id = ?2) AND agent_id = ?3",
                params![canon, res, agent_id],
            );
        }
        let _ = tx.execute(
            "DELETE FROM resource_leases WHERE agent_id = ?1 AND metadata LIKE ?2",
            params![agent_id, format!("task:{task_id}%")],
        );

        // Auto-unblock downstream blocked tasks
        let completed_ids: Vec<String> = {
            let mut completed_stmt = tx
                .prepare("SELECT task_id FROM tasks WHERE status = 'completed'")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let completed_rows = completed_stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| StrataError::Database(e.to_string()))?;
            completed_rows.filter_map(|r| r.ok()).collect()
        };

        let blocked_tasks: Vec<(String, String)> = {
            let mut blocked_stmt = tx
                .prepare("SELECT task_id, dependencies_json FROM tasks WHERE status = 'blocked'")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows = blocked_stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| StrataError::Database(e.to_string()))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        for (b_id, b_deps_str) in blocked_tasks {
            let deps: Vec<String> = serde_json::from_str(&b_deps_str).unwrap_or_default();
            if deps.iter().all(|d| completed_ids.contains(d)) {
                let _ = tx.execute(
                    "UPDATE tasks SET status = 'pending', updated_at = ?1 WHERE task_id = ?2",
                    params![now, b_id],
                );
            }
        }

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit completion transaction: {e}")))?;

        Ok(true)
    }

    /// Marks a task as failed and releases its resource leases.
    pub fn fail_task(
        &self,
        task_id: &str,
        agent_id: &str,
        error_message: &str,
    ) -> Result<bool, StrataError> {
        self.fail_task_with_generation(task_id, agent_id, error_message, None)
    }

    /// Marks a task as failed with optional fencing token (`claim_generation`) verification.
    ///
    /// Rejects the failure transition if:
    /// - The task does not exist
    /// - The task is not currently in 'claimed' or 'running' status
    /// - The assigned agent does not match `agent_id`
    /// - An `expected_generation` is provided and does not match the stored `claim_generation`
    pub fn fail_task_with_generation(
        &self,
        task_id: &str,
        agent_id: &str,
        error_message: &str,
        expected_generation: Option<i64>,
    ) -> Result<bool, StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StrataError::Database(format!("Failed to start transaction: {e}")))?;

        let task_meta: Option<(String, Option<String>, String, i64)> = {
            let mut stmt = tx
                .prepare("SELECT status, assigned_agent, required_resources_json, claim_generation FROM tasks WHERE task_id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            stmt
                .query_row(params![task_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .optional()
                .map_err(|e| StrataError::Database(e.to_string()))?
        };

        let (status_str, assigned_agent, res_json, current_gen) = match task_meta {
            Some(m) => m,
            None => return Ok(false),
        };

        if status_str != "claimed" && status_str != "running" {
            return Ok(false);
        }

        if assigned_agent.as_deref() != Some(agent_id) {
            return Ok(false);
        }

        if let Some(expected) = expected_generation {
            if current_gen != expected {
                return Ok(false);
            }
        }

        let err_json = serde_json::to_string(&serde_json::json!({
            "error": error_message,
            "failed_by": agent_id
        }))
        .unwrap_or_default();

        let rows = tx
            .execute(
                "UPDATE tasks
                 SET status = 'failed',
                     outputs_json = ?1,
                     updated_at = ?2
                 WHERE task_id = ?3
                   AND assigned_agent = ?4
                   AND status IN ('claimed', 'running')",
                params![err_json, now, task_id, agent_id],
            )
            .map_err(|e| StrataError::Database(format!("Failed to mark task as failed: {e}")))?;

        if rows == 0 {
            return Ok(false);
        }

        // Release required_resources leases and task metadata leases
        let resources: Vec<String> = serde_json::from_str(&res_json).unwrap_or_default();
        for res in resources {
            let canon = strata_core::paths::canonical_resource_id(&res);
            let _ = tx.execute(
                "DELETE FROM resource_leases WHERE (resource_id = ?1 OR resource_id = ?2) AND agent_id = ?3",
                params![canon, res, agent_id],
            );
        }
        let _ = tx.execute(
            "DELETE FROM resource_leases WHERE agent_id = ?1 AND metadata LIKE ?2",
            params![agent_id, format!("task:{task_id}%")],
        );

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit fail_task transaction: {e}")))?;

        Ok(true)
    }

    /// Updates ongoing task progress or heartbeat.
    pub fn update_task_progress(
        &self,
        task_id: &str,
        agent_id: &str,
        message: &str,
    ) -> Result<bool, StrataError> {
        self.update_task_progress_with_generation(task_id, agent_id, message, None)
    }

    /// Updates ongoing task progress or heartbeat with optional fencing token verification.
    pub fn update_task_progress_with_generation(
        &self,
        task_id: &str,
        agent_id: &str,
        message: &str,
        expected_generation: Option<i64>,
    ) -> Result<bool, StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StrataError::Database(format!("Failed to start transaction: {e}")))?;

        let task_meta: Option<(String, Option<String>, i64)> = {
            let mut stmt = tx
                .prepare("SELECT status, assigned_agent, claim_generation FROM tasks WHERE task_id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            stmt
                .query_row(params![task_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .optional()
                .map_err(|e| StrataError::Database(e.to_string()))?
        };

        let (status_str, assigned_agent, current_gen) = match task_meta {
            Some(m) => m,
            None => return Ok(false),
        };

        if status_str != "claimed" && status_str != "running" {
            return Ok(false);
        }

        if assigned_agent.as_deref() != Some(agent_id) {
            return Ok(false);
        }

        if let Some(expected) = expected_generation {
            if current_gen != expected {
                return Ok(false);
            }
        }

        let progress_json = serde_json::to_string(&serde_json::json!({
            "last_progress": message,
            "agent_id": agent_id,
            "updated_at": now
        }))
        .unwrap_or_default();

        let rows = tx
            .execute(
                "UPDATE tasks
                 SET status = 'running',
                     outputs_json = ?1,
                     updated_at = ?2
                 WHERE task_id = ?3
                   AND assigned_agent = ?4
                   AND status IN ('claimed', 'running')",
                params![progress_json, now, task_id, agent_id],
            )
            .map_err(|e| StrataError::Database(format!("Failed to update task progress: {e}")))?;

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit update_task_progress transaction: {e}")))?;

        Ok(rows > 0)
    }

    /// Atomically renews all leases associated with an active task.
    ///
    /// Validates that:
    /// - The task exists and is in 'claimed' or 'running' status
    /// - The task is assigned to `agent_id`
    /// - If `expected_generation` is provided, `claim_generation` matches
    /// - All required resources are currently held by `agent_id`
    ///
    /// If valid, extends the expiration of all required resources and any leases
    /// held with metadata `task:{task_id}%` to `now + lease_ttl_secs`, and touches `tasks.updated_at`.
    ///
    /// Returns `Ok(true)` if successfully renewed, or `Ok(false)` if the lease or task was revoked/lost.
    pub fn renew_task_leases(
        &self,
        task_id: &str,
        agent_id: &str,
        lease_ttl_secs: i64,
    ) -> Result<bool, StrataError> {
        self.renew_task_leases_with_generation(task_id, agent_id, lease_ttl_secs, None)
    }

    pub fn renew_task_leases_with_generation(
        &self,
        task_id: &str,
        agent_id: &str,
        lease_ttl_secs: i64,
        expected_generation: Option<i64>,
    ) -> Result<bool, StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let new_expires_at = now + lease_ttl_secs;

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StrataError::Database(format!("Failed to start transaction: {e}")))?;

        // Verify task existence and ownership
        let task_meta: Option<(String, Option<String>, String, i64)> = {
            let mut stmt = tx
                .prepare("SELECT status, assigned_agent, required_resources_json, claim_generation FROM tasks WHERE task_id = ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            stmt
                .query_row(params![task_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .optional()
                .map_err(|e| StrataError::Database(e.to_string()))?
        };

        let (status_str, assigned_agent, res_json, current_gen) = match task_meta {
            Some(m) => m,
            None => return Ok(false),
        };

        if status_str != "claimed" && status_str != "running" {
            return Ok(false);
        }

        if assigned_agent.as_deref() != Some(agent_id) {
            return Ok(false);
        }

        if let Some(expected) = expected_generation {
            if current_gen != expected {
                return Ok(false);
            }
        }

        let resources: Vec<String> = serde_json::from_str(&res_json).unwrap_or_default();
        for res in &resources {
            let canon = strata_core::paths::canonical_resource_id(res);
            let mut stmt = tx
                .prepare("SELECT agent_id, lease_expires_at FROM resource_leases WHERE resource_id = ?1 OR resource_id = ?2")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let lease_opt = stmt
                .query_row(params![canon, res], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .optional()
                .map_err(|e| StrataError::Database(e.to_string()))?;

            match lease_opt {
                Some((held_by, _exp)) => {
                    if held_by != agent_id {
                        return Ok(false);
                    }
                }
                None => {
                    return Ok(false);
                }
            }
        }

        // Renew required resource leases
        for res in &resources {
            let canon = strata_core::paths::canonical_resource_id(res);
            tx.execute(
                "UPDATE resource_leases
                 SET lease_expires_at = ?1
                 WHERE (resource_id = ?2 OR resource_id = ?3) AND agent_id = ?4",
                params![new_expires_at, canon, res, agent_id],
            )
            .map_err(|e| StrataError::Database(format!("Failed to renew lease for {res}: {e}")))?;
        }

        // Also renew any other leases held with metadata 'task:{task_id}%'
        tx.execute(
            "UPDATE resource_leases
             SET lease_expires_at = ?1
             WHERE agent_id = ?2 AND metadata LIKE ?3",
            params![new_expires_at, agent_id, format!("task:{}%", task_id)],
        )
        .map_err(|e| StrataError::Database(format!("Failed to renew metadata leases: {e}")))?;

        // Touch tasks.updated_at so task remains un-stale
        tx.execute(
            "UPDATE tasks SET updated_at = ?1 WHERE task_id = ?2 AND assigned_agent = ?3",
            params![now, task_id, agent_id],
        )
        .map_err(|e| StrataError::Database(format!("Failed to touch task updated_at: {e}")))?;

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit renew transaction: {e}")))?;

        Ok(true)
    }

    /// Returns `claimed`/`running` tasks silent for longer than `stale_secs` to
    /// `pending` (unassigned) and drops their task-owned leases. `task progress`
    /// refreshes `updated_at`, so it acts as the liveness heartbeat.
    /// Increments `claim_generation` so any in-flight requests from the stale worker are rejected.
    pub fn reclaim_stale_tasks(&self, stale_secs: i64) -> Result<Vec<String>, StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StrataError::Database(format!("Failed to start transaction: {e}")))?;

        let ids: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT task_id FROM tasks WHERE status IN ('claimed', 'running') AND updated_at <= ?1")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows = stmt
                .query_map(params![now - stale_secs], |row| row.get::<_, String>(0))
                .map_err(|e| StrataError::Database(e.to_string()))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| StrataError::Database(e.to_string()))?
        };

        for id in &ids {
            tx.execute(
                "UPDATE tasks SET status = 'pending', assigned_agent = NULL, updated_at = ?1, claim_generation = claim_generation + 1 WHERE task_id = ?2",
                params![now, id],
            )
            .map_err(|e| StrataError::Database(format!("Failed to reclaim task {id}: {e}")))?;
            tx.execute(
                "DELETE FROM resource_leases WHERE metadata = ?1",
                params![format!("task:{id}")],
            )
            .map_err(|e| StrataError::Database(format!("Failed to drop leases for task {id}: {e}")))?;
        }

        tx.commit()
            .map_err(|e| StrataError::Database(format!("Failed to commit reclaim transaction: {e}")))?;

        Ok(ids)
    }

    /// Stores a task artifact.
    pub fn save_task_artifact(&self, artifact: &TaskArtifact) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "INSERT INTO task_artifacts (artifact_id, task_id, agent_id, kind, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(artifact_id) DO UPDATE SET payload = excluded.payload",
            params![
                artifact.artifact_id,
                artifact.task_id,
                artifact.agent_id,
                artifact.kind,
                artifact.payload,
                artifact.created_at
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to save task artifact: {e}")))?;

        Ok(())
    }

    /// Fetches all artifacts associated with a task.
    pub fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT artifact_id, task_id, agent_id, kind, payload, created_at
                 FROM task_artifacts
                 WHERE task_id = ?1
                 ORDER BY created_at ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(TaskArtifact {
                    artifact_id: row.get(0)?,
                    task_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    kind: row.get(3)?,
                    payload: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    // =========================================================================
    // A2A Local Mailbox Messaging
    // =========================================================================

    /// Persists an A2A message into the mailbox.
    pub fn save_a2a_message(&self, msg: &A2aMessage) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "INSERT INTO a2a_messages (
                message_id, from_agent, to_agent, task_id, kind, content, read_at, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                msg.message_id,
                msg.from_agent,
                msg.to_agent,
                msg.task_id,
                msg.kind,
                msg.content,
                msg.read_at,
                msg.created_at
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to save A2A message: {e}")))?;

        Ok(())
    }

    /// Fetches inbox messages for an agent (including broadcasts).
    pub fn get_a2a_inbox(&self, agent_id: &str, unread_only: bool) -> Result<Vec<A2aMessage>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let sql = if unread_only {
            "SELECT message_id, from_agent, to_agent, task_id, kind, content, read_at, created_at
             FROM a2a_messages
             WHERE (to_agent = ?1 OR to_agent IS NULL) AND read_at IS NULL
             ORDER BY created_at ASC"
        } else {
            "SELECT message_id, from_agent, to_agent, task_id, kind, content, read_at, created_at
             FROM a2a_messages
             WHERE (to_agent = ?1 OR to_agent IS NULL)
             ORDER BY created_at ASC"
        };

        let mut stmt = conn.prepare(sql).map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok(A2aMessage {
                    message_id: row.get(0)?,
                    from_agent: row.get(1)?,
                    to_agent: row.get(2)?,
                    task_id: row.get(3)?,
                    kind: row.get(4)?,
                    content: row.get(5)?,
                    read_at: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Marks a list of messages as read by message IDs.
    pub fn mark_a2a_read(&self, message_ids: &[String]) -> Result<usize, StrataError> {
        if message_ids.is_empty() {
            return Ok(0);
        }

        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let now = Utc::now().timestamp();
        let mut count = 0;

        for id in message_ids {
            let rows = conn
                .execute(
                    "UPDATE a2a_messages SET read_at = ?1 WHERE message_id = ?2 AND read_at IS NULL",
                    params![now, id],
                )
                .map_err(|e| StrataError::Database(format!("Failed to mark message {id} as read: {e}")))?;
            count += rows;
        }

        Ok(count)
    }

    // ==========================================
    // Transactive Memory Directory (A2A Frontier 2)
    // ==========================================

    /// Upserts an agent's capability card and synchronizes its declared domain authorities.
    pub fn record_agent_card(&self, card: &AgentCard) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let caps_json = serde_json::to_string(&card.memory_capabilities)
            .map_err(|e| StrataError::Serialization(e.to_string()))?;
        let domains_json = serde_json::to_string(&card.domain_scopes)
            .map_err(|e| StrataError::Serialization(e.to_string()))?;
        let tenants_json = serde_json::to_string(&card.tenant_scopes)
            .map_err(|e| StrataError::Serialization(e.to_string()))?;

        conn.execute(
            "INSERT INTO agent_cards (
                agent_id, host, memory_capabilities_json, domain_scopes_json,
                memory_persistence, tenant_scopes_json, memory_privacy_mode,
                confidence_score, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(agent_id) DO UPDATE SET
                host = excluded.host,
                memory_capabilities_json = excluded.memory_capabilities_json,
                domain_scopes_json = excluded.domain_scopes_json,
                memory_persistence = excluded.memory_persistence,
                tenant_scopes_json = excluded.tenant_scopes_json,
                memory_privacy_mode = excluded.memory_privacy_mode,
                confidence_score = excluded.confidence_score,
                updated_at = excluded.updated_at",
            params![
                card.agent_id,
                card.host,
                caps_json,
                domains_json,
                card.memory_persistence,
                tenants_json,
                card.memory_privacy_mode,
                card.confidence_score,
                card.updated_at
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to record agent card: {e}")))?;

        // Auto-provision domain authority for each declared scope if not already held with higher confidence
        for scope in &card.domain_scopes {
            let auth = DomainAuthority::new(scope, &card.agent_id, card.confidence_score);
            let _ = self.set_domain_authority_internal(&conn, &auth);
        }

        Ok(())
    }

    /// Fetches an agent capability card by agent ID.
    pub fn get_agent_card(&self, agent_id: &str) -> Result<Option<AgentCard>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT agent_id, host, memory_capabilities_json, domain_scopes_json,
                        memory_persistence, tenant_scopes_json, memory_privacy_mode,
                        confidence_score, updated_at
                 FROM agent_cards
                 WHERE agent_id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![agent_id], |row| {
                let caps_str: String = row.get(2)?;
                let domains_str: String = row.get(3)?;
                let tenants_str: String = row.get(5)?;

                let memory_capabilities: Vec<String> = serde_json::from_str(&caps_str).unwrap_or_default();
                let domain_scopes: Vec<String> = serde_json::from_str(&domains_str).unwrap_or_default();
                let tenant_scopes: Vec<String> = serde_json::from_str(&tenants_str).unwrap_or_default();

                Ok(AgentCard {
                    agent_id: row.get(0)?,
                    host: row.get(1)?,
                    memory_capabilities,
                    domain_scopes,
                    memory_persistence: row.get(4)?,
                    tenant_scopes,
                    memory_privacy_mode: row.get(6)?,
                    confidence_score: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(res)
    }

    /// Lists all registered agent capability cards.
    pub fn list_agent_cards(&self) -> Result<Vec<AgentCard>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT agent_id, host, memory_capabilities_json, domain_scopes_json,
                        memory_persistence, tenant_scopes_json, memory_privacy_mode,
                        confidence_score, updated_at
                 FROM agent_cards
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let caps_str: String = row.get(2)?;
                let domains_str: String = row.get(3)?;
                let tenants_str: String = row.get(5)?;

                let memory_capabilities: Vec<String> = serde_json::from_str(&caps_str).unwrap_or_default();
                let domain_scopes: Vec<String> = serde_json::from_str(&domains_str).unwrap_or_default();
                let tenant_scopes: Vec<String> = serde_json::from_str(&tenants_str).unwrap_or_default();

                Ok(AgentCard {
                    agent_id: row.get(0)?,
                    host: row.get(1)?,
                    memory_capabilities,
                    domain_scopes,
                    memory_persistence: row.get(4)?,
                    tenant_scopes,
                    memory_privacy_mode: row.get(6)?,
                    confidence_score: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    fn set_domain_authority_internal(
        &self,
        conn: &Connection,
        authority: &DomainAuthority,
    ) -> Result<(), StrataError> {
        conn.execute(
            "INSERT INTO domain_authorities (
                domain, authoritative_agent, confidence_score, lease_held, active_tasks_count, last_active_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(domain) DO UPDATE SET
                 authoritative_agent = excluded.authoritative_agent,
                 confidence_score = excluded.confidence_score,
                 lease_held = excluded.lease_held,
                 active_tasks_count = excluded.active_tasks_count,
                 last_active_at = excluded.last_active_at",
            params![
                authority.domain,
                authority.authoritative_agent,
                authority.confidence_score,
                if authority.lease_held { 1 } else { 0 },
                authority.active_tasks_count,
                authority.last_active_at,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to set domain authority: {e}")))?;

        Ok(())
    }

    /// Registers or updates authority over a specific code domain or topic space.
    pub fn set_domain_authority(&self, authority: &DomainAuthority) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        self.set_domain_authority_internal(&conn, authority)
    }

    /// Fetches the explicit domain authority record for a domain.
    pub fn get_domain_authority(&self, domain: &str) -> Result<Option<DomainAuthority>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT domain, authoritative_agent, confidence_score, lease_held, active_tasks_count, last_active_at
                 FROM domain_authorities
                 WHERE domain = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let res = stmt
            .query_row(params![domain], |row| {
                Ok(DomainAuthority {
                    domain: row.get(0)?,
                    authoritative_agent: row.get(1)?,
                    confidence_score: row.get(2)?,
                    lease_held: row.get::<_, i64>(3)? != 0,
                    active_tasks_count: row.get(4)?,
                    last_active_at: row.get(5)?,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(res)
    }

    /// Lists all domain authorities currently indexed.
    pub fn list_domain_authorities(&self) -> Result<Vec<DomainAuthority>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT domain, authoritative_agent, confidence_score, lease_held, active_tasks_count, last_active_at
                 FROM domain_authorities
                 ORDER BY domain ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(DomainAuthority {
                    domain: row.get(0)?,
                    authoritative_agent: row.get(1)?,
                    confidence_score: row.get(2)?,
                    lease_held: row.get::<_, i64>(3)? != 0,
                    active_tasks_count: row.get(4)?,
                    last_active_at: row.get(5)?,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Finds the best authoritative specialist agent for a domain using exact and hierarchical prefix resolution.
    pub fn find_best_authority_for_domain(
        &self,
        query_domain: &str,
    ) -> Result<Option<DomainAuthority>, StrataError> {
        let clean_query = query_domain.strip_prefix("file:").unwrap_or(query_domain);
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        // 1. Exact match
        let mut stmt = conn
            .prepare(
                "SELECT domain, authoritative_agent, confidence_score, lease_held, active_tasks_count, last_active_at
                 FROM domain_authorities
                 WHERE domain = ?1 OR domain = ?2
                 ORDER BY lease_held DESC, length(domain) DESC
                 LIMIT 1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let exact = stmt
            .query_row(params![clean_query, query_domain], |row| {
                Ok(DomainAuthority {
                    domain: row.get(0)?,
                    authoritative_agent: row.get(1)?,
                    confidence_score: row.get(2)?,
                    lease_held: row.get::<_, i64>(3)? != 0,
                    active_tasks_count: row.get(4)?,
                    last_active_at: row.get(5)?,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if exact.is_some() {
            return Ok(exact);
        }

        // 2. Hierarchical prefix matching: matches parent module or directory
        let mut stmt = conn
            .prepare(
                "SELECT domain, authoritative_agent, confidence_score, lease_held, active_tasks_count, last_active_at
                 FROM domain_authorities
                 WHERE ?1 LIKE (domain || '%') OR domain LIKE (?1 || '%')
                    OR ?2 LIKE (domain || '%') OR domain LIKE (?2 || '%')
                 ORDER BY lease_held DESC, length(domain) DESC, confidence_score DESC
                 LIMIT 1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let hierarchical = stmt
            .query_row(params![clean_query, query_domain], |row| {
                Ok(DomainAuthority {
                    domain: row.get(0)?,
                    authoritative_agent: row.get(1)?,
                    confidence_score: row.get(2)?,
                    lease_held: row.get::<_, i64>(3)? != 0,
                    active_tasks_count: row.get(4)?,
                    last_active_at: row.get(5)?,
                })
            })
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(hierarchical)
    }

    // ==========================================
    // Episodic Memory & Hippocampal Consolidation
    // ==========================================

    /// Persists an episodic trace with multi-tier hierarchy support.
    pub fn save_episode(&self, episode: &EpisodeRecord) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = episode.id.to_string();
        let event_trace_json = serde_json::to_string(&episode.event_trace)
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize event trace: {e}")))?;
        let outcomes_json = serde_json::to_string(&episode.outcomes)
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize outcomes: {e}")))?;
        let snapshot_json = episode
            .state_snapshot
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize state snapshot: {e}")))?;
        let child_ids_json = serde_json::to_string(&episode.child_ids)
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize child_ids: {e}")))?;

        conn.execute(
            "INSERT INTO episodes (
                id, session_id, agent_id, task_id, summary, event_trace_json,
                outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                consolidated, tier, parent_id, child_ids_json, created_at, git_branch
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(id) DO UPDATE SET
                summary = excluded.summary,
                event_trace_json = excluded.event_trace_json,
                outcomes_json = excluded.outcomes_json,
                salience_score = excluded.salience_score,
                state_snapshot_json = excluded.state_snapshot_json,
                causal_provenance = excluded.causal_provenance,
                consolidated = excluded.consolidated,
                tier = excluded.tier,
                parent_id = excluded.parent_id,
                child_ids_json = excluded.child_ids_json,
                git_branch = COALESCE(excluded.git_branch, episodes.git_branch)",
            params![
                id_str,
                episode.session_id,
                episode.agent_id,
                episode.task_id,
                episode.summary,
                event_trace_json,
                outcomes_json,
                episode.salience_score,
                snapshot_json,
                episode.causal_provenance,
                if episode.consolidated { 1 } else { 0 },
                episode.tier as i64,
                episode.parent_id.map(|p| p.to_string()),
                child_ids_json,
                episode.created_at,
                episode.git_branch,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert episode: {e}")))?;

        Ok(())
    }

    /// Writes an episodic execution trace for hippocampal ingestion (alias for `save_episode`).
    pub fn write_episode(&self, episode: &EpisodeRecord) -> Result<(), StrataError> {
        self.save_episode(episode)
    }

    /// Persists a rollup episode and atomically updates parent_id of all child episodes.
    pub fn save_episode_rollup(&self, rollup: &EpisodeRecord) -> Result<(), StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to begin transaction: {e}")))?;

        let id_str = rollup.id.to_string();
        let event_trace_json = serde_json::to_string(&rollup.event_trace)
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize event trace: {e}")))?;
        let outcomes_json = serde_json::to_string(&rollup.outcomes)
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize outcomes: {e}")))?;
        let snapshot_json = rollup
            .state_snapshot
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize state snapshot: {e}")))?;
        let child_ids_json = serde_json::to_string(&rollup.child_ids)
            .map_err(|e| StrataError::Serialization(format!("Failed to serialize child_ids: {e}")))?;

        tx.execute(
            "INSERT INTO episodes (
                id, session_id, agent_id, task_id, summary, event_trace_json,
                outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                consolidated, tier, parent_id, child_ids_json, created_at, git_branch
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(id) DO UPDATE SET
                summary = excluded.summary,
                event_trace_json = excluded.event_trace_json,
                outcomes_json = excluded.outcomes_json,
                salience_score = excluded.salience_score,
                state_snapshot_json = excluded.state_snapshot_json,
                causal_provenance = excluded.causal_provenance,
                consolidated = excluded.consolidated,
                tier = excluded.tier,
                parent_id = excluded.parent_id,
                child_ids_json = excluded.child_ids_json,
                git_branch = COALESCE(excluded.git_branch, episodes.git_branch)",
            params![
                id_str,
                rollup.session_id,
                rollup.agent_id,
                rollup.task_id,
                rollup.summary,
                event_trace_json,
                outcomes_json,
                rollup.salience_score,
                snapshot_json,
                rollup.causal_provenance,
                if rollup.consolidated { 1 } else { 0 },
                rollup.tier as i64,
                rollup.parent_id.map(|p| p.to_string()),
                child_ids_json,
                rollup.created_at,
                rollup.git_branch,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert episode rollup: {e}")))?;

        if !rollup.child_ids.is_empty() {
            let mut update_stmt = tx
                .prepare("UPDATE episodes SET parent_id = ?1 WHERE id = ?2")
                .map_err(|e| StrataError::Database(e.to_string()))?;
            for child_id in &rollup.child_ids {
                update_stmt
                    .execute(params![id_str, child_id.to_string()])
                    .map_err(|e| StrataError::Database(format!("Failed to update parent_id for child {child_id}: {e}")))?;
            }
        }

        tx.commit().map_err(|e| {
            StrataError::Database(format!("Failed to commit episode rollup transaction: {e}"))
        })?;

        Ok(())
    }

    /// Fetches an episodic trace by ID.
    pub fn get_episode(&self, id: &Uuid) -> Result<Option<EpisodeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let id_str = id.to_string();
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                        outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                        consolidated, created_at, tier, parent_id, child_ids_json, git_branch
                 FROM episodes WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![id_str], Self::row_to_episode)
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        Ok(result)
    }

    /// Lists episodic execution traces with optional filters.
    pub fn list_episodes(
        &self,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut query = String::from(
            "SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                    outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                    consolidated, created_at, tier, parent_id, child_ids_json, git_branch
             FROM episodes WHERE 1=1",
        );

        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(s) = session_id {
            query.push_str(" AND session_id = ?");
            param_values.push(Box::new(s.to_string()));
        }

        if let Some(a) = agent_id {
            query.push_str(" AND agent_id = ?");
            param_values.push(Box::new(a.to_string()));
        }

        query.push_str(" ORDER BY created_at DESC LIMIT ?");
        param_values.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&query).map_err(|e| StrataError::Database(e.to_string()))?;
        let rusqlite_params: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(rusqlite_params.as_slice(), Self::row_to_episode)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Lists episodic execution traces by hierarchical tier with optional session filtering.
    pub fn list_episodes_by_tier(
        &self,
        tier: u8,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut query = String::from(
            "SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                    outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                    consolidated, created_at, tier, parent_id, child_ids_json, git_branch
             FROM episodes WHERE tier = ?",
        );

        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tier as i64)];

        if let Some(s) = session_id {
            query.push_str(" AND session_id = ?");
            param_values.push(Box::new(s.to_string()));
        }

        query.push_str(" ORDER BY created_at DESC LIMIT ?");
        param_values.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&query).map_err(|e| StrataError::Database(e.to_string()))?;
        let rusqlite_params: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(rusqlite_params.as_slice(), Self::row_to_episode)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Queries episodic execution traces by tier with optional filters for session, task_id, and temporal bounds.
    pub fn query_episodes_by_tier(
        &self,
        tier: u8,
        session_id: Option<&str>,
        task_id: Option<&str>,
        start_time: Option<i64>,
        end_time: Option<i64>,
        limit: usize,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut query = String::from(
            "SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                    outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                    consolidated, created_at, tier, parent_id, child_ids_json, git_branch
             FROM episodes WHERE tier = ?",
        );

        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tier as i64)];

        if let Some(s) = session_id {
            query.push_str(" AND session_id = ?");
            param_values.push(Box::new(s.to_string()));
        }

        if let Some(t) = task_id {
            query.push_str(" AND task_id = ?");
            param_values.push(Box::new(t.to_string()));
        }

        if let Some(st) = start_time {
            query.push_str(" AND created_at >= ?");
            param_values.push(Box::new(st));
        }

        if let Some(et) = end_time {
            query.push_str(" AND created_at <= ?");
            param_values.push(Box::new(et));
        }

        query.push_str(" ORDER BY created_at DESC LIMIT ?");
        param_values.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&query).map_err(|e| StrataError::Database(e.to_string()))?;
        let rusqlite_params: Vec<&dyn rusqlite::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(rusqlite_params.as_slice(), Self::row_to_episode)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Lists unrolled episodes of a given tier (parent_id IS NULL) with optional session filtering.
    pub fn list_unrolled_episodes(
        &self,
        tier: u8,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EpisodeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut query = String::from(
            "SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                    outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                    consolidated, created_at, tier, parent_id, child_ids_json, git_branch
             FROM episodes WHERE tier = ? AND parent_id IS NULL",
        );

        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tier as i64)];

        if let Some(s) = session_id {
            query.push_str(" AND session_id = ?");
            param_values.push(Box::new(s.to_string()));
        }

        query.push_str(" ORDER BY created_at ASC LIMIT ?");
        param_values.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&query).map_err(|e| StrataError::Database(e.to_string()))?;
        let rusqlite_params: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(rusqlite_params.as_slice(), Self::row_to_episode)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Retrieves immediate child episodes whose parent_id equals the given ID.
    pub fn get_child_episodes(&self, parent_id: &Uuid) -> Result<Vec<EpisodeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let parent_id_str = parent_id.to_string();
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                        outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                        consolidated, created_at, tier, parent_id, child_ids_json, git_branch
                 FROM episodes WHERE parent_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![parent_id_str], Self::row_to_episode)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        Ok(list)
    }

    /// Recursively retrieves an episode and all of its transitive descendants ordered
    /// topologicamente por tier decrescente.
    pub fn get_drill_down_tree(&self, root_id: &Uuid) -> Result<Vec<EpisodeRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let root_str = root_id.to_string();
        let mut stmt = conn
            .prepare(
                "WITH RECURSIVE episode_tree(id) AS (
                    SELECT id FROM episodes WHERE id = ?1
                    UNION
                    SELECT e.id FROM episodes e
                    JOIN episode_tree tree ON e.parent_id = tree.id
                )
                SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                       outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                       consolidated, created_at, tier, parent_id, child_ids_json, git_branch
                FROM episodes
                WHERE id IN (SELECT id FROM episode_tree)
                ORDER BY tier DESC, created_at ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![root_str], Self::row_to_episode)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
        }

        // Augment with any child_ids referenced in rollup JSON if not already reached via parent_id
        let mut known_ids: std::collections::HashSet<Uuid> = list.iter().map(|e| e.id).collect();
        let mut queue: std::collections::VecDeque<Uuid> = std::collections::VecDeque::new();
        for item in &list {
            for child_id in &item.child_ids {
                if known_ids.insert(*child_id) {
                    queue.push_back(*child_id);
                }
            }
        }
        while let Some(child_id) = queue.pop_front() {
            if let Some(child_ep) = self.get_episode(&child_id)? {
                for cid in &child_ep.child_ids {
                    if known_ids.insert(*cid) {
                        queue.push_back(*cid);
                    }
                }
                list.push(child_ep);
            }
        }

        list.sort_by(|a, b| b.tier.cmp(&a.tier).then_with(|| a.created_at.cmp(&b.created_at)));

        Ok(list)
    }

    fn row_to_episode(row: &rusqlite::Row<'_>) -> rusqlite::Result<EpisodeRecord> {
        let id_str: String = row.get(0)?;
        let id = Uuid::parse_str(&id_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?;
        let session_id: String = row.get(1)?;
        let agent_id: String = row.get(2)?;
        let task_id: Option<String> = row.get(3)?;
        let summary: String = row.get(4)?;
        let event_trace_json: String = row.get(5)?;
        let outcomes_json: String = row.get(6)?;
        let salience_score: f64 = row.get(7)?;
        let state_snapshot_json: Option<String> = row.get(8)?;
        let causal_provenance: Option<String> = row.get(9)?;
        let consolidated_int: i32 = row.get(10)?;
        let created_at: i64 = row.get(11)?;

        let tier: u8 = row.get::<_, i64>(12).map(|t| t as u8).unwrap_or(1);
        let parent_id: Option<Uuid> = row
            .get::<_, Option<String>>(13)
            .ok()
            .flatten()
            .and_then(|s| Uuid::parse_str(&s).ok());
        let child_ids: Vec<Uuid> = row
            .get::<_, Option<String>>(14)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let event_trace: Vec<EpisodeStep> =
            serde_json::from_str(&event_trace_json).unwrap_or_default();
        let outcomes: Vec<String> = serde_json::from_str(&outcomes_json).unwrap_or_default();
        let state_snapshot = state_snapshot_json.and_then(|s| serde_json::from_str(&s).ok());
        let git_branch: Option<String> = row.get(15).ok().flatten();

        Ok(EpisodeRecord {
            id,
            session_id,
            agent_id,
            task_id,
            summary,
            event_trace,
            outcomes,
            salience_score: salience_score as f32,
            state_snapshot,
            causal_provenance,
            consolidated: consolidated_int != 0,
            tier,
            parent_id,
            child_ids,
            created_at,
            git_branch,
        })
    }

    /// Replays an episodic trajectory, computing simulated reward and extracting candidate insights.
    pub fn replay_episode(&self, id: &Uuid) -> Result<Option<EpisodeReplay>, StrataError> {
        let episode = match self.get_episode(id)? {
            Some(ep) => ep,
            None => return Ok(None),
        };

        let total_steps = episode.event_trace.len();
        let mut successful_steps = 0;
        let mut failed_steps = 0;
        let mut candidate_anti_patterns = Vec::new();

        for step in &episode.event_trace {
            if step.error.is_some() {
                failed_steps += 1;
                if let Some(err) = &step.error {
                    if err != "none" && !err.is_empty() {
                        candidate_anti_patterns.push(format!("Action '{}' failed: {}", step.action, err));
                    }
                }
            } else {
                successful_steps += 1;
            }
        }

        let base_reward = if total_steps > 0 {
            (successful_steps as f32) / (total_steps as f32)
        } else {
            1.0
        };
        let simulated_reward = (base_reward * 0.7 + episode.salience_score * 0.3).clamp(0.0, 1.0);

        let mut candidate_facts = Vec::new();
        if simulated_reward >= 0.5 {
            candidate_facts.push(format!("Successful trajectory: {}", episode.summary));
            for out in &episode.outcomes {
                candidate_facts.push(format!("Outcome observed: {out}"));
            }
        }

        let mut extracted_insights = Vec::new();
        if let Some(prov) = &episode.causal_provenance {
            extracted_insights.push(format!("Causal context: {prov}"));
        }
        extracted_insights.push(format!("Step success rate: {:.1}%", base_reward * 100.0));

        Ok(Some(EpisodeReplay {
            episode_id: episode.id,
            session_id: episode.session_id,
            agent_id: episode.agent_id,
            total_steps,
            successful_steps,
            failed_steps,
            simulated_reward,
            extracted_insights,
            candidate_facts,
            candidate_anti_patterns,
        }))
    }

    /// Consolidates pending episodic traces into semantic facts and procedural failure patterns
    /// using Justification-based Truth Maintenance (JTMS) for non-monotonic contradiction arbitration.
    pub fn consolidate_episodes(&self, session_id: Option<&str>) -> Result<HippocampalConsolidationResult, StrataError> {
        let episodes = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

            let mut query = String::from(
                "SELECT id, session_id, agent_id, task_id, summary, event_trace_json,
                        outcomes_json, salience_score, state_snapshot_json, causal_provenance,
                        consolidated, created_at, tier, parent_id, child_ids_json
                 FROM episodes WHERE consolidated = 0",
            );
            let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(sid) = session_id {
                query.push_str(" AND session_id = ?");
                param_values.push(Box::new(sid.to_string()));
            }
            query.push_str(" ORDER BY created_at ASC");

            let mut stmt = conn.prepare(&query).map_err(|e| StrataError::Database(e.to_string()))?;
            let rusqlite_params: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(rusqlite_params.as_slice(), Self::row_to_episode)
                .map_err(|e| StrataError::Database(e.to_string()))?;

            let mut list = Vec::new();
            for r in rows {
                list.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
            }
            list
        };

        let mut result = HippocampalConsolidationResult::default();
        if episodes.is_empty() {
            return Ok(result);
        }

        for ep in episodes {
            result.episodes_processed += 1;

            // 1. Synthesize candidate semantic facts from successes
            if ep.salience_score >= 0.4 {
                let scope = Scope::Project(ep.session_id.clone());
                let statement = if ep.outcomes.is_empty() {
                    ep.summary.clone()
                } else {
                    format!("{}: {}", ep.summary, ep.outcomes.join("; "))
                };

                let mut fact = SemanticFact::new(statement, "hippocampal_consolidation", scope)
                    .with_importance(ep.salience_score)
                    .with_confidence(0.9)
                    .with_evidence(vec![EvidenceRef::new("episode", ep.id.to_string(), ep.salience_score)]);

                if ep.salience_score >= 0.8 {
                    fact.tier = MemoryTier::Core;
                    result.promoted_to_core += 1;
                } else {
                    fact.tier = MemoryTier::Working;
                }

                // Check for contradiction against existing facts in the store using JTMS arbitration
                let existing_facts = self.get_all_semantic_facts(None, Some(FactStatus::Active), 1000)?;
                let jtms = crate::jtms::TruthMaintenanceSystem::with_default_threshold();

                for existing in &existing_facts {
                    if existing.id == fact.id || existing.status != FactStatus::Active {
                        continue;
                    }
                    if !existing.scope.is_compatible(&fact.scope) {
                        continue;
                    }
                    let (is_contradiction, cues) = jtms.detect_lexical_contradiction(&existing.statement, &fact.statement);
                    if is_contradiction || (!cues.is_empty() && existing.statement != fact.statement) {
                        let (resolution, _reason) = jtms.arbitrate_pair(existing, &fact, &cues);
                        match resolution {
                            crate::jtms::ConflictResolution::Supersede => {
                                if let Some(mut old) = self.get_semantic_fact(&existing.id)? {
                                    old.deprecate_and_replace(fact.id);
                                    let _ = self.insert_or_update_semantic_fact(&old);
                                    result.contradictions_resolved += 1;
                                }
                            }
                            crate::jtms::ConflictResolution::Reject => {
                                fact.status = FactStatus::Deprecated;
                                result.contradictions_resolved += 1;
                            }
                            _ => {}
                        }
                    }
                }

                if fact.status == FactStatus::Active {
                    self.insert_or_update_semantic_fact(&fact)?;
                    result.facts_consolidated += 1;
                }
            }

            // 2. Extract anti-patterns / failure rules from failed steps
            for step in &ep.event_trace {
                if let Some(err) = &step.error {
                    if err != "none" && !err.is_empty() {
                        let sig = format!("err-{}-{:x}", step.action, ep.id.as_fields().0);
                        let mut fp = FailurePattern::new(
                            sig,
                            format!("Action '{}' failure", step.action),
                            err.clone(),
                            format!("Avoid repeating failed invocation in action '{}'", step.action),
                        );
                        fp.scope = Scope::Project(ep.session_id.clone());
                        fp.severity = FailureSeverity::Medium;
                        if let Err(e) = self.upsert_failure_pattern(&fp) {
                            tracing::warn!("Failed to persist learned failure pattern: {e}");
                        } else {
                            result.anti_patterns_learned += 1;
                        }
                    }
                }
            }

            // Mark episode as consolidated
            {
                let conn = self
                    .conn
                    .lock()
                    .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
                let _ = conn.execute(
                    "UPDATE episodes SET consolidated = 1 WHERE id = ?1",
                    params![ep.id.to_string()],
                );
            }
        }

        Ok(result)
    }

    /// Merges and promotes memories from a feature branch into a target branch (e.g. main/master)
    /// within an atomic SQLite transaction. Updates all 5 memory and event tables.
    pub fn merge_branch_memories(
        &self,
        source_branch: &str,
        target_branch: &str,
    ) -> Result<BranchMergeReport, StrataError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let tx = conn
            .transaction()
            .map_err(|e| StrataError::Database(format!("Failed to begin branch merge transaction: {e}")))?;

        let now_str = Utc::now().to_rfc3339();
        let mut report = BranchMergeReport::new(source_branch, target_branch);

        // 1. memories
        let memories_merged = tx.execute(
            "UPDATE memories
             SET git_branch = ?1,
                 metadata_json = json_set(metadata_json, '$.git_branch', ?1),
                 updated_at = ?2
             WHERE git_branch = ?3",
            params![target_branch, now_str, source_branch],
        ).map_err(|e| StrataError::Database(format!("Failed to merge memories for branch '{source_branch}': {e}")))?;
        report.memories_merged = memories_merged;

        // 2. semantic_facts
        let facts_merged = tx.execute(
            "UPDATE semantic_facts
             SET git_branch = ?1,
                 code_anchor_json = CASE
                     WHEN code_anchor_json IS NOT NULL THEN json_set(code_anchor_json, '$.git_branch', ?1)
                     ELSE NULL
                 END,
                 last_updated_at = ?2
             WHERE git_branch = ?3",
            params![target_branch, now_str, source_branch],
        ).map_err(|e| StrataError::Database(format!("Failed to merge semantic facts for branch '{source_branch}': {e}")))?;
        report.semantic_facts_merged = facts_merged;

        // 3. events
        let events_merged = tx.execute(
            "UPDATE events
             SET git_branch = ?1
             WHERE git_branch = ?2",
            params![target_branch, source_branch],
        ).map_err(|e| StrataError::Database(format!("Failed to merge events for branch '{source_branch}': {e}")))?;
        report.events_merged = events_merged;

        // 4. episodic_memories
        let episodic_merged = tx.execute(
            "UPDATE episodic_memories
             SET git_branch = ?1
             WHERE git_branch = ?2",
            params![target_branch, source_branch],
        ).map_err(|e| StrataError::Database(format!("Failed to merge episodic memories for branch '{source_branch}': {e}")))?;
        report.episodic_memories_merged = episodic_merged;

        // 5. episodes
        let episodes_merged = tx.execute(
            "UPDATE episodes
             SET git_branch = ?1
             WHERE git_branch = ?2",
            params![target_branch, source_branch],
        ).map_err(|e| StrataError::Database(format!("Failed to merge episodes for branch '{source_branch}': {e}")))?;
        report.episodes_merged = episodes_merged;

        tx.commit().map_err(|e| {
            StrataError::Database(format!("Failed to commit branch merge transaction: {e}"))
        })?;

        tracing::info!(
            source_branch = source_branch,
            target_branch = target_branch,
            total_merged = report.total_merged(),
            "Successfully merged branch memories"
        );

        Ok(report)
    }

    // ==========================================
    // Procedural Self-Editing & Rule Versioning
    // ==========================================

    pub fn insert_or_update_procedural_rule(&self, rule: &ProceduralRule) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let created_at_str = rule.created_at.to_rfc3339();
        let updated_at_str = rule.updated_at.to_rfc3339();

        conn.execute(
            "INSERT INTO procedural_rules (
                id, name, category, current_version, current_content, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                category = excluded.category,
                current_version = excluded.current_version,
                current_content = excluded.current_content,
                updated_at = excluded.updated_at",
            params![
                rule.id,
                rule.name,
                rule.category,
                rule.current_version,
                rule.current_content,
                created_at_str,
                updated_at_str,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert or update procedural rule: {e}")))?;

        // Persist all associated versions
        for ver in &rule.versions {
            let ver_id = format!("{}:v{}", rule.id, ver.version);
            let ver_created_at = ver.timestamp.to_rfc3339();
            let feedback_json = ver
                .feedback_applied
                .as_ref()
                .map(|fb| serde_json::to_string(fb))
                .transpose()
                .map_err(|e| StrataError::Serialization(e.to_string()))?;

            conn.execute(
                "INSERT INTO procedural_rule_versions (
                    id, rule_id, version, content, rationale, feedback_json, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(id) DO UPDATE SET
                    content = excluded.content,
                    rationale = excluded.rationale,
                    feedback_json = excluded.feedback_json,
                    created_at = excluded.created_at",
                params![
                    ver_id,
                    rule.id,
                    ver.version,
                    ver.content,
                    ver.rationale,
                    feedback_json,
                    ver_created_at,
                ],
            )
            .map_err(|e| StrataError::Database(format!("Failed to insert or update procedural rule version: {e}")))?;
        }

        Ok(())
    }

    pub fn record_procedural_rule_version(
        &self,
        rule_id: &str,
        version: &ProceduralRuleVersion,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let ver_id = format!("{}:v{}", rule_id, version.version);
        let ver_created_at = version.timestamp.to_rfc3339();
        let feedback_json = version
            .feedback_applied
            .as_ref()
            .map(|fb| serde_json::to_string(fb))
            .transpose()
            .map_err(|e| StrataError::Serialization(e.to_string()))?;

        conn.execute(
            "INSERT INTO procedural_rule_versions (
                id, rule_id, version, content, rationale, feedback_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                content = excluded.content,
                rationale = excluded.rationale,
                feedback_json = excluded.feedback_json,
                created_at = excluded.created_at",
            params![
                ver_id,
                rule_id,
                version.version,
                version.content,
                version.rationale,
                feedback_json,
                ver_created_at,
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to record procedural rule version: {e}")))?;

        let now_str = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE procedural_rules
             SET current_version = ?1, current_content = ?2, updated_at = ?3
             WHERE id = ?4 AND current_version < ?1",
            params![version.version, version.content, now_str, rule_id],
        )
        .map_err(|e| StrataError::Database(format!("Failed to update rule current version: {e}")))?;

        Ok(())
    }

    pub fn get_procedural_rule_versions(&self, rule_id: &str) -> Result<Vec<ProceduralRuleVersion>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT version, content, rationale, feedback_json, created_at
                 FROM procedural_rule_versions
                 WHERE rule_id = ?1
                 ORDER BY version ASC",
            )
            .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![rule_id], |row| {
                let version: u32 = row.get(0)?;
                let content: String = row.get(1)?;
                let rationale: String = row.get(2)?;
                let feedback_json: Option<String> = row.get(3)?;
                let created_at: String = row.get(4)?;
                Ok((version, content, rationale, feedback_json, created_at))
            })
            .map_err(|e| StrataError::Database(format!("Failed to execute query: {e}")))?;

        let mut versions = Vec::new();
        for r in rows {
            let (version, content, rationale, feedback_json, created_at) =
                r.map_err(|e| StrataError::Database(e.to_string()))?;

            let feedback_applied = feedback_json
                .as_deref()
                .and_then(|json_str| serde_json::from_str::<TextGradientFeedback>(json_str).ok());

            let timestamp = DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            versions.push(ProceduralRuleVersion {
                version,
                content,
                rationale,
                feedback_applied,
                timestamp,
            });
        }

        Ok(versions)
    }

    pub fn get_procedural_rule(&self, id: &str) -> Result<Option<ProceduralRule>, StrataError> {
        let versions = self.get_procedural_rule_versions(id)?;

        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, name, category, current_version, current_content, created_at, updated_at
                 FROM procedural_rules
                 WHERE id = ?1",
            )
            .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;

        let mut rows = stmt
            .query_map(params![id], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let category: String = row.get(2)?;
                let current_version: u32 = row.get(3)?;
                let current_content: String = row.get(4)?;
                let created_at: String = row.get(5)?;
                let updated_at: String = row.get(6)?;
                Ok((id, name, category, current_version, current_content, created_at, updated_at))
            })
            .map_err(|e| StrataError::Database(format!("Failed to query procedural rule: {e}")))?;

        if let Some(r) = rows.next() {
            let (id, name, category, current_version, current_content, created_at_str, updated_at_str) =
                r.map_err(|e| StrataError::Database(e.to_string()))?;

            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(Some(ProceduralRule {
                id,
                name,
                category,
                current_version,
                current_content,
                versions,
                created_at,
                updated_at,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_procedural_rule_by_name(&self, name: &str) -> Result<Option<ProceduralRule>, StrataError> {
        let rule_id: Option<String> = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

            let mut stmt = conn
                .prepare("SELECT id FROM procedural_rules WHERE name = ?1 LIMIT 1")
                .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;

            let mut rows = stmt
                .query_map(params![name], |row| row.get::<_, String>(0))
                .map_err(|e| StrataError::Database(format!("Query failed: {e}")))?;

            if let Some(r) = rows.next() {
                Some(r.map_err(|e| StrataError::Database(e.to_string()))?)
            } else {
                None
            }
        };

        match rule_id {
            Some(id) => self.get_procedural_rule(&id),
            None => Ok(None),
        }
    }

    pub fn list_procedural_rules(&self, category: Option<&str>) -> Result<Vec<ProceduralRule>, StrataError> {
        let rule_ids: Vec<String> = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

            if let Some(cat) = category {
                let mut stmt = conn
                    .prepare("SELECT id FROM procedural_rules WHERE category = ?1 ORDER BY name ASC")
                    .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;
                let rows = stmt
                    .query_map(params![cat], |row| row.get::<_, String>(0))
                    .map_err(|e| StrataError::Database(format!("Query failed: {e}")))?;
                let mut ids = Vec::new();
                for r in rows {
                    ids.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
                }
                ids
            } else {
                let mut stmt = conn
                    .prepare("SELECT id FROM procedural_rules ORDER BY name ASC")
                    .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| StrataError::Database(format!("Query failed: {e}")))?;
                let mut ids = Vec::new();
                for r in rows {
                    ids.push(r.map_err(|e| StrataError::Database(e.to_string()))?);
                }
                ids
            }
        };

        let mut rules = Vec::new();
        for id in rule_ids {
            if let Some(rule) = self.get_procedural_rule(&id)? {
                rules.push(rule);
            }
        }
        Ok(rules)
    }

    pub fn rollback_procedural_rule(
        &self,
        rule_id: &str,
        target_version: u32,
    ) -> Result<Option<ProceduralRule>, StrataError> {
        let target_content: Option<String> = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

            let mut stmt = conn
                .prepare(
                    "SELECT content FROM procedural_rule_versions WHERE rule_id = ?1 AND version = ?2",
                )
                .map_err(|e| StrataError::Database(format!("Failed to prepare query: {e}")))?;

            let mut rows = stmt
                .query_map(params![rule_id, target_version], |row| row.get::<_, String>(0))
                .map_err(|e| StrataError::Database(format!("Query failed: {e}")))?;

            if let Some(r) = rows.next() {
                Some(r.map_err(|e| StrataError::Database(e.to_string()))?)
            } else {
                None
            }
        };

        let content = match target_content {
            Some(c) => c,
            None => return Ok(None),
        };

        {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

            let now_str = Utc::now().to_rfc3339();
            let rows_updated = conn
                .execute(
                    "UPDATE procedural_rules
                     SET current_version = ?1, current_content = ?2, updated_at = ?3
                     WHERE id = ?4",
                    params![target_version, content, now_str, rule_id],
                )
                .map_err(|e| StrataError::Database(format!("Failed to rollback procedural rule: {e}")))?;

            if rows_updated == 0 {
                return Ok(None);
            }
        }

        self.get_procedural_rule(rule_id)
    }

    // ==========================================
    // Context Rot & Injection Outcomes Operations
    // ==========================================

    pub fn record_injection_outcome(
        &self,
        record: &crate::context_rot::MemoryInjectionRecord,
    ) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        conn.execute(
            "INSERT INTO memory_injection_outcomes (
                id, memory_id, session_id, task_id, command, outcome, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.id.to_string(),
                record.memory_id.to_string(),
                record.session_id,
                record.task_id,
                record.command,
                record.outcome.as_str(),
                record.created_at.to_rfc3339(),
            ],
        )
        .map_err(|e| StrataError::Database(format!("Failed to insert injection outcome: {e}")))?;

        Ok(())
    }

    pub fn get_injection_outcomes_for_memory(
        &self,
        memory_id: &Uuid,
    ) -> Result<Vec<crate::context_rot::MemoryInjectionRecord>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, memory_id, session_id, task_id, command, outcome, created_at
                 FROM memory_injection_outcomes
                 WHERE memory_id = ?1
                 ORDER BY created_at ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![memory_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let mem_id_str: String = row.get(1)?;
                let session_id: String = row.get(2)?;
                let task_id: Option<String> = row.get(3)?;
                let command: Option<String> = row.get(4)?;
                let outcome_str: String = row.get(5)?;
                let created_str: String = row.get(6)?;

                Ok((
                    id_str,
                    mem_id_str,
                    session_id,
                    task_id,
                    command,
                    outcome_str,
                    created_str,
                ))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut records = Vec::new();
        for row in rows {
            let (id_str, mem_id_str, session_id, task_id, command, outcome_str, created_str) =
                row.map_err(|e| StrataError::Database(e.to_string()))?;

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in injection id: {e}")))?;
            let memory_id = Uuid::parse_str(&mem_id_str)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in memory_id: {e}")))?;
            let outcome = outcome_str
                .parse::<crate::context_rot::InjectionOutcome>()
                .map_err(|e| StrataError::Validation(format!("Invalid injection outcome: {e}")))?;
            let created_at = DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            records.push(crate::context_rot::MemoryInjectionRecord {
                id,
                memory_id,
                session_id,
                task_id,
                command,
                outcome,
                created_at,
            });
        }

        Ok(records)
    }

    pub fn get_all_memory_injection_metrics(
        &self,
        min_samples: usize,
        threshold: f32,
    ) -> Result<Vec<crate::context_rot::MemoryEfficacyMetric>, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, memory_id, session_id, task_id, command, outcome, created_at
                 FROM memory_injection_outcomes
                 ORDER BY created_at ASC",
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let mem_id_str: String = row.get(1)?;
                let session_id: String = row.get(2)?;
                let task_id: Option<String> = row.get(3)?;
                let command: Option<String> = row.get(4)?;
                let outcome_str: String = row.get(5)?;
                let created_str: String = row.get(6)?;

                Ok((
                    id_str,
                    mem_id_str,
                    session_id,
                    task_id,
                    command,
                    outcome_str,
                    created_str,
                ))
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut map: std::collections::HashMap<Uuid, Vec<crate::context_rot::MemoryInjectionRecord>> =
            std::collections::HashMap::new();

        for row in rows {
            let (id_str, mem_id_str, session_id, task_id, command, outcome_str, created_str) =
                row.map_err(|e| StrataError::Database(e.to_string()))?;

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in injection id: {e}")))?;
            let memory_id = Uuid::parse_str(&mem_id_str)
                .map_err(|e| StrataError::Validation(format!("Invalid UUID in memory_id: {e}")))?;
            let outcome = outcome_str
                .parse::<crate::context_rot::InjectionOutcome>()
                .map_err(|e| StrataError::Validation(format!("Invalid injection outcome: {e}")))?;
            let created_at = DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let record = crate::context_rot::MemoryInjectionRecord {
                id,
                memory_id,
                session_id,
                task_id,
                command,
                outcome,
                created_at,
            };

            map.entry(memory_id).or_default().push(record);
        }

        let config = crate::context_rot::ContextRotConfig {
            rejection_threshold: threshold,
            min_samples,
            penalty_multiplier: 0.1,
        };

        let metrics = map
            .into_iter()
            .map(|(mem_id, records)| {
                crate::context_rot::MemoryEfficacyMetric::compute(mem_id, &records, &config)
            })
            .collect();

        Ok(metrics)
    }

    pub fn quarantine_memory(&self, memory_id: &Uuid, reason: &str) -> Result<(), StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare("SELECT metadata_json FROM memories WHERE id = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let row_opt: Option<String> = stmt
            .query_row(params![memory_id.to_string()], |row| row.get(0))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if let Some(meta_str) = row_opt {
            let mut val: serde_json::Value =
                serde_json::from_str(&meta_str).unwrap_or_else(|_| serde_json::json!({}));
            if !val.is_object() {
                val = serde_json::json!({});
            }
            if let Some(obj) = val.as_object_mut() {
                obj.insert("quarantined".to_string(), serde_json::Value::Bool(true));
                obj.insert("suppressed".to_string(), serde_json::Value::Bool(true));
                obj.insert(
                    "quarantine_reason".to_string(),
                    serde_json::Value::String(reason.to_string()),
                );
                obj.insert(
                    "quarantined_at".to_string(),
                    serde_json::Value::String(Utc::now().to_rfc3339()),
                );
            }
            let new_meta = serde_json::to_string(&val)?;
            let now_str = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE memories SET metadata_json = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_meta, now_str, memory_id.to_string()],
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;
            Ok(())
        } else {
            Err(StrataError::NotFound(format!(
                "Memory {} not found for quarantine",
                memory_id
            )))
        }
    }

    pub fn is_memory_quarantined(&self, memory_id: &Uuid) -> Result<bool, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        let mut stmt = conn
            .prepare("SELECT metadata_json FROM memories WHERE id = ?1")
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let row_opt: Option<String> = stmt
            .query_row(params![memory_id.to_string()], |row| row.get(0))
            .optional()
            .map_err(|e| StrataError::Database(e.to_string()))?;

        if let Some(meta_str) = row_opt {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&meta_str) {
                if val.get("quarantined").and_then(|v| v.as_bool()).unwrap_or(false)
                    || val.get("suppressed").and_then(|v| v.as_bool()).unwrap_or(false)
                    || val.get("status").and_then(|v| v.as_str()).map_or(false, |s| {
                        s.eq_ignore_ascii_case("quarantined") || s.eq_ignore_ascii_case("suppressed")
                    })
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Collect comprehensive vector consistency metrics across memories, semantic facts, procedural skills, and episodes.
    pub fn get_vector_consistency_stats(
        &self,
        active_model: Option<&str>,
    ) -> Result<VectorConsistencyStats, StrataError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;

        // 1. Total and embedded counts per table
        let (total_memories, memories_with_embeddings): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COUNT(embedding) FROM memories",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let (total_semantic_facts, facts_with_embeddings): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COUNT(embedding) FROM semantic_facts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let (total_procedural_skills, skills_with_embeddings): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COUNT(embedding) FROM procedural_skills",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let (total_episodic_memories, episodes_with_embeddings): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COUNT(embedding) FROM episodic_memories",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| StrataError::Database(e.to_string()))?;

        // 2. Model breakdown across tables
        let breakdown_sql = "
            SELECT 'memories' as tbl, COALESCE(embedding_model, '<legacy-none>') as mdl, COALESCE(embedding_version, 'none') as ver, COUNT(*) as cnt
            FROM memories WHERE embedding IS NOT NULL GROUP BY mdl, ver
            UNION ALL
            SELECT 'semantic_facts' as tbl, COALESCE(embedding_model, '<legacy-none>') as mdl, COALESCE(embedding_version, 'none') as ver, COUNT(*) as cnt
            FROM semantic_facts WHERE embedding IS NOT NULL GROUP BY mdl, ver
            UNION ALL
            SELECT 'procedural_skills' as tbl, COALESCE(embedding_model, '<legacy-none>') as mdl, COALESCE(embedding_version, 'none') as ver, COUNT(*) as cnt
            FROM procedural_skills WHERE embedding IS NOT NULL GROUP BY mdl, ver
            UNION ALL
            SELECT 'episodic_memories' as tbl, COALESCE(embedding_model, '<legacy-none>') as mdl, COALESCE(embedding_version, 'none') as ver, COUNT(*) as cnt
            FROM episodic_memories WHERE embedding IS NOT NULL GROUP BY mdl, ver
        ";

        let mut stmt = conn
            .prepare(breakdown_sql)
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |r| {
                Ok(ModelVectorCount {
                    table: r.get(0)?,
                    model: r.get(1)?,
                    version: r.get(2)?,
                    count: r.get::<_, i64>(3)? as usize,
                })
            })
            .map_err(|e| StrataError::Database(e.to_string()))?;

        let mut model_breakdown = Vec::new();
        let mut distinct_models = std::collections::HashSet::new();
        let mut has_legacy = false;

        for r in rows {
            let item = r.map_err(|e| StrataError::Database(e.to_string()))?;
            if item.count > 0 {
                distinct_models.insert(item.model.clone());
                if item.model == "<legacy-none>" {
                    has_legacy = true;
                }
                model_breakdown.push(item);
            }
        }

        let total_vectors = (memories_with_embeddings
            + facts_with_embeddings
            + skills_with_embeddings
            + episodes_with_embeddings) as usize;

        let distinct_models_count = distinct_models.len();

        let mut is_consistent = true;
        let mut recommendations = Vec::new();

        if total_vectors == 0 {
            recommendations.push("No vector embeddings stored yet. Run 'strata memory reindex' to generate embeddings.".to_string());
        } else if distinct_models_count > 1 {
            is_consistent = false;
            let models_list: Vec<_> = distinct_models.into_iter().collect();
            recommendations.push(format!(
                "Mixed embedding spaces detected across database ({}). Incompatible models: [{}]. Run 'strata memory reindex' to unify.",
                distinct_models_count,
                models_list.join(", ")
            ));
        } else if let Some(act) = active_model {
            let active_norm = act.trim().to_lowercase();
            let stored_model = distinct_models.iter().next().cloned().unwrap_or_default();
            if !crate::embedding::are_embedding_models_compatible(&active_norm, &stored_model) {
                is_consistent = false;
                recommendations.push(format!(
                    "Stored embeddings model ('{stored_model}') differs from active model ('{act}'). Run 'strata memory reindex' to align vector spaces."
                ));
            } else {
                recommendations.push(format!(
                    "All {total_vectors} vectors are unified and match the active model ('{act}')."
                ));
            }
        } else {
            recommendations.push(format!(
                "All {total_vectors} vectors share a single consistent model space."
            ));
        }

        if has_legacy {
            recommendations.push("Found legacy records without model metadata. Run 'strata memory reindex' to re-tag them.".to_string());
        }

        Ok(VectorConsistencyStats {
            total_memories: total_memories as usize,
            memories_with_embeddings: memories_with_embeddings as usize,
            total_semantic_facts: total_semantic_facts as usize,
            facts_with_embeddings: facts_with_embeddings as usize,
            total_procedural_skills: total_procedural_skills as usize,
            skills_with_embeddings: skills_with_embeddings as usize,
            total_episodic_memories: total_episodic_memories as usize,
            episodes_with_embeddings: episodes_with_embeddings as usize,
            total_vectors,
            model_breakdown,
            distinct_models_count,
            is_consistent,
            active_model: active_model.map(|s| s.to_string()),
            recommendations,
        })
    }

    /// Reindex vectors across memories, semantic facts, and procedural skills using the given embedder.
    /// If `force_all` is true, recomputes vectors for all records regardless of existing model tags.
    /// If `force_all` is false, only updates records missing embeddings or tagged with a different model.
    pub async fn reindex_embeddings(
        &self,
        embedder: &dyn crate::embedding::EmbeddingProvider,
        force_all: bool,
        batch_size: usize,
    ) -> Result<ReindexReport, StrataError> {
        let start = std::time::Instant::now();
        let target_model = embedder.model_name().to_string();
        let target_version = embedder.model_version().to_string();
        let dimension = embedder.dimension();
        let bsize = if batch_size == 0 { 50 } else { batch_size };

        let mut report = ReindexReport {
            target_model: target_model.clone(),
            target_version: target_version.clone(),
            dimension,
            ..Default::default()
        };

        // 1. Reindex memories
        let candidate_memories = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
            let mut sql = "SELECT id, content, summary FROM memories WHERE 1=1".to_string();
            if !force_all {
                sql.push_str(" AND (embedding IS NULL OR embedding_model IS NULL OR embedding_model != ?1)");
            }
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows: Vec<(String, String, Option<String>)> = if force_all {
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            } else {
                stmt.query_map(params![target_model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            };
            rows
        };

        for chunk in candidate_memories.chunks(bsize) {
            let texts: Vec<String> = chunk
                .iter()
                .map(|(_, content, summary)| {
                    if let Some(ref s) = summary {
                        format!("{s}\n{content}")
                    } else {
                        content.clone()
                    }
                })
                .collect();
            let vectors = embedder.embed_batch(&texts).await?;
            {
                let mut conn = self
                    .conn
                    .lock()
                    .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                for ((id_str, _, _), vec) in chunk.iter().zip(vectors.into_iter()) {
                    let blob = crate::embedding::embedding_to_bytes(&vec);
                    let rows_affected = tx.execute(
                        "UPDATE memories SET embedding = ?1, embedding_model = ?2, embedding_version = ?3, updated_at = ?4 WHERE id = ?5",
                        params![blob, target_model, target_version, Utc::now().to_rfc3339(), id_str],
                    ).map_err(|e| StrataError::Database(e.to_string()))?;
                    report.memories_reindexed += rows_affected;
                }
                tx.commit().map_err(|e| StrataError::Database(e.to_string()))?;
            }
        }

        // 2. Reindex semantic_facts
        let candidate_facts = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
            let mut sql = "SELECT id, statement FROM semantic_facts WHERE 1=1".to_string();
            if !force_all {
                sql.push_str(" AND (embedding IS NULL OR embedding_model IS NULL OR embedding_model != ?1)");
            }
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows: Vec<(String, String)> = if force_all {
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            } else {
                stmt.query_map(params![target_model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            };
            rows
        };

        for chunk in candidate_facts.chunks(bsize) {
            let texts: Vec<String> = chunk.iter().map(|(_, stmt)| stmt.clone()).collect();
            let vectors = embedder.embed_batch(&texts).await?;
            {
                let mut conn = self
                    .conn
                    .lock()
                    .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                for ((id_str, _), vec) in chunk.iter().zip(vectors.into_iter()) {
                    let blob = crate::embedding::embedding_to_bytes(&vec);
                    let rows_affected = tx.execute(
                        "UPDATE semantic_facts SET embedding = ?1, embedding_model = ?2, embedding_version = ?3, last_updated_at = ?4 WHERE id = ?5",
                        params![blob, target_model, target_version, Utc::now().to_rfc3339(), id_str],
                    ).map_err(|e| StrataError::Database(e.to_string()))?;
                    report.facts_reindexed += rows_affected;
                }
                tx.commit().map_err(|e| StrataError::Database(e.to_string()))?;
            }
        }

        // 3. Reindex procedural_skills
        let candidate_skills = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
            let mut sql = "SELECT id, name, description FROM procedural_skills WHERE 1=1".to_string();
            if !force_all {
                sql.push_str(" AND (embedding IS NULL OR embedding_model IS NULL OR embedding_model != ?1)");
            }
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows: Vec<(String, String, String)> = if force_all {
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            } else {
                stmt.query_map(params![target_model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            };
            rows
        };

        for chunk in candidate_skills.chunks(bsize) {
            let texts: Vec<String> = chunk.iter().map(|(_, name, desc)| format!("{name}: {desc}")).collect();
            let vectors = embedder.embed_batch(&texts).await?;
            {
                let mut conn = self
                    .conn
                    .lock()
                    .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                for ((id_str, _, _), vec) in chunk.iter().zip(vectors.into_iter()) {
                    let blob = crate::embedding::embedding_to_bytes(&vec);
                    let rows_affected = tx.execute(
                        "UPDATE procedural_skills SET embedding = ?1, embedding_model = ?2, embedding_version = ?3 WHERE id = ?4",
                        params![blob, target_model, target_version, id_str],
                    ).map_err(|e| StrataError::Database(e.to_string()))?;
                    report.skills_reindexed += rows_affected;
                }
                tx.commit().map_err(|e| StrataError::Database(e.to_string()))?;
            }
        }

        // 4. Reindex episodic_memories
        let candidate_episodes = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
            let mut sql = "SELECT id, summary FROM episodic_memories WHERE 1=1".to_string();
            if !force_all {
                sql.push_str(" AND (embedding IS NULL OR embedding_model IS NULL OR embedding_model != ?1)");
            }
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StrataError::Database(e.to_string()))?;
            let rows: Vec<(String, String)> = if force_all {
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            } else {
                stmt.query_map(params![target_model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| StrataError::Database(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| StrataError::Database(e.to_string()))?
            };
            rows
        };

        for chunk in candidate_episodes.chunks(bsize) {
            let texts: Vec<String> = chunk.iter().map(|(_, summary)| summary.clone()).collect();
            let vectors = embedder.embed_batch(&texts).await?;
            {
                let mut conn = self
                    .conn
                    .lock()
                    .map_err(|_| StrataError::Database("Lock poisoned on SQLite connection".to_string()))?;
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(|e| StrataError::Database(e.to_string()))?;
                for ((id_str, _), vec) in chunk.iter().zip(vectors.into_iter()) {
                    let blob = crate::embedding::embedding_to_bytes(&vec);
                    let rows_affected = tx.execute(
                        "UPDATE episodic_memories SET embedding = ?1, embedding_model = ?2, embedding_version = ?3 WHERE id = ?4",
                        params![blob, target_model, target_version, id_str],
                    ).map_err(|e| StrataError::Database(e.to_string()))?;
                    report.episodes_reindexed += rows_affected;
                }
                tx.commit().map_err(|e| StrataError::Database(e.to_string()))?;
            }
        }

        report.total_reindexed = report.memories_reindexed + report.facts_reindexed + report.skills_reindexed + report.episodes_reindexed;
        report.duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(report)
    }
}

/// Vector consistency and model distribution summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VectorConsistencyStats {
    pub total_memories: usize,
    pub memories_with_embeddings: usize,
    pub total_semantic_facts: usize,
    pub facts_with_embeddings: usize,
    pub total_procedural_skills: usize,
    pub skills_with_embeddings: usize,
    pub total_episodic_memories: usize,
    pub episodes_with_embeddings: usize,
    pub total_vectors: usize,
    pub model_breakdown: Vec<ModelVectorCount>,
    pub distinct_models_count: usize,
    pub is_consistent: bool,
    pub active_model: Option<String>,
    pub recommendations: Vec<String>,
}

/// Model vector count breakdown.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelVectorCount {
    pub table: String,
    pub model: String,
    pub version: String,
    pub count: usize,
}

/// Report returned by `reindex_embeddings`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReindexReport {
    pub target_model: String,
    pub target_version: String,
    pub dimension: usize,
    pub memories_reindexed: usize,
    pub facts_reindexed: usize,
    pub skills_reindexed: usize,
    pub episodes_reindexed: usize,
    pub total_reindexed: usize,
    pub total_skipped: usize,
    pub duration_ms: f64,
}

fn map_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    let task_id: String = row.get(0)?;
    let plan_id: Option<String> = row.get(1)?;
    let parent_id: Option<String> = row.get(2)?;
    let title: String = row.get(3)?;
    let description: Option<String> = row.get(4)?;
    let status_str: String = row.get(5)?;
    let assigned_agent: Option<String> = row.get(6)?;
    let req_res_str: String = row.get(7)?;
    let deps_str: String = row.get(8)?;
    let outputs_str: Option<String> = row.get(9)?;
    let created_at: i64 = row.get(10)?;
    let updated_at: i64 = row.get(11)?;
    let claim_generation: i64 = row.get(12).unwrap_or(0);
    let payload_str: Option<String> = row.get(13).ok();

    let status = status_str.parse::<TaskStatus>().unwrap_or(TaskStatus::Pending);
    let required_resources: Vec<String> = serde_json::from_str(&req_res_str).unwrap_or_default();
    let dependencies: Vec<String> = serde_json::from_str(&deps_str).unwrap_or_default();
    let outputs: Option<serde_json::Value> = outputs_str.and_then(|s| serde_json::from_str(&s).ok());
    let payload: Option<TaskPayload> = payload_str.and_then(|s| serde_json::from_str(&s).ok());

    Ok(TaskRecord {
        task_id,
        plan_id,
        parent_id,
        title,
        description,
        status,
        assigned_agent,
        required_resources,
        dependencies,
        outputs,
        created_at,
        updated_at,
        claim_generation,
        payload,
    })
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
