use std::sync::Arc;
use anyhow::Result;
use chrono::{Duration, Utc};
use uuid::Uuid;

use strata_core::events::{
    Event, EventPayload, ObservationReceived, SessionEnded, SessionStarted, TaskCompleted,
    TaskStarted, ToolInvoked, ToolResultReceived, ErrorObserved,
};
use strata_core::schemas::FactStatus;
use strata_memory::{
    ConsolidationPipeline, MockEmbeddingProvider, PipelineConfig, SqliteStore,
};
use strata_reasoning::OpenRouterAdapter;

#[tokio::test]
async fn test_live_openrouter_consolidation() -> Result<()> {
    let api_key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            println!("Skipping live OpenRouter test: OPENROUTER_API_KEY not set");
            return Ok(());
        }
    };

    println!("\n========================================================");
    println!("🚀 RUNNING LIVE OPENROUTER FREE CONSOLIDATION TEST");
    println!("========================================================");

    // 1. Initialize in-memory SQLite store and pipeline
    let store = SqliteStore::open_in_memory()?;
    let embedder = MockEmbeddingProvider::default();
    let openrouter = OpenRouterAdapter::new(api_key, "openrouter/free");

    let pipeline = ConsolidationPipeline::new(PipelineConfig::default());
    let session_id = "session-live-openrouter-01";
    let now = Utc::now();

    // 2. Generate a realistic developer trajectory
    let events = vec![
        Event::new(
            session_id,
            "claude-code",
            EventPayload::SessionStarted(SessionStarted {
                session_id: session_id.to_string(),
                agent_id: "claude-code".to_string(),
                organization_id: None,
                environment: serde_json::json!({ "os": "windows", "host": "claude-code-cli" }),
                timestamp: now,
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::TaskStarted(TaskStarted {
                task_id: "task-db-resilience".to_string(),
                title: "Implement SQLite connection resilience and busy_timeout".to_string(),
                description: Some("Configure SQLite connection pool with 250ms busy_timeout and exponential backoff retry on database lock errors".to_string()),
                parent_task_id: None,
                session_id: session_id.to_string(),
                timestamp: now + Duration::seconds(1),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ToolInvoked(ToolInvoked {
                invocation_id: Uuid::new_v4(),
                tool_name: "read_file".to_string(),
                input: serde_json::json!({ "path": "crates/strata-memory/src/store.rs" }),
                session_id: session_id.to_string(),
                timestamp: now + Duration::seconds(2),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ToolResultReceived(ToolResultReceived {
                invocation_id: Uuid::new_v4(),
                tool_name: "read_file".to_string(),
                result: serde_json::json!({ "content": "pub struct SqliteStore { conn: Mutex<Connection> }" }),
                is_error: false,
                duration_ms: Some(15),
                timestamp: now + Duration::seconds(3),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ToolInvoked(ToolInvoked {
                invocation_id: Uuid::new_v4(),
                tool_name: "run_command".to_string(),
                input: serde_json::json!({ "command": "cargo check -p strata-memory" }),
                session_id: session_id.to_string(),
                timestamp: now + Duration::seconds(4),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ToolResultReceived(ToolResultReceived {
                invocation_id: Uuid::new_v4(),
                tool_name: "run_command".to_string(),
                result: serde_json::json!({ "stderr": "error[E0599]: no method named `retry_with_backoff` on Connection" }),
                is_error: true,
                duration_ms: Some(950),
                timestamp: now + Duration::seconds(5),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ErrorObserved(ErrorObserved {
                error_type: "SqliteLockError".to_string(),
                message: "database is locked (error code 5): cannot acquire write lock concurrently".to_string(),
                severity: "high".to_string(),
                context: Some(serde_json::json!({ "busy_timeout_ms": 0 })),
                stack_trace: None,
                timestamp: now + Duration::seconds(6),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ToolInvoked(ToolInvoked {
                invocation_id: Uuid::new_v4(),
                tool_name: "edit_file".to_string(),
                input: serde_json::json!({ "path": "crates/strata-memory/src/store.rs", "change": "Set PRAGMA busy_timeout = 250 and add exponential backoff retry loop" }),
                session_id: session_id.to_string(),
                timestamp: now + Duration::seconds(7),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ToolResultReceived(ToolResultReceived {
                invocation_id: Uuid::new_v4(),
                tool_name: "edit_file".to_string(),
                result: serde_json::json!({ "status": "success", "lines_changed": 18 }),
                is_error: false,
                duration_ms: Some(30),
                timestamp: now + Duration::seconds(8),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::ObservationReceived(ObservationReceived {
                session_id: session_id.to_string(),
                source: "sqlite_store".to_string(),
                observation_type: "architectural_decision".to_string(),
                content: serde_json::json!("SQLite WAL mode with PRAGMA busy_timeout = 250ms prevents SQLite locking errors in concurrent multi-agent reading"),
                timestamp: now + Duration::seconds(9),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::TaskCompleted(TaskCompleted {
                task_id: "task-db-resilience".to_string(),
                success: true,
                outcome_summary: "SQLite retry loop and 250ms busy_timeout successfully eliminated concurrent lock errors".to_string(),
                evaluation: None,
                timestamp: now + Duration::seconds(10),
            }),
        ),
        Event::new(
            session_id,
            "claude-code",
            EventPayload::SessionEnded(SessionEnded {
                session_id: session_id.to_string(),
                agent_id: "claude-code".to_string(),
                final_state: Some("completed".to_string()),
                reason: Some("Task finished successfully".to_string()),
                summary: Some("Resilience update for SQLite store completed with verification".to_string()),
                timestamp: now + Duration::seconds(11),
            }),
        ),
    ];

    for ev in &events {
        store.insert_event(ev)?;
    }

    println!("📥 Ingested {} raw events into SQLite store.", events.len());
    println!("🤖 Calling OpenRouter Free Tier (model: {}) for offline cognitive distillation...", openrouter.model());

    // 3. Run Consolidation Pipeline using OpenRouter Live
    let result = pipeline
        .run_pipeline(&store, &embedder, &events, Some(&openrouter))
        .await?;

    println!("\n✅ [CONSOLIDATION COMPLETED SUCCESSFULLY]");
    println!("--------------------------------------------------------");
    println!("📊 Events Processed:        {}", result.events_processed);
    println!("📖 Episodic Memories:       {}", result.episodic_memories.len());
    for (i, ep) in result.episodic_memories.iter().enumerate() {
        println!("   [{}] Summary:     {}", i + 1, ep.summary);
        println!("       Goals:       {:?}", ep.goals);
        println!("       Obstacles:   {:?}", ep.obstacles);
        println!("       Outcomes:    {:?}", ep.outcomes);
        println!("       Signals:     success={:.2}, frustration={:.2}, novelty={:.2}, importance={:.2}",
            ep.signals.success, ep.signals.frustration, ep.signals.novelty, ep.signals.importance);
    }

    println!("\n💡 Semantic Facts Created:  {}", result.semantic_facts.len());
    for (i, fact) in result.semantic_facts.iter().enumerate() {
        println!("   [{}] Statement:   {}", i + 1, fact.statement);
        println!("       Category:    {}", fact.category);
        println!("       Status:      {:?} (v{})", fact.status, fact.version);
        println!("       Importance:  {:.2}, Confidence: {:.2}", fact.importance, fact.confidence);
    }

    println!("\n🛠️ Procedural Skills:       {}", result.procedural_skills.len());
    for (i, skill) in result.procedural_skills.iter().enumerate() {
        println!("   [{}] Skill Name:  {}", i + 1, skill.name);
        println!("       Description: {}", skill.description);
        println!("       Steps ({} total):", skill.steps.len());
        for step in &skill.steps {
            println!("         {}. [tool: {}] action: {}", step.order, step.tool, step.action);
        }
    }

    println!("\n🧹 Pruned Memories:         {}", result.memories_pruned);
    println!("🔄 JTMS Conflicts Resolved: {}", result.conflicts_resolved);
    println!("--------------------------------------------------------");

    // 4. Verify SQLite storage persistence
    let stored_facts = store.get_all_semantic_facts(None, Some(FactStatus::Active), 10)?;
    assert!(!stored_facts.is_empty(), "Semantic facts must be persisted to SQLite");
    let stored_episodes = store.get_all_episodic_memories(None, 10)?;
    assert!(!stored_episodes.is_empty(), "Episodic memories must be persisted to SQLite");

    println!("\n✨ Live OpenRouter consolidation verified end-to-end with SQLite persistence!\n");

    Ok(())
}
