use anyhow::{bail, Result};
use strata_core::{
    events::{Event, EventPayload, Provenance, SessionStarted},
    state::{MemoryRecord, MemoryType, Scope},
    traits::{EventStore, MemoryEngine},
};
use strata_memory::SqliteMemoryEngine;

/// Scenario 1: Cross-Host Memory Transfer
/// Verifies that a memory/decision written in one simulated session (e.g. Cursor)
/// is accurately retrieved in a subsequent session (e.g. Claude Code).
pub async fn run_cross_host_transfer_scenario() -> Result<()> {
    println!("\n▶ Running Eval Scenario: Cross-Host Memory Transfer");

    // 1. Setup isolated in-memory memory engine
    let engine = SqliteMemoryEngine::open_in_memory(None)?;

    // 2. Simulated Host 1: Cursor Session
    let cursor_session = "sess-cursor-morning-01";
    let cursor_agent = "agent-cursor-v1";
    let project_scope = Scope::Project("mem-research".to_string());

    let mut cursor_prov = Provenance::new(cursor_agent, cursor_session);
    cursor_prov.client = Some("cursor".to_string());

    let cursor_start_event = Event::new(
        cursor_session,
        cursor_agent,
        EventPayload::SessionStarted(SessionStarted {
            session_id: cursor_session.to_string(),
            agent_id: cursor_agent.to_string(),
            organization_id: None,
            environment: serde_json::json!({ "host": "cursor", "os": "windows" }),
            timestamp: chrono::Utc::now(),
        }),
    ).with_provenance(cursor_prov.clone());

    engine.append(&cursor_start_event).await?;

    // Record architectural decision in Cursor
    let decision_content = "Decided to adopt embedded SQLite with FTS5 and WAL mode for Strata local-first storage to guarantee transactional safety and cross-host file portability without running external daemon services.";
    let decision_record = MemoryRecord::new(
        MemoryType::Semantic,
        decision_content,
        project_scope.clone(),
    )
    .with_summary("Architectural Decision: Embedded SQLite with FTS5")
    .with_importance(0.95)
    .with_confidence(1.0)
    .with_tags(vec!["storage".to_string(), "sqlite".to_string(), "architecture".to_string()]);

    let handle = engine.write(&decision_record).await?;
    println!("  [Cursor] Wrote decision memory [id: {}, title: '{}']", handle.id, handle.title);

    // 3. Simulated Host 2: Claude Code Session (afternoon, different session)
    let claude_session = "sess-claude-afternoon-02";
    let claude_agent = "agent-claude-code";

    let mut claude_prov = Provenance::new(claude_agent, claude_session);
    claude_prov.client = Some("claude-code".to_string());

    let claude_start_event = Event::new(
        claude_session,
        claude_agent,
        EventPayload::SessionStarted(SessionStarted {
            session_id: claude_session.to_string(),
            agent_id: claude_agent.to_string(),
            organization_id: None,
            environment: serde_json::json!({ "host": "claude-code", "os": "windows" }),
            timestamp: chrono::Utc::now(),
        }),
    ).with_provenance(claude_prov);

    engine.append(&claude_start_event).await?;

    // Claude Code performs hybrid search for project database architecture
    let search_query = "What storage engine did we choose for local persistence?";
    let retrieved_memories = engine.search(search_query, Some(&project_scope), 3).await?;

    if retrieved_memories.is_empty() {
        bail!("Cross-host retrieval failed: Claude Code received 0 memories for query '{search_query}'");
    }

    let top_match = &retrieved_memories[0];
    println!("  [Claude Code] Retrieved top memory: '{}' [id: {}]", top_match.summary.as_deref().unwrap_or(&top_match.content), top_match.id);

    // 4. Assertions
    if !top_match.content.contains("embedded SQLite with FTS5") {
        bail!("Cross-host memory content mismatch. Expected SQLite mention, got: {}", top_match.content);
    }

    if top_match.memory_type != MemoryType::Semantic {
        bail!("Expected MemoryType::Semantic, got {:?}", top_match.memory_type);
    }

    // Claude Code checks session digest
    let digest = engine.digest(claude_session, Some(500)).await?;
    println!("  [Claude Code] Generated digest: {} pointers available", digest.key_pointers.len());

    println!("  ✓ Cross-host transfer eval scenario PASSED cleanly.");
    Ok(())
}
