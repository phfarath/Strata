pub mod adapters;
pub mod causal;
pub mod engine;
pub mod mock;
pub mod planning;
pub mod prompts;
pub mod training;

pub use adapters::*;
pub use causal::{
    BlastRadiusReport, CausalEdge, CausalEdgeKind, CausalGraph, CausalNode, CausalNodeKind,
    CodebaseCausalIndexer, ImpactedNode, PatchSimulationResult, WorldModel,
};
pub use engine::*;
pub use mock::*;
pub use planning::{
    DagExecutionReport, DagScheduler, DynamicReplanner, ExecutionWave, GoalDag, GoalDagExport,
    GoalDecomposer, GoalEdge, GoalEdgeKind, GoalNode, GoalNodeKind, GoalStatus, RecoveryAction,
    SerializedGoalEdge, TaskExecutor,
};
pub use prompts::*;
pub use training::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_mock_reasoning_engine_text_response() {
        let mock = MockReasoningEngine::new();
        mock.push_text("Hello from mock reasoning engine!").await;

        let ctx = PromptContext::new()
            .with_system("You are a helpful assistant")
            .with_message(ChatMessage::user("Hi there"));

        let output = mock.complete(&ctx).await.unwrap();
        assert_eq!(
            output.content.as_deref(),
            Some("Hello from mock reasoning engine!")
        );
        assert!(!output.has_tool_calls());

        mock.assert_called_times(1).await;
        mock.assert_last_message_contains("Hi there").await;
    }

    #[tokio::test]
    async fn test_mock_reasoning_engine_tool_call() {
        let mock = MockReasoningEngine::new();
        mock.push_tool_call(
            "memory_search",
            json!({ "query": "Rust compiler error E0382" }),
        )
        .await;

        let tool_def = ToolDefinition::new(
            "memory_search",
            "Search persistent memory",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }),
        );

        let ctx = PromptContext::new()
            .with_tools(vec![tool_def])
            .with_message(ChatMessage::user("Search for borrow checker error"));

        let output = mock.complete(&ctx).await.unwrap();
        assert!(output.has_tool_calls());
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].name, "memory_search");
        assert_eq!(
            output.tool_calls[0].arguments["query"],
            "Rust compiler error E0382"
        );

        mock.assert_has_tool_registered("memory_search").await;
    }

    #[tokio::test]
    async fn test_mock_reasoning_engine_error() {
        let mock = MockReasoningEngine::new();
        mock.set_error("Rate limit from upstream LLM provider")
            .await;

        let ctx = PromptContext::new().with_message(ChatMessage::user("Test"));
        let err = mock.complete(&ctx).await.unwrap_err();

        match err {
            strata_core::errors::StrataError::Reasoning(msg)
            | strata_core::errors::StrataError::ReasoningError(msg) => {
                assert!(msg.contains("Rate limit from upstream"));
            }
            _ => panic!("Expected Reasoning error, got {:?}", err),
        }
    }

    #[test]
    fn test_chat_message_constructors() {
        let sys = ChatMessage::system("System prompt");
        assert_eq!(sys.role, Role::System);
        assert_eq!(sys.content, "System prompt");

        let user = ChatMessage::user("User prompt");
        assert_eq!(user.role, Role::User);

        let tool_call = ToolCall::new("call_1", "get_weather", json!({"location": "SF"}));
        let assistant = ChatMessage::assistant_with_tools("Calling weather", vec![tool_call]);
        assert_eq!(assistant.role, Role::Assistant);
        assert_eq!(assistant.tool_calls.as_ref().unwrap().len(), 1);

        let tool_res = ToolResult::new("call_1", "get_weather", json!({"temp": 72}), false);
        let tool_msg = ChatMessage::tool_response(vec![tool_res]);
        assert_eq!(tool_msg.role, Role::Tool);
    }

    #[test]
    fn test_build_distillation_prompt() {
        use strata_core::events::{Event, EventPayload, SessionStarted};
        let event = Event::new(
            "sess-1",
            "agent-1",
            EventPayload::SessionStarted(SessionStarted {
                session_id: "sess-1".to_string(),
                agent_id: "agent-1".to_string(),
                organization_id: None,
                environment: json!({"host": "cursor"}),
                timestamp: chrono::Utc::now(),
            }),
        );

        let prompt = build_distillation_prompt(&[event]);
        assert!(prompt.contains("episodic_memories"));
        assert!(prompt.contains("semantic_facts"));
        assert!(prompt.contains("procedural_skills"));
        assert!(prompt.contains("negative_patterns"));
        assert!(prompt.contains("SessionStarted"));
    }

    #[test]
    fn test_build_jtms_arbitration_prompt() {
        let old_fact = SemanticFact::new("System uses REST JSON API")
            .with_summary("API Architecture")
            .with_importance(0.9);
        let new_fact =
            SemanticFact::new("System migrated to gRPC Protobuf").with_summary("gRPC Migration");

        let prompt = build_jtms_arbitration_prompt(&old_fact, &new_fact);
        assert!(prompt.contains("REST JSON API"));
        assert!(prompt.contains("gRPC Protobuf"));
        assert!(prompt.contains("update"));
        assert!(prompt.contains("duplicate"));
        assert!(prompt.contains("refinement"));
        assert!(prompt.contains("outlier"));
    }

    #[tokio::test]
    async fn test_mock_distillation_output() {
        let mock = MockReasoningEngine::new();
        let mut distillation = DistillationOutput::default();
        distillation
            .semantic_facts
            .push(SemanticFact::new("Database is SQLite WAL mode").with_importance(0.95));
        distillation.episodic_memories.push(EpisodicMemoryItem {
            summary: "Completed migration".to_string(),
            content: "Successfully migrated to SQLite storage".to_string(),
            importance: 0.8,
            tags: vec!["migration".to_string()],
        });

        mock.push_distillation_output(&distillation).await;

        let ctx = PromptContext::new().with_message(ChatMessage::user("Distill events"));
        let output = mock.complete(&ctx).await.unwrap();
        let parsed = parse_distillation_output(output.content.as_deref().unwrap()).unwrap();

        assert_eq!(parsed.semantic_facts.len(), 1);
        assert_eq!(
            parsed.semantic_facts[0].statement,
            "Database is SQLite WAL mode"
        );
        assert_eq!(parsed.episodic_memories.len(), 1);
        assert_eq!(parsed.episodic_memories[0].summary, "Completed migration");
    }

    #[test]
    fn test_openrouter_adapter_config() {
        let adapter = OpenRouterAdapter::new("test_key", "meta-llama/llama-3.3-70b-instruct:free");
        assert_eq!(adapter.api_key(), "test_key");
        assert_eq!(adapter.model(), "meta-llama/llama-3.3-70b-instruct:free");
        assert_eq!(adapter.base_url(), "https://openrouter.ai/api/v1");

        let openai_router = OpenAiAdapter::new_openrouter("test_key", "openrouter/free");
        assert_eq!(openai_router.base_url(), "https://openrouter.ai/api/v1");
    }

    #[test]
    fn test_goal_dag_wave_computation_and_ascii() {
        let mut dag = GoalDag::new();

        let root = GoalNode::root("root", "Master Plan");
        let task_a = GoalNode::task("task_a", "Analyze dependencies");
        let task_b = GoalNode::task("task_b", "Prepare sandbox");
        let task_c = GoalNode::task("task_c", "Apply refactoring");
        let verify = GoalNode::verification("verify_tests", "Run full test suite");

        dag.add_node(root);
        dag.add_node(task_a);
        dag.add_node(task_b);
        dag.add_node(task_c);
        dag.add_node(verify);

        dag.add_dependency("task_c", "task_a").unwrap();
        dag.add_dependency("task_c", "task_b").unwrap();
        dag.add_dependency("verify_tests", "task_c").unwrap();

        assert!(!dag.contains_cycle());
        dag.validate().unwrap();

        let waves = dag.compute_waves().unwrap();
        assert_eq!(waves.len(), 3);
        assert!(waves[0].node_ids.contains(&"root".to_string()));
        assert!(waves[0].node_ids.contains(&"task_a".to_string()));
        assert!(waves[0].node_ids.contains(&"task_b".to_string()));
        assert_eq!(waves[1].node_ids, vec!["task_c".to_string()]);
        assert_eq!(waves[2].node_ids, vec!["verify_tests".to_string()]);

        let tree = dag.to_ascii_tree();
        assert!(tree.contains("WAVE 0"));
        assert!(tree.contains("WAVE 1"));
        assert!(tree.contains("WAVE 2"));
        assert!(tree.contains("task_c"));
    }

    #[test]
    fn test_goal_dag_cycle_detection() {
        let mut dag = GoalDag::new();
        dag.add_node(GoalNode::task("n1", "Node 1"));
        dag.add_node(GoalNode::task("n2", "Node 2"));

        dag.add_dependency("n2", "n1").unwrap();
        dag.add_dependency("n1", "n2").unwrap();

        assert!(dag.contains_cycle());
        assert!(dag.validate().is_err());
        assert!(dag.compute_waves().is_err());
    }

    #[test]
    fn test_goal_decomposer_templates() {
        let decomposer = GoalDecomposer::new();
        let dag = decomposer
            .decompose("Refactor memory engine and add Redis sync")
            .unwrap();

        assert!(dag.node_count() >= 5);
        assert!(dag.contains_node("analyze_architecture"));
        assert!(dag.contains_node("implement_core_logic"));
        assert!(dag.contains_node("verify_contract_invariants"));
        assert!(dag.contains_node("consolidate_documentation"));

        let waves = dag.compute_waves().unwrap();
        assert!(waves.len() >= 3);
    }

    #[tokio::test]
    async fn test_dag_scheduler_execution() {
        let decomposer = GoalDecomposer::new();
        let dag = decomposer
            .decompose("Build distributed sync layer")
            .unwrap();

        let scheduler = DagScheduler::new().with_concurrency(2);
        let (finished_dag, report) = scheduler.execute(dag).await.unwrap();

        assert!(report.success);
        assert_eq!(report.failed_nodes, 0);
        assert!(report.completed_nodes > 0);
        assert!(report.total_waves >= 3);

        for node in finished_dag.all_nodes() {
            if node.kind != GoalNodeKind::Root {
                assert_eq!(node.status, GoalStatus::Completed);
            }
        }
    }

    #[test]
    fn test_training_config_validation() {
        let mut config = TrainingConfig::default();
        assert!(config.validate().is_ok());

        config.lora_r = 0;
        assert!(config.validate().is_err());

        config.lora_r = 16;
        config.learning_rate = -1.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_training_generator_python_and_modelfile() {
        let config = TrainingConfig::new("unsloth/Qwen2.5-Coder-7B-Instruct")
            .with_method(TrainingMethod::Dpo)
            .with_lora(32, 64, 0.05)
            .with_learning_rate(2e-5)
            .with_max_steps(100);

        let script = generate_unsloth_training_script(&config, "data/dpo.jsonl");
        assert!(script.contains("FastLanguageModel.from_pretrained"));
        assert!(script.contains("unsloth/Qwen2.5-Coder-7B-Instruct"));
        assert!(script.contains("load_in_4bit = True"));
        assert!(script.contains("DPOTrainer"));
        assert!(script.contains("r = 32"));
        assert!(script.contains("lora_alpha = 64"));

        let modelfile = generate_ollama_modelfile(&config, "outputs/lora_adapter");
        assert!(modelfile.contains("FROM unsloth/Qwen2.5-Coder-7B-Instruct"));
        assert!(modelfile.contains("ADAPTER outputs/lora_adapter"));
        assert!(modelfile.contains("SYSTEM"));

        let temp_dir = std::env::temp_dir().join("strata_test_pipeline_artifacts");
        let pipeline = TrainingPipeline::new(config);
        let res = pipeline
            .generate_artifacts(
                &temp_dir,
                Some("{\"prompt\":\"p\",\"chosen\":\"c\",\"rejected\":\"r\"}\n"),
                1,
            )
            .unwrap();

        assert!(res.success);
        assert!(std::path::Path::new(&res.script_path).exists());
        assert!(std::path::Path::new(&res.dataset_path).exists());
        assert!(res.modelfile_path.is_some());

        let table = pipeline.format_summary_table(1);
        assert!(table.contains("STRATA LORA FINE-TUNING PIPELINE"));
        assert!(table.contains("unsloth/Qwen2.5-Coder-7B-Instruct"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
