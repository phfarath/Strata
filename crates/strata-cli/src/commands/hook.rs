use std::sync::Arc;
use anyhow::Result;
use clap::Subcommand;
use tracing::{debug, error, info};
use strata_core::{
    state::{FailurePattern, FailureSeverity, Scope},
    traits::MemoryEngine,
};
use strata_memory::{ConsolidationPipeline, SqliteMemoryEngine};
use crate::commands::consolidate::resolve_reasoning_engine;

#[derive(Subcommand, Debug, Clone)]
pub enum HookCommand {
    /// Injects compact session start context and pointers (~300-500 tokens)
    SessionStart {
        #[arg(long, default_value = "default")]
        session_id: String,

        #[arg(long)]
        project: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Searches relevant memories and warns about known failure anti-patterns for prompt
    UserPrompt {
        #[arg(long)]
        query: String,

        #[arg(long, default_value_t = 3)]
        limit: usize,

        #[arg(long)]
        scope: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Injects critical state reminder after context compaction
    Compact {
        #[arg(long, default_value = "default")]
        session_id: String,

        #[arg(long)]
        json: bool,
    },

    /// Triggers background consolidation and decay at session end
    SessionEnd {
        #[arg(long, default_value = "default")]
        session_id: String,
    },

    /// Silently captures tool failure out-of-band without polluting chat context
    PostTool {
        #[arg(long)]
        tool: String,

        #[arg(long)]
        error: Option<String>,

        #[arg(long)]
        params: Option<String>,

        #[arg(long)]
        context: Option<String>,
    },
}

pub async fn handle_hook(command: HookCommand, engine: Arc<SqliteMemoryEngine>) -> Result<()> {
    match command {
        HookCommand::SessionStart { session_id, project, json } => {
            debug!("Running session-start hook for session '{session_id}'");
            let digest = engine.digest(&session_id, Some(450)).await?;

            // Trigger background sync check
            let store_bg = engine.store_arc();
            let sid_bg = session_id.clone();
            tokio::spawn(async move {
                if let Ok(endpoint) = std::env::var("STRATA_SYNC_ENDPOINT") {
                    if !endpoint.trim().is_empty() {
                        let mut config = strata_core::schemas::SyncConfig::new(&sid_bg);
                        config.endpoint = Some(endpoint);
                        config.token = std::env::var("STRATA_SYNC_TOKEN").ok();
                        let sync_engine = strata_memory::SyncEngine::new(store_bg, config);
                        let _ = sync_engine.sync_cycle().await;
                    }
                }
            });

            if json {
                println!("{}", serde_json::to_string_pretty(&digest)?);
            } else {
                let mut output = String::new();
                output.push_str("🧠 [Strata Memory Context]\n");

                if !digest.summary.is_empty() {
                    output.push_str(&format!("Summary: {}\n", digest.summary));
                }

                if let Some(p) = project {
                    output.push_str(&format!("Project Scope: {p}\n"));
                }

                if !digest.recent_decisions.is_empty() {
                    output.push_str("Recent Decisions:\n");
                    for d in &digest.recent_decisions {
                        output.push_str(&format!("  • {d}\n"));
                    }
                }

                if !digest.failure_warnings.is_empty() {
                    output.push_str("⚠️ Known Anti-Patterns / Failures:\n");
                    for f in &digest.failure_warnings {
                        output.push_str(&format!("  • [{}] {}: {}\n", f.error_type, f.pattern_name, f.mitigation));
                    }
                }

                if !digest.key_pointers.is_empty() {
                    output.push_str("Pointers:\n");
                    for p in &digest.key_pointers {
                        output.push_str(&format!("  • ({}) {} [id: {}]\n", p.memory_type, p.title, p.id));
                    }
                }

                print!("{output}");
            }
        }


        HookCommand::UserPrompt { query, limit, scope, json } => {
            debug!("Running user-prompt hook for query: '{query}'");
            let parsed_scope = scope.as_deref().and_then(|s| s.parse::<Scope>().ok());

            let memories = engine.search(&query, parsed_scope.as_ref(), limit).await?;
            let failures = engine.get_known_failures(Some(&query), parsed_scope.as_ref(), 2).await?;

            if json {
                let payload = serde_json::json!({
                    "query": query,
                    "memories": memories.iter().map(|m| m.to_handle(None)).collect::<Vec<_>>(),
                    "known_failures": failures
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                let mut sections = Vec::new();

                if !failures.is_empty() {
                    let mut fail_text = String::from("⚠️ [Strata Pre-emptive Warning: Known Failures]\n");
                    for f in &failures {
                        fail_text.push_str(&format!("  - Action/Tool '{}': {}\n    Remedy: {}\n", f.signature, f.description, f.mitigation));
                    }
                    sections.push(fail_text);
                }

                if !memories.is_empty() {
                    let mut mem_text = String::from("🧠 [Strata Relevant Context]\n");
                    for m in &memories {
                        let handle = m.to_handle(None);
                        mem_text.push_str(&format!("  - [{}] {}: {}\n", handle.memory_type, handle.title, handle.summary));
                    }
                    sections.push(mem_text);
                }

                if !sections.is_empty() {
                    print!("{}", sections.join("\n"));
                }
            }
        }

        HookCommand::Compact { session_id, json } => {
            debug!("Running compact hook for session '{session_id}'");
            let digest = engine.digest(&session_id, Some(250)).await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&digest)?);
            } else {
                let mut text = String::from("🧠 [Strata Post-Compaction Reminder]\n");
                text.push_str("Active invariant: Check memory_search for prior decisions before applying breaking changes.\n");
                if !digest.recent_decisions.is_empty() {
                    text.push_str("Active Decisions:\n");
                    for d in digest.recent_decisions.iter().take(3) {
                        text.push_str(&format!("  • {d}\n"));
                    }
                }
                print!("{text}");
            }
        }

        HookCommand::SessionEnd { session_id } => {
            info!("Running session-end background consolidation and sync for session '{session_id}'");
            let store = engine.store_arc();
            let embedder = engine.embedding_provider();
            let reasoning = resolve_reasoning_engine(None);
            let pipeline = ConsolidationPipeline::with_default_config();
            let sid = session_id.clone();

            tokio::spawn(async move {
                if let Ok(events) = store.get_events(&sid, None, None) {
                    if let Err(e) = pipeline.run_pipeline(&store, embedder.as_ref(), &events, Some(reasoning.as_ref())).await {
                        error!("Async consolidation failed for session '{sid}': {e}");
                    } else {
                        info!("Async consolidation completed successfully for session '{sid}'");
                    }
                }

                // Trigger background sync cycle
                let mut config = strata_core::schemas::SyncConfig::new(&sid);
                config.endpoint = std::env::var("STRATA_SYNC_ENDPOINT").ok();
                config.token = std::env::var("STRATA_SYNC_TOKEN").ok();
                let sync_engine = strata_memory::SyncEngine::new(store, config);
                let _ = sync_engine.sync_cycle().await;
            });
        }


        HookCommand::PostTool { tool, error, params, context } => {
            if let Some(err_msg) = error {
                if !err_msg.trim().is_empty() {
                    debug!("Out-of-band capturing silent tool failure: tool='{tool}', error='{err_msg}'");
                    let mut failure = FailurePattern::new(
                        format!("{tool}_failure"),
                        format!("{tool} execution error"),
                        err_msg.clone(),
                        "Avoid repeating identical invalid parameters or unverified flags",
                    );
                    failure.error_type = "ToolExecutionError".to_string();
                    failure.trigger_condition = params.unwrap_or_default();
                    failure.severity = FailureSeverity::High;
                    if let Some(ctx) = context {
                        failure.metadata = serde_json::json!({ "context": ctx });
                    }

                    if let Err(e) = engine.record_failure(&failure).await {
                        error!("Failed to silently record tool failure: {e}");
                    }
                }
            }
        }
    }

    Ok(())
}
