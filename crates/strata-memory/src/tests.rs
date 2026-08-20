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
    SignalKind, SignalScores, SymbolType, SyncConfig, SyncDelta,
};

use strata_core::state::{
    FailureSeverity, MemoryRecord, MemoryType, Scope,
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



