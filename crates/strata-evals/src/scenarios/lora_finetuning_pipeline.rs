use anyhow::{bail, Result};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;

use strata_cli::mcp::server::McpServer;
use strata_core::{
    schemas::{EpisodicMemory, ProceduralSkill, ProceduralStep, SignalScores},
    state::{FailurePattern, MemoryRecord, MemoryType, Scope},
    traits::MemoryEngine,
};
use strata_memory::{ExportFormat, PreferenceMiner, SqliteMemoryEngine};
use strata_reasoning::{
    generate_ollama_modelfile, generate_run_script, generate_unsloth_training_script,
    QuantizationType, TrainingConfig, TrainingManifest, TrainingMethod, TrainingPipeline,
};
use strata_tools::TrainPipelineTool;

/// Evaluation Scenario 12: Pipeline One-Click de Fine-Tuning LoRA Local via Unsloth/Ollama
pub async fn run_lora_finetuning_pipeline_scenario() -> Result<()> {
    println!("\n========================================================");
    println!("▶ Running Eval Scenario 12: One-Click LoRA Fine-Tuning Pipeline (Unsloth/Ollama)");
    println!("========================================================");

    // -------------------------------------------------------------------------
    // Step 1: Initialize in-memory cognitive engine and populate continuous traces
    // -------------------------------------------------------------------------
    println!(
        "  [Step 1] Populating continuous memory traces, failure patterns, and episodic signals..."
    );
    let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None)?);
    let store = engine.store_arc();

    // 1. Record Failure Pattern (Anti-pattern -> rejected vs Mitigation -> chosen)
    let mut failure = FailurePattern::new(
        "use of moved value in closure",
        "Async Closure Ownership Error",
        "Moving ownership of shared state into async spawn causes E0382 borrow checker error",
        "Clone Arc handle before moving into async block: let handle = state.clone();",
    );
    failure.trigger_condition =
        "Capturing non-copy struct into async task without Arc clone".to_string();
    failure.error_type = "BorrowCheckerE0382".to_string();
    store.upsert_failure_pattern(&failure)?;

    // 2. Record Episodic Memory with high success
    let now = Utc::now();
    let episode = EpisodicMemory::new(
        "eval-lora-session",
        "agent-1",
        "Implemented SQLite FTS5 index for hybrid retrieval",
        now,
        now,
    )
    .with_goals(vec!["Add fast lexical search".to_string()])
    .with_obstacles(vec!["Tokenizer mismatch on hyphenated tokens".to_string()])
    .with_outcomes(vec!["Search latency dropped from 120ms to 4ms".to_string()])
    .with_signals(SignalScores {
        success: 0.95,
        frustration: 0.05,
        novelty: 0.5,
        importance: 0.9,
    });
    store.insert_episodic_memory(&episode)?;

    // 3. Record Procedural Skill
    let skill = ProceduralSkill::new(
        "safe_git_branch_workflow",
        "Standard protocol for safe git branching and pull request creation",
    )
    .with_preconditions(vec!["Clean git working tree".to_string()])
    .with_steps(vec![
        ProceduralStep::new(1, "safe_shell", "git checkout -b feature/topic", json!({}))
            .with_expected_result("Switched to new branch"),
        ProceduralStep::new(2, "safe_shell", "cargo test --workspace", json!({}))
            .with_expected_result("All tests pass"),
    ])
    .with_project("strata");
    store.insert_or_update_procedural_skill(&skill)?;

    // 4. Record Semantic Fact
    let mem = MemoryRecord::new(
        MemoryType::Semantic,
        "Strata uses Unsloth FastLanguageModel for 4-bit LoRA training and exports GGUF directly for local Ollama serving.",
        Scope::Global,
    )
    .with_summary("Strata LoRA Architecture")
    .with_importance(0.95);
    engine.write(&mem).await?;

    println!("    ✓ Cognitive traces, failure patterns, and procedural skills recorded.");

    // -------------------------------------------------------------------------
    // Step 2: Test Dataset Mining (DPO and SFT)
    // -------------------------------------------------------------------------
    println!("  [Step 2] Mining DPO preference pairs and SFT instruction samples...");
    let miner = PreferenceMiner::new(store.clone());

    let dpo_pairs = miner.mine_dpo_pairs(None)?;
    if dpo_pairs.is_empty() {
        bail!("Expected at least 1 mined DPO pair, got 0");
    }
    println!(
        "    ✓ Mined {} DPO preference pairs from failure patterns and episodic outcomes",
        dpo_pairs.len()
    );
    let pair0 = &dpo_pairs[0];
    assert!(!pair0.prompt.is_empty());
    assert!(!pair0.chosen.is_empty());
    assert!(!pair0.rejected.is_empty());

    let sft_samples = miner.mine_sft_samples()?;
    if sft_samples.is_empty() {
        bail!("Expected at least 1 mined SFT sample from procedural skills, got 0");
    }
    println!(
        "    ✓ Mined {} SFT instruction demonstrations from procedural skills",
        sft_samples.len()
    );

    let dpo_jsonl = miner.export(ExportFormat::Dpo, None)?;
    assert!(dpo_jsonl.contains("chosen"));
    assert!(dpo_jsonl.contains("rejected"));

    // -------------------------------------------------------------------------
    // Step 3: Test TrainingConfig Validation & Invariants
    // -------------------------------------------------------------------------
    println!("  [Step 3] Verifying TrainingConfig hyperparameter validation rules...");
    let mut valid_config = TrainingConfig::new("unsloth/Llama-3.2-3B-Instruct")
        .with_method(TrainingMethod::Dpo)
        .with_quantization(QuantizationType::Bits4)
        .with_lora(16, 32, 0.0)
        .with_learning_rate(5e-5)
        .with_batch_size(2, 4)
        .with_max_steps(60)
        .with_max_seq_length(2048)
        .with_ollama_model("strata-custom-coder");

    assert!(valid_config.validate().is_ok());

    // Test invalid rank
    let mut invalid_config = valid_config.clone();
    invalid_config.lora_r = 0;
    if invalid_config.validate().is_ok() {
        bail!("Validation should fail for lora_r = 0");
    }

    // Test invalid learning rate
    invalid_config = valid_config.clone();
    invalid_config.learning_rate = 0.0;
    if invalid_config.validate().is_ok() {
        bail!("Validation should fail for learning_rate = 0.0");
    }
    println!("    ✓ TrainingConfig parameter invariants validated successfully.");

    // -------------------------------------------------------------------------
    // Step 4: Test Unsloth Python Training Script Synthesis
    // -------------------------------------------------------------------------
    println!("  [Step 4] Synthesizing Unsloth Python fine-tuning script...");
    let python_script = generate_unsloth_training_script(&valid_config, "outputs/dataset.jsonl");

    assert!(python_script.contains("from unsloth import FastLanguageModel"));
    assert!(python_script.contains("unsloth/Llama-3.2-3B-Instruct"));
    assert!(python_script.contains("load_in_4bit = True"));
    assert!(python_script.contains("FastLanguageModel.get_peft_model"));
    assert!(python_script.contains("r = 16"));
    assert!(python_script.contains("lora_alpha = 32"));
    assert!(python_script.contains("q_proj"));
    assert!(python_script.contains("DPOTrainer"));
    assert!(python_script.contains("DPOConfig"));
    assert!(python_script.contains("model.save_pretrained"));
    assert!(python_script.contains("save_pretrained_gguf"));
    println!("    ✓ Unsloth Python script generated with PEFT LoRA, DPO trainer, and GGUF export.");

    // -------------------------------------------------------------------------
    // Step 5: Test Ollama Modelfile and Run Script Generation
    // -------------------------------------------------------------------------
    println!("  [Step 5] Synthesizing Ollama Modelfile and runner script...");
    let modelfile = generate_ollama_modelfile(&valid_config, "outputs/lora_adapter");
    assert!(modelfile.contains("FROM unsloth/Llama-3.2-3B-Instruct"));
    assert!(modelfile.contains("ADAPTER outputs/lora_adapter"));
    assert!(modelfile.contains("PARAMETER temperature 0.2"));
    assert!(modelfile.contains("PARAMETER top_p 0.95"));
    assert!(modelfile.contains("PARAMETER stop \"<|im_end|>\""));
    assert!(modelfile.contains("SYSTEM"));

    let run_script = generate_run_script(&valid_config, "outputs/train_lora.py", "outputs");
    assert!(run_script.contains("python"));
    assert!(run_script.contains("outputs/train_lora.py"));
    assert!(run_script.contains("ollama create \"strata-custom-coder\""));
    println!("    ✓ Ollama Modelfile and run_training.sh verified.");

    // -------------------------------------------------------------------------
    // Step 6: End-to-End Artifact Synthesis via TrainingPipeline
    // -------------------------------------------------------------------------
    println!("  [Step 6] Executing TrainingPipeline end-to-end artifact generation...");
    let temp_artifacts_dir = std::env::temp_dir().join("strata_eval_scenario12_artifacts");
    valid_config.output_dir = temp_artifacts_dir.to_string_lossy().to_string();

    let pipeline = TrainingPipeline::new(valid_config);
    let result =
        pipeline.generate_artifacts(&temp_artifacts_dir, Some(&dpo_jsonl), dpo_pairs.len())?;

    if !result.success {
        bail!("TrainingPipeline reported failure during artifact generation");
    }

    assert!(std::path::Path::new(&result.script_path).exists());
    assert!(std::path::Path::new(&result.dataset_path).exists());
    assert!(result
        .modelfile_path
        .as_ref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false));
    assert!(result
        .run_script_path
        .as_ref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false));

    // Verify manifest
    let manifest_file = temp_artifacts_dir.join("manifest.json");
    assert!(manifest_file.exists());
    let manifest_raw = std::fs::read_to_string(&manifest_file)?;
    let parsed_manifest: TrainingManifest = serde_json::from_str(&manifest_raw)?;
    assert_eq!(parsed_manifest.base_model, "unsloth/Llama-3.2-3B-Instruct");
    assert_eq!(parsed_manifest.method, TrainingMethod::Dpo);
    assert_eq!(parsed_manifest.total_samples, dpo_pairs.len());
    assert_eq!(parsed_manifest.status, "ready");
    println!(
        "    ✓ Manifest generated and validated: {}",
        parsed_manifest.id
    );

    // Summary table formatting
    let table = pipeline.format_summary_table(dpo_pairs.len());
    assert!(table.contains("STRATA LORA FINE-TUNING PIPELINE"));
    assert!(table.contains("DPO"));
    assert!(table.contains("4bit"));
    println!("    ✓ Formatted hyperparameter summary table verified.");

    // Clean up temp dir
    let _ = std::fs::remove_dir_all(&temp_artifacts_dir);

    // -------------------------------------------------------------------------
    // Step 7: Test TrainPipelineTool via Tool Gateway
    // -------------------------------------------------------------------------
    println!("  [Step 7] Testing TrainPipelineTool execution...");
    let tool_temp_dir = std::env::temp_dir().join("strata_eval_scenario12_tool_run");
    let tool = TrainPipelineTool::with_store(store.clone());

    use strata_core::traits::Tool;
    let tool_res = tool
        .execute(json!({
            "base_model": "unsloth/Qwen2.5-Coder-7B-Instruct",
            "method": "sft",
            "quantization": "4bit",
            "lora_r": 32,
            "lora_alpha": 64,
            "learning_rate": 2e-5,
            "output_dir": tool_temp_dir.to_string_lossy(),
            "ollama_model_name": "strata-qwen-coder",
            "dry_run": true
        }))
        .await;

    if tool_res.is_err() {
        bail!("TrainPipelineTool execution failed: {:?}", tool_res.err());
    }
    let res_val = tool_res.unwrap();
    assert_eq!(res_val["status"], "success");
    assert!(res_val["total_samples"].as_u64().unwrap() > 0);
    assert!(res_val["script_path"]
        .as_str()
        .unwrap()
        .contains("train_lora.py"));
    println!("    ✓ TrainPipelineTool executed cleanly and generated SFT artifacts.");

    let _ = std::fs::remove_dir_all(&tool_temp_dir);

    // -------------------------------------------------------------------------
    // Step 8: Test MCP Server tool execution of 'train_pipeline'
    // -------------------------------------------------------------------------
    println!("  [Step 8] Testing MCP Server invocation of 'train_pipeline'...");
    let mcp_server = McpServer::new(engine.clone());
    let mcp_temp_dir = std::env::temp_dir().join("strata_eval_scenario12_mcp_run");

    let mcp_res = mcp_server
        .execute_tool(
            "train_pipeline",
            json!({
                "base_model": "unsloth/Llama-3.2-3B-Instruct",
                "method": "dpo",
                "output_dir": mcp_temp_dir.to_string_lossy(),
                "ollama_model_name": "strata-mcp-coder",
                "dry_run": true
            }),
        )
        .await;

    if mcp_res.is_error == Some(true) {
        bail!(
            "MCP train_pipeline tool execution failed: {:?}",
            mcp_res.content
        );
    }
    let structured = mcp_res
        .structured_content
        .expect("Expected structured_content from train_pipeline");
    assert_eq!(
        structured.get("status").and_then(|v| v.as_str()),
        Some("success")
    );
    println!("    ✓ MCP Server handled 'train_pipeline' tool invocation seamlessly.");

    let _ = std::fs::remove_dir_all(&mcp_temp_dir);

    println!("  ✓ LoRA Fine-Tuning Pipeline evaluation scenario PASSED (8/8 steps).\n");
    Ok(())
}
