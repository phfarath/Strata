pub mod commands;
pub mod config;
pub mod mcp;

pub use commands::*;
pub use config::*;
pub use mcp::*;

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
        use strata_memory::SqliteMemoryEngine;
        use strata_core::state::Scope;
        use strata_core::traits::MemoryEngine;

        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        // 1. Test PostTool with compilation error
        let post_tool = HookCommand::PostTool {
            tool: "cargo_test".to_string(),
            error: Some("error: package ID specification 'strata-xyz' did not match any packages".to_string()),
            params: Some("--package strata-xyz".to_string()),
            context: Some("build_step".to_string()),
        };
        handle_hook(post_tool, Arc::clone(&engine)).await.expect("handle post-tool");

        // 2. Test UserPrompt retrieves the preemptive guardrail
        let user_prompt = HookCommand::UserPrompt {
            query: "cargo test --package strata-xyz".to_string(),
            limit: 3,
            scope: Some(Scope::Global.to_string()),
            json: true,
        };
        handle_hook(user_prompt, Arc::clone(&engine)).await.expect("handle user-prompt");

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
        let res_dir = run_callgraph(temp_dir.to_str().unwrap(), Some("process_data"), "callers", true, 10).await;
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
}


