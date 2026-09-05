use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

use crate::errors::StrataError;
use crate::events::{Event, EventId};
use crate::state::{DigestOutput, FailurePattern, MemoryHandle, MemoryRecord, Scope};

#[async_trait]
pub trait EventStore: Send + Sync {
    async fn append(&self, event: &Event) -> Result<EventId, StrataError>;

    async fn append_batch(&self, events: &[Event]) -> Result<Vec<EventId>, StrataError> {
        let mut ids = Vec::with_capacity(events.len());
        for event in events {
            ids.push(self.append(event).await?);
        }
        Ok(ids)
    }

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

    async fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError>;

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
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, input: serde_json::Value) -> Result<serde_json::Value, StrataError>;
}

#[async_trait]
pub trait ToolGateway: Send + Sync {
    async fn register_tool(&self, tool: Arc<dyn Tool>) -> Result<(), StrataError>;
    async fn get_tool(&self, name: &str) -> Result<Option<Arc<dyn Tool>>, StrataError>;
    async fn list_tools(&self) -> Result<Vec<String>, StrataError>;
    async fn invoke(
        &self,
        tool_name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, StrataError>;
}

#[async_trait]
pub trait ReasoningEngine: Send + Sync {
    async fn prompt(
        &self,
        system: Option<&str>,
        user: &str,
        context: Option<serde_json::Value>,
    ) -> Result<String, StrataError>;
}
