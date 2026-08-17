pub mod adapters;
pub mod engine;
pub mod mock;
pub mod prompts;

pub use adapters::*;
pub use engine::*;
pub use mock::*;
pub use prompts::*;

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
        assert_eq!(output.content.as_deref(), Some("Hello from mock reasoning engine!"));
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
        mock.set_error("Rate limit from upstream LLM provider").await;

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
        let new_fact = SemanticFact::new("System migrated to gRPC Protobuf")
            .with_summary("gRPC Migration");

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
        distillation.semantic_facts.push(
            SemanticFact::new("Database is SQLite WAL mode").with_importance(0.95),
        );
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
        assert_eq!(parsed.semantic_facts[0].statement, "Database is SQLite WAL mode");
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
}
