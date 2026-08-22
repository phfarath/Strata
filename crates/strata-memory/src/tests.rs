use chrono::{Duration, Utc};
use uuid::Uuid;

use strata_core::events::{
    Event, EventPayload, ObservationReceived, SessionEnded, SessionStarted, TaskCompleted,
    TaskStarted, ToolInvoked, ToolResultReceived,
};
use strata_core::schemas::{
    CodeAnchor, ContextBudgetConfig, DecayConfig, EpisodicMemory, ExportFormat, FactStatus, FeedbackEvent,
    FeedbackRating, HostTargetConfig, ImplicitSignal, MemoryFeedback, ParameterDef,
    PreferencePair, ProceduralExample, ProceduralSkill, ProceduralStep, SemanticFact,
    SftSample, SignalKind, SignalScores, SymbolType, SyncConfig, SyncDelta,
};

use strata_core::state::{
    FailureSeverity, MemoryRecord, MemoryTier, MemoryType, Scope,
};
use strata_core::traits::{EventStore, MemoryEngine};

use crate::alignment::PreferenceMiner;
use crate::ast::{AstParser, CodeAnchorEngine};
use crate::compiler::MultiHostCompiler;
use crate::decay::DecayCalculator;
use crate::embedding::{
    bytes_to_embedding, cosine_similarity, embedding_to_bytes, EmbeddingProvider,
    MockEmbeddingProvider,
};
use crate::jtms::{ConflictResolution, TruthMaintenanceSystem};
use crate::pipeline::ConsolidationPipeline;
use crate::store::SqliteStore;
use crate::workspace::{MonorepoPackage, PackageType, WorkspaceBoundary};
use crate::sync::{compute_version_hash, SyncEngine};
use crate::SqliteMemoryEngine;
use std::sync::Arc;

#[tokio::test]
async fn test_memory_crud_and_access_metrics() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("init engine");

    let record = MemoryRecord::new(
        MemoryType::Semantic,
        "Rust ownership model guarantees thread safety and prevents data races.",
        Scope::Project("strata".to_string()),
    )
    .with_summary("Rust ownership model")
    .with_importance(0.9)
    .with_tags(vec!["rust".to_string(), "concurrency".to_string()]);

    let handle = engine.write(&record).await.expect("write memory");
    assert_eq!(handle.id, record.id);
    assert_eq!(handle.title, "Rust ownership model");

    // Fetch memory
    let fetched = engine.get(&record.id).await.expect("get memory").expect("found");
    assert_eq!(fetched.id, record.id);
    assert_eq!(fetched.access_count, 1);
    assert!(fetched.last_accessed_at.is_some());
    assert!(fetched.embedding.is_some(), "Embedding should be auto-computed");

    // Second get increments access count
    let fetched_again = engine.get(&record.id).await.expect("get memory 2").expect("found");
    assert_eq!(fetched_again.access_count, 2);
}

#[tokio::test]
async fn test_fts5_full_text_search() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("init engine");

    let mem1 = MemoryRecord::new(
        MemoryType::Semantic,
        "SQLite with FTS5 enables high-performance full-text search and BM25 ranking.",
        Scope::Global,
    )
    .with_summary("SQLite FTS5")
    .with_tags(vec!["database".to_string(), "fts".to_string()]);

    let mem2 = MemoryRecord::new(
        MemoryType::Procedural,
        "Always execute database migrations inside transactions in WAL mode.",
        Scope::Global,
    )
    .with_summary("Database migrations")
    .with_tags(vec!["database".to_string(), "ops".to_string()]);

    let mem3 = MemoryRecord::new(
        MemoryType::Episodic,
        "Agent finished task to optimize neural network inference pipeline.",
        Scope::Global,
    )
    .with_summary("Neural network optimization")
    .with_tags(vec!["ml".to_string()]);

    engine.write(&mem1).await.expect("write mem1");
    engine.write(&mem2).await.expect("write mem2");
    engine.write(&mem3).await.expect("write mem3");

    // FTS query on "ranking" matches "rank" stem
    let results = engine.store().search_fts("ranking", None, 10).expect("fts query");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.id, mem1.id);

    // FTS query on "database" matches mem1 and mem2
    let results_db = engine.store().search_fts("database", None, 10).expect("fts query db");
    assert_eq!(results_db.len(), 2);
}

#[tokio::test]
async fn test_cosine_similarity_and_byte_serialization() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    let c = vec![0.0, 1.0, 0.0];
    let d = vec![-1.0, 0.0, 0.0];

    assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-5);
    assert!((cosine_similarity(&a, &c) - 0.0).abs() < 1e-5);
    assert!((cosine_similarity(&a, &d) - (-1.0)).abs() < 1e-5);

    // Serialization
    let raw = vec![0.123f32, -0.456, 0.789, 1.0];
    let bytes = embedding_to_bytes(&raw);
    assert_eq!(bytes.len(), 16);
    let recovered = bytes_to_embedding(&bytes).expect("recover embedding");
    assert_eq!(raw, recovered);
}

#[tokio::test]
async fn test_hybrid_retrieval_rrf() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("init engine");

    let mem1 = MemoryRecord::new(
        MemoryType::Semantic,
        "Reciprocal Rank Fusion RRF combines rankings from distinct information retrieval algorithms.",
        Scope::Project("strata".to_string()),
    )
    .with_importance(0.95);

    let mem2 = MemoryRecord::new(
        MemoryType::Semantic,
        "BM25 is a probabilistic relevance framework widely used in search engines.",
        Scope::Project("strata".to_string()),
    )
    .with_importance(0.8);

    let mem3 = MemoryRecord::new(
        MemoryType::Semantic,
        "Cooking pasta requires boiling water with salt.",
        Scope::Project("cooking".to_string()),
    );

    engine.write(&mem1).await.expect("write 1");
    engine.write(&mem2).await.expect("write 2");
    engine.write(&mem3).await.expect("write 3");

    let results = engine
        .search("Rank Fusion algorithms", Some(&Scope::Project("strata".to_string())), 5)
        .await
        .expect("search");

    assert!(!results.is_empty());
    assert_eq!(results[0].id, mem1.id);
}

#[tokio::test]
async fn test_silent_failure_recording_and_alerting() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("init engine");

    // Record tool failure 1
    let fail1 = engine
        .record_tool_failure(
            "http_fetch",
            "Connection timed out after 30000ms connecting to api.internal",
            "Fetching upstream endpoint",
            Some(&Scope::Global),
        )
        .await
        .expect("record failure 1");

    assert_eq!(fail1.occurrences, 1);
    assert_eq!(fail1.severity, FailureSeverity::High);
    assert!(fail1.mitigation.contains("timeout"));

    // Record same tool failure pattern again (simulating recurring silent error)
    let fail2 = engine
        .record_tool_failure(
            "http_fetch",
            "Connection timed out after 30000ms connecting to api.internal",
            "Retry 2",
            Some(&Scope::Global),
        )
        .await
        .expect("record failure 2");

    assert_eq!(fail2.signature, fail1.signature);
    assert_eq!(fail2.occurrences, 2);

    // Query known failures
    let known = engine
        .get_known_failures(Some("http_fetch"), None, 5)
        .await
        .expect("get known failures");

    assert_eq!(known.len(), 1);
    assert_eq!(known[0].signature, fail1.signature);
    assert_eq!(known[0].occurrences, 2);
}

#[tokio::test]
async fn test_event_store_and_stream_read() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("init engine");
    let session_id = "sess-alpha-1";

    let ev1 = Event::new(
        session_id,
        "agent-1",
        EventPayload::SessionStarted(SessionStarted {
            session_id: session_id.to_string(),
            agent_id: "agent-1".to_string(),
            organization_id: None,
            environment: serde_json::json!({"env": "test"}),
            timestamp: Utc::now(),
        }),
    );

    let ev2 = Event::new(
        session_id,
        "agent-1",
        EventPayload::TaskCompleted(TaskCompleted {
            task_id: "task-01".to_string(),
            success: true,
            outcome_summary: "Compiled crates successfully".to_string(),
            evaluation: None,
            timestamp: Utc::now(),
        }),
    );

    let id1 = engine.append(&ev1).await.expect("append ev1");
    let id2 = engine.append(&ev2).await.expect("append ev2");

    let stream = engine
        .read_stream(session_id, None, None)
        .await
        .expect("read stream");

    assert_eq!(stream.len(), 2);
    assert_eq!(stream[0].id, id1);
    assert_eq!(stream[1].id, id2);
}

#[tokio::test]
async fn test_digest_generation() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("init engine");
    let session_id = "sess-digest-test";

    // Append events
    let ev1 = Event::new(
        session_id,
        "agent-x",
        EventPayload::TaskCompleted(TaskCompleted {
            task_id: "init-db".to_string(),
            success: true,
            outcome_summary: "SQLite schema migrated and FTS5 indices created".to_string(),
            evaluation: None,
            timestamp: Utc::now(),
        }),
    );
    engine.append(&ev1).await.expect("append ev1");

    // Record a failure in session
    let _ = engine
        .record_tool_failure(
            "file_writer",
            "Permission denied writing to /root/config",
            "Configuration setup",
            Some(&Scope::Session(session_id.to_string())),
        )
        .await
        .expect("record tool failure");

    // Write a memory
    let mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Working digest aggregates decisions, handles, and failure alerts in under 500 tokens.",
        Scope::Session(session_id.to_string()),
    )
    .with_summary("Digest Specification");
    engine.write(&mem).await.expect("write memory");

    // Generate digest
    let digest = engine.digest(session_id, Some(500)).await.expect("digest");

    assert_eq!(digest.session_id, session_id);
    assert!(!digest.summary.is_empty());
    assert!(!digest.recent_decisions.is_empty());
    assert!(!digest.key_pointers.is_empty());
    assert!(!digest.failure_warnings.is_empty());
    assert!(digest.estimated_tokens > 0 && digest.estimated_tokens <= 500);
}

#[test]
fn test_mathematical_decay_act_r_and_ebbinghaus() {
    let config = DecayConfig {
        alpha: 1.0,
        beta: 0.5,
        gamma: 0.5,
        d: 0.5,
        s0: 24.0,
        lambda: 0.1,
        mu: 0.2,
        prune_threshold: 0.05,
        invariant_threshold: 0.95,
    };
    let calculator = DecayCalculator::new(config);

    // 1. ACT-R power-law verification
    // t = [1.0, 2.0, 4.0]
    // 1^(-0.5) = 1.0, 2^(-0.5) = 0.70710678, 4^(-0.5) = 0.5
    // sum = 2.20710678, ln(sum) = 0.7916823
    // alpha * ln(sum) + beta * I_m + gamma * C_m = 0.7916823 + 0.5*0.8 + 0.5*1.0 = 1.6916823
    let elapsed = vec![1.0, 2.0, 4.0];
    let act_r = calculator.calculate_act_r_activation(&elapsed, 0.8, 1.0);
    assert!((act_r - 1.69168).abs() < 1e-3, "ACT-R activation was {}", act_r);

    // 2. Stability calculation: S_m = S_0 * (1 + lambda * ln(u+1) + mu * I_m)
    // S_0 = 24, lambda = 0.1, u = 3 (ln(4) = 1.386294), mu = 0.2, I_m = 0.5
    // S_m = 24 * (1 + 0.1386294 + 0.1) = 24 * 1.2386294 = 29.7271
    let stability = calculator.calculate_stability(3, 0.5);
    assert!((stability - 29.7271).abs() < 1e-3, "Stability was {}", stability);

    // 3. Ebbinghaus retention: R(t) = exp(-t / S_m)
    // For t = 29.7271 and S_m = 29.7271 => R(t) = exp(-1) = 0.367879
    let retention = calculator.calculate_ebbinghaus_retention(stability, stability);
    assert!((retention - 0.367879).abs() < 1e-3, "Retention was {}", retention);

    // 4. Invariant memory retains 1.0 and never expires
    let now = Utc::now();
    let metrics_invariant = calculator.evaluate_decay(0.98, 1.0, now - Duration::days(365), &[], now);
    assert_eq!(metrics_invariant.retention, 1.0);
    assert!(!metrics_invariant.is_expired);

    // 5. Expired memory evaluation
    let metrics_expired = calculator.evaluate_decay(0.1, 0.5, now - Duration::days(100), &[], now);
    assert!(metrics_expired.retention < 0.05);
    assert!(metrics_expired.is_expired);
}

#[tokio::test]
async fn test_jtms_belief_revision_and_status_transitions() {
    let store = SqliteStore::open_in_memory().expect("open sqlite in memory");
    let jtms = TruthMaintenanceSystem::with_default_threshold();
    let embedder = MockEmbeddingProvider::default();

    // 1. Insert initial active fact
    let statement_1 = "PostgreSQL is the designated primary relational database for production.";
    let fact1 = SemanticFact::new(statement_1, "architecture", Scope::Global)
        .with_importance(0.8)
        .with_confidence(0.9);
    let emb1 = embedder.embed_text(statement_1).await.expect("embed 1");
    store.insert_or_update_semantic_fact(&fact1).expect("insert fact 1");
    store.update_semantic_fact_embedding(&fact1.id, &emb1).expect("embed fact 1");

    assert_eq!(fact1.status, FactStatus::Active);
    assert_eq!(fact1.version, 1);
    assert!(fact1.replaced_by.is_none());

    // 2. Insert contradictory newer fact containing lexical negation / opposition
    let statement_2 = "PostgreSQL is deprecated and MySQL is now the primary relational database.";
    let mut fact2 = SemanticFact::new(statement_2, "architecture", Scope::Global)
        .with_importance(0.9)
        .with_confidence(0.95);
    let emb2 = embedder.embed_text(statement_2).await.expect("embed 2");

    // Check lexical contradiction
    let (is_contra, cues) = jtms.detect_lexical_contradiction(statement_1, statement_2);
    assert!(is_contra);
    assert!(!cues.is_empty());

    // Resolve and upsert fact2
    let conflicts = jtms.resolve_and_upsert(&store, &mut fact2, &emb2).expect("resolve conflicts");
    assert!(!conflicts.is_empty(), "A conflict should have been detected");
    assert_eq!(conflicts[0].existing_fact_id, fact1.id);

    // Verify fact1 is now Deprecated and points to fact2
    let fact1_updated = store.get_semantic_fact(&fact1.id).expect("get fact 1").expect("found");
    assert_eq!(fact1_updated.status, FactStatus::Deprecated);
    assert_eq!(fact1_updated.replaced_by, Some(fact2.id));

    // Verify fact2 is now Active and has incremented version
    let fact2_updated = store.get_semantic_fact(&fact2.id).expect("get fact 2").expect("found");
    assert_eq!(fact2_updated.status, FactStatus::Active);
    assert_eq!(fact2_updated.version, 2);

    // 3. Test Reject resolution
    let statement_3 = "MySQL is unsupported and should not be used.";
    let mut fact3 = SemanticFact::new(statement_3, "architecture", Scope::Global);
    jtms.apply_belief_update(&store, &fact2.id, &mut fact3, ConflictResolution::Reject).expect("reject fact 3");

    let fact3_updated = store.get_semantic_fact(&fact3.id).expect("get fact 3").expect("found");
    assert_eq!(fact3_updated.status, FactStatus::Deprecated);
    assert_eq!(fact3_updated.replaced_by, Some(fact2.id));
}

#[tokio::test]
async fn test_full_consolidation_pipeline_execution() {
    let store = SqliteStore::open_in_memory().expect("open in memory store");
    let embedder = MockEmbeddingProvider::default();
    let pipeline = ConsolidationPipeline::with_default_config();

    let session_id = "sess-consolidation-p2";
    let now = Utc::now();

    let events = vec![
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::SessionStarted(SessionStarted {
                session_id: session_id.to_string(),
                agent_id: "coder-agent".to_string(),
                organization_id: None,
                environment: serde_json::json!({"os": "windows"}),
                timestamp: now,
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::TaskStarted(TaskStarted {
                task_id: "t1".to_string(),
                title: "Refactor SQLite migration routines".to_string(),
                description: Some("Refactor SQLite migration routines".to_string()),
                parent_task_id: None,
                session_id: session_id.to_string(),
                timestamp: now + Duration::seconds(1),
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::ToolInvoked(ToolInvoked {
                invocation_id: Uuid::new_v4(),
                tool_name: "run_migrations".to_string(),
                input: serde_json::json!({"path": "crates/strata-memory/migrations"}),
                session_id: session_id.to_string(),
                timestamp: now + Duration::seconds(2),
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::ToolResultReceived(ToolResultReceived {
                invocation_id: Uuid::new_v4(),
                tool_name: "run_migrations".to_string(),
                result: serde_json::json!({"migrated": 4}),
                is_error: false,
                duration_ms: Some(120),
                timestamp: now + Duration::seconds(3),
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::ToolInvoked(ToolInvoked {
                invocation_id: Uuid::new_v4(),
                tool_name: "verify_indices".to_string(),
                input: serde_json::json!({"tables": ["episodic_memories", "semantic_facts"]}),
                session_id: session_id.to_string(),
                timestamp: now + Duration::seconds(4),
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::ToolResultReceived(ToolResultReceived {
                invocation_id: Uuid::new_v4(),
                tool_name: "verify_indices".to_string(),
                result: serde_json::json!({"indices_valid": true}),
                is_error: false,
                duration_ms: Some(45),
                timestamp: now + Duration::seconds(5),
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::ObservationReceived(ObservationReceived {
                session_id: session_id.to_string(),
                source: "sqlite_engine".to_string(),
                observation_type: "database_optimization".to_string(),
                content: serde_json::json!("WAL mode with synchronous NORMAL ensures ACID safety and 5x write speed."),
                timestamp: now + Duration::seconds(6),
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::TaskCompleted(TaskCompleted {
                task_id: "t1".to_string(),
                success: true,
                outcome_summary: "SQLite migration completed successfully with full index validation.".to_string(),
                evaluation: None,
                timestamp: now + Duration::seconds(7),
            }),
        ),
        Event::new(
            session_id,
            "coder-agent",
            EventPayload::SessionEnded(SessionEnded {
                session_id: session_id.to_string(),
                agent_id: "coder-agent".to_string(),
                final_state: Some("Completed".to_string()),
                reason: Some("Task finished".to_string()),
                summary: Some("Consolidated SQLite migrations and index verification pipeline.".to_string()),
                timestamp: now + Duration::seconds(8),
            }),
        ),
    ];

    let result = pipeline
        .run_pipeline(&store, &embedder, &events, None)
        .await
        .expect("run consolidation pipeline");

    assert_eq!(result.events_processed, events.len());
    assert_eq!(result.episodic_memories.len(), 1);
    assert!(!result.semantic_facts.is_empty());
    assert!(!result.procedural_skills.is_empty());

    let ep = &result.episodic_memories[0];
    assert_eq!(ep.session_id, session_id);
    assert_eq!(ep.tools_used.len(), 2);
    assert_eq!(ep.signals.success, 1.0);

    // Verify stored items can be retrieved
    let retrieved_ep = store.get_episodic_memory(&ep.id).expect("get episodic").expect("found");
    assert_eq!(retrieved_ep.id, ep.id);

    let all_facts = store.get_all_semantic_facts(None, Some(FactStatus::Active), 10).expect("get facts");
    assert!(!all_facts.is_empty());

    let all_skills = store.get_all_procedural_skills(None, 10).expect("get skills");
    assert!(!all_skills.is_empty());
    assert_eq!(all_skills[0].steps.len(), 2);
}

#[tokio::test]
async fn test_phase2_store_crud_and_access_logs() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let now = Utc::now();

    // 1. Episodic Memory CRUD
    let ep = EpisodicMemory::new(
        "sess-store-test",
        "agent-1",
        "Tested episodic CRUD",
        now,
        now,
    )
    .with_project("strata")
    .with_tools(vec!["cargo_test".to_string()])
    .with_signals(SignalScores::default());

    store.insert_episodic_memory(&ep).expect("insert episodic");
    let fetched_ep = store.get_episodic_memory(&ep.id).expect("get episodic").expect("found");
    assert_eq!(fetched_ep.summary, "Tested episodic CRUD");

    let fts_ep = store.search_episodic_memories_fts("episodic", 10).expect("search episodic fts");
    assert_eq!(fts_ep.len(), 1);
    assert_eq!(fts_ep[0].0.id, ep.id);

    // 2. Semantic Fact CRUD
    let fact = SemanticFact::new("Memory access logs track recency", "telemetry", Scope::Global)
        .with_importance(0.85);
    store.insert_or_update_semantic_fact(&fact).expect("insert fact");
    let fetched_fact = store.get_semantic_fact(&fact.id).expect("get fact").expect("found");
    assert_eq!(fetched_fact.statement, "Memory access logs track recency");

    let fts_facts = store.search_semantic_facts_fts("recency", 10).expect("search fact fts");
    assert_eq!(fts_facts.len(), 1);
    assert_eq!(fts_facts[0].0.id, fact.id);

    // 3. Procedural Skill CRUD
    let mut skill = ProceduralSkill::new("test_suite", "Run all automated tests")
        .with_steps(vec![ProceduralStep::new(
            1,
            "cargo",
            "test",
            serde_json::json!({"workspace": true}),
        )]);
    skill.record_usage(true);
    store.insert_or_update_procedural_skill(&skill).expect("insert skill");

    let fetched_skill = store.get_procedural_skill(&skill.id).expect("get skill").expect("found");
    assert_eq!(fetched_skill.name, "test_suite");
    assert_eq!(fetched_skill.usage_count, 1);

    let fetched_by_name = store.get_procedural_skill_by_name("test_suite").expect("get by name").expect("found");
    assert_eq!(fetched_by_name.id, skill.id);

    // 4. Memory Access Logs
    let access_count = store.get_memory_access_count(&fact.id).expect("get access count");
    assert!(access_count >= 1, "Access count should be >= 1 after get_semantic_fact");

    let logs = store.get_memory_access_logs(&fact.id).expect("get logs");
    assert!(!logs.is_empty());
}

#[tokio::test]
async fn test_cdc_outbox_operations() {
    let store = SqliteStore::open_in_memory().expect("init store");
    let ws = "ws-alpha";

    let delta1 = SyncDelta::new(
        ws,
        1,
        "event",
        serde_json::json!({"action": "start"}),
        "hash-001",
    );
    let delta2 = SyncDelta::new(
        ws,
        2,
        "fact",
        serde_json::json!({"statement": "Atomic facts"}),
        "hash-002",
    );

    store.enqueue_delta(&delta1).expect("enqueue delta 1");
    store.enqueue_delta(&delta2).expect("enqueue delta 2");

    // Check status
    let (pending_count, max_seq) = store.get_sync_status(ws).expect("get sync status");
    assert_eq!(pending_count, 2);
    assert_eq!(max_seq, 2);

    // Get pending deltas
    let pending = store.get_pending_deltas(ws, 10).expect("get pending");
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].seq, 1);
    assert_eq!(pending[1].seq, 2);

    // Mark delta1 synced
    store.mark_deltas_synced(&[delta1.id]).expect("mark synced");

    let (pending_count_after, _) = store.get_sync_status(ws).expect("get sync status");
    assert_eq!(pending_count_after, 1);

    let pending_after = store.get_pending_deltas(ws, 10).expect("get pending after");
    assert_eq!(pending_after.len(), 1);
    assert_eq!(pending_after[0].id, delta2.id);
}

#[tokio::test]
async fn test_sync_metadata_and_delta_failure() {
    let store = SqliteStore::open_in_memory().expect("init store");
    let ws = "ws-beta";

    let delta = SyncDelta::new(
        ws,
        10,
        "skill",
        serde_json::json!({"name": "deploy"}),
        "hash-010",
    );
    store.enqueue_delta(&delta).expect("enqueue");

    // Record failure
    store
        .record_delta_failure(&[delta.id], "Network timeout")
        .expect("record failure");

    // Test metadata operations
    store.set_sync_metadata("last_remote_seq", "42").expect("set meta");
    let val = store.get_sync_metadata("last_remote_seq").expect("get meta");
    assert_eq!(val.as_deref(), Some("42"));
}

#[tokio::test]
async fn test_memory_feedback_adjustment() {
    let store = SqliteStore::open_in_memory().expect("init store");

    // 1. Create a memory record
    let mut mem = MemoryRecord::new(MemoryType::Semantic, "Test statement for feedback", Scope::Global);
    mem.importance = 0.5;
    mem.confidence = 0.5;
    store.insert_or_update_memory(&mem).expect("insert mem");

    // 2. Positive feedback reinforces importance & confidence
    let fb_pos = MemoryFeedback::positive(mem.id);
    store.record_memory_feedback(&fb_pos).expect("record positive fb");

    let updated_mem = store.get_memory(&mem.id).expect("get mem").expect("found");
    assert!(updated_mem.importance > 0.5);
    assert!(updated_mem.confidence > 0.5);

    // 3. Negative feedback reduces importance & confidence
    let fb_neg = MemoryFeedback::negative(mem.id, Some("Incorrect assumption".to_string()));
    store.record_memory_feedback(&fb_neg).expect("record negative fb");

    let updated_neg = store.get_memory(&mem.id).expect("get mem").expect("found");
    assert!(updated_neg.importance < updated_mem.importance);

    // 4. Access log recorded
    let logs = store.get_memory_access_logs(&mem.id).expect("get logs");
    assert!(logs.len() >= 2);
}

#[tokio::test]
async fn test_sync_engine_push_pull_and_cycle() {
    let store = Arc::new(SqliteStore::open_in_memory().expect("init store"));
    let config = SyncConfig::new("ws-cycle").with_batch_size(10);
    let engine = SyncEngine::new(Arc::clone(&store), config);

    // 1. Enqueue local deltas
    let delta = SyncDelta::new(
        "ws-cycle",
        1,
        "memory",
        serde_json::to_value(MemoryRecord::new(
            MemoryType::Semantic,
            "Offline local fact",
            Scope::Global,
        ))
        .expect("to value"),
        "hash-local",
    );
    store.enqueue_delta(&delta).expect("enqueue delta");

    // 2. Push in offline mode (endpoint is None)
    let pushed = engine.push_deltas().await.expect("push deltas");
    assert_eq!(pushed, 1);

    let (pending_count, _) = store.get_sync_status("ws-cycle").expect("status");
    assert_eq!(pending_count, 0);

    // 3. Pull incoming remote deltas
    let remote_fact = SemanticFact::new("SQLite WAL ensures concurrent readers", "db", Scope::Global);
    let remote_fact_payload = serde_json::to_value(&remote_fact).expect("serialize fact");
    let remote_hash = compute_version_hash(&remote_fact_payload);

    let remote_delta = SyncDelta::new(
        "ws-cycle",
        5,
        "semantic_fact",
        remote_fact_payload,
        remote_hash,
    );

    let pulled = engine.pull_deltas(vec![remote_delta]).await.expect("pull deltas");
    assert_eq!(pulled, 1);

    let fetched_fact = store.get_semantic_fact(&remote_fact.id).expect("get fact").expect("found");
    assert_eq!(fetched_fact.statement, "SQLite WAL ensures concurrent readers");

    // 4. Full sync cycle
    let report = engine.sync_cycle().await.expect("sync cycle");
    assert_eq!(report.last_seq, 5);
}

#[tokio::test]
async fn test_sync_engine_jtms_conflict_resolution() {
    let store = Arc::new(SqliteStore::open_in_memory().expect("init store"));
    let config = SyncConfig::new("ws-jtms");
    let engine = SyncEngine::new(Arc::clone(&store), config);

    // Local fact
    let fact_id = Uuid::new_v4();
    let mut local_fact = SemanticFact::new("Always enable debug logging in production", "logging", Scope::Global);
    local_fact.id = fact_id;
    local_fact.version = 1;
    store.insert_or_update_semantic_fact(&local_fact).expect("insert local");

    // Remote divergent delta with opposite statement
    let mut remote_fact = SemanticFact::new("Always disable debug logging in production", "logging", Scope::Global);
    remote_fact.id = fact_id;
    let remote_payload = serde_json::to_value(&remote_fact).expect("serialize");
    let remote_hash = compute_version_hash(&remote_payload);

    let delta = SyncDelta::new("ws-jtms", 10, "fact", remote_payload, remote_hash);

    // Pull delta should trigger JTMS supersede
    let pulled = engine.pull_deltas(vec![delta]).await.expect("pull delta");
    assert_eq!(pulled, 1);

    let updated_fact = store.get_semantic_fact(&fact_id).expect("get fact").expect("found");
    assert_eq!(updated_fact.status, FactStatus::Active);
    assert_eq!(updated_fact.version, 2);
    assert_eq!(updated_fact.statement, "Always disable debug logging in production");
}

#[tokio::test]
async fn test_implicit_signals_and_feedback_recording() {
    let store = Arc::new(SqliteStore::open_in_memory().expect("init store"));

    // 1. Record implicit signals
    let sig1 = ImplicitSignal::new(SignalKind::ToolLoop, "sess-track3", "agent-001")
        .with_tool_name("file_search")
        .with_file_path("crates/strata-memory/src/lib.rs")
        .with_extra("Repeated call 3 times");
    let sig2 = ImplicitSignal::new(SignalKind::TestRerunSuccess, "sess-track3", "agent-001")
        .with_extra("All 10 tests passed");

    store.record_implicit_signal(&sig1).expect("record sig1");
    store.record_implicit_signal(&sig2).expect("record sig2");

    let signals = store.get_implicit_signals(Some("sess-track3")).expect("get signals");
    assert_eq!(signals.len(), 2);
    assert_eq!(signals[0].kind, SignalKind::ToolLoop);
    assert_eq!(signals[1].kind, SignalKind::TestRerunSuccess);

    // 2. Memory Record and Feedback Events
    let mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Use WAL mode for high concurrency in SQLite",
        Scope::Global,
    )
    .with_importance(0.6)
    .with_confidence(0.8);
    store.insert_or_update_memory(&mem).expect("insert mem");

    let fb1 = FeedbackEvent::new(FeedbackRating::Positive, "user_chat")
        .with_memory_id(mem.id)
        .with_comment("Extremely helpful hint");
    let fb2 = FeedbackEvent::new(FeedbackRating::Negative, "telemetry")
        .with_signal_id(sig1.id)
        .with_comment("Agent got stuck in loop");

    store.record_feedback_event(&fb1).expect("record fb1");
    store.record_feedback_event(&fb2).expect("record fb2");

    let mem_feedback = store.get_feedback_events_for_memory(&mem.id).expect("get fb for mem");
    assert_eq!(mem_feedback.len(), 1);
    assert_eq!(mem_feedback[0].rating, FeedbackRating::Positive);

    // Verify memory importance adjusted (+0.1)
    let fetched_mem = store.get_memory(&mem.id).expect("get mem").expect("found");
    assert!((fetched_mem.importance - 0.7).abs() < 1e-4);

    // 3. Preference Pairs
    let pair = PreferencePair::new(
        "Compile crates with warnings treated as errors",
        "cargo clippy --all-targets -- -D warnings",
        "cargo check",
        "sess-track3",
    );
    store.record_preference_pair(&pair).expect("record pair");

    let pairs = store.get_preference_pairs(Some("sess-track3")).expect("get pairs");
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].prompt, "Compile crates with warnings treated as errors");
    assert_eq!(pairs[0].chosen, "cargo clippy --all-targets -- -D warnings");
}

#[tokio::test]
async fn test_preference_miner_dpo_kto_sft_and_export() {
    use strata_core::state::FailurePattern;

    let store = Arc::new(SqliteStore::open_in_memory().expect("init store"));

    // 1. Seed Failure Pattern
    let failure = FailurePattern::new(
        "sig-sql-lock",
        "DatabaseLockContention",
        "Concurrent writers causing SQLite busy timeout",
        "Enable WAL mode and use exponential backoff retry",
    );
    store.upsert_failure_pattern(&failure).expect("upsert failure");

    // 2. Seed Episodic Memory
    let now = Utc::now();
    let mut ep = EpisodicMemory::new(
        "sess-miner",
        "agent-1",
        "Resolved SQLite deadlock issue",
        now,
        now,
    );
    ep.goals = vec!["Fix DB locks".to_string()];
    ep.obstacles = vec!["BusyTimeout on simultaneous write transactions".to_string()];
    ep.outcomes = vec!["Configured WAL journal mode with PRAGMA".to_string()];
    ep.signals = SignalScores {
        success: 0.95,
        frustration: 0.0,
        novelty: 0.8,
        importance: 0.9,
    };
    store.insert_episodic_memory(&ep).expect("insert episode");

    // 3. Seed Procedural Skill
    let mut skill = ProceduralSkill::new("enable_wal", "Configure SQLite in WAL mode");
    skill.preconditions = vec!["SQLite connection open".to_string()];
    skill.parameters = vec![ParameterDef::new("busy_timeout", "u32", "Timeout in ms")];
    skill.steps = vec![ProceduralStep::new(
        1,
        "sqlite",
        "pragma",
        serde_json::json!({"pragma": "journal_mode=WAL"}),
    )];
    skill.examples = vec![ProceduralExample::new("sess-miner", "WAL mode enabled successfully")];
    store.insert_or_update_procedural_skill(&skill).expect("insert skill");

    // 4. Seed Implicit Signal
    let sig_success = ImplicitSignal::new(SignalKind::TestRerunSuccess, "sess-miner", "agent-1")
        .with_extra("Unit tests passed after WAL enabled");
    store.record_implicit_signal(&sig_success).expect("record signal");

    // 5. Test PreferenceMiner
    let miner = PreferenceMiner::new(Arc::clone(&store));

    let dpo_pairs = miner.mine_dpo_pairs(Some("sess-miner")).expect("mine dpo");
    assert!(!dpo_pairs.is_empty());
    assert!(dpo_pairs.iter().any(|p| p.prompt.contains("DatabaseLockContention") || p.prompt.contains("Fix DB locks")));

    let kto_samples = miner.mine_kto_samples(Some("sess-miner")).expect("mine kto");
    assert!(!kto_samples.is_empty());
    assert!(kto_samples.iter().any(|s| s.label));

    let sft_samples = miner.mine_sft_samples().expect("mine sft");
    assert!(!sft_samples.is_empty());
    assert!(sft_samples.iter().any(|s| s.instruction.contains("enable_wal")));

    // 6. Test Exports
    let dpo_export = miner.export(ExportFormat::Dpo, Some("sess-miner")).expect("export dpo");
    assert!(!dpo_export.is_empty());
    assert!(dpo_export.contains("\"chosen\""));

    let kto_export = miner.export(ExportFormat::Kto, Some("sess-miner")).expect("export kto");
    assert!(!kto_export.is_empty());
    assert!(kto_export.contains("\"label\":true"));

    let sft_export = miner.export(ExportFormat::Sft, None).expect("export sft");
    assert!(!sft_export.is_empty());
    assert!(sft_export.contains("\"instruction\""));

    let md_export = miner.export(ExportFormat::Markdown, Some("sess-miner")).expect("export markdown");
    assert!(md_export.contains("# Strata Alignment & Preference Dataset"));
    assert!(md_export.contains("## DPO Preference Pairs"));

    let jsonl_export = miner.export(ExportFormat::Jsonl, Some("sess-miner")).expect("export jsonl");
    assert!(!jsonl_export.is_empty());
}

#[tokio::test]
async fn test_oracle_verified_gating_and_metadata() {
    use strata_core::state::FailurePattern;

    let store = Arc::new(SqliteStore::open_in_memory().expect("init store"));

    // 1. Explicit Preference Pairs: 1 Verified, 1 Unverified
    let verified_pair = PreferencePair::new(
        "Fix compiler lifetime error",
        "Add explicit 'a lifetime bounds on struct references",
        "Transmute to static reference using unsafe",
        "sess-oracle-1",
    )
    .with_verification(true, Some("cargo_check_oracle".to_string()));
    store.record_preference_pair(&verified_pair).expect("record verified pair");

    let unverified_pair = PreferencePair::new(
        "Suggest algorithm optimization",
        "Rewrite with custom unsafe SIMD assembly",
        "Keep scalar loop",
        "sess-oracle-1",
    )
    .with_verification(false, None::<String>);
    store.record_preference_pair(&unverified_pair).expect("record unverified pair");

    // Verify SQLite roundtrip preserves metadata
    let pairs_from_db = store.get_preference_pairs(Some("sess-oracle-1")).expect("get pairs");
    assert_eq!(pairs_from_db.len(), 2);
    let p_verified = pairs_from_db.iter().find(|p| p.oracle_verified).expect("find verified");
    assert_eq!(p_verified.verification_source.as_deref(), Some("cargo_check_oracle"));
    let p_unverified = pairs_from_db.iter().find(|p| !p.oracle_verified).expect("find unverified");
    assert_eq!(p_unverified.verification_source, None);

    // 2. Episodic Memories: 1 Verified (success = 0.95), 1 Unverified (success = 0.72)
    let now = Utc::now();
    let mut ep_high = EpisodicMemory::new(
        "sess-oracle-1",
        "agent-1",
        "Fix broken unit tests in memory engine",
        now,
        now,
    );
    ep_high.goals = vec!["Pass all unit tests".to_string()];
    ep_high.obstacles = vec!["Lock poisoned error in concurrent test".to_string()];
    ep_high.outcomes = vec!["Replaced Mutex with parking_lot RwLock".to_string()];
    ep_high.signals = SignalScores {
        success: 0.95,
        frustration: 0.0,
        novelty: 0.5,
        importance: 0.8,
    };
    store.insert_episodic_memory(&ep_high).expect("insert ep_high");

    let mut ep_med = EpisodicMemory::new(
        "sess-oracle-1",
        "agent-1",
        "Exploratory refactor of cache layer",
        now,
        now,
    );
    ep_med.goals = vec!["Explore cache strategies".to_string()];
    ep_med.obstacles = vec!["Memory usage slightly increased".to_string()];
    ep_med.outcomes = vec!["Partial LRU cache implemented".to_string()];
    ep_med.signals = SignalScores {
        success: 0.72,
        frustration: 0.1,
        novelty: 0.6,
        importance: 0.5,
    };
    store.insert_episodic_memory(&ep_med).expect("insert ep_med");

    // 3. Failure Patterns: 1 Verified (with mitigation), 1 Unverified (empty mitigation)
    let mut fp_verified = FailurePattern::new(
        "sig-borrow-mut",
        "BorrowMutCollision",
        "Multiple mutable borrows of same RefCell",
        "Scope mutable borrow with explicit block or use RefCell::try_borrow_mut",
    );
    fp_verified.trigger_condition = "Calling borrow_mut twice in same scope".to_string();
    store.upsert_failure_pattern(&fp_verified).expect("upsert fp_verified");

    let mut fp_unverified = FailurePattern::new(
        "sig-unknown-crash",
        "MysteriousCrash",
        "Unknown panic occurred in worker thread",
        "", // empty mitigation
    );
    fp_unverified.trigger_condition = "Thread panic".to_string();
    store.upsert_failure_pattern(&fp_unverified).expect("upsert fp_unverified");

    // 4. Procedural Skills: 1 Verified (success_rate = 0.95), 1 Unverified (success_rate = 0.50)
    let mut skill_verified = ProceduralSkill::new("fix_borrow_error", "Resolve RefCell borrow collisions");
    skill_verified.success_rate = 0.95;
    skill_verified.steps = vec![ProceduralStep::new(1, "rustc", "check", serde_json::json!({}))];
    skill_verified.examples = vec![ProceduralExample::new("sess-oracle-1", "Passed cargo test")];
    store.insert_or_update_procedural_skill(&skill_verified).expect("insert skill_verified");

    let mut skill_unverified = ProceduralSkill::new("experimental_jit", "Experimental JIT compiler pass");
    skill_unverified.success_rate = 0.50;
    skill_unverified.steps = vec![ProceduralStep::new(1, "jit", "compile", serde_json::json!({}))];
    store.insert_or_update_procedural_skill(&skill_unverified).expect("insert skill_unverified");

    // 5. Test PreferenceMiner Gating
    let miner = PreferenceMiner::new(Arc::clone(&store));

    // DPO Gating Test
    let all_dpo = miner.mine_dpo_pairs_filtered(Some("sess-oracle-1"), false).expect("mine all dpo");
    let verified_dpo = miner.mine_dpo_pairs_filtered(Some("sess-oracle-1"), true).expect("mine verified dpo");
    assert!(all_dpo.len() > verified_dpo.len(), "All DPO ({}) should be strictly greater than verified DPO ({})", all_dpo.len(), verified_dpo.len());
    assert!(verified_dpo.iter().all(|p| p.oracle_verified), "Every pair in verified_dpo must have oracle_verified == true");
    assert!(verified_dpo.iter().all(|p| p.verification_source.is_some()), "Every verified pair must have a verification_source");

    // KTO Gating Test
    let all_kto = miner.mine_kto_samples_filtered(Some("sess-oracle-1"), false).expect("mine all kto");
    let verified_kto = miner.mine_kto_samples_filtered(Some("sess-oracle-1"), true).expect("mine verified kto");
    assert!(all_kto.len() > verified_kto.len(), "All KTO ({}) should be greater than verified KTO ({})", all_kto.len(), verified_kto.len());
    assert!(verified_kto.iter().all(|s| s.oracle_verified), "Every sample in verified_kto must have oracle_verified == true");

    // SFT Gating Test
    let all_sft = miner.mine_sft_samples_filtered(false).expect("mine all sft");
    let verified_sft = miner.mine_sft_samples_filtered(true).expect("mine verified sft");
    assert!(all_sft.len() > verified_sft.len(), "All SFT ({}) should be greater than verified SFT ({})", all_sft.len(), verified_sft.len());
    assert!(verified_sft.iter().all(|s| s.oracle_verified), "Every sample in verified_sft must have oracle_verified == true");
    assert!(verified_sft.iter().any(|s| s.instruction.contains("fix_borrow_error")));
    assert!(!verified_sft.iter().any(|s| s.instruction.contains("experimental_jit")));

    // 6. Test Exports with Gating
    let dpo_jsonl_verified = miner.export_with_gating(ExportFormat::Dpo, Some("sess-oracle-1"), true).expect("export dpo verified");
    for line in dpo_jsonl_verified.lines() {
        let p: PreferencePair = serde_json::from_str(line).expect("deserialize dpo line");
        assert!(p.oracle_verified, "Exported line must be oracle_verified");
        assert!(p.verification_source.is_some(), "Exported line must have verification_source");
    }

    let dpo_jsonl_all = miner.export_with_gating(ExportFormat::Dpo, Some("sess-oracle-1"), false).expect("export dpo all");
    let has_unverified_in_all = dpo_jsonl_all.lines().any(|line| {
        let p: PreferencePair = serde_json::from_str(line).expect("deserialize dpo line");
        !p.oracle_verified
    });
    assert!(has_unverified_in_all, "Unrestricted export must include unverified pairs");

    // SFT JSONL gating
    let sft_jsonl_verified = miner.export_with_gating(ExportFormat::Sft, None, true).expect("export sft verified");
    for line in sft_jsonl_verified.lines() {
        let s: SftSample = serde_json::from_str(line).expect("deserialize sft line");
        assert!(s.oracle_verified, "Exported SFT sample must be oracle_verified");
    }

    // Markdown export gating
    let md_verified = miner.export_with_gating(ExportFormat::Markdown, Some("sess-oracle-1"), true).expect("export md verified");
    assert!(md_verified.contains("Oracle-Verified Only"));
    assert!(md_verified.contains("Oracle Verification"));
}

#[tokio::test]
async fn test_multi_host_compiler_context_and_sync() {
    use std::fs;

    let store = Arc::new(SqliteStore::open_in_memory().expect("init store"));

    // 1. Seed semantic facts
    let fact1 = SemanticFact::new("SQLite WAL ensures concurrent readers without blocking", "storage", Scope::Global)
        .with_importance(0.9);
    store.insert_or_update_semantic_fact(&fact1).expect("insert fact");

    // 2. Seed failure pattern
    let failure = strata_core::state::FailurePattern::new(
        "sig-cargo-lock-conflict",
        "CargoLockConflict",
        "Cargo.lock merge conflicts on concurrent commits",
        "Run cargo check --lockfile-path after rebasing",
    );
    store.upsert_failure_pattern(&failure).expect("upsert failure");

    // 3. Seed procedural skill
    let skill = ProceduralSkill::new("run_workspace_tests", "Execute tests across workspace")
        .with_preconditions(vec!["Cargo installed".to_string()]);
    store.insert_or_update_procedural_skill(&skill).expect("insert skill");

    // 4. Test MultiHostCompiler compilation
    let compiler = MultiHostCompiler::new(Arc::clone(&store));
    let config = ContextBudgetConfig::new(2048, 10);
    let compiled = compiler.compile_context(&config).expect("compile context");

    assert!(compiled.contains("Strata Persistent Memory Protocol"));
    assert!(compiled.contains("Verified Semantic Facts"));
    assert!(compiled.contains("SQLite WAL ensures concurrent readers"));
    assert!(compiled.contains("Known Failure Anti-Patterns"));
    assert!(compiled.contains("Reusable Procedural Skills"));

    // 5. Test sync_hosts across temp directory
    let temp_workspace = std::env::temp_dir().join(format!("strata_sync_test_{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_workspace).expect("create temp dir");

    // Seed existing CLAUDE.md with markers
    let initial_claude = format!("<!-- STRATA_MEMORY_START -->\nOld Context\n<!-- STRATA_MEMORY_END -->\n\n## Custom User Rules\n- Custom rule 1\n");
    fs::write(temp_workspace.join("CLAUDE.md"), initial_claude).expect("write claude.md");

    // Seed existing .cursor/rules/strata.mdc with frontmatter
    let cursor_rules_dir = temp_workspace.join(".cursor").join("rules");
    fs::create_dir_all(&cursor_rules_dir).expect("create cursor rules dir");
    let initial_cursor = "---\ndescription: Cursor Rules\nglobs: *\nalwaysApply: true\n---\nOld content without markers\n";
    fs::write(cursor_rules_dir.join("strata.mdc"), initial_cursor).expect("write strata.mdc");

    let synced = compiler
        .sync_hosts(&temp_workspace, &config, &HostTargetConfig::all())
        .expect("sync hosts");

    assert_eq!(synced.len(), 4);

    // Verify CLAUDE.md was updated and custom rules preserved
    let updated_claude = fs::read_to_string(temp_workspace.join("CLAUDE.md")).expect("read claude");
    assert!(updated_claude.contains("<!-- STRATA_MEMORY_START -->"));
    assert!(updated_claude.contains("SQLite WAL ensures concurrent readers"));
    assert!(updated_claude.contains("## Custom User Rules"));

    // Verify .cursor/rules/strata.mdc has frontmatter preserved and markers inserted
    let updated_cursor = fs::read_to_string(cursor_rules_dir.join("strata.mdc")).expect("read cursor");
    assert!(updated_cursor.starts_with("---"));
    assert!(updated_cursor.contains("alwaysApply: true"));
    assert!(updated_cursor.contains("<!-- STRATA_MEMORY_START -->"));
    assert!(updated_cursor.contains("SQLite WAL ensures concurrent readers"));

    // Verify AGENTS.md and .gemini/GEMINI.md created
    assert!(temp_workspace.join("AGENTS.md").exists());
    assert!(temp_workspace.join(".gemini").join("GEMINI.md").exists());

    // Test idempotency: sync again
    let resynced = compiler
        .sync_hosts(&temp_workspace, &config, &HostTargetConfig::all())
        .expect("resync hosts");
    assert_eq!(resynced.len(), 4);

    let claude_resynced = fs::read_to_string(temp_workspace.join("CLAUDE.md")).expect("read claude resynced");
    assert_eq!(claude_resynced.matches("<!-- STRATA_MEMORY_START -->").count(), 1);

    // Cleanup
    let _ = fs::remove_dir_all(&temp_workspace);
}

#[test]
fn test_ast_parser_multi_language() {
    let parser = AstParser::new();

    // 1. Rust parsing
    let rust_code = r#"
        pub struct MemoryStore {
            capacity: usize,
        }

        pub trait SearchEngine {
            fn search(&self, query: &str) -> Vec<String>;
        }

        impl MemoryStore {
            pub fn new(capacity: usize) -> Self {
                Self { capacity }
            }

            pub fn get_capacity(&self) -> usize {
                self.capacity
            }
        }

        pub fn helper_function(x: u32) -> u32 {
            x * 2
        }
    "#;

    let rust_symbols = parser.parse_file("src/store.rs", rust_code).expect("parse rust");
    assert!(!rust_symbols.is_empty());
    assert!(rust_symbols.iter().any(|s| s.name == "MemoryStore" && s.symbol_type == SymbolType::Struct));
    assert!(rust_symbols.iter().any(|s| s.name == "SearchEngine" && s.symbol_type == SymbolType::Trait));
    assert!(rust_symbols.iter().any(|s| s.symbol_path == "MemoryStore::new" && s.symbol_type == SymbolType::Method));
    assert!(rust_symbols.iter().any(|s| s.symbol_path == "MemoryStore::get_capacity" && s.symbol_type == SymbolType::Method));
    assert!(rust_symbols.iter().any(|s| s.name == "helper_function" && s.symbol_type == SymbolType::Function));

    // 2. TypeScript parsing
    let ts_code = r#"
        export interface UserProfile {
            id: string;
            name: string;
        }

        export type Status = "active" | "inactive";

        export class AgentEngine {
            private ready: boolean = true;

            public async executeTask(taskId: string): Promise<boolean> {
                return true;
            }
        }

        export function createEngine(): AgentEngine {
            return new AgentEngine();
        }
    "#;

    let ts_symbols = parser.parse_file("src/engine.ts", ts_code).expect("parse typescript");
    assert!(!ts_symbols.is_empty());
    assert!(ts_symbols.iter().any(|s| s.name == "UserProfile" && s.symbol_type == SymbolType::Interface));
    assert!(ts_symbols.iter().any(|s| s.name == "Status" && s.symbol_type == SymbolType::TypeAlias));
    assert!(ts_symbols.iter().any(|s| s.name == "AgentEngine" && s.symbol_type == SymbolType::Class));
    assert!(ts_symbols.iter().any(|s| s.symbol_path == "AgentEngine.executeTask" && s.symbol_type == SymbolType::Method));
    assert!(ts_symbols.iter().any(|s| s.name == "createEngine" && s.symbol_type == SymbolType::Function));

    // 3. Python parsing
    let py_code = r#"
class CognitiveRuntime:
    def __init__(self, name: str):
        self.name = name

    def run_step(self, step_idx: int) -> bool:
        return True

def top_level_bootstrap():
    pass
    "#;

    let py_symbols = parser.parse_file("runtime.py", py_code).expect("parse python");
    assert!(!py_symbols.is_empty());
    assert!(py_symbols.iter().any(|s| s.name == "CognitiveRuntime" && s.symbol_type == SymbolType::Class));
    assert!(py_symbols.iter().any(|s| s.symbol_path == "CognitiveRuntime.run_step" && s.symbol_type == SymbolType::Method));
    assert!(py_symbols.iter().any(|s| s.name == "top_level_bootstrap" && s.symbol_type == SymbolType::Function));
}

#[test]
fn test_merkle_tree_hashing_and_diff() {
    let engine = CodeAnchorEngine::new();
    let parser = AstParser::new();

    let v1_source = r#"
        pub fn calculate_decay(time_delta: f32) -> f32 {
            (-time_delta).exp()
        }

        pub fn stable_feature() -> bool {
            true
        }
    "#;

    let symbols_v1 = parser.parse_file("decay.rs", v1_source).expect("parse v1");
    let merkle_root_v1 = CodeAnchorEngine::compute_merkle_tree_hash(&symbols_v1);
    assert!(!merkle_root_v1.is_empty());

    let anchors_v1: Vec<CodeAnchor> = symbols_v1
        .iter()
        .map(|s| engine.create_anchor("decay.rs", s, Some("commit-v1")))
        .collect();

    // v2: calculate_decay is modified, stable_feature is untouched, new_feature is added
    let v2_source = r#"
        pub fn calculate_decay(time_delta: f32) -> f32 {
            // Modified decay algorithm with power-law parameter
            let exponent = 0.5;
            (-time_delta * exponent).exp()
        }

        pub fn stable_feature() -> bool {
            true
        }

        pub fn new_feature() -> &'static str {
            "v2"
        }
    "#;

    let symbols_v2 = parser.parse_file("decay.rs", v2_source).expect("parse v2");
    let merkle_root_v2 = CodeAnchorEngine::compute_merkle_tree_hash(&symbols_v2);
    assert_ne!(merkle_root_v1, merkle_root_v2);

    let diff = engine
        .diff_anchors("decay.rs", &anchors_v1, v2_source, Some("commit-v2"))
        .expect("diff anchors");

    // stable_feature should be unchanged
    assert_eq!(diff.unchanged.len(), 1);
    assert_eq!(diff.unchanged[0].symbol_path, "stable_feature");

    // calculate_decay should be modified
    assert_eq!(diff.modified.len(), 1);
    let (old_a, new_a) = &diff.modified[0];
    assert_eq!(old_a.symbol_path, "calculate_decay");
    assert_eq!(new_a.symbol_path, "calculate_decay");
    assert_ne!(old_a.ast_node_hash, new_a.ast_node_hash);
    assert_eq!(new_a.git_commit_hash.as_deref(), Some("commit-v2"));

    // new_feature should be in added
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.added[0].name, "new_feature");
}

#[tokio::test]
async fn test_bitemporal_code_anchor_reconciliation_and_store_persistence() {
    let store = SqliteStore::open_in_memory().expect("init store");
    let engine = CodeAnchorEngine::new();
    let parser = AstParser::new();

    let initial_source = r#"
        pub fn execute_transaction(tx_id: &str) -> Result<(), String> {
            Ok(())
        }
    "#;

    let symbols = parser.parse_file("tx.rs", initial_source).expect("parse initial");
    let tx_symbol = symbols.iter().find(|s| s.name == "execute_transaction").unwrap();
    let anchor = engine.create_anchor("tx.rs", tx_symbol, Some("commit-abc"));

    assert!(anchor.is_valid);
    assert!(anchor.is_active());
    assert_eq!(anchor.git_commit_hash.as_deref(), Some("commit-abc"));

    // Create a SemanticFact anchored to this symbol
    let fact = SemanticFact::new(
        "execute_transaction is synchronous and returns Result<(), String>",
        "architecture",
        Scope::Project("strata".to_string()),
    )
    .with_importance(0.85)
    .with_code_anchor(anchor);

    store.insert_or_update_semantic_fact(&fact).expect("insert fact");

    // Verify retrieval from SQLite preserves CodeAnchor
    let retrieved = store.get_semantic_fact(&fact.id).expect("get fact").expect("fact exists");
    assert!(retrieved.code_anchor.is_some());
    let ret_anchor = retrieved.code_anchor.as_ref().unwrap();
    assert_eq!(ret_anchor.file_path, "tx.rs");
    assert_eq!(ret_anchor.symbol_path, "execute_transaction");
    assert_eq!(ret_anchor.git_commit_hash.as_deref(), Some("commit-abc"));
    assert!(ret_anchor.is_valid);

    // Verify filter by file anchor
    let file_facts = store.get_facts_by_file_anchor("tx.rs").expect("get facts by file");
    assert_eq!(file_facts.len(), 1);
    assert_eq!(file_facts[0].id, fact.id);

    // Now simulate code modification: execute_transaction signature changes to async with custom Error enum
    let modified_source = r#"
        pub async fn execute_transaction(tx_id: &str) -> Result<(), TransactionError> {
            Err(TransactionError::Aborted)
        }
    "#;

    let mut facts_to_reconcile = vec![retrieved];
    let report = engine
        .reconcile_facts_bi_temporal(&mut facts_to_reconcile, modified_source, "tx.rs")
        .expect("reconcile facts");

    // Invalidation check
    assert_eq!(report.invalidated_facts.len(), 1);
    assert_eq!(report.invalidated_facts[0], fact.id);

    let invalidated_fact = &facts_to_reconcile[0];
    assert_eq!(invalidated_fact.status, FactStatus::Deprecated);
    assert!(invalidated_fact.code_anchor.is_some());
    let inv_anchor = invalidated_fact.code_anchor.as_ref().unwrap();
    assert!(!inv_anchor.is_valid);
    assert!(inv_anchor.valid_until.is_some());

    // Update store with invalidated fact and verify persistent bi-temporal state
    store.insert_or_update_semantic_fact(invalidated_fact).expect("update invalidated fact");
    let re_retrieved = store.get_semantic_fact(&fact.id).expect("get updated fact").unwrap();
    assert_eq!(re_retrieved.status, FactStatus::Deprecated);
    assert!(!re_retrieved.code_anchor.unwrap().is_valid);
}

#[tokio::test]
async fn test_monorepo_package_scoped_search_and_isolation() {
    let engine = SqliteMemoryEngine::open_in_memory(None).expect("open engine");

    // 1. Create a simulated monorepo boundary with 2 independent crates: `auth-crate` and `billing-crate`
    let mut boundary = WorkspaceBoundary::new("repo_root", PackageType::CargoWorkspace);
    boundary.add_package(MonorepoPackage::new(
        "auth-crate",
        PackageType::CargoCrate,
        "repo_root/crates/auth",
        "repo_root/crates/auth/Cargo.toml",
    ));
    boundary.add_package(MonorepoPackage::new(
        "billing-crate",
        PackageType::CargoCrate,
        "repo_root/crates/billing",
        "repo_root/crates/billing/Cargo.toml",
    ));

    // 2. Write memories into specific package scopes and global scope
    let auth_mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Token validation uses RSA-256 JWT key in auth crate",
        Scope::Project("auth-crate".to_string()),
    );
    let billing_mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Stripe webhook subscription validation in billing crate",
        Scope::Project("billing-crate".to_string()),
    );
    let global_mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Code style requires explicit error handling across entire repository",
        Scope::Global,
    );

    engine.write(&auth_mem).await.expect("write auth mem");
    engine.write(&billing_mem).await.expect("write billing mem");
    engine.write(&global_mem).await.expect("write global mem");

    // 3. Search scoped to an auth crate file: `repo_root/crates/auth/src/token.rs`
    let auth_results = engine
        .search_scoped_to_file("validation", "repo_root/crates/auth/src/token.rs", Some(&boundary), 5)
        .await
        .expect("search scoped to auth");

    // Must find auth-crate memory first and global memory, but NOT billing-crate memory
    assert!(auth_results.iter().any(|m| m.content.contains("Token validation")));
    assert!(auth_results.iter().any(|m| m.content.contains("Code style requires")));
    assert!(!auth_results.iter().any(|m| m.content.contains("Stripe webhook")));

    // 4. Search scoped to a billing crate file: `repo_root/crates/billing/src/stripe.rs`
    let billing_results = engine
        .search_scoped_to_file("validation", "repo_root/crates/billing/src/stripe.rs", Some(&boundary), 5)
        .await
        .expect("search scoped to billing");

    assert!(billing_results.iter().any(|m| m.content.contains("Stripe webhook")));
    assert!(billing_results.iter().any(|m| m.content.contains("Code style requires")));
    assert!(!billing_results.iter().any(|m| m.content.contains("Token validation")));
}

#[test]
fn test_sqlite_architecture_summary_caching() {
    let store = SqliteStore::open_in_memory().expect("open in-memory sqlite");

    let summary = crate::community::CommunityDetector::default().detect_from_edges(
        &[
            crate::call_graph::CallEdge::new("src/auth.rs", "login", "validate", 10, crate::call_graph::CallType::FunctionCall),
            crate::call_graph::CallEdge::new("src/db.rs", "query", "connect", 20, crate::call_graph::CallType::FunctionCall),
        ],
        "test-ws",
    );

    // Cache summary
    store.cache_architecture_summary(&summary).expect("cache architecture summary");

    // Retrieve cached summary
    let cached = store.get_cached_architecture_summary("test-ws")
        .expect("get cached architecture summary")
        .expect("should find cached summary");

    assert_eq!(cached.workspace_id, "test-ws");
    assert_eq!(cached.total_edges, 2);
    assert_eq!(cached.clusters.len(), summary.clusters.len());
}

#[tokio::test]
async fn test_hitl_core_tier_approval_and_promotion() {
    use strata_core::state::{MemoryTier, MemoryType};

    let engine = SqliteMemoryEngine::open_in_memory(None).expect("open in-memory engine");
    let store = engine.store();

    // 1. Attempting to write directly to Core Tier without human approval must fail
    let unapproved_core = MemoryRecord::new(
        MemoryType::Semantic,
        "Critical architectural rule: DB queries must use parameterized statements",
        Scope::Global,
    ).with_tier(MemoryTier::Core); // approved_by_human is false by default

    let write_res = engine.write(&unapproved_core).await;
    assert!(write_res.is_err(), "Direct write to Core Tier without human approval must be rejected");

    // 2. Writing to Working Tier initially succeeds
    let working_mem = MemoryRecord::new(
        MemoryType::Procedural,
        "Use exponential backoff for network calls",
        Scope::Global,
    ).with_tier(MemoryTier::Working);

    let handle = engine.write(&working_mem).await.expect("write working memory");

    // 3. Attempting to promote without human approval (approved_by_human = false) must fail
    let unapproved_promote_res = engine.promote_to_core(&handle.id, false, Some("Policy update")).await;
    assert!(unapproved_promote_res.is_err(), "Promotion without human approval must be rejected");

    // 4. Promoting with human approval (approved_by_human = true) succeeds
    let promoted = engine
        .promote_to_core(&handle.id, true, Some("Approved in ADR-042 by security team"))
        .await
        .expect("promote to core with human approval");

    assert_eq!(promoted.tier, MemoryTier::Core);
    assert!(promoted.approved_by_human);
    assert_eq!(promoted.importance, 1.0);

    // Verify metadata was attached
    let reason = promoted.metadata.get("promotion_reason").and_then(|v| v.as_str());
    assert_eq!(reason, Some("Approved in ADR-042 by security team"));

    // Verify persistence in SQLite
    let persisted = store.get_memory(&handle.id).expect("get memory").expect("memory exists");
    assert_eq!(persisted.tier, MemoryTier::Core);
    assert!(persisted.approved_by_human);
    assert_eq!(persisted.importance, 1.0);

    // 5. Test SemanticFact promotion with HITL
    let fact = SemanticFact::new("PostgreSQL 16 requires pgvector 0.7+", "db_config", Scope::Global)
        .with_tier(MemoryTier::Working);
    store.insert_or_update_semantic_fact(&fact).expect("insert fact");

    // Rejection without approval
    let fact_unapproved = store.promote_semantic_fact_to_core(&fact.id, false, None);
    assert!(fact_unapproved.is_err());

    // Approval succeeds
    let fact_promoted = store
        .promote_semantic_fact_to_core(&fact.id, true, Some("Production verified"))
        .expect("promote fact");
    assert_eq!(fact_promoted.tier, MemoryTier::Core);
    assert!(fact_promoted.approved_by_human);
    assert_eq!(fact_promoted.importance, 1.0);

    let persisted_fact = store.get_semantic_fact(&fact.id).expect("get fact").expect("fact exists");
    assert_eq!(persisted_fact.tier, MemoryTier::Core);
    assert!(persisted_fact.approved_by_human);
}

#[tokio::test]
async fn test_ast_blake3_content_hashing_and_merkle_tree() {
    let parser = AstParser::new();
    let code_v1 = r#"
        pub fn authenticate_user(token: &str) -> bool {
            !token.is_empty()
        }
    "#;

    let symbols = parser.parse_file("auth.rs", code_v1).expect("parse file");
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "authenticate_user");
    assert!(!sym.content_hash.is_empty(), "Blake3 content hash must be computed");
    assert_eq!(sym.content_hash, AstParser::blake3_content_hash(&sym.raw_code));

    // Whitespace line endings (\r\n vs \n) shouldn't change hash due to line normalization
    let code_v1_crlf = code_v1.replace('\n', "\r\n");
    let symbols_crlf = parser.parse_file("auth.rs", &code_v1_crlf).expect("parse crlf");
    assert_eq!(symbols_crlf[0].content_hash, sym.content_hash);
}

#[tokio::test]
async fn test_on_commit_reconciliation_stale_invalidation() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let engine = CodeAnchorEngine::new();
    let parser = AstParser::new();

    let initial_code = r#"
        pub fn process_payment(amount: u64) -> Result<(), String> {
            Ok(())
        }
    "#;

    let symbols = parser.parse_file("billing.rs", initial_code).expect("parse billing");
    let anchor = engine.create_anchor("billing.rs", &symbols[0], Some("commit-1"));

    let fact = SemanticFact::new(
        "process_payment accepts u64 amount and returns synchronous Result",
        "finance",
        Scope::Project("strata".to_string()),
    )
    .with_importance(0.8)
    .with_code_anchor(anchor);

    store.insert_or_update_semantic_fact(&fact).expect("insert fact");

    // Commit 2 alters the function body and signature
    let modified_code = r#"
        pub async fn process_payment(amount: u64, currency: &str) -> Result<(), PaymentError> {
            Err(PaymentError::GatewayTimeout)
        }
    "#;

    let workspace_files = [("billing.rs", modified_code)];
    let report = engine
        .reconcile_workspace_on_commit(&store, &workspace_files, Some("commit-2"), None)
        .await
        .expect("reconcile commit");

    assert_eq!(report.stale_facts.len(), 1);
    assert_eq!(report.stale_facts[0], fact.id);
    assert_eq!(report.active_facts.len(), 0);

    let updated_fact = store.get_semantic_fact(&fact.id).unwrap().unwrap();
    assert_eq!(updated_fact.status, FactStatus::Stale);
    assert!(updated_fact.is_stale());
    assert!(!updated_fact.code_anchor.as_ref().unwrap().is_valid);
}

#[tokio::test]
async fn test_blake3_fallback_tolerates_file_renames_and_symbol_relocation() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let engine = CodeAnchorEngine::new();
    let parser = AstParser::new();

    let symbol_body = r#"
        pub fn calculate_tax(subtotal: f64) -> f64 {
            subtotal * 0.20
        }
    "#;

    let symbols = parser.parse_file("legacy_tax.rs", symbol_body).expect("parse legacy tax");
    let anchor = engine.create_anchor("legacy_tax.rs", &symbols[0], Some("commit-1"));

    let fact = SemanticFact::new(
        "calculate_tax applies standard 20% VAT rate",
        "tax",
        Scope::Project("strata".to_string()),
    )
    .with_importance(0.9)
    .with_code_anchor(anchor);

    store.insert_or_update_semantic_fact(&fact).expect("insert fact");

    // File was moved/renamed to `crates/finance/src/vat.rs` but function body is 100% IDENTICAL
    let workspace_files = [("crates/finance/src/vat.rs", symbol_body)];

    let report = engine
        .reconcile_workspace_on_commit(&store, &workspace_files, Some("commit-2"), None)
        .await
        .expect("reconcile renamed file");

    assert_eq!(report.stale_facts.len(), 0, "Fact should NOT be stale because content is identical");
    assert_eq!(report.moved_anchors.len(), 1, "Should detect relocated anchor via Blake3");
    assert_eq!(report.moved_anchors[0], fact.id);
    assert_eq!(report.active_facts.len(), 1);

    let updated_fact = store.get_semantic_fact(&fact.id).unwrap().unwrap();
    assert_eq!(updated_fact.status, FactStatus::Active);
    let anc = updated_fact.code_anchor.unwrap();
    assert_eq!(anc.file_path, "crates/finance/src/vat.rs");
    assert_eq!(anc.symbol_path, "calculate_tax");
    assert!(anc.is_valid);
    assert_eq!(anc.git_commit_hash.as_deref(), Some("commit-2"));
}

#[tokio::test]
async fn test_stale_fact_decay_boost_accelerated_pruning() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let calc = DecayCalculator::with_default_config();

    let now = Utc::now();
    let old_time = now - Duration::hours(100);

    // 1. Active fact with Core Tier
    let mut active_fact = SemanticFact::new(
        "Standard invariant rule",
        "rules",
        Scope::Global,
    )
    .with_importance(0.9)
    .with_tier(strata_core::state::MemoryTier::Core);
    active_fact.created_at = old_time;
    active_fact.last_updated_at = old_time;
    active_fact.status = FactStatus::Active;
    store.insert_or_update_semantic_fact(&active_fact).expect("insert active");

    // 2. Stale fact (previously Core, but anchor invalidated)
    let mut stale_fact = SemanticFact::new(
        "Outdated anchor fact",
        "rules",
        Scope::Global,
    )
    .with_importance(0.9)
    .with_tier(strata_core::state::MemoryTier::Core);
    stale_fact.created_at = old_time;
    stale_fact.last_updated_at = old_time;
    stale_fact.status = FactStatus::Stale;
    store.insert_or_update_semantic_fact(&stale_fact).expect("insert stale");

    // Evaluate decay metrics directly
    let active_metrics = calc.evaluate_semantic_fact(&active_fact, &[], now);
    let stale_metrics = calc.evaluate_semantic_fact(&stale_fact, &[], now);

    assert_eq!(active_metrics.retention, 1.0, "Active Core fact has frozen retention");
    assert!(stale_metrics.retention < 0.1, "Stale fact suffers boosted decay");
    assert!(stale_metrics.is_expired, "Stale fact should be expired after 100 hours of inactivity");

    // Pruning run
    let report = calc.prune_expired(&store, Some(0.1), Some(now)).expect("prune expired");
    assert_eq!(report.core_protected, 1, "Active Core fact was protected");
    assert_eq!(report.facts_pruned, 1, "Stale fact was pruned to Deprecated");

    let re_active = store.get_semantic_fact(&active_fact.id).unwrap().unwrap();
    assert_eq!(re_active.status, FactStatus::Active);

    let re_stale = store.get_semantic_fact(&stale_fact.id).unwrap().unwrap();
    assert_eq!(re_stale.status, FactStatus::Deprecated);
}

#[tokio::test]
async fn test_causal_blast_radius_suspicious_marking_on_commit() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let engine = CodeAnchorEngine::new();
    let parser = AstParser::new();

    // Setup World Model with Causal Graph: storage.rs depends on db.rs
    let world_model = strata_reasoning::WorldModel::new();
    let _ = world_model.register_invariant("DB Connection Pool Invariant", "db must be pooling", "storage.rs").await;

    let db_source = r#"
        pub fn connect_db() -> bool {
            true
        }
    "#;

    let storage_source = r#"
        pub fn save_record() -> bool {
            true
        }
    "#;

    let db_sym = &parser.parse_file("db.rs", db_source).unwrap()[0];
    let storage_sym = &parser.parse_file("storage.rs", storage_source).unwrap()[0];

    // Fact 1 anchored to db.rs
    let fact_db = SemanticFact::new(
        "connect_db initializes global connection pool",
        "database",
        Scope::Global,
    )
    .with_code_anchor(engine.create_anchor("db.rs", db_sym, Some("commit-1")));
    store.insert_or_update_semantic_fact(&fact_db).unwrap();

    // Fact 2 anchored to storage.rs which depends on db.rs
    let fact_storage = SemanticFact::new(
        "save_record writes entities through db.rs connection",
        "storage",
        Scope::Global,
    )
    .with_code_anchor(engine.create_anchor("storage.rs", storage_sym, Some("commit-1")));
    store.insert_or_update_semantic_fact(&fact_storage).unwrap();

    // Modify db.rs
    let modified_db = r#"
        pub async fn connect_db(url: &str) -> Result<(), String> {
            Ok(())
        }
    "#;

    let workspace_files = [("db.rs", modified_db), ("storage.rs", storage_source)];

    let report = engine
        .reconcile_workspace_on_commit(&store, &workspace_files, Some("commit-2"), Some(&world_model))
        .await
        .expect("reconcile commit");

    // Fact 1 is stale (directly modified)
    assert!(report.stale_facts.contains(&fact_db.id));

    // Fact 2 is suspicious (statement references db.rs / blast radius)
    assert!(report.suspicious_facts.contains(&fact_storage.id));

    let f1 = store.get_semantic_fact(&fact_db.id).unwrap().unwrap();
    let f2 = store.get_semantic_fact(&fact_storage.id).unwrap().unwrap();

    assert_eq!(f1.status, FactStatus::Stale);
    assert_eq!(f2.status, FactStatus::Suspicious);
    assert!(f2.is_suspicious());
}

#[tokio::test]
async fn test_reconciliation_idempotence_on_clean_workspace() {
    let store = SqliteStore::open_in_memory().expect("open store");
    let engine = CodeAnchorEngine::new();
    let parser = AstParser::new();

    let code = r#"
        pub fn stable_logic() -> u32 {
            42
        }
    "#;

    let sym = &parser.parse_file("logic.rs", code).unwrap()[0];
    let fact = SemanticFact::new("stable logic is 42", "math", Scope::Global)
        .with_code_anchor(engine.create_anchor("logic.rs", sym, Some("commit-1")));
    store.insert_or_update_semantic_fact(&fact).unwrap();

    let workspace_files = [("logic.rs", code)];

    // Pass 1
    let rep1 = engine
        .reconcile_workspace_on_commit(&store, &workspace_files, Some("commit-1"), None)
        .await
        .unwrap();

    assert_eq!(rep1.stale_facts.len(), 0);
    assert_eq!(rep1.suspicious_facts.len(), 0);
    assert_eq!(rep1.active_facts.len(), 1);

    // Pass 2 (immediate re-run)
    let rep2 = engine
        .reconcile_workspace_on_commit(&store, &workspace_files, Some("commit-1"), None)
        .await
        .unwrap();

    assert_eq!(rep2.stale_facts.len(), 0);
    assert_eq!(rep2.suspicious_facts.len(), 0);
    assert_eq!(rep2.active_facts.len(), 1);

    let fact_after = store.get_semantic_fact(&fact.id).unwrap().unwrap();
    assert_eq!(fact_after.status, FactStatus::Active);
}

#[tokio::test]
async fn test_jtms_v2_replay_consistency() {
    let embedder = MockEmbeddingProvider::default();
    let jtms = TruthMaintenanceSystem::with_default_threshold();

    let stmt_1 = "The backend microservices communication layer is implemented using REST JSON APIs.";
    let stmt_2 = "The backend microservices communication layer is migrated to gRPC Protobuf, deprecating REST JSON APIs.";

    let emb_1 = embedder.embed_text(stmt_1).await.unwrap();
    let emb_2 = embedder.embed_text(stmt_2).await.unwrap();

    // Order 1: Ingest Fact 1, then Fact 2
    let store_a = SqliteStore::open_in_memory().unwrap();
    let mut fact1_a = SemanticFact::new(stmt_1, "architecture", Scope::Global)
        .with_importance(0.8)
        .with_confidence(0.9);
    fact1_a.created_at = Utc::now() - chrono::Duration::hours(2);

    let mut fact2_a = SemanticFact::new(stmt_2, "architecture", Scope::Global)
        .with_importance(0.9)
        .with_confidence(0.95);
    fact2_a.created_at = Utc::now();

    jtms.resolve_and_upsert(&store_a, &mut fact1_a, &emb_1).unwrap();
    jtms.resolve_and_upsert(&store_a, &mut fact2_a, &emb_2).unwrap();

    // Order 2: Ingest Fact 2, then Fact 1
    let store_b = SqliteStore::open_in_memory().unwrap();
    let mut fact1_b = SemanticFact::new(stmt_1, "architecture", Scope::Global)
        .with_importance(0.8)
        .with_confidence(0.9)
        .with_id(fact1_a.id);
    fact1_b.created_at = fact1_a.created_at;

    let mut fact2_b = SemanticFact::new(stmt_2, "architecture", Scope::Global)
        .with_importance(0.9)
        .with_confidence(0.95)
        .with_id(fact2_a.id);
    fact2_b.created_at = fact2_a.created_at;

    jtms.resolve_and_upsert(&store_b, &mut fact2_b, &emb_2).unwrap();
    jtms.resolve_and_upsert(&store_b, &mut fact1_b, &emb_1).unwrap();

    // Verify Store A and Store B state parity
    let final1_a = store_a.get_semantic_fact(&fact1_a.id).unwrap().unwrap();
    let final2_a = store_a.get_semantic_fact(&fact2_a.id).unwrap().unwrap();

    let final1_b = store_b.get_semantic_fact(&fact1_b.id).unwrap().unwrap();
    let final2_b = store_b.get_semantic_fact(&fact2_b.id).unwrap().unwrap();

    assert_eq!(final1_a.status, FactStatus::Deprecated);
    assert_eq!(final1_b.status, FactStatus::Deprecated);
    assert_eq!(final1_a.replaced_by, Some(fact2_a.id));
    assert_eq!(final1_b.replaced_by, Some(fact2_b.id));

    assert_eq!(final2_a.status, FactStatus::Active);
    assert_eq!(final2_b.status, FactStatus::Active);
}

#[tokio::test]
async fn test_jtms_v2_downstream_invalidation_propagation() {
    let store = SqliteStore::open_in_memory().unwrap();
    let embedder = MockEmbeddingProvider::default();
    let jtms = TruthMaintenanceSystem::with_default_threshold();

    // 1. Fact A: PostgreSQL is the primary database
    let stmt_a = "PostgreSQL is the primary database.";
    let emb_a = embedder.embed_text(stmt_a).await.unwrap();
    let mut fact_a = SemanticFact::new(stmt_a, "database", Scope::Global);
    fact_a.created_at = Utc::now() - chrono::Duration::hours(3);
    jtms.resolve_and_upsert(&store, &mut fact_a, &emb_a).unwrap();

    // 2. Fact B: Sqlx connection pool is configured for PostgreSQL (depends on A)
    let stmt_b = "Sqlx pool is connected to PostgreSQL on port 5432.";
    let emb_b = embedder.embed_text(stmt_b).await.unwrap();
    let mut fact_b = SemanticFact::new(stmt_b, "database", Scope::Global)
        .with_dependency(fact_a.id);
    fact_b.created_at = Utc::now() - chrono::Duration::hours(2);
    jtms.resolve_and_upsert(&store, &mut fact_b, &emb_b).unwrap();

    // 3. Fact C: User repository utilizes Sqlx pool (depends on B)
    let stmt_c = "User repository executes queries via Sqlx connection pool.";
    let emb_c = embedder.embed_text(stmt_c).await.unwrap();
    let mut fact_c = SemanticFact::new(stmt_c, "repository", Scope::Global)
        .with_dependency(fact_b.id);
    fact_c.created_at = Utc::now() - chrono::Duration::hours(1);
    jtms.resolve_and_upsert(&store, &mut fact_c, &emb_c).unwrap();

    assert!(jtms.is_belief_valid(&store, &fact_a.id).unwrap());
    assert!(jtms.is_belief_valid(&store, &fact_b.id).unwrap());
    assert!(jtms.is_belief_valid(&store, &fact_c.id).unwrap());

    // 4. Ingest superseding Fact A2: Database migrated to MySQL
    let stmt_a2 = "Primary database is migrated to MySQL, deprecating PostgreSQL.";
    let emb_a2 = embedder.embed_text(stmt_a2).await.unwrap();
    let mut fact_a2 = SemanticFact::new(stmt_a2, "database", Scope::Global);
    fact_a2.created_at = Utc::now();
    jtms.resolve_and_upsert(&store, &mut fact_a2, &emb_a2).unwrap();

    // Check statuses
    let fact_a_res = store.get_semantic_fact(&fact_a.id).unwrap().unwrap();
    let fact_a2_res = store.get_semantic_fact(&fact_a2.id).unwrap().unwrap();
    let fact_b_res = store.get_semantic_fact(&fact_b.id).unwrap().unwrap();
    let fact_c_res = store.get_semantic_fact(&fact_c.id).unwrap().unwrap();

    assert_eq!(fact_a2_res.status, FactStatus::Active);
    assert_eq!(fact_a_res.status, FactStatus::Deprecated);
    assert_eq!(fact_b_res.status, FactStatus::Stale);
    assert_eq!(fact_c_res.status, FactStatus::Stale);

    // Check audits
    let audits_b = store.get_jtms_audits_for_fact(&fact_b.id).unwrap();
    assert!(!audits_b.is_empty());
    assert_eq!(audits_b[0].resolution_type, "invalidation");

    let audits_c = store.get_jtms_audits_for_fact(&fact_c.id).unwrap();
    assert!(!audits_c.is_empty());
    assert_eq!(audits_c[0].resolution_type, "invalidation");
}

#[tokio::test]
async fn test_jtms_v2_core_tier_and_human_approval_authority() {
    let store = SqliteStore::open_in_memory().unwrap();
    let embedder = MockEmbeddingProvider::default();
    let jtms = TruthMaintenanceSystem::with_default_threshold();

    // Fact 1: Core Tier + Human Approved
    let stmt_1 = "Auth tokens must use asymmetric RS256 algorithm with public key verification.";
    let emb_1 = embedder.embed_text(stmt_1).await.unwrap();
    let mut fact1 = SemanticFact::new(stmt_1, "security", Scope::Global)
        .with_tier(MemoryTier::Core)
        .with_human_approval(true);
    fact1.created_at = Utc::now() - chrono::Duration::days(10);
    jtms.resolve_and_upsert(&store, &mut fact1, &emb_1).unwrap();

    // Fact 2: Peripheral Tier + Unapproved, chronologically newer
    let stmt_2 = "Auth tokens are configured to use symmetric HS256 algorithm instead of RS256.";
    let emb_2 = embedder.embed_text(stmt_2).await.unwrap();
    let mut fact2 = SemanticFact::new(stmt_2, "security", Scope::Global)
        .with_tier(MemoryTier::Peripheral)
        .with_human_approval(false);
    fact2.created_at = Utc::now();

    jtms.resolve_and_upsert(&store, &mut fact2, &emb_2).unwrap();

    // Core Tier fact with human approval should WIN and reject Fact 2
    let fact1_res = store.get_semantic_fact(&fact1.id).unwrap().unwrap();
    let fact2_res = store.get_semantic_fact(&fact2.id).unwrap().unwrap();

    assert_eq!(fact1_res.status, FactStatus::Active);
    assert_eq!(fact2_res.status, FactStatus::Deprecated);
    assert_eq!(fact2_res.replaced_by, Some(fact1.id));
}

#[tokio::test]
async fn test_jtms_v2_orthogonal_coexistence_no_false_conflicts() {
    let store = SqliteStore::open_in_memory().unwrap();
    let embedder = MockEmbeddingProvider::default();
    let jtms = TruthMaintenanceSystem::with_default_threshold();

    let stmt_fe = "The frontend user interface is built using React and Tailwind CSS.";
    let stmt_be = "The backend microservice server is built using Rust Axum and Tokio.";

    let emb_fe = embedder.embed_text(stmt_fe).await.unwrap();
    let emb_be = embedder.embed_text(stmt_be).await.unwrap();

    let mut fact_fe = SemanticFact::new(stmt_fe, "stack", Scope::Global);
    let mut fact_be = SemanticFact::new(stmt_be, "stack", Scope::Global);

    let conf_fe = jtms.resolve_and_upsert(&store, &mut fact_fe, &emb_fe).unwrap();
    let conf_be = jtms.resolve_and_upsert(&store, &mut fact_be, &emb_be).unwrap();

    assert!(conf_fe.is_empty());
    assert!(conf_be.is_empty());

    let res_fe = store.get_semantic_fact(&fact_fe.id).unwrap().unwrap();
    let res_be = store.get_semantic_fact(&fact_be.id).unwrap().unwrap();

    assert_eq!(res_fe.status, FactStatus::Active);
    assert_eq!(res_be.status, FactStatus::Active);
}







