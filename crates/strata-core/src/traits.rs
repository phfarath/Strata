use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::StrataError;
use crate::events::{Event, EventId};
use crate::state::{DigestOutput, FailurePattern, MemoryHandle, MemoryRecord, Scope};

#[async_trait]
pub trait EventStore: Send + Sync {
    async fn append(&self, event: &Event) -> Result<EventId, StrataError>;

    async fn read_stream(
        &self,
        session_id: &str,
        from_seq: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<Event>, StrataError>;
}

#[async_trait]
pub trait MemoryEngine: Send + Sync {
    async fn search(
        &self,
        query: &str,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError>;

    async fn search_as_of(
        &self,
        query: &str,
        scope: Option<&Scope>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        if as_of_time.is_none() && as_of_commit.is_none() && as_of_branch.is_none() {
            self.search(query, scope, limit).await
        } else {
            Err(StrataError::Validation(
                "Bi-temporal/branch search_as_of is not supported by this engine".to_string(),
            ))
        }
    }

    async fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError>;

    async fn get_as_of(
        &self,
        id: &Uuid,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
    ) -> Result<Option<MemoryRecord>, StrataError> {
        if as_of_time.is_none() && as_of_commit.is_none() && as_of_branch.is_none() {
            self.get(id).await
        } else {
            Err(StrataError::Validation(
                "Bi-temporal/branch get_as_of is not supported by this engine".to_string(),
            ))
        }
    }

    async fn write(&self, record: &MemoryRecord) -> Result<MemoryHandle, StrataError>;

    async fn digest(
        &self,
        session_id: &str,
        max_tokens: Option<usize>,
    ) -> Result<DigestOutput, StrataError>;

    async fn record_failure(&self, failure: &FailurePattern) -> Result<(), StrataError>;

    async fn get_known_failures(
        &self,
        query: Option<&str>,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError>;

    async fn get_symbol_anti_patterns(
        &self,
        _file: &str,
        _symbol: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        Ok(Vec::new())
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, input: serde_json::Value) -> Result<serde_json::Value, StrataError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ToolCall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub result: serde_json::Value,
    pub is_error: bool,
}

impl ToolResult {
    pub fn new(
        call_id: impl Into<String>,
        name: impl Into<String>,
        result: serde_json::Value,
        is_error: bool,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            result,
            is_error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<Vec<ToolResult>>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_results: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_results: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_results: None,
        }
    }

    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Some(tool_calls),
            tool_results: None,
        }
    }

    pub fn tool_response(tool_results: Vec<ToolResult>) -> Self {
        Self {
            role: Role::Tool,
            content: String::new(),
            tool_calls: None,
            tool_results: Some(tool_results),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PromptContext {
    pub messages: Vec<ChatMessage>,
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub metadata: serde_json::Value,
}

impl PromptContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system_prompt = Some(system.into());
        self
    }

    pub fn with_message(mut self, msg: ChatMessage) -> Self {
        self.messages.push(msg);
        self
    }

    pub fn with_messages(mut self, msgs: Vec<ChatMessage>) -> Self {
        self.messages = msgs;
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = Some(temp);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningOutput {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
    pub usage: Option<TokenUsage>,
}

impl ReasoningOutput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            tool_calls: Vec::new(),
            finish_reason: "stop".to_string(),
            usage: None,
        }
    }

    pub fn tool_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            content: None,
            tool_calls: calls,
            finish_reason: "tool_calls".to_string(),
            usage: None,
        }
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Abstract contract for LLM reasoning engines and structured inference.
#[async_trait]
pub trait ReasoningEngine: Send + Sync {
    async fn complete(&self, context: &PromptContext) -> Result<ReasoningOutput, StrataError>;

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
