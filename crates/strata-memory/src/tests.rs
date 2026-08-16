use chrono::Utc;

use strata_core::events::{
    Event, EventPayload, SessionStarted, TaskCompleted,
};
use strata_core::state::{
    FailureSeverity, MemoryRecord, MemoryType, Scope,
};
use strata_core::traits::{EventStore, MemoryEngine};

use crate::embedding::{
    bytes_to_embedding, cosine_similarity, embedding_to_bytes,
};
use crate::SqliteMemoryEngine;

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
