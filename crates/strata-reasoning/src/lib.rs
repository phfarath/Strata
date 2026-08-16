pub mod adapters;
pub mod engine;
pub mod mock;

pub use adapters::*;
pub use engine::*;
pub use mock::*;

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
}
