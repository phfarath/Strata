use chrono::{Duration, Utc};
use uuid::Uuid;

use strata_core::events::{
    Event, EventPayload, ObservationReceived, SessionEnded, SessionStarted, TaskCompleted,
    TaskStarted, ToolInvoked, ToolResultReceived,
};
use strata_core::schemas::{
    DecayConfig, EpisodicMemory, FactStatus, MemoryFeedback,
    ProceduralSkill, ProceduralStep, SemanticFact, SignalScores,
    SyncConfig, SyncDelta,
};

use strata_core::state::{
    FailureSeverity, MemoryRecord, MemoryType, Scope,
};
use strata_core::traits::{EventStore, MemoryEngine};

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

