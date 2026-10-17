pub mod agent_env;
pub mod commands;
pub mod config;
pub mod mcp;
pub mod tui;

pub use agent_env::*;
pub use commands::*;
pub use config::*;
pub use mcp::*;
pub use tui::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use strata_memory::SqliteStore;

    #[tokio::test]
    async fn test_cli_train_command_dry_run() {
        let temp_dir = std::env::temp_dir().join("strata_cli_train_test");
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());

        let args = TrainArgs {
            base_model: "unsloth/Llama-3.2-3B-Instruct".to_string(),
            method: "dpo".to_string(),
            quantization: "4bit".to_string(),
            lora_r: 16,
            lora_alpha: 32,
            lora_dropout: 0.0,
            learning_rate: 5e-5,
            batch_size: 2,
            gradient_accumulation_steps: 4,
            max_steps: 60,
            max_seq_length: 2048,
            out_dir: temp_dir.clone(),
            deploy_ollama: Some("strata-custom-coder".to_string()),
            dataset: None,
            scope: None,
            session: None,
            dry_run: true,
            json: false,
        };

        let res = run_train(args, store).await;
        assert!(res.is_ok());

        assert!(temp_dir.join("train_lora.py").exists());
        assert!(temp_dir.join("Modelfile").exists());
        assert!(temp_dir.join("run_training.sh").exists());
        assert!(temp_dir.join("manifest.json").exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cli_hook_wrap_and_user_prompt_guardrails() {
        use strata_core::state::Scope;
        use strata_core::traits::MemoryEngine;
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        // 1. Test PostTool with compilation error
        let post_tool = HookCommand::PostTool {
            tool: "cargo_test".to_string(),
            error: Some(
                "error: package ID specification 'strata-xyz' did not match any packages"
                    .to_string(),
            ),
            params: Some("--package strata-xyz".to_string()),
            context: Some("build_step".to_string()),
            agent: None,
            task: None,
            target: None,
            symbol: None,
        };
        handle_hook(post_tool, Arc::clone(&engine))
            .await
            .expect("handle post-tool");

        // 2. Test UserPrompt retrieves the preemptive guardrail
        let user_prompt = HookCommand::UserPrompt {
            query: "cargo test --package strata-xyz".to_string(),
            limit: 3,
            scope: Some(Scope::Global.to_string()),
            json: true,
        };
        handle_hook(user_prompt, Arc::clone(&engine))
            .await
            .expect("handle user-prompt");

        let failures = engine.get_known_failures(None, None, 5).await.unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].signature, "cargo_package_not_found");
    }

    #[tokio::test]
    async fn test_cli_callgraph_command_file_and_directory() {
        let temp_dir = std::env::temp_dir().join("strata_cli_callgraph_test");
        let _ = std::fs::create_dir_all(&temp_dir);

        let file_a = temp_dir.join("service.rs");
        let file_b = temp_dir.join("handler.rs");

        std::fs::write(
            &file_a,
            r#"
use crate::handler::process_data;

pub fn start_service() {
    process_data("hello");
}
"#,
        )
        .unwrap();

        std::fs::write(
            &file_b,
            r#"
pub fn process_data(msg: &str) {
    println!("{}", msg);
}
"#,
        )
        .unwrap();

        // Test running callgraph on a single file
        let res_file =
            run_callgraph(file_a.to_str().unwrap(), None, "all", true, 10, false, None).await;
        assert!(res_file.is_ok());

        // Test running callgraph on the whole directory
        let res_dir = run_callgraph(
            temp_dir.to_str().unwrap(),
            Some("process_data"),
            "callers",
            true,
            10,
            false,
            None,
        )
        .await;
        assert!(res_dir.is_ok());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cli_heuristic_index_workspace_and_mcp_tool() {
        use crate::mcp::server::McpServer;
        use strata_memory::SqliteMemoryEngine;

        let temp_dir = std::env::temp_dir().join(format!(
            "strata_cli_heuristic_index_test_{}",
            uuid::Uuid::new_v4()
        ));
        let src_dir = temp_dir.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        let auth_file = src_dir.join("auth.ts");
        let user_file = src_dir.join("user.ts");

        std::fs::write(
            &user_file,
            r#"
export function validateUser(name: string): boolean {
    return name.length > 0;
}
"#,
        )
        .unwrap();

        std::fs::write(
            &auth_file,
            r#"
import { validateUser } from "./user";

export function login(username: string): boolean {
    return validateUser(username);
}
"#,
        )
        .unwrap();

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let store = engine.store_arc();

        // 1. Test CLI run_callgraph with --index
        let res_index = run_callgraph(
            temp_dir.to_str().unwrap(),
            None,
            "all",
            true,
            50,
            true,
            Some(Arc::clone(&store)),
        )
        .await;
        assert!(res_index.is_ok(), "CLI workspace indexing must succeed");

        // Verify call edges persisted in SQLite
        let edges = store.get_call_edges_for_file("src/auth.ts").unwrap();
        assert!(!edges.is_empty(), "Call edges must be persisted to SQLite");
        let call_to_validate = edges.iter().find(|e| e.callee_symbol == "validateUser");
        assert!(call_to_validate.is_some());
        assert_eq!(
            call_to_validate.unwrap().callee_file_hint.as_deref(),
            Some("src/user.ts")
        );

        // Verify knowledge graph edges persisted
        let kg_count = store.get_memory_edges_count().unwrap();
        assert!(kg_count > 0, "Knowledge Graph memory edges must be persisted");

        // 2. Test MCP tool definition and execution
        let defs = McpServer::tool_definitions();
        assert!(
            defs.iter().any(|d| d.name == "memory_index_workspace"),
            "memory_index_workspace must be listed in MCP tool definitions"
        );

        let server = McpServer::new_with_engine(Arc::clone(&engine));
        let mcp_res = server
            .execute_tool(
                "memory_index_workspace",
                serde_json::json!({
                    "path": temp_dir.to_str().unwrap()
                }),
            )
            .await;

        assert!(
            mcp_res.is_error != Some(true),
            "MCP memory_index_workspace execution must succeed"
        );
        let output_text = &mcp_res.content[0].text;
        assert!(output_text.contains("Workspace Heuristic Indexing Complete"));
        assert!(output_text.contains("Files scanned: 2"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cli_workspace_command() {
        let current_dir = std::env::current_dir().unwrap();

        let args = crate::commands::workspace::WorkspaceArgs {
            path: current_dir.to_str().unwrap().to_string(),
            file: Some("crates/strata-memory/src/workspace.rs".to_string()),
            json: true,
        };

        let res = crate::commands::workspace::run_workspace(args).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_cli_promote_command_and_mcp_integration() {
        use crate::commands::promote::{run_promote, PromoteArgs};
        use crate::mcp::server::McpServer;
        use strata_core::schemas::SemanticFact;
        use strata_core::state::{MemoryRecord, MemoryTier, MemoryType, Scope};
        use strata_core::traits::MemoryEngine;
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        // 1. Write working memory record
        let mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Security rule: all auth tokens must be rotated every 24 hours",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);

        let handle = engine.write(&mem).await.expect("write memory");

        // 2. Test CLI promote command with `--yes` (bypass modal)
        let promote_args = PromoteArgs {
            id: handle.id.to_string(),
            entity_type: "memory".to_string(),
            reason: Some("ADR-099 Security audit approval".to_string()),
            yes: true,
            to_global: false,
            json: true,
        };

        let cli_res = run_promote(promote_args, Arc::clone(&engine)).await;
        assert!(cli_res.is_ok(), "CLI promotion with --yes must succeed");

        // Verify persisted state
        let fetched = engine
            .get(&handle.id)
            .await
            .unwrap()
            .expect("memory exists");
        assert_eq!(fetched.tier, MemoryTier::Core);
        assert!(fetched.approved_by_human);
        assert_eq!(fetched.importance, 1.0);

        // 3. Test MCP `strata_promote` tool
        let fact = SemanticFact::new(
            "Database connections must use SSL/TLS",
            "security",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);
        engine
            .store()
            .insert_or_update_semantic_fact(&fact)
            .expect("insert fact");

        let mcp_server =
            McpServer::new(Arc::clone(&engine) as Arc<dyn strata_core::traits::MemoryEngine>);

        // Attempt without approved_by_human
        let unapproved_call = mcp_server
            .execute_tool(
                "strata_promote",
                serde_json::json!({
                    "id": fact.id.to_string(),
                    "approved_by_human": false,
                }),
            )
            .await;
        assert!(
            unapproved_call.is_error.unwrap_or(false),
            "MCP promote without approval must fail"
        );

        // Call with approved_by_human = true for memory
        let approved_mem = MemoryRecord::new(
            MemoryType::Procedural,
            "Always run migration scripts in a transaction",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);
        let mem_handle = engine.write(&approved_mem).await.expect("write mem");

        let approved_call = mcp_server
            .execute_tool(
                "strata_promote",
                serde_json::json!({
                    "id": mem_handle.id.to_string(),
                    "approved_by_human": true,
                    "reason": "Production reliability policy",
                }),
            )
            .await;
        assert!(
            !approved_call.is_error.unwrap_or(false),
            "MCP promote with approval must succeed"
        );

        let fetched_mem = engine
            .get(&mem_handle.id)
            .await
            .unwrap()
            .expect("mem exists");
        assert_eq!(fetched_mem.tier, MemoryTier::Core);
        assert!(fetched_mem.approved_by_human);
    }

    #[tokio::test]
    async fn test_cli_export_command_with_require_verified() {
        use crate::commands::export::{run_export, ExportArgs};
        use strata_core::schemas::PreferencePair;

        let temp_dir =
            std::env::temp_dir().join(format!("strata_cli_export_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let store = Arc::new(SqliteStore::open_in_memory().unwrap());

        // Seed 1 verified pair and 1 unverified pair
        let verified_pair = PreferencePair::new(
            "Compile error: borrow checker",
            "Use clone or scoped block",
            "Use unsafe transmute",
            "sess-cli-export",
        )
        .with_verification(true, Some("cargo_test_oracle".to_string()));
        store
            .record_preference_pair(&verified_pair)
            .expect("record verified");

        let unverified_pair = PreferencePair::new(
            "Unverified experiment",
            "Try random change",
            "Do nothing",
            "sess-cli-export",
        )
        .with_verification(false, None);
        store
            .record_preference_pair(&unverified_pair)
            .expect("record unverified");

        let out_verified = temp_dir.join("verified.jsonl");
        let out_all = temp_dir.join("all.jsonl");

        // 1. Export with require_verified = true (default)
        let args_verified = ExportArgs {
            format: "dpo".to_string(),
            out: Some(out_verified.clone()),
            scope: None,
            session: Some("sess-cli-export".to_string()),
            require_verified: true,
        };
        let res_v = run_export(args_verified, Arc::clone(&store)).await;
        assert!(res_v.is_ok(), "Gated export must succeed");

        let content_v = std::fs::read_to_string(&out_verified).expect("read verified output");
        let lines_v: Vec<&str> = content_v.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            lines_v.len(),
            1,
            "Only verified pairs should be exported when require_verified is true"
        );
        let p_v: PreferencePair = serde_json::from_str(lines_v[0]).expect("parse json line");
        assert!(p_v.oracle_verified);
        assert_eq!(
            p_v.verification_source.as_deref(),
            Some("cargo_test_oracle")
        );

        // 2. Export with require_verified = false (unrestricted)
        let args_all = ExportArgs {
            format: "dpo".to_string(),
            out: Some(out_all.clone()),
            scope: None,
            session: Some("sess-cli-export".to_string()),
            require_verified: false,
        };
        let res_all = run_export(args_all, Arc::clone(&store)).await;
        assert!(res_all.is_ok(), "Unrestricted export must succeed");

        let content_all = std::fs::read_to_string(&out_all).expect("read all output");
        let lines_all: Vec<&str> = content_all
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert_eq!(
            lines_all.len(),
            2,
            "Both verified and unverified pairs should be exported when require_verified is false"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cli_reconcile_command() {
        use crate::commands::reconcile::{run_reconcile, ReconcileArgs};
        use strata_core::schemas::{FactStatus, SemanticFact};
        use strata_core::state::Scope;
        use strata_memory::{AstParser, CodeAnchorEngine};

        let temp_dir = std::env::temp_dir().join(format!(
            "strata_cli_reconcile_test_{}",
            uuid::Uuid::new_v4()
        ));
        let src_dir = temp_dir.join("src");
        let _ = std::fs::create_dir_all(&src_dir);

        let file_path = src_dir.join("service.rs");
        let initial_code = r#"
            pub fn handle_request(req_id: &str) -> bool {
                !req_id.is_empty()
            }
        "#;
        std::fs::write(&file_path, initial_code).expect("write file");

        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let parser = AstParser::new();
        let engine = CodeAnchorEngine::new();

        let sym = &parser.parse_file("src/service.rs", initial_code).unwrap()[0];
        let anchor = engine.create_anchor("src/service.rs", sym, Some("commit-1"));

        let fact = SemanticFact::new(
            "handle_request validates non-empty request IDs",
            "api",
            Scope::Global,
        )
        .with_code_anchor(anchor);
        store
            .insert_or_update_semantic_fact(&fact)
            .expect("insert fact");

        // 1. Initial reconcile on clean directory -> Fact remains active
        let args_clean = ReconcileArgs {
            workspace: temp_dir.clone(),
            commit: Some("commit-1".to_string()),
            files: vec![],
            json: true,
        };
        let res_clean = run_reconcile(args_clean, Arc::clone(&store)).await;
        assert!(res_clean.is_ok());

        let fact_clean = store.get_semantic_fact(&fact.id).unwrap().unwrap();
        assert_eq!(fact_clean.status, FactStatus::Active);

        // 2. Modify source file on disk
        let modified_code = r#"
            pub async fn handle_request(req_id: &str, auth: &str) -> Result<bool, String> {
                Ok(true)
            }
        "#;
        std::fs::write(&file_path, modified_code).expect("write modified");

        let args_mod = ReconcileArgs {
            workspace: temp_dir.clone(),
            commit: Some("commit-2".to_string()),
            files: vec![],
            json: true,
        };
        let res_mod = run_reconcile(args_mod, Arc::clone(&store)).await;
        assert!(res_mod.is_ok());

        let fact_stale = store.get_semantic_fact(&fact.id).unwrap().unwrap();
        assert_eq!(fact_stale.status, FactStatus::Stale);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cli_a2a_command_workflow() {
        use strata_memory::SqliteStore;
        use strata_memory::StigmergyCoordinator;

        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let coordinator = StigmergyCoordinator::new(store);

        // 1. Acquire lease via CLI
        let acquire_args = A2aArgs {
            action: Some(A2aAction::Acquire {
                resource: "crate:strata-cli".to_string(),
                agent: "agent-cursor".to_string(),
                ttl: 30,
                metadata: Some("Refactoring CLI".to_string()),
                json: true,
            }),
            ttl: 60,
            json: true,
        };
        let res = run_a2a(acquire_args, coordinator.clone()).await;
        assert!(res.is_ok());

        // 2. Status inspection via CLI
        let status_args = A2aArgs {
            action: Some(A2aAction::Status {
                ttl: 60,
                json: true,
            }),
            ttl: 60,
            json: true,
        };
        let res_status = run_a2a(status_args, coordinator.clone()).await;
        assert!(res_status.is_ok());

        // 3. Release lease via CLI
        let release_args = A2aArgs {
            action: Some(A2aAction::Release {
                resource: "crate:strata-cli".to_string(),
                agent: "agent-cursor".to_string(),
                json: true,
            }),
            ttl: 60,
            json: true,
        };
        let res_release = run_a2a(release_args, coordinator.clone()).await;
        assert!(res_release.is_ok());
    }

    #[tokio::test]
    async fn test_mcp_stigmergy_tools_execution() {
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        let test_id = uuid::Uuid::new_v4();
        #[cfg(windows)]
        let endpoint = format!(r"\\.\pipe\strata-mcp-test-{}", test_id);
        #[cfg(unix)]
        let endpoint = format!("/tmp/strata-mcp-test-{}.sock", &test_id.to_string()[..8]);

        let ipc_server = Arc::new(
            strata_memory::IpcServer::bind(&endpoint)
                .await
                .expect("bind server"),
        );
        let ipc_client = Arc::new(
            strata_memory::IpcClient::connect(&endpoint)
                .await
                .expect("connect client"),
        );
        let mut ipc_sub = ipc_client.subscribe();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let server = McpServer::new_with_engine(Arc::clone(&engine))
            .with_ipc(Some(Arc::clone(&ipc_client)), Some(ipc_server));

        // 1. Verify tool definitions
        let defs = McpServer::tool_definitions();
        assert!(defs.iter().any(|t| t.name == "lease_acquire"));
        assert!(defs.iter().any(|t| t.name == "lease_release"));
        assert!(defs.iter().any(|t| t.name == "agent_who"));
        assert!(defs.iter().any(|t| t.name == "agent_heartbeat"));

        // 2. Heartbeat tool
        let hb_res = server
            .execute_tool(
                "agent_heartbeat",
                serde_json::json!({
                    "agent_id": "cursor-01",
                    "host": "cursor",
                    "pid": 9999,
                    "active_task": "editing mcp"
                }),
            )
            .await;
        assert!(hb_res.is_error != Some(true));

        // 3. Lease acquire tool (should broadcast LeaseAcquired over IPC)
        let acq_res = server
            .execute_tool(
                "lease_acquire",
                serde_json::json!({
                    "resource_id": "file:crates/strata-cli/src/main.rs",
                    "agent_id": "cursor-01",
                    "ttl_seconds": 60,
                    "metadata": "updating CLI commands"
                }),
            )
            .await;
        assert!(acq_res.is_error != Some(true));

        // Verify real-time IPC broadcast received
        let ipc_event = tokio::time::timeout(std::time::Duration::from_secs(3), ipc_sub.recv())
            .await
            .expect("timeout waiting for LeaseAcquired event")
            .expect("receive LeaseAcquired event");
        match ipc_event {
            strata_core::a2a::IpcEvent::LeaseAcquired {
                resource_id,
                agent_id,
                ..
            } => {
                assert_eq!(resource_id, "file:crates/strata-cli/src/main.rs");
                assert_eq!(agent_id, "cursor-01");
            }
            other => panic!("Expected LeaseAcquired, got {other:?}"),
        }

        // 4. Agent who tool
        let who_res = server
            .execute_tool("agent_who", serde_json::json!({ "ttl_seconds": 60 }))
            .await;
        assert!(who_res.is_error != Some(true));

        // 5. Conflict detection: second agent attempts to acquire same file
        let conflict_res = server
            .execute_tool(
                "lease_acquire",
                serde_json::json!({
                    "resource_id": "file:crates/strata-cli/src/main.rs",
                    "agent_id": "claude-02",
                    "ttl_seconds": 60
                }),
            )
            .await;
        assert!(conflict_res.is_error != Some(true));
        let conflict_text = &conflict_res.content[0].text;
        assert!(conflict_text.contains("\"status\": \"conflict\""));
        assert!(conflict_text.contains("\"held_by\": \"cursor-01\""));

        // 6. Release tool (should broadcast LeaseReleased over IPC)
        let rel_res = server
            .execute_tool(
                "lease_release",
                serde_json::json!({
                    "resource_id": "file:crates/strata-cli/src/main.rs",
                    "agent_id": "cursor-01"
                }),
            )
            .await;
        assert!(rel_res.is_error != Some(true));

        // Verify real-time IPC broadcast received for release
        let rel_event = tokio::time::timeout(std::time::Duration::from_secs(3), ipc_sub.recv())
            .await
            .expect("timeout waiting for LeaseReleased event")
            .expect("receive LeaseReleased event");
        match rel_event {
            strata_core::a2a::IpcEvent::LeaseReleased {
                resource_id,
                agent_id,
                ..
            } => {
                assert_eq!(resource_id, "file:crates/strata-cli/src/main.rs");
                assert_eq!(agent_id, "cursor-01");
            }
            other => panic!("Expected LeaseReleased, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_cli_consolidate_neuro_symbolic_offline() {
        use crate::commands::consolidate::{run_consolidate, ConsolidateOptions};
        use chrono::Utc;
        use strata_core::events::{
            ErrorObserved, Event, EventPayload, ObservationReceived, TaskCompleted, ToolInvoked,
            ToolResultReceived,
        };
        use strata_core::schemas::FactStatus;
        use uuid::Uuid;

        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let session_id = "test-cli-consolidate";

        // Record a sequence of lifecycle events
        let ev1 = Event::new(
            session_id,
            "cursor",
            EventPayload::ObservationReceived(ObservationReceived {
                session_id: session_id.to_string(),
                source: "user".to_string(),
                content: serde_json::json!("Architecture decision: Migrated primary database from SQLite to PostgreSQL for concurrency"),
                observation_type: "architecture".to_string(),
                timestamp: Utc::now(),
            }),
        );
        let ev2 = Event::new(
            session_id,
            "cursor",
            EventPayload::ErrorObserved(ErrorObserved {
                error_type: "CompilerError".to_string(),
                message: "E0432: unresolved import serde_json".to_string(),
                severity: "warning".to_string(),
                context: None,
                stack_trace: None,
                timestamp: Utc::now(),
            }),
        );
        let ev3 = Event::new(
            session_id,
            "cursor",
            EventPayload::ToolInvoked(ToolInvoked {
                invocation_id: Uuid::new_v4(),
                tool_name: "bash".to_string(),
                input: serde_json::json!({ "command": "cargo add serde_json" }),
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
            }),
        );
        let ev4 = Event::new(
            session_id,
            "cursor",
            EventPayload::ToolResultReceived(ToolResultReceived {
                invocation_id: Uuid::new_v4(),
                tool_name: "bash".to_string(),
                result: serde_json::json!("    Adding serde_json v1.0 to dependencies"),
                is_error: false,
                duration_ms: Some(120),
                timestamp: Utc::now(),
            }),
        );
        let ev5 = Event::new(
            session_id,
            "cursor",
            EventPayload::TaskCompleted(TaskCompleted {
                task_id: "fix-import".to_string(),
                success: true,
                outcome_summary: "Resolved missing serde_json import".to_string(),
                evaluation: None,
                timestamp: Utc::now(),
            }),
        );

        store.insert_event(&ev1).unwrap();
        store.insert_event(&ev2).unwrap();
        store.insert_event(&ev3).unwrap();
        store.insert_event(&ev4).unwrap();
        store.insert_event(&ev5).unwrap();

        // Run CLI consolidate command offline
        let opts = ConsolidateOptions {
            session: Some(session_id.to_string()),
            all: false,
            model: None,
            provider: None,
            json: true,
            hippocampus: false,
        };

        let res = run_consolidate(opts, Arc::clone(&store)).await;
        assert!(res.is_ok(), "CLI run_consolidate must succeed");

        // Verify that semantic facts and procedural skills were mined
        let facts = store
            .get_all_semantic_facts(None, Some(FactStatus::Active), 10)
            .unwrap();
        assert!(
            !facts.is_empty(),
            "Must consolidate architectural decision into semantic fact"
        );

        let skills = store.get_all_procedural_skills(None, 10).unwrap();
        assert!(!skills.is_empty(), "Must mine procedural recovery skill");
        assert!(skills
            .iter()
            .any(|s| s.name.contains("Recover_CompilerError")
                && s.steps
                    .iter()
                    .any(|step| step.arguments.to_string().contains("cargo add serde_json"))));
    }

    #[tokio::test]
    async fn test_mcp_memory_consolidate_tool() {
        use crate::mcp::server::McpServer;
        use chrono::Utc;
        use strata_core::events::{Event, EventPayload, ObservationReceived};
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let store = engine.store_arc();
        let session_id = "test-mcp-consolidate";

        let ev = Event::new(
            session_id,
            "claude-code",
            EventPayload::ObservationReceived(ObservationReceived {
                session_id: session_id.to_string(),
                source: "system".to_string(),
                content: serde_json::json!(
                    "Protocol decision: All inter-agent comms use Stigmergic leases"
                ),
                observation_type: "architecture".to_string(),
                timestamp: Utc::now(),
            }),
        );
        store.insert_event(&ev).unwrap();

        let server = McpServer::new_with_engine(Arc::clone(&engine));

        // Call memory_consolidate tool
        let tool_res = server
            .execute_tool(
                "memory_consolidate",
                serde_json::json!({
                    "session_id": session_id,
                    "all_sessions": false,
                    "enrich_with_llm": false
                }),
            )
            .await;

        assert!(
            tool_res.is_error != Some(true),
            "MCP memory_consolidate must succeed"
        );
        let text = &tool_res.content[0].text;
        assert!(text.contains("Consolidation complete"));
        assert!(text.contains("(0 tokens)"));
    }

    #[tokio::test]
    async fn test_mcp_memory_graph_explore_tool() {
        use crate::mcp::server::McpServer;
        use strata_core::state::{MemoryRecord, MemoryType, Scope};
        use strata_core::traits::MemoryEngine;
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        // Insert connected memories in Knowledge Graph
        let mut mem1 = MemoryRecord::new(
            MemoryType::Semantic,
            "Database pool initialized via init_db in src/database/postgres.rs",
            Scope::Project("test-proj".to_string()),
        );
        mem1.metadata = serde_json::json!({
            "symbols": ["init_db"],
            "file_paths": ["src/database/postgres.rs"]
        });
        mem1.tags = vec!["database".to_string()];
        engine.write(&mem1).await.unwrap();

        let mut mem2 = MemoryRecord::new(
            MemoryType::NegativePattern,
            "Connection pool starvation: deadpool_postgres max_size must match server limits",
            Scope::Project("test-proj".to_string()),
        );
        mem2.metadata = serde_json::json!({
            "symbols": ["init_db"],
            "file_paths": ["src/database/postgres.rs"]
        });
        mem2.tags = vec!["database".to_string(), "tuning".to_string()];
        mem2.importance = 0.95;
        engine.write(&mem2).await.unwrap();

        let server = McpServer::new_with_engine(Arc::clone(&engine));

        // 1. Verify tool definition exists
        let defs = McpServer::tool_definitions();
        assert!(
            defs.iter().any(|d| d.name == "memory_graph_explore"),
            "memory_graph_explore must be registered in MCP tool definitions"
        );

        // 2. Call memory_graph_explore with seed "init_db"
        let tool_res = server
            .execute_tool(
                "memory_graph_explore",
                serde_json::json!({
                    "seed": "init_db",
                    "limit": 5
                }),
            )
            .await;

        assert!(
            tool_res.is_error != Some(true),
            "memory_graph_explore must succeed"
        );
        let text = &tool_res.content[0].text;
        assert!(
            text.contains("spreading_activation_score"),
            "Result must include spreading activation scores"
        );
        assert!(
            text.contains(&mem2.id.to_string()),
            "Connected memory must be recalled via graph activation"
        );
    }

    #[tokio::test]
    async fn test_cli_hook_session_end_automatic_consolidation() {
        use crate::commands::hook::{handle_hook, HookCommand};
        use chrono::Utc;
        use strata_core::events::{Event, EventPayload, ObservationReceived};
        use strata_core::schemas::FactStatus;
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let store = engine.store_arc();
        let session_id = "test-hook-session-end";

        let ev = Event::new(
            session_id,
            "cursor",
            EventPayload::ObservationReceived(ObservationReceived {
                session_id: session_id.to_string(),
                source: "user".to_string(),
                content: serde_json::json!("Decision: Set maximum retry count to 3"),
                observation_type: "architecture".to_string(),
                timestamp: Utc::now(),
            }),
        );
        store.insert_event(&ev).unwrap();

        // Run automatic SessionEnd hook
        let hook_cmd = HookCommand::SessionEnd {
            session_id: session_id.to_string(),
        };
        let hook_res = handle_hook(hook_cmd, Arc::clone(&engine)).await;
        assert!(hook_res.is_ok(), "Hook execution must succeed");

        // Wait briefly for background tokio task
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let facts = store
            .get_all_semantic_facts(None, Some(FactStatus::Active), 10)
            .unwrap();
        assert!(
            !facts.is_empty(),
            "Automatic consolidation must persist semantic fact"
        );
    }

    #[tokio::test]
    async fn test_multi_agent_blackboard_cli_workflow() {
        use crate::commands::task::{run_task, TaskAction, TaskArgs};
        use crate::commands::a2a::{run_a2a, A2aAction, A2aArgs};
        use crate::commands::plan::{run_plan, PlanArgs};
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let coordinator = engine.stigmergy();

        // 1. Create a persistent plan with --persist
        let plan_args = PlanArgs {
            goal: Some("Build user authentication and write tests".to_string()),
            execute: false,
            concurrency: 2,
            no_recover: false,
            persist: true,
            json: true,
        };
        let plan_res = run_plan(plan_args, Some(coordinator.clone())).await;
        assert!(plan_res.is_ok());

        // 2. List tasks
        let list_args = TaskArgs {
            action: TaskAction::List {
                plan: None,
                status: None,
                agent: None,
                limit: 20,
                json: true,
            },
        };
        assert!(run_task(list_args, coordinator.clone()).await.is_ok());

        // 3. Agent 1 (claude) claims next task
        let claim_args1 = TaskArgs {
            action: TaskAction::Claim {
                task: None,
                next: true,
                agent: Some("agent-claude".to_string()),
                ttl: 60,
                json: true,
            },
        };
        assert!(run_task(claim_args1, coordinator.clone()).await.is_ok());

        // Verify task is claimed by claude
        let filter_claude = strata_core::blackboard::TaskFilter {
            assigned_agent: Some("agent-claude".to_string()),
            ..Default::default()
        };
        let claude_tasks = coordinator.list_tasks(&filter_claude).unwrap();
        assert_eq!(claude_tasks.len(), 1);
        let claimed_id = claude_tasks[0].task_id.clone();

        // 4. Claude updates progress
        let prog_args = TaskArgs {
            action: TaskAction::Progress {
                task: Some(claimed_id.clone()),
                message: "Writing authentication models".to_string(),
                agent: Some("agent-claude".to_string()),
                generation: None,
                json: true,
            },
        };
        assert!(run_task(prog_args, coordinator.clone()).await.is_ok());

        // 5. Claude sends A2A message to agent-cursor
        let send_args = A2aArgs {
            action: Some(A2aAction::Send {
                to: "agent-cursor".to_string(),
                content: "Auth model finished, please review when ready".to_string(),
                task: Some(claimed_id.clone()),
                agent: Some("agent-claude".to_string()),
                kind: "handoff".to_string(),
                json: true,
            }),
            ttl: 60,
            json: true,
        };
        assert!(run_a2a(send_args, coordinator.clone()).await.is_ok());

        // 6. Cursor inspects inbox
        let inbox_args = A2aArgs {
            action: Some(A2aAction::Inbox {
                agent: Some("agent-cursor".to_string()),
                unread: true,
                mark_read: true,
                json: true,
            }),
            ttl: 60,
            json: true,
        };
        assert!(run_a2a(inbox_args, coordinator.clone()).await.is_ok());

        // 7. Claude completes task with artifact
        let comp_args = TaskArgs {
            action: TaskAction::Complete {
                task: Some(claimed_id.clone()),
                summary: Some("Auth models and middleware implemented".to_string()),
                artifact: None,
                artifact_content: Some("User struct, JwtValidator, TokenPair".to_string()),
                agent: Some("agent-claude".to_string()),
                generation: None,
                json: true,
            },
        };
        assert!(run_task(comp_args, coordinator.clone()).await.is_ok());

        // 8. Inspect task
        let inspect_args = TaskArgs {
            action: TaskAction::Inspect {
                task: claimed_id.clone(),
                json: true,
            },
        };
        assert!(run_task(inspect_args, coordinator.clone()).await.is_ok());

        // 9. Agent Cursor now claims next unblocked task
        let claim_args2 = TaskArgs {
            action: TaskAction::Claim {
                task: None,
                next: true,
                agent: Some("agent-cursor".to_string()),
                ttl: 60,
                json: true,
            },
        };
        assert!(run_task(claim_args2, coordinator.clone()).await.is_ok());

        let filter_cursor = strata_core::blackboard::TaskFilter {
            assigned_agent: Some("agent-cursor".to_string()),
            ..Default::default()
        };
        let cursor_tasks = coordinator.list_tasks(&filter_cursor).unwrap();
        assert!(!cursor_tasks.is_empty(), "Cursor must successfully claim next unblocked task");

        // 10. Cursor goes silent: reclaim returns its task to the pool
        let reclaim_args = TaskArgs {
            action: TaskAction::Reclaim {
                stale_after: 0,
                json: true,
            },
        };
        assert!(run_task(reclaim_args, coordinator.clone()).await.is_ok());
        let orphan = &coordinator.list_tasks(&filter_cursor).unwrap();
        assert!(orphan.is_empty(), "Reclaimed task must be unassigned from the silent agent");
    }

    #[tokio::test]
    async fn test_mcp_blackboard_tools_end_to_end() {
        use strata_core::blackboard::{TaskRecord, TaskStatus};
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let coordinator = engine.stigmergy();
        let server = McpServer::new_with_engine(Arc::clone(&engine));

        let defs = McpServer::tool_definitions();
        for name in [
            "task_list",
            "task_claim",
            "task_progress",
            "task_complete",
            "task_fail",
            "task_reclaim",
            "a2a_send",
            "a2a_inbox",
        ] {
            assert!(defs.iter().any(|t| t.name == name), "missing MCP tool {name}");
        }

        coordinator.create_task(&TaskRecord::new("t1", "First")).unwrap();
        coordinator
            .create_task(
                &TaskRecord::new("t2", "Second")
                    .with_status(TaskStatus::Blocked)
                    .with_dependencies(vec!["t1".to_string()]),
            )
            .unwrap();

        async fn call(server: &McpServer, tool: &str, args: serde_json::Value) -> serde_json::Value {
            let res = server.execute_tool(tool, args).await;
            assert!(res.is_error != Some(true), "{tool} failed: {}", res.content[0].text);
            serde_json::from_str(&res.content[0].text).expect("tool output must be JSON")
        }

        // Work-stealing: a1 gets t1, a2 finds nothing (t2 still blocked)
        let c1 = call(&server, "task_claim", serde_json::json!({ "agent_id": "a1" })).await;
        assert_eq!(c1["status"], "claimed");
        assert_eq!(c1["task"]["task_id"], "t1");
        let c2 = call(&server, "task_claim", serde_json::json!({ "agent_id": "a2" })).await;
        assert_eq!(c2["status"], "none_available");

        call(
            &server,
            "task_progress",
            serde_json::json!({ "task_id": "t1", "agent_id": "a1", "message": "halfway" }),
        )
        .await;
        let running = call(&server, "task_list", serde_json::json!({ "status": "running" })).await;
        assert_eq!(running.as_array().unwrap().len(), 1);

        call(
            &server,
            "task_complete",
            serde_json::json!({
                "task_id": "t1", "agent_id": "a1",
                "summary": "done", "artifact_content": "schema.sql"
            }),
        )
        .await;
        let inspected = call(&server, "task_list", serde_json::json!({ "task_id": "t1" })).await;
        assert_eq!(inspected["task"]["status"], "completed");
        assert_eq!(inspected["artifacts"].as_array().unwrap().len(), 1);

        // Downstream unblocked, upstream artifacts handed to the claimer
        let c3 = call(&server, "task_claim", serde_json::json!({ "agent_id": "a2" })).await;
        assert_eq!(c3["task"]["task_id"], "t2");
        assert_eq!(c3["upstream_artifacts"][0]["payload"], "schema.sql");

        call(
            &server,
            "task_fail",
            serde_json::json!({ "task_id": "t2", "agent_id": "a2", "error": "boom" }),
        )
        .await;
        assert_eq!(coordinator.get_task("t2").unwrap().unwrap().status, TaskStatus::Failed);

        // Orphan recovery
        coordinator.create_task(&TaskRecord::new("t3", "Orphan")).unwrap();
        call(&server, "task_claim", serde_json::json!({ "task_id": "t3", "agent_id": "a3" })).await;
        let rec = call(&server, "task_reclaim", serde_json::json!({ "stale_secs": 0 })).await;
        assert_eq!(rec["reclaimed"], serde_json::json!(["t3"]));

        // A2A mailbox: direct + broadcast reach a2
        call(&server, "a2a_send", serde_json::json!({ "agent_id": "a1", "to": "a2", "content": "review?" })).await;
        call(&server, "a2a_send", serde_json::json!({ "agent_id": "a1", "content": "all green" })).await;
        let inbox = call(&server, "a2a_inbox", serde_json::json!({ "agent_id": "a2", "unread": true })).await;
        assert_eq!(inbox.as_array().unwrap().len(), 2);
        let after = call(&server, "a2a_inbox", serde_json::json!({ "agent_id": "a2", "unread": true })).await;
        assert!(after.as_array().unwrap().is_empty(), "inbox read must mark messages as read");

        // Validation
        let bad = server.execute_tool("task_complete", serde_json::json!({ "agent_id": "a1" })).await;
        assert_eq!(bad.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_mcp_and_cli_episode_tools_and_consolidation() {
        use crate::commands::episode::{run_episode, EpisodeArgs, EpisodeCommand};
        use crate::mcp::server::McpServer;
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let store = engine.store_arc();
        let server = McpServer::new_with_engine(Arc::clone(&engine));

        // 1. Record an episode via MCP
        let record_res = server
            .execute_tool(
                "episode_record",
                serde_json::json!({
                    "summary": "Fix connection timeout in Redis client",
                    "session_id": "session-redis",
                    "agent_id": "agent-redis-worker",
                    "task_id": "task-redis-101",
                    "salience_score": 0.85,
                    "outcomes": ["Timeout eliminated", "Throughput doubled"],
                    "steps": [
                        {
                            "action": "Increase socket timeout to 5000ms",
                            "observation": "Socket connection established reliably"
                        },
                        {
                            "action": "Disable TCP keepalive prematurely",
                            "observation": "Broken pipe error",
                            "error": "ConnectionResetByPeer"
                        }
                    ],
                    "causal_provenance": "High latency cloud VPC environment"
                }),
            )
            .await;

        assert!(record_res.is_error != Some(true), "episode_record must succeed");
        let content_text = &record_res.content[0].text;
        assert!(content_text.contains("Recorded episode trace"));

        // List episodes from store
        let episodes = store.list_episodes(Some("session-redis"), None, 10).unwrap();
        assert_eq!(episodes.len(), 1);
        let ep = &episodes[0];
        assert_eq!(ep.salience_score, 0.85);
        assert_eq!(ep.event_trace.len(), 2);
        assert!(!ep.consolidated);

        // 2. Replay the episode via MCP
        let replay_res = server
            .execute_tool(
                "episode_replay",
                serde_json::json!({ "id": ep.id.to_string() }),
            )
            .await;
        assert!(replay_res.is_error != Some(true), "episode_replay must succeed");
        let replay_text = &replay_res.content[0].text;
        assert!(replay_text.contains("total steps"));

        // 3. Consolidate via MCP
        let consolidate_res = server
            .execute_tool(
                "episode_consolidate",
                serde_json::json!({ "session_id": "session-redis" }),
            )
            .await;
        assert!(consolidate_res.is_error != Some(true), "episode_consolidate must succeed");
        let cons_text = &consolidate_res.content[0].text;
        assert!(cons_text.contains("Hippocampal consolidation completed"));
        assert!(cons_text.contains("1 episodes processed"));

        // Verify that facts and anti-patterns are persisted and marked consolidated
        let updated_episodes = store.list_episodes(Some("session-redis"), None, 10).unwrap();
        assert!(updated_episodes[0].consolidated);

        let facts = store.get_all_semantic_facts(None, None, 10).unwrap();
        assert!(!facts.is_empty(), "Must synthesize semantic fact from high-salience episode");
        assert_eq!(facts[0].tier, strata_core::state::MemoryTier::Core);

        let failures = store.search_failures(None, None, 10).unwrap();
        assert!(!failures.is_empty(), "Must learn anti-pattern from failed step");

        // 4. Test CLI EpisodeCommand execution
        let cli_record_args = EpisodeArgs {
            command: EpisodeCommand::Record {
                summary: "CLI episode verification".to_string(),
                session: "cli-session".to_string(),
                task_id: None,
                agent: "cli-agent".to_string(),
                salience: 0.9,
                outcomes: Some("CLI works cleanly".to_string()),
                steps_json: None,
                provenance: None,
                json: true,
            },
        };
        assert!(run_episode(cli_record_args, Arc::clone(&store)).await.is_ok());

        let cli_list_args = EpisodeArgs {
            command: EpisodeCommand::List {
                session: Some("cli-session".to_string()),
                limit: 10,
                json: true,
            },
        };
        assert!(run_episode(cli_list_args, Arc::clone(&store)).await.is_ok());

        let cli_cons_args = EpisodeArgs {
            command: EpisodeCommand::Consolidate {
                session: Some("cli-session".to_string()),
                json: true,
            },
        };
        assert!(run_episode(cli_cons_args, Arc::clone(&store)).await.is_ok());
    }

    #[tokio::test]
    async fn test_mcp_and_cli_hierarchical_pyramid() {
        use crate::commands::episode::{run_episode, EpisodeArgs, EpisodeCommand};
        use crate::mcp::server::McpServer;
        use strata_core::schemas::{EpisodeRecord, EpisodeStep};
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let store = engine.store_arc();
        let server = McpServer::new_with_engine(Arc::clone(&engine));

        // 1. Insert 2 episodes with shared task_id to form a cluster
        let ep1 = EpisodeRecord::new("sess-pyr", "agent-1", "Step 1: Setup migration")
            .with_task_id("task-cluster-1")
            .with_salience(0.8)
            .with_outcomes(vec!["Migration tables ready".to_string()])
            .with_steps(vec![
                EpisodeStep::new(0, "Create table auth_users").with_output(serde_json::json!({ "observation": "Table created" })),
            ]);
        let ep2 = EpisodeRecord::new("sess-pyr", "agent-2", "Step 2: Migrate JWT keys")
            .with_task_id("task-cluster-1")
            .with_salience(0.9)
            .with_outcomes(vec!["JWT keys migrated".to_string()])
            .with_steps(vec![
                EpisodeStep::new(0, "Rotate HMAC secret").with_output(serde_json::json!({ "observation": "Secret updated" })),
            ]);

        store.write_episode(&ep1).unwrap();
        store.write_episode(&ep2).unwrap();

        // 2. Test CLI Rollup (Tier 1 -> Tier 2)
        let cli_rollup_args = EpisodeArgs {
            command: EpisodeCommand::Rollup {
                session: Some("sess-pyr".to_string()),
                min_cluster: 2,
                similarity: 0.70,
                slm_narrative: false,
                adapter_path: None,
                python_bin: None,
                json: true,
            },
        };
        assert!(run_episode(cli_rollup_args, Arc::clone(&store)).await.is_ok());

        let rollups = store.list_episodes_by_tier(2, Some("sess-pyr"), 10).unwrap();
        assert_eq!(rollups.len(), 1);
        let rollup_id = rollups[0].id;

        // 2b. Test CLI Rollup with --slm-narrative (Tier 2 -> Tier 3)
        let cli_rollup_slm_args = EpisodeArgs {
            command: EpisodeCommand::Rollup {
                session: Some("sess-pyr".to_string()),
                min_cluster: 2,
                similarity: 0.70,
                slm_narrative: true,
                adapter_path: None,
                python_bin: None,
                json: true,
            },
        };
        assert!(run_episode(cli_rollup_slm_args, Arc::clone(&store)).await.is_ok());
        let tier3_list = store.list_episodes_by_tier(3, Some("sess-pyr"), 10).unwrap();
        assert_eq!(tier3_list.len(), 1);
        assert_eq!(tier3_list[0].tier, 3);
        assert!(tier3_list[0].summary.starts_with("Narrativa:"));

        // 3. Test CLI DrillDown (both json and ascii)
        let cli_drill_json = EpisodeArgs {
            command: EpisodeCommand::DrillDown {
                id: rollup_id.to_string(),
                json: true,
                ascii: false,
            },
        };
        assert!(run_episode(cli_drill_json, Arc::clone(&store)).await.is_ok());

        let cli_drill_ascii = EpisodeArgs {
            command: EpisodeCommand::DrillDown {
                id: rollup_id.to_string(),
                json: false,
                ascii: true,
            },
        };
        assert!(run_episode(cli_drill_ascii, Arc::clone(&store)).await.is_ok());

        // 4. Test CLI Tree
        let cli_tree = EpisodeArgs {
            command: EpisodeCommand::Tree {
                session: Some("sess-pyr".to_string()),
                limit: 10,
                json: true,
            },
        };
        assert!(run_episode(cli_tree, Arc::clone(&store)).await.is_ok());

        // 5. Test MCP memory_episode_pyramid zoom_out
        let mcp_zoom = server
            .execute_tool(
                "memory_episode_pyramid",
                serde_json::json!({
                    "action": "zoom_out",
                    "session_id": "sess-pyr",
                    "limit": 10
                }),
            )
            .await;
        assert!(mcp_zoom.is_error != Some(true));
        assert!(mcp_zoom.content[0].text.contains("1 Tier 2 milestone rollups"));

        // 6. Test MCP memory_episode_pyramid drill_down
        let mcp_drill = server
            .execute_tool(
                "memory_episode_pyramid",
                serde_json::json!({
                    "action": "drill_down",
                    "rollup_id": rollup_id.to_string()
                }),
            )
            .await;
        assert!(mcp_drill.is_error != Some(true));
        assert!(mcp_drill.content[0].text.contains("Rollup"));

        // 7. Test MCP memory_episode_pyramid tree
        let mcp_tree = server
            .execute_tool(
                "memory_episode_pyramid",
                serde_json::json!({
                    "action": "tree",
                    "session_id": "sess-pyr",
                    "limit": 10
                }),
            )
            .await;
        assert!(mcp_tree.is_error != Some(true));

        // 8. Test MCP memory_episode_pyramid rollup
        let mcp_rollup = server
            .execute_tool(
                "memory_episode_pyramid",
                serde_json::json!({
                    "action": "rollup",
                    "session_id": "sess-pyr"
                }),
            )
            .await;
        assert!(mcp_rollup.is_error != Some(true));

        // 9. Test MCP memory_pyramid_drilldown compact (expand_episodes = false)
        let mcp_p_compact = server
            .execute_tool(
                "memory_pyramid_drilldown",
                serde_json::json!({
                    "session_id": "sess-pyr",
                    "expand_episodes": false
                }),
            )
            .await;
        assert!(mcp_p_compact.is_error != Some(true));
        assert!(mcp_p_compact.content[0].text.contains("Hierarchical Pyramid Rollups (Compact Tier 2)"));
        let val_compact: serde_json::Value = serde_json::from_str(&mcp_p_compact.structured_content.unwrap().to_string()).unwrap();
        assert_eq!(val_compact["expanded"], false);
        assert_eq!(val_compact["rollups_count"], 1);
        assert_eq!(val_compact["total_steps"], 0);
        assert_eq!(val_compact["summaries"][0]["id"], rollup_id.to_string());

        // 10. Test MCP memory_pyramid_drilldown expanded (expand_episodes = true)
        let mcp_p_expanded = server
            .execute_tool(
                "memory_pyramid_drilldown",
                serde_json::json!({
                    "cluster_id": "task-cluster-1",
                    "expand_episodes": true
                }),
            )
            .await;
        assert!(mcp_p_expanded.is_error != Some(true));
        assert!(mcp_p_expanded.content[0].text.contains("Hierarchical Pyramid Drill-Down (Expanded)"));
        let val_expanded: serde_json::Value = serde_json::from_str(&mcp_p_expanded.structured_content.unwrap().to_string()).unwrap();
        assert_eq!(val_expanded["expanded"], true);
        assert_eq!(val_expanded["rollups_count"], 1);
        assert_eq!(val_expanded["total_steps"], 2);
        assert_eq!(val_expanded["trees"][0]["root"]["id"], rollup_id.to_string());
        assert_eq!(val_expanded["trees"][0]["leaves"].as_array().unwrap().len(), 2);

        // 11. Test MCP memory_pyramid_drilldown by rollup_id
        let mcp_p_id = server
            .execute_tool(
                "memory_pyramid_drilldown",
                serde_json::json!({
                    "rollup_id": rollup_id.to_string(),
                    "expand_episodes": true
                }),
            )
            .await;
        assert!(mcp_p_id.is_error != Some(true));
        let val_id: serde_json::Value = serde_json::from_str(&mcp_p_id.structured_content.unwrap().to_string()).unwrap();
        assert_eq!(val_id["rollups_count"], 1);
        assert_eq!(val_id["trees"][0]["root"]["id"], rollup_id.to_string());
    }

    #[tokio::test]
    async fn test_autonomous_blackboard_worker_daemon_lifecycle() {
        use crate::commands::worker::{run_worker, WorkerArgs};
        use strata_core::blackboard::{TaskRecord, TaskStatus};
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let coordinator = engine.stigmergy();

        // 1. Setup a DAG on the blackboard: task1 -> task2 (depends on task1)
        let task1 = TaskRecord::new("w-task-1", "Compile core models")
            .with_description("cmd: echo \"Core compiled\"");
        let task2 = TaskRecord::new("w-task-2", "Package binary")
            .with_status(TaskStatus::Blocked)
            .with_dependencies(vec!["w-task-1".to_string()]);
        let task_failing = TaskRecord::new("w-task-fail", "Run broken oracle")
            .with_description("cmd: exit 42");

        coordinator.create_task(&task1).unwrap();
        coordinator.create_task(&task2).unwrap();
        coordinator.create_task(&task_failing).unwrap();

        // Check initial statuses
        assert_eq!(coordinator.get_task("w-task-1").unwrap().unwrap().status, TaskStatus::Pending);
        assert_eq!(coordinator.get_task("w-task-2").unwrap().unwrap().status, TaskStatus::Blocked);
        assert_eq!(coordinator.get_task("w-task-fail").unwrap().unwrap().status, TaskStatus::Pending);

        // 2. Run worker to claim and execute ready tasks
        let args = WorkerArgs {
            agent: Some("autonomous-worker-1".to_string()),
            host: Some("antigravity-daemon".to_string()),
            plan: None,
            poll_interval: 1,
            heartbeat_interval: 1,
            lease_ttl: 60,
            stale_timeout: 600,
            auto_reclaim: true,
            max_tasks: Some(1),
            exit_when_idle: true,
            dry_run: false,
            json: true,
        };

        // Worker executes w-task-1
        let stats1 = run_worker(args.clone(), coordinator.clone()).await.unwrap();
        assert_eq!(stats1.tasks_executed, 1);
        assert_eq!(stats1.tasks_succeeded, 1);
        assert_eq!(coordinator.get_task("w-task-1").unwrap().unwrap().status, TaskStatus::Completed);

        // Verify task2 is now unblocked
        assert_eq!(coordinator.get_task("w-task-2").unwrap().unwrap().status, TaskStatus::Pending);

        // Worker executes w-task-2
        let stats2 = run_worker(args.clone(), coordinator.clone()).await.unwrap();
        assert_eq!(stats2.tasks_executed, 1);
        assert_eq!(stats2.tasks_succeeded, 1);
        assert_eq!(coordinator.get_task("w-task-2").unwrap().unwrap().status, TaskStatus::Completed);

        // Worker executes failing task
        let stats3 = run_worker(args.clone(), coordinator.clone()).await.unwrap();
        assert_eq!(stats3.tasks_executed, 1);
        assert_eq!(stats3.tasks_failed, 1);
        assert_eq!(coordinator.get_task("w-task-fail").unwrap().unwrap().status, TaskStatus::Failed);

        // 3. Test orphan reclamation by worker
        let mut orphan_task = TaskRecord::new("w-orphan", "Orphaned background task");
        orphan_task.status = TaskStatus::Running;
        orphan_task.assigned_agent = Some("dead-worker".to_string());
        orphan_task.updated_at = chrono::Utc::now().timestamp() - 1000;
        coordinator.create_task(&orphan_task).unwrap();

        let reclaim_worker_args = WorkerArgs {
            agent: Some("cleaner-worker".to_string()),
            host: Some("antigravity-daemon".to_string()),
            plan: None,
            poll_interval: 1,
            heartbeat_interval: 1,
            lease_ttl: 60,
            stale_timeout: 50,
            auto_reclaim: true,
            max_tasks: Some(1),
            exit_when_idle: true,
            dry_run: true,
            json: true,
        };

        let stats_reclaim = run_worker(reclaim_worker_args, coordinator.clone()).await.unwrap();
        assert!(stats_reclaim.orphans_reclaimed >= 1);
        assert_eq!(stats_reclaim.tasks_succeeded, 1);
        assert_eq!(coordinator.get_task("w-orphan").unwrap().unwrap().status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn test_worker_heartbeat_lease_renewal_and_abort_on_revocation() {
        use crate::commands::worker::{run_worker, WorkerArgs};
        use strata_core::blackboard::{TaskRecord, TaskStatus};
        use strata_memory::SqliteMemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let coordinator = engine.stigmergy();

        #[cfg(target_os = "windows")]
        let sleep_cmd = "cmd: Start-Sleep -Milliseconds 600";
        #[cfg(not(target_os = "windows"))]
        let sleep_cmd = "cmd: sleep 0.6";

        let task1 = TaskRecord::new("w-hb-task-1", "Heartbeat renewal task")
            .with_description(sleep_cmd)
            .with_required_resources(vec!["src/critical_module.rs".to_string()]);
        coordinator.create_task(&task1).unwrap();

        let args = WorkerArgs {
            agent: Some("hb-worker".to_string()),
            host: Some("cli".to_string()),
            plan: None,
            poll_interval: 1,
            heartbeat_interval: 1,
            lease_ttl: 10,
            stale_timeout: 60,
            auto_reclaim: false,
            max_tasks: Some(1),
            exit_when_idle: true,
            dry_run: false,
            json: true,
        };

        let stats = run_worker(args.clone(), coordinator.clone()).await.unwrap();
        assert_eq!(stats.tasks_succeeded, 1);
        assert_eq!(coordinator.get_task("w-hb-task-1").unwrap().unwrap().status, TaskStatus::Completed);
        // Resource lease should be released upon completion
        assert!(coordinator.get_lease("src/critical_module.rs").unwrap().is_none());

        // Test abort on concurrent lease revocation
        #[cfg(target_os = "windows")]
        let long_sleep_cmd = "cmd: Start-Sleep -Milliseconds 2500";
        #[cfg(not(target_os = "windows"))]
        let long_sleep_cmd = "cmd: sleep 2.5";

        let task2 = TaskRecord::new("w-hb-task-2", "Task to revoke")
            .with_description(long_sleep_cmd)
            .with_required_resources(vec!["src/stolen_module.rs".to_string()]);
        coordinator.create_task(&task2).unwrap();

        let coord_clone = coordinator.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
            // Revoke the lease from hb-worker and transfer to rival-agent
            let _ = coord_clone.release_lease("src/stolen_module.rs", "hb-worker");
            let _ = coord_clone.acquire_lease("src/stolen_module.rs", "rival-agent", 60, Some("stolen"));
        });

        let stats_revoked = run_worker(args, coordinator.clone()).await.unwrap();
        assert_eq!(stats_revoked.tasks_failed, 1);
        let task2_record = coordinator.get_task("w-hb-task-2").unwrap().unwrap();
        assert_eq!(task2_record.status, TaskStatus::Failed);
    }

    #[tokio::test]
    async fn test_realtime_drift_watcher_jtms_invalidation_cascade() {
        use crate::commands::watch::{run_watch, WatchArgs};
        use strata_core::schemas::{FactStatus, SemanticFact};
        use strata_core::state::Scope;
        use strata_memory::{AstParser, CodeAnchorEngine, SqliteStore};

        // Create temporary workspace directory with src/auth.rs
        let temp_dir = std::env::temp_dir().join(format!("strata_watch_test_{}", uuid::Uuid::new_v4()));
        let src_dir = temp_dir.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src dir");
        let auth_file = src_dir.join("auth.rs");

        let initial_code = r#"
pub fn verify_token(token: &str) -> bool {
    !token.is_empty() && token.starts_with("Bearer ")
}
"#;
        std::fs::write(&auth_file, initial_code).expect("write auth.rs");

        let store = Arc::new(SqliteStore::open_in_memory().expect("open store"));
        let engine = CodeAnchorEngine::new();
        let parser = AstParser::new();

        // Parse symbol and create code anchor for verify_token
        let rel_path = "src/auth.rs";
        let symbols = parser.parse_file(rel_path, initial_code).expect("parse auth.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].symbol_path, "verify_token");

        let anchor = engine.create_anchor(rel_path, &symbols[0], Some("commit-initial"));
        let initial_merkle_root = CodeAnchorEngine::compute_merkle_tree_hash(&symbols);
        assert!(!initial_merkle_root.is_empty());

        // 1. Grave no SqliteStore um SemanticFact ancorado a verify_token com um CodeAnchor
        let fact1 = SemanticFact::new(
            "Tokens must start with Bearer prefix",
            "auth",
            Scope::Global,
        )
        .with_code_anchor(anchor);
        store.insert_or_update_semantic_fact(&fact1).expect("insert fact1");

        // 2. Grave um segundo SemanticFact dependente (depends_on: [fact_1])
        let mut fact2 = SemanticFact::new(
            "API gateway authorizes requests using verify_token",
            "gateway",
            Scope::Global,
        );
        fact2.depends_on = vec![fact1.id];
        store.insert_or_update_semantic_fact(&fact2).expect("insert fact2");

        // Verifique que ambos estão inicialmente ativos
        let loaded_fact1 = store.get_semantic_fact(&fact1.id).expect("get fact1").expect("found");
        let loaded_fact2 = store.get_semantic_fact(&fact2.id).expect("get fact2").expect("found");
        assert_eq!(loaded_fact1.status, FactStatus::Active);
        assert_eq!(loaded_fact2.status, FactStatus::Active);

        // Altere o arquivo src/auth.rs modificando o corpo ou assinatura da função
        let modified_code = r#"
pub fn verify_token(token: &str) -> bool {
    // Modified signature / body to validate JWT claims format
    token.len() > 32 && token.starts_with("eyJ")
}
"#;
        std::fs::write(&auth_file, modified_code).expect("write modified auth.rs");

        // Execute o ciclo de drift watch (args.once = true)
        let watch_args = WatchArgs {
            workspace: temp_dir.clone(),
            debounce_ms: 100,
            once: true,
            json: false,
        };

        run_watch(watch_args, store.clone()).await.expect("run_watch failed");

        // Verifique:
        // 1. O fato original foi marcado como FactStatus::Stale
        let updated_fact1 = store.get_semantic_fact(&fact1.id).expect("get fact1").expect("found");
        assert_eq!(
            updated_fact1.status,
            FactStatus::Stale,
            "Original fact anchored to modified code must transition to Stale"
        );

        // 2. O fato dependente downstream foi invalidado em cascata para FactStatus::Stale no JTMS
        let updated_fact2 = store.get_semantic_fact(&fact2.id).expect("get fact2").expect("found");
        assert_eq!(
            updated_fact2.status,
            FactStatus::Stale,
            "Downstream dependent fact must transition to Stale via JTMS cascade"
        );

        // Verify JTMS audit trail was recorded
        let audits = store.get_all_jtms_audits(10).expect("get jtms audits");
        assert!(
            audits.iter().any(|a| a.winning_fact_id == fact1.id && a.losing_fact_id == fact2.id),
            "JTMS audit row must record invalidation of fact2 due to prerequisite fact1"
        );

        // 3. O Merkle root foi recalculado
        let modified_symbols = parser.parse_file(rel_path, modified_code).expect("parse modified code");
        let new_merkle_root = CodeAnchorEngine::compute_merkle_tree_hash(&modified_symbols);
        assert_ne!(
            initial_merkle_root, new_merkle_root,
            "Workspace Merkle root must be recomputed and reflect modified AST"
        );

        // Cleanup temp dir
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_drift_watcher_path_filtering_and_noisy_directories() {
        use crate::commands::watch::{is_ignored_path, is_supported_source_file};
        use std::path::Path;

        // Noisy and build directories should be ignored
        assert!(is_ignored_path(Path::new(".git/HEAD")));
        assert!(is_ignored_path(Path::new("target/debug/build.rs")));
        assert!(is_ignored_path(Path::new(".strata/facts.db")));
        assert!(is_ignored_path(Path::new("node_modules/package/index.js")));
        assert!(is_ignored_path(Path::new("dist/bundle.js")));
        assert!(is_ignored_path(Path::new(".cargo/config.toml")));

        // Temporary files should be ignored
        assert!(is_ignored_path(Path::new("src/auth.rs.tmp")));
        assert!(is_ignored_path(Path::new("src/auth.rs~")));
        assert!(is_ignored_path(Path::new("src/.auth.rs.swp")));

        // Supported source files
        assert!(is_supported_source_file(Path::new("src/auth.rs")));
        assert!(is_supported_source_file(Path::new("frontend/App.tsx")));
        assert!(is_supported_source_file(Path::new("frontend/index.ts")));
        assert!(is_supported_source_file(Path::new("backend/server.js")));
        assert!(is_supported_source_file(Path::new("scripts/model.py")));

        // Non-code files should not be watched
        assert!(!is_supported_source_file(Path::new("README.md")));
        assert!(!is_supported_source_file(Path::new("Cargo.toml")));
        assert!(!is_supported_source_file(Path::new("package.json")));
    }

    #[tokio::test]
    async fn test_drift_watcher_json_output_mode() {
        use crate::commands::watch::{run_watch, WatchArgs};
        use strata_memory::SqliteStore;

        let temp_dir = std::env::temp_dir().join(format!("strata_watch_json_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let store = Arc::new(SqliteStore::open_in_memory().expect("open store"));
        let watch_args = WatchArgs {
            workspace: temp_dir.clone(),
            debounce_ms: 100,
            once: true,
            json: true,
        };

        let res = run_watch(watch_args, store).await;
        assert!(res.is_ok());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_realtime_drift_watcher_live_fs_event_debounce() {
        use crate::commands::watch::{run_watch, WatchArgs};
        use strata_core::schemas::{FactStatus, SemanticFact};
        use strata_core::state::Scope;
        use strata_memory::{AstParser, CodeAnchorEngine, SqliteStore};
        use std::time::Duration;

        let temp_dir = std::env::temp_dir().join(format!("strata_watch_live_{}", uuid::Uuid::new_v4()));
        let src_dir = temp_dir.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src dir");
        let calc_file = src_dir.join("calc.rs");

        let initial_code = "pub fn add_numbers(a: i32, b: i32) -> i32 { a + b }\n";
        std::fs::write(&calc_file, initial_code).expect("write calc.rs");

        let store = Arc::new(SqliteStore::open_in_memory().expect("open store"));
        let engine = CodeAnchorEngine::new();
        let parser = AstParser::new();

        let rel_path = "src/calc.rs";
        let symbols = parser.parse_file(rel_path, initial_code).expect("parse calc.rs");
        let anchor = engine.create_anchor(rel_path, &symbols[0], None);

        let fact = SemanticFact::new("Addition logic", "math", Scope::Global).with_code_anchor(anchor);
        store.insert_or_update_semantic_fact(&fact).expect("insert fact");

        // Spawn watcher in background with short debounce
        let watch_dir = temp_dir.clone();
        let watch_store = store.clone();
        let handle = tokio::spawn(async move {
            let watch_args = WatchArgs {
                workspace: watch_dir,
                debounce_ms: 100,
                once: false,
                json: false,
            };
            let _ = run_watch(watch_args, watch_store).await;
        });

        // Allow watcher thread to start listening
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Modify the file on disk
        let modified_code = "pub fn add_numbers(a: i32, b: i32) -> i32 { a.saturating_add(b) }\n";
        std::fs::write(&calc_file, modified_code).expect("write modified calc.rs");

        // Wait for notify event + debounce
        let mut transitioned = false;
        for _ in 0..25 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(Some(f)) = store.get_semantic_fact(&fact.id) {
                if f.status == FactStatus::Stale {
                    transitioned = true;
                    break;
                }
            }
        }

        handle.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);

        assert!(transitioned, "Fact should transition to Stale upon live fs modification event");
    }

    #[tokio::test]
    async fn test_cli_sync_merge_branch() {
        use crate::commands::sync::{run_sync, SyncArgs};
        use strata_core::state::{MemoryRecord, MemoryType, Scope};

        let store = Arc::new(SqliteStore::open_in_memory().expect("open store"));

        let rec = MemoryRecord::new(
            MemoryType::Semantic,
            "Feature-isolated API contract",
            Scope::Global,
        )
        .with_branch("feat/payment-gateway");
        store.insert_or_update_memory(&rec).expect("insert memory");

        // Pre-merge: query on main should find 0 memories
        let main_before = store.search_fts_as_of("contract", None, None, None, Some("main"), 10).expect("search main before");
        assert_eq!(main_before.len(), 0);

        // Pre-merge: query on feat branch should find 1 memory
        let feat_before = store.search_fts_as_of("contract", None, None, None, Some("feat/payment-gateway"), 10).expect("search feat before");
        assert_eq!(feat_before.len(), 1);

        // Run sync command with --merge-branch
        let sync_args = SyncArgs {
            action: None,
            endpoint: None,
            token: None,
            workspace: None,
            json: true,
            merge_branch: Some("feat/payment-gateway".to_string()),
            target_branch: "main".to_string(),
        };

        let res = run_sync(sync_args, store.clone()).await;
        assert!(res.is_ok(), "run_sync with --merge-branch must succeed");

        // Post-merge: query on main should now find 1 memory!
        let main_after = store.search_fts_as_of("contract", None, None, None, Some("main"), 10).expect("search main after");
        assert_eq!(main_after.len(), 1);
        assert_eq!(main_after[0].0.id, rec.id);
    }

    #[tokio::test]
    async fn test_cli_multi_worker_swarm_concurrent_work_stealing_and_arbitration() {
        use crate::commands::worker::{run_worker, WorkerArgs};
        use strata_core::blackboard::{TaskFilter, TaskRecord, TaskStatus};
        use strata_memory::SqliteMemoryEngine;

        let temp_dir = std::env::temp_dir().join(format!("strata_swarm_cli_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let db_path = temp_dir.join("swarm_blackboard.db");

        // 1. Initialize coordinator on disk SQLite with WAL
        let engine = Arc::new(SqliteMemoryEngine::open(&db_path, None).expect("open db"));
        let coordinator = engine.stigmergy();

        // 2. Populate blackboard with DAG and contested resources
        // - task-1: "Implement auth token refresh" (locks file:src/auth.rs)
        // - task-2: "Add rate limiter middleware" (also needs file:src/auth.rs, must wait for task-1 lease)
        // - task-3: "Create database schema" (locks file:migrations/001.sql)
        // - task-4: "Seed test data" (blocked on task-3)
        // - task-5: "Integration test auth and db" (blocked on task-1 and task-4)
        // - task-6: "Write release documentation" (blocked on task-5)
        let t1 = TaskRecord::new("swarm-task-1", "Implement auth token refresh")
            .with_plan_id("plan-swarm")
            .with_required_resources(vec!["file:src/auth.rs".to_string()]);
        let t2 = TaskRecord::new("swarm-task-2", "Add rate limiter middleware")
            .with_plan_id("plan-swarm")
            .with_required_resources(vec!["file:src/auth.rs".to_string()]);
        let t3 = TaskRecord::new("swarm-task-3", "Create database schema")
            .with_plan_id("plan-swarm")
            .with_required_resources(vec!["file:migrations/001.sql".to_string()]);
        let t4 = TaskRecord::new("swarm-task-4", "Seed test data")
            .with_plan_id("plan-swarm")
            .with_dependencies(vec!["swarm-task-3".to_string()])
            .with_status(TaskStatus::Blocked);
        let t5 = TaskRecord::new("swarm-task-5", "Integration test auth and db")
            .with_plan_id("plan-swarm")
            .with_dependencies(vec!["swarm-task-1".to_string(), "swarm-task-4".to_string()])
            .with_status(TaskStatus::Blocked);
        let t6 = TaskRecord::new("swarm-task-6", "Write release documentation")
            .with_plan_id("plan-swarm")
            .with_dependencies(vec!["swarm-task-5".to_string()])
            .with_status(TaskStatus::Blocked);

        coordinator.create_task(&t1).unwrap();
        coordinator.create_task(&t2).unwrap();
        coordinator.create_task(&t3).unwrap();
        coordinator.create_task(&t4).unwrap();
        coordinator.create_task(&t5).unwrap();
        coordinator.create_task(&t6).unwrap();

        // 3. Spawn 3 concurrent workers in separate async tasks
        let mut worker_tasks = Vec::new();
        let agent_names = vec!["worker-alpha", "worker-beta", "worker-gamma"];

        for agent in &agent_names {
            let coord = coordinator.clone();
            let agent_id = agent.to_string();
            let worker_args = WorkerArgs {
                agent: Some(agent_id),
                host: Some("antigravity".to_string()),
                plan: Some("plan-swarm".to_string()),
                poll_interval: 1,
                heartbeat_interval: 1,
                lease_ttl: 15,
                stale_timeout: 30,
                auto_reclaim: true,
                max_tasks: None,
                exit_when_idle: true,
                dry_run: true,
                json: true,
            };

            worker_tasks.push(tokio::spawn(async move {
                run_worker(worker_args, coord).await
            }));
        }

        // 4. Await all workers
        let mut total_succeeded = 0;
        let mut worker_contributions = std::collections::HashSet::new();

        for handle in worker_tasks {
            let res = handle.await.expect("worker task join").expect("worker run");
            total_succeeded += res.tasks_succeeded;
            if res.tasks_succeeded > 0 {
                worker_contributions.insert(res.agent_id);
            }
        }

        // 5. Assertions
        assert_eq!(total_succeeded, 6, "All 6 tasks must be executed successfully");
        assert!(worker_contributions.len() >= 2, "Tasks must be distributed across multiple workers");

        let filter = TaskFilter {
            plan_id: Some("plan-swarm".to_string()),
            ..Default::default()
        };
        let final_tasks = coordinator.list_tasks(&filter).unwrap();
        assert_eq!(final_tasks.len(), 6);
        for task in &final_tasks {
            assert_eq!(task.status, TaskStatus::Completed, "Task {} should be Completed", task.task_id);
            assert!(task.claim_generation >= 1, "Task {} should have valid fencing generation", task.task_id);
            assert!(task.assigned_agent.is_some(), "Task {} should record executing agent", task.task_id);

            let artifacts = coordinator.get_task_artifacts(&task.task_id).unwrap();
            assert_eq!(artifacts.len(), 1, "Task {} must have an artifact", task.task_id);
        }

        // All resource leases released cleanly
        let active = coordinator.active_leases().unwrap();
        assert!(active.is_empty(), "All resource leases must be released at the end");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cli_memory_command_status_and_reindex() {
        use crate::commands::memory::{run_memory, MemoryAction, MemoryArgs};
        use strata_core::state::{MemoryRecord, MemoryTier, MemoryType, Scope};
        use strata_memory::SqliteStore;

        let temp_dir = std::env::temp_dir().join(format!("strata_cli_mem_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let db_path = temp_dir.join("strata.db");

        let store = Arc::new(SqliteStore::open(&db_path).unwrap());

        // 1. Insert a memory with mock-hash model
        let mut mem1 = MemoryRecord::new(
            MemoryType::Semantic,
            "JWT Authentication Bearer",
            Scope::Global,
        )
        .with_summary("JWT")
        .with_tier(MemoryTier::Working);
        mem1.embedding = Some(vec![0.1f32; 384]);
        mem1.embedding_model = Some("mock-hash".to_string());
        mem1.embedding_version = Some("1".to_string());
        store.insert_or_update_memory(&mem1).unwrap();

        // 2. Insert another memory with an incompatible model
        let mut mem2 = MemoryRecord::new(
            MemoryType::Semantic,
            "Postgres connection pool configuration",
            Scope::Global,
        )
        .with_summary("Postgres")
        .with_tier(MemoryTier::Working);
        mem2.embedding = Some(vec![0.2f32; 384]);
        mem2.embedding_model = Some("legacy-other-model".to_string());
        mem2.embedding_version = Some("1".to_string());
        store.insert_or_update_memory(&mem2).unwrap();

        // 3. Run status command
        let status_args = MemoryArgs {
            action: MemoryAction::Status { json: true },
        };
        run_memory(status_args, Arc::clone(&store)).await.expect("status action");

        // 4. Run reindex with dry_run = true
        let dry_run_args = MemoryArgs {
            action: MemoryAction::Reindex {
                batch_size: 10,
                force: false,
                dry_run: true,
                json: true,
            },
        };
        run_memory(dry_run_args, Arc::clone(&store)).await.expect("dry run action");

        // 5. Run reindex with dry_run = false
        let reindex_args = MemoryArgs {
            action: MemoryAction::Reindex {
                batch_size: 10,
                force: false,
                dry_run: false,
                json: true,
            },
        };
        run_memory(reindex_args, Arc::clone(&store)).await.expect("reindex action");

        // 6. Verify consistency stats against active model
        let config = strata_core::config::StrataConfig::load();
        let resolved = strata_memory::EmbeddingProviderFactory::create_from_config(&config).unwrap();
        let active_model = resolved.provider.model_name();
        let stats = store.get_vector_consistency_stats(Some(active_model)).unwrap();
        assert!(stats.is_consistent);
        assert_eq!(stats.distinct_models_count, 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}


