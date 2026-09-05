use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for EventId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for EventId {
    fn from(u: Uuid) -> Self {
        Self(u)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DataClassification {
    Public,
    #[default]
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RetentionPolicy {
    Ephemeral,
    Session,
    RetainDays(u32),
    #[default]
    Permanent,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_agent: String,
    pub source_session: String,
    pub organization_id: Option<String>,
    pub client: Option<String>,
    pub trace_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Provenance {
    pub fn new(source_agent: impl Into<String>, source_session: impl Into<String>) -> Self {
        Self {
            source_agent: source_agent.into(),
            source_session: source_session.into(),
            organization_id: None,
            client: None,
            trace_id: None,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStarted {
    pub session_id: String,
    pub agent_id: String,
    pub organization_id: Option<String>,
    #[serde(default)]
    pub environment: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEnded {
    pub session_id: String,
    pub agent_id: String,
    pub final_state: Option<String>,
    pub reason: Option<String>,
    pub summary: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationReceived {
    pub session_id: String,
    pub source: String,
    pub observation_type: String,
    pub content: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryWritten {
    pub memory_id: Uuid,
    pub memory_type: String,
    pub scope: String,
    pub content_summary: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryConsolidated {
    pub source_memory_ids: Vec<Uuid>,
    pub target_memory_id: Option<Uuid>,
    pub consolidation_type: String,
    pub summary: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInvoked {
    pub invocation_id: Uuid,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultReceived {
    pub invocation_id: Uuid,
    pub tool_name: String,
    pub result: serde_json::Value,
    pub is_error: bool,
    pub duration_ms: Option<u64>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskStarted {
    pub task_id: String,
    pub title: String,
    pub description: Option<String>,
    pub parent_task_id: Option<String>,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskCompleted {
    pub task_id: String,
    pub success: bool,
    pub outcome_summary: String,
    pub evaluation: Option<serde_json::Value>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObserved {
    pub error_type: String,
    pub message: String,
    pub severity: String,
    pub context: Option<serde_json::Value>,
    pub stack_trace: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    SessionStarted(SessionStarted),
    SessionEnded(SessionEnded),
    ObservationReceived(ObservationReceived),
    MemoryWritten(MemoryWritten),
    MemoryConsolidated(MemoryConsolidated),
    ToolInvoked(ToolInvoked),
    ToolResultReceived(ToolResultReceived),
    TaskStarted(TaskStarted),
    TaskCompleted(TaskCompleted),
    ErrorObserved(ErrorObserved),
    Custom {
        event_name: String,
        payload: serde_json::Value,
    },
}

impl EventPayload {
    pub fn event_type(&self) -> &'static str {
        match self {
            EventPayload::SessionStarted(_) => "SessionStarted",
            EventPayload::SessionEnded(_) => "SessionEnded",
            EventPayload::ObservationReceived(_) => "ObservationReceived",
            EventPayload::MemoryWritten(_) => "MemoryWritten",
            EventPayload::MemoryConsolidated(_) => "MemoryConsolidated",
            EventPayload::ToolInvoked(_) => "ToolInvoked",
            EventPayload::ToolResultReceived(_) => "ToolResultReceived",
            EventPayload::TaskStarted(_) => "TaskStarted",
            EventPayload::TaskCompleted(_) => "TaskCompleted",
            EventPayload::ErrorObserved(_) => "ErrorObserved",
            EventPayload::Custom { .. } => "Custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub sequence: Option<u64>,
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
    pub agent_id: String,
    pub organization_id: Option<String>,
    pub provenance: Provenance,
    pub classification: DataClassification,
    pub retention: RetentionPolicy,
    pub payload: EventPayload,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

pub type CanonicalEvent = Event;

impl Event {
    pub fn new(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        payload: EventPayload,
    ) -> Self {
        let s_id = session_id.into();
        let a_id = agent_id.into();
        let prov = Provenance::new(a_id.clone(), s_id.clone());
        Self {
            id: EventId::new(),
            sequence: None,
            timestamp: Utc::now(),
            session_id: s_id,
            agent_id: a_id,
            organization_id: None,
            provenance: prov,
            classification: DataClassification::default(),
            retention: RetentionPolicy::default(),
            payload,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    pub fn with_classification(mut self, classification: DataClassification) -> Self {
        self.classification = classification;
        self
    }

    pub fn with_retention(mut self, retention: RetentionPolicy) -> Self {
        self.retention = retention;
        self
    }

    pub fn with_organization(mut self, organization_id: impl Into<String>) -> Self {
        self.organization_id = Some(organization_id.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}
