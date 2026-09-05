use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::engine::{ChatMessage, PromptContext, ReasoningEngine, ReasoningOutput, ToolCall};
use strata_core::errors::StrataError;

/// Mock reasoning engine designed for deterministic, repeatable testing of cognitive loops.
#[derive(Clone)]
pub struct MockReasoningEngine {
    canned_responses: Arc<Mutex<Vec<ReasoningOutput>>>,
    recorded_contexts: Arc<Mutex<Vec<PromptContext>>>,
    error_on_next: Arc<Mutex<Option<String>>>,
}

impl MockReasoningEngine {
    pub fn new() -> Self {
        Self {
            canned_responses: Arc::new(Mutex::new(Vec::new())),
            recorded_contexts: Arc::new(Mutex::new(Vec::new())),
            error_on_next: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_responses(responses: Vec<ReasoningOutput>) -> Self {
        Self {
            canned_responses: Arc::new(Mutex::new(responses)),
            recorded_contexts: Arc::new(Mutex::new(Vec::new())),
            error_on_next: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn push_response(&self, response: ReasoningOutput) {
        let mut canned = self.canned_responses.lock().await;
        canned.push(response);
    }

    pub async fn push_text(&self, text: impl Into<String>) {
        self.push_response(ReasoningOutput::text(text)).await;
    }

    pub async fn push_tool_call(&self, name: impl Into<String>, args: serde_json::Value) {
        let call = ToolCall::new(Uuid::new_v4().to_string(), name, args);
        self.push_response(ReasoningOutput::tool_calls(vec![call]))
            .await;
    }

    pub async fn push_tool_calls(&self, calls: Vec<ToolCall>) {
        self.push_response(ReasoningOutput::tool_calls(calls)).await;
    }

    pub async fn push_json<T: serde::Serialize>(&self, value: &T) {
        let json_str = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
        self.push_text(json_str).await;
    }

    pub async fn push_distillation_output(&self, output: &crate::prompts::DistillationOutput) {
        self.push_json(output).await;
    }

    pub async fn push_jtms_response(&self, classification: &str, reason: &str, confidence: f32) {
        let payload = serde_json::json!({
            "classification": classification,
            "reason": reason,
            "confidence": confidence
        });
        self.push_text(payload.to_string()).await;
    }

    pub async fn set_error(&self, error_message: impl Into<String>) {
        let mut err = self.error_on_next.lock().await;
        *err = Some(error_message.into());
    }

    pub async fn call_count(&self) -> usize {
        let contexts = self.recorded_contexts.lock().await;
        contexts.len()
    }

    pub async fn recorded_contexts(&self) -> Vec<PromptContext> {
        let contexts = self.recorded_contexts.lock().await;
        contexts.clone()
    }

    pub async fn last_context(&self) -> Option<PromptContext> {
        let contexts = self.recorded_contexts.lock().await;
        contexts.last().cloned()
    }

    pub async fn assert_called_times(&self, expected: usize) {
        let count = self.call_count().await;
        assert_eq!(
            count, expected,
            "Expected MockReasoningEngine to be called {} times, but got {}",
            expected, count
        );
    }

    pub async fn assert_last_message_contains(&self, substring: &str) {
        let ctx = self.last_context().await.expect("No context was recorded");
        let last_msg = ctx
            .messages
            .last()
            .expect("Recorded context has no messages");
        assert!(
            last_msg.content.contains(substring),
            "Expected last message to contain '{}', but got '{}'",
            substring,
            last_msg.content
        );
    }

    pub async fn assert_has_tool_registered(&self, tool_name: &str) {
        let ctx = self.last_context().await.expect("No context was recorded");
        let found = ctx.tools.iter().any(|t| t.name == tool_name);
        assert!(
            found,
            "Expected tool '{}' to be registered in PromptContext tools",
            tool_name
        );
    }
}

impl Default for MockReasoningEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ReasoningEngine for MockReasoningEngine {
    async fn complete(&self, context: &PromptContext) -> Result<ReasoningOutput, StrataError> {
        // Record incoming context for inspection
        {
            let mut recorded = self.recorded_contexts.lock().await;
            recorded.push(context.clone());
        }

        // Check if an error was requested
        {
            let mut err = self.error_on_next.lock().await;
            if let Some(msg) = err.take() {
                return Err(StrataError::Reasoning(msg));
            }
        }

        // Pop the next canned response, or return a default fallback response
        let mut canned = self.canned_responses.lock().await;
        if !canned.is_empty() {
            Ok(canned.remove(0))
        } else {
            // Default deterministic echo response
            let last_content = context
                .messages
                .last()
                .map(|m| m.content.as_str())
                .unwrap_or("default response");
            Ok(ReasoningOutput::text(format!(
                "Mock response for: {}",
                last_content
            )))
        }
    }
}

#[async_trait]
impl strata_core::traits::ReasoningEngine for MockReasoningEngine {
    async fn prompt(
        &self,
        system: Option<&str>,
        user: &str,
        context: Option<serde_json::Value>,
    ) -> Result<String, StrataError> {
        let mut ctx = PromptContext::new().with_message(ChatMessage::user(user));
        if let Some(sys) = system {
            ctx = ctx.with_system(sys);
        }
        if let Some(meta) = context {
            ctx.metadata = meta;
        }
        let output = self.complete(&ctx).await?;
        Ok(output.content.unwrap_or_default())
    }
}
