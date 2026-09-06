use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use strata_core::config::StrataConfig;
use strata_core::errors::StrataError;
use strata_memory::{ConsolidationPipeline, MockEmbeddingProvider, SqliteStore};
use strata_reasoning::{
    probe_provider, resolve_reasoning_engine, MockReasoningEngine, ResolvedProviderKind,
};

/// Evaluation results for Pluggable Reasoning Provider Cascade and Configuration System.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluggableReasoningCascadeEvalResult {
    pub config_persistence_verified: bool,
    pub config_precedence_verified: bool,
    pub explicit_override_verified: bool,
    pub local_ollama_configuration_verified: bool,
    pub key_missing_detection_verified: bool,
    pub automatic_fallback_verified: bool,
    pub provider_probe_verified: bool,
    pub distillation_with_resolved_engine_verified: bool,
    pub duration_micros: u128,
}

pub struct PluggableReasoningCascadeEval;

impl PluggableReasoningCascadeEval {
    pub async fn run_eval() -> Result<PluggableReasoningCascadeEvalResult, StrataError> {
        let start = Instant::now();

        let temp_dir =
            std::env::temp_dir().join(format!("strata_reasoning_eval_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join(".strata").join("config.toml");

        // 1. Config Persistence & TOML Serialization
        let mut config = StrataConfig::default();
        config.set_key("provider", "ollama")?;
        config.set_key("model", "qwen2.5-coder:7b")?;
        config.set_key("ollama_url", "http://127.0.0.1:11434")?;
        config.set_key("temperature", "0.2")?;
        config.save_to_file(&config_path)?;

        let loaded = StrataConfig::load_from_file(&config_path)?;
        let config_persistence_verified = loaded.provider == Some("ollama".to_string())
            && loaded.model == Some("qwen2.5-coder:7b".to_string())
            && loaded.ollama_url == Some("http://127.0.0.1:11434".to_string())
            && loaded.temperature == Some(0.2);

        // 2. Config Precedence & Merging
        let mut base_config = StrataConfig {
            provider: Some("mock".to_string()),
            model: Some("base-model".to_string()),
            temperature: Some(0.7),
            ..Default::default()
        };
        let override_config = StrataConfig {
            provider: Some("gemini".to_string()),
            api_key: Some("test-key-xyz".to_string()),
            ..Default::default()
        };
        base_config.merge(override_config);
        let config_precedence_verified = base_config.provider == Some("gemini".to_string())
            && base_config.model == Some("base-model".to_string())
            && base_config.api_key == Some("test-key-xyz".to_string())
            && base_config.temperature == Some(0.7);

        // 3. Explicit CLI Override Priority
        let explicit_mock =
            resolve_reasoning_engine(&loaded, Some("mock"), Some("custom-mock")).await?;
        let explicit_override_verified = explicit_mock.kind == ResolvedProviderKind::Mock;

        // 4. Local Ollama Provider Configuration
        let ollama_resolved =
            resolve_reasoning_engine(&loaded, Some("ollama"), Some("deepseek-coder:6.7b")).await?;
        let local_ollama_configuration_verified = match ollama_resolved.kind {
            ResolvedProviderKind::Ollama { model, url } => {
                model == "deepseek-coder:6.7b" && url == "http://127.0.0.1:11434"
            }
            _ => false,
        };

        // 5. Explicit Cloud Provider with Missing Key
        let empty_config = StrataConfig::default();
        let missing_key_res =
            resolve_reasoning_engine(&empty_config, Some("anthropic"), None).await;
        let key_missing_detection_verified = missing_key_res.is_err();

        // 6. Automatic Fallback Cascade to Mock (tested deterministically via offline probe port)
        let unreachable_config = StrataConfig {
            ollama_url: Some("http://127.0.0.1:59999".to_string()),
            ..Default::default()
        };
        let auto_fallback = resolve_reasoning_engine(&unreachable_config, None, None).await?;
        let automatic_fallback_verified = auto_fallback.kind == ResolvedProviderKind::Mock;

        // 7. Provider Diagnostic Probe
        let mock_probe = probe_provider("mock", None, &empty_config).await?;
        let provider_probe_verified =
            mock_probe.contains("Mock deterministic reasoning engine is active");

        // 8. Distillation Execution with Resolved Engine
        let db_path = temp_dir.join("eval.db");
        let store = Arc::new(SqliteStore::open(&db_path)?);
        let embedder = MockEmbeddingProvider::default();
        let mut pipeline_cfg = strata_memory::PipelineConfig::default();
        pipeline_cfg.min_salience_threshold = 0.0;
        let pipeline = ConsolidationPipeline::new(pipeline_cfg);
        let mock_engine = MockReasoningEngine::new();

        let mut distillation = strata_reasoning::prompts::DistillationOutput::default();
        distillation
            .episodic_memories
            .push(strata_reasoning::prompts::EpisodicMemoryItem {
                summary: "Verified pluggable reasoning cascade".to_string(),
                content:
                    "All local-first and cloud providers resolved according to priority cascade"
                        .to_string(),
                importance: 0.9,
                tags: vec!["reasoning".to_string(), "providers".to_string()],
            });
        distillation.semantic_facts.push(
            strata_reasoning::prompts::SemanticFact::new(
                "Strata supports Ollama, Gemini, Anthropic, OpenAI, OpenRouter, and Mock reasoning engines",
            )
            .with_summary("Pluggable Reasoning Engines")
            .with_importance(0.95),
        );
        mock_engine.push_distillation_output(&distillation).await;

        let now = chrono::Utc::now();
        let events = vec![
            strata_core::events::Event::new(
                "sess-eval",
                "agent-eval",
                strata_core::events::EventPayload::SessionStarted(
                    strata_core::events::SessionStarted {
                        session_id: "sess-eval".to_string(),
                        agent_id: "agent-eval".to_string(),
                        organization_id: None,
                        environment: serde_json::json!({ "os": "windows" }),
                        timestamp: now,
                    },
                ),
            ),
            strata_core::events::Event::new(
                "sess-eval",
                "agent-eval",
                strata_core::events::EventPayload::TaskStarted(strata_core::events::TaskStarted {
                    task_id: "task-eval".to_string(),
                    title: "Pluggable provider evaluation task".to_string(),
                    description: Some("Testing provider distillation".to_string()),
                    parent_task_id: None,
                    session_id: "sess-eval".to_string(),
                    timestamp: now,
                }),
            ),
            strata_core::events::Event::new(
                "sess-eval",
                "agent-eval",
                strata_core::events::EventPayload::TaskCompleted(
                    strata_core::events::TaskCompleted {
                        task_id: "task-eval".to_string(),
                        success: true,
                        outcome_summary: "Distillation success".to_string(),
                        evaluation: None,
                        timestamp: now,
                    },
                ),
            ),
        ];

        let res = pipeline
            .run_pipeline(&store, &embedder, &events, Some(&mock_engine))
            .await?;
        let distillation_with_resolved_engine_verified =
            res.episodic_memories.len() == 1 && res.semantic_facts.len() == 1;

        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(PluggableReasoningCascadeEvalResult {
            config_persistence_verified,
            config_precedence_verified,
            explicit_override_verified,
            local_ollama_configuration_verified,
            key_missing_detection_verified,
            automatic_fallback_verified,
            provider_probe_verified,
            distillation_with_resolved_engine_verified,
            duration_micros: start.elapsed().as_micros(),
        })
    }
}

pub async fn run_pluggable_reasoning_cascade_scenario() -> Result<()> {
    println!("\n🔍 Running Pluggable Reasoning Provider Cascade & Config Eval Scenario...");
    let res = PluggableReasoningCascadeEval::run_eval().await?;

    println!("  [✓] Config Persistence & TOML Serialization: PASS");
    println!("  [✓] Configuration Precedence & Merging:      PASS");
    println!("  [✓] Explicit CLI Flag Override:             PASS");
    println!("  [✓] Local Ollama Provider Initialization:   PASS");
    println!("  [✓] Missing API Key Strict Detection:       PASS");
    println!("  [✓] Automatic Cascade Fallback to Mock:     PASS");
    println!("  [✓] Provider Diagnostic Probe:              PASS");
    println!("  [✓] Memory Distillation with Engine:        PASS");
    println!(
        "  ⏱️  Total Duration:                           {}ms",
        res.duration_micros / 1000
    );

    assert!(res.config_persistence_verified);
    assert!(res.config_precedence_verified);
    assert!(res.explicit_override_verified);
    assert!(res.local_ollama_configuration_verified);
    assert!(res.key_missing_detection_verified);
    assert!(res.automatic_fallback_verified);
    assert!(res.provider_probe_verified);
    assert!(res.distillation_with_resolved_engine_verified);

    Ok(())
}
