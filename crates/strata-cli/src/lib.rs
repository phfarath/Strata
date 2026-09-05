pub mod commands;
pub mod config;
pub mod mcp;
pub mod tui;

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
        let res_file = run_callgraph(file_a.to_str().unwrap(), None, "all", true, 10).await;
        assert!(res_file.is_ok());

        // Test running callgraph on the whole directory
        let res_dir = run_callgraph(
            temp_dir.to_str().unwrap(),
            Some("process_data"),
            "callers",
            true,
            10,
        )
        .await;
        assert!(res_dir.is_ok());

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
        let server = McpServer::new_with_engine(Arc::clone(&engine));

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

        // 3. Lease acquire tool
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

        // 6. Release tool
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
    }
}
