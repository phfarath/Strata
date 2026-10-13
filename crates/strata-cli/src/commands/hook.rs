use anyhow::Result;
use clap::Subcommand;
use std::sync::Arc;
use strata_core::{
    state::{FailurePattern, FailureSeverity, Scope},
    traits::MemoryEngine,
};
use strata_memory::{
    record_shadow_failure, record_shadow_success, Consolidator, NeuroSymbolicConsolidator,
    SqliteMemoryEngine, SqliteStore,
};
use strata_tools::{AntiPatternParser, CommandInterceptor};
use tracing::{debug, error, info};

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

    /// Enforces mechanical A2A guardrails before a tool executes (aborts if resource is locked)
    PreTool {
        #[arg(long)]
        tool: String,

        #[arg(long)]
        target: Option<String>,

        #[arg(long)]
        params: Option<String>,

        #[arg(long, default_value = "claude-code")]
        agent: String,

        #[arg(long, default_value_t = 30)]
        ttl: u64,
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

        /// Agent identity (falls back to STRATA_AGENT_ID)
        #[arg(long)]
        agent: Option<String>,

        /// Task key (falls back to STRATA_TASK_ID)
        #[arg(long)]
        task: Option<String>,
    },

    /// Intercepts shell command execution, captures compiler/test errors out-of-band and persists AntiPatterns
    Wrap {
        /// Working directory
        #[arg(long)]
        cwd: Option<String>,

        /// Context hint or trigger description
        #[arg(long)]
        context: Option<String>,

        /// Scope filter
        #[arg(long)]
        scope: Option<String>,

        /// Agent identity (falls back to STRATA_AGENT_ID)
        #[arg(long)]
        agent: Option<String>,

        /// Task key (falls back to STRATA_TASK_ID)
        #[arg(long)]
        task: Option<String>,

        /// Command and arguments to execute
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
}

pub async fn handle_hook(command: HookCommand, engine: Arc<SqliteMemoryEngine>) -> Result<()> {
    match command {
        HookCommand::SessionStart {
            session_id,
            project,
            json,
        } => {
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
                        output.push_str(&format!(
                            "  • [{}] {}: {}\n",
                            f.error_type, f.pattern_name, f.mitigation
                        ));
                    }
                }

                if !digest.key_pointers.is_empty() {
                    output.push_str("Pointers:\n");
                    for p in &digest.key_pointers {
                        output.push_str(&format!(
                            "  • ({}) {} [id: {}]\n",
                            p.memory_type, p.title, p.id
                        ));
                    }
                }

                print!("{output}");
            }
        }

        HookCommand::UserPrompt {
            query,
            limit,
            scope,
            json,
        } => {
            debug!("Running user-prompt hook for query: '{query}'");
            let parsed_scope = scope.as_deref().and_then(|s| s.parse::<Scope>().ok());

            let memories = engine.search(&query, parsed_scope.as_ref(), limit).await?;
            let failures = engine
                .get_known_failures(Some(&query), parsed_scope.as_ref(), 2)
                .await?;

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
                    let mut fail_text =
                        String::from("⚠️ [Strata Pre-emptive Guardrails: Known Anti-Patterns]\n");
                    for f in &failures {
                        let surgical = AntiPatternParser::format_surgical_guardrail(f);
                        fail_text.push_str(&format!("  • {surgical}\n"));
                    }
                    sections.push(fail_text);
                }

                if !memories.is_empty() {
                    let mut mem_text = String::from("🧠 [Strata Relevant Context]\n");
                    for m in &memories {
                        let handle = m.to_handle(None);
                        mem_text.push_str(&format!(
                            "  • [{}] {}: {}\n",
                            handle.memory_type, handle.title, handle.summary
                        ));
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
            info!(
                "Running session-end background neuro-symbolic consolidation and sync for session '{session_id}'"
            );
            let store = engine.store_arc();
            let embedder = engine.embedding_provider();
            let sid = session_id.clone();

            tokio::spawn(async move {
                let mut consolidator = NeuroSymbolicConsolidator::new(store.clone(), embedder);
                match consolidator.consolidate_session(&sid).await {
                    Ok(res) => {
                        info!(
                            "Async neuro-symbolic consolidation completed for '{sid}': {} events, {} facts, {} skills, {} episodes",
                            res.events_processed,
                            res.semantic_facts.len(),
                            res.procedural_skills.len(),
                            res.episodic_memories.len()
                        );
                    }
                    Err(e) => {
                        error!(
                            "Async neuro-symbolic consolidation failed for session '{sid}': {e}"
                        );
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

        HookCommand::PreTool {
            tool,
            target,
            params,
            agent,
            ttl,
        } => {
            let resolved_target = target.or_else(|| {
                params.as_deref().and_then(|p| {
                    serde_json::from_str::<serde_json::Value>(p)
                        .ok()
                        .and_then(|v| {
                            v.get("file_path")
                                .or_else(|| v.get("path"))
                                .or_else(|| v.get("target"))
                                .or_else(|| v.get("file"))
                                .and_then(|f| f.as_str())
                                .map(|s| s.to_string())
                        })
                })
            });

            if let Some(target_path) = resolved_target {
                let lower_tool = tool.to_lowercase();
                let is_mutation = lower_tool.contains("edit")
                    || lower_tool.contains("write")
                    || lower_tool.contains("create")
                    || lower_tool.contains("delete")
                    || lower_tool.contains("patch")
                    || lower_tool.contains("replace")
                    || lower_tool.contains("touch")
                    || lower_tool.contains("rm");

                if is_mutation {
                    let resource_id = target_path.replace('\\', "/");
                    let store = engine.store_arc();
                    let coordinator = strata_memory::StigmergyCoordinator::new(store);

                    // Mechanical Lock Check
                    if let Ok(Some(conflict)) = coordinator.check_conflict(&resource_id, &agent) {
                        let now = chrono::Utc::now().timestamp();
                        let rem = conflict.remaining_seconds(now);
                        eprintln!(
                            "\n❌ [Strata A2A Concurrency Blocked]: Resource '{}' is exclusively leased by '{}' ({}s remaining).",
                            conflict.resource_id, conflict.agent_id, rem
                        );
                        eprintln!(
                            "Tool '{}' execution aborted mechanically to prevent concurrent edit collisions.",
                            tool
                        );
                        std::process::exit(1);
                    }

                    // Automatically acquire/renew lease for this agent while executing
                    let metadata = format!("PreTool: {}", tool);
                    let _ = coordinator.acquire_lease(
                        &resource_id,
                        &agent,
                        ttl as i64,
                        Some(&metadata),
                    );
                }
            }
        }

        HookCommand::PostTool {
            tool,
            error,
            params,
            context,
            agent,
            task,
        } => {
            let (agent_id, task_key) =
                resolve_futility_ids(agent, task, |k| std::env::var(k).ok());
            // Tool identity includes params so an unrelated success cannot mask a loop.
            let tool_ident = match params.as_deref() {
                Some(p) if !p.trim().is_empty() => format!("{tool} {p}"),
                _ => tool.clone(),
            };
            let shadow_store = engine.store_arc();
            if error.as_deref().is_none_or(|e| e.trim().is_empty()) {
                shadow_success(&shadow_store, &agent_id, &task_key, &tool_ident);
            }
            if let Some(err_msg) = error {
                if !err_msg.trim().is_empty() {
                    shadow_failure(&shadow_store, &agent_id, &task_key, &tool_ident, &err_msg);
                    debug!("Out-of-band capturing silent tool failure: tool='{tool}', error='{err_msg}'");
                    let failure =
                        AntiPatternParser::parse(&tool, "", &err_msg, 1, context.as_deref(), None)
                            .unwrap_or_else(|| {
                                let mut f = FailurePattern::new(
                            format!("{tool}_failure"),
                            format!("{tool} execution error"),
                            err_msg.clone(),
                            "Avoid repeating identical invalid parameters or unverified flags",
                        );
                                f.error_type = "ToolExecutionError".to_string();
                                f.trigger_condition = params.unwrap_or_default();
                                f.severity = FailureSeverity::High;
                                if let Some(ctx) = context {
                                    f.metadata = serde_json::json!({ "context": ctx });
                                }
                                f
                            });

                    if let Err(e) = engine.record_failure(&failure).await {
                        error!("Failed to silently record tool failure: {e}");
                    }
                }
            }
        }

        HookCommand::Wrap {
            cwd,
            context,
            scope,
            agent,
            task,
            command,
        } => {
            let (agent_id, task_key) =
                resolve_futility_ids(agent, task, |k| std::env::var(k).ok());
            let command_ident = command.join(" ");
            let parsed_scope = scope.as_deref().and_then(|s| s.parse::<Scope>().ok());
            let interceptor =
                CommandInterceptor::with_engine(Arc::clone(&engine) as Arc<dyn MemoryEngine>);

            let res = interceptor
                .execute_and_intercept(
                    &command,
                    cwd.as_deref(),
                    None,
                    context.as_deref(),
                    parsed_scope,
                )
                .await?;

            if !res.stdout.is_empty() {
                print!("{}", res.stdout);
            }
            if !res.stderr.is_empty() {
                eprint!("{}", res.stderr);
            }

            if let Some(code) = res.exit_code {
                let shadow_store = engine.store_arc();
                if code == 0 {
                    shadow_success(&shadow_store, &agent_id, &task_key, &command_ident);
                } else {
                    let detail = if res.stderr.trim().is_empty() { &res.stdout } else { &res.stderr };
                    let tail: String = detail.chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect();
                    let error = if tail.trim().is_empty() { format!("exit code {code}") } else { tail };
                    shadow_failure(&shadow_store, &agent_id, &task_key, &command_ident, &error);
                }
            }

            if let Some(guardrail) = res.surgical_guardrail {
                eprintln!("\n🧠 [Strata AntiPattern Captured]: {guardrail}");
            }

            if let Some(code) = res.exit_code {
                if code != 0 {
                    std::process::exit(code);
                }
            }
        }
    }

    Ok(())
}

/// Shadow futility recording must never affect the hook, so all helpers swallow errors.
fn resolve_futility_ids(
    agent: Option<String>,
    task: Option<String>,
    env: impl Fn(&str) -> Option<String>,
) -> (String, String) {
    let pick = |v: Option<String>, key: &str, default: &str| {
        v.or_else(|| env(key))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default.to_string())
    };
    (
        pick(agent, "STRATA_AGENT_ID", "unknown"),
        pick(task, "STRATA_TASK_ID", "default"),
    )
}

fn shadow_failure(store: &SqliteStore, agent: &str, task: &str, tool: &str, error: &str) {
    let signature = Consolidator::failure_signature(tool, error);
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = record_shadow_failure(store, agent, task, tool, &signature, now) {
        debug!("futility shadow failure not recorded: {e}");
    }
}

fn shadow_success(store: &SqliteStore, agent: &str, task: &str, tool: &str) {
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = record_shadow_success(store, agent, task, tool, now) {
        debug!("futility shadow success not recorded: {e}");
    }
}

#[cfg(test)]
mod futility_shadow_tests {
    use super::*;

    #[test]
    fn ids_prefer_args_then_env_then_defaults() {
        let env = |k: &str| match k {
            "STRATA_AGENT_ID" => Some("env-agent".to_string()),
            "STRATA_TASK_ID" => Some("env-task".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_futility_ids(Some("a".into()), Some("t".into()), env),
            ("a".to_string(), "t".to_string())
        );
        assert_eq!(
            resolve_futility_ids(None, None, env),
            ("env-agent".to_string(), "env-task".to_string())
        );
        assert_eq!(
            resolve_futility_ids(None, None, |_| None),
            ("unknown".to_string(), "default".to_string())
        );
        // Blank values are treated as missing.
        assert_eq!(
            resolve_futility_ids(Some("  ".into()), None, |_| None).0,
            "unknown".to_string()
        );
    }

    #[test]
    fn failures_with_changing_numbers_share_a_signature_and_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        for i in 0..4 {
            shadow_failure(&store, "a", "t", "cargo test", &format!("error[E0599] at line {i}"));
        }
        let trace = store.list_futility_trace("a", "t", 50).unwrap();
        assert_eq!(trace.len(), 4);
        assert_eq!(trace[3].verdict, "futile");
        assert!(trace.iter().all(|r| r.signature == trace[0].signature));
    }

    #[test]
    fn success_after_same_tool_failed_resets() {
        let store = SqliteStore::open_in_memory().unwrap();
        for _ in 0..3 {
            shadow_failure(&store, "a", "t", "cargo test", "boom");
        }
        shadow_success(&store, "a", "t", "cargo test");
        let trace = store.list_futility_trace("a", "t", 50).unwrap();
        assert_eq!(trace.last().unwrap().signal, "success");
        assert_eq!(trace.last().unwrap().evidence, 0);
    }

    #[tokio::test]
    async fn post_tool_records_shadow_trace_via_handle_hook() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let cmd = HookCommand::PostTool {
            tool: "read_file".to_string(),
            error: Some("file not found".to_string()),
            params: Some("foo.rs".to_string()),
            context: None,
            agent: Some("agent-42".to_string()),
            task: Some("task-99".to_string()),
        };
        handle_hook(cmd, Arc::clone(&engine)).await.unwrap();

        let trace = engine
            .store_arc()
            .list_futility_trace("agent-42", "task-99", 10)
            .unwrap();
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].signal, "failure");
        assert_eq!(trace[0].tool, "read_file foo.rs");

        // Now post success
        let ok_cmd = HookCommand::PostTool {
            tool: "read_file".to_string(),
            error: None,
            params: Some("foo.rs".to_string()),
            context: None,
            agent: Some("agent-42".to_string()),
            task: Some("task-99".to_string()),
        };
        handle_hook(ok_cmd, Arc::clone(&engine)).await.unwrap();

        let trace2 = engine
            .store_arc()
            .list_futility_trace("agent-42", "task-99", 10)
            .unwrap();
        assert_eq!(trace2.len(), 2);
        assert_eq!(trace2[1].signal, "success");
    }
}