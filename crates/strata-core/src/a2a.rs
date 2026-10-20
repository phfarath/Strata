use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Represents an active agent instance in the local workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPresence {
    pub agent_id: String,
    pub host: String, // "cursor", "claude-code", "gemini-cli", etc.
    pub pid: u32,
    pub active_task: Option<String>,
    pub heartbeat_at: i64,
}

impl AgentPresence {
    pub fn new(agent_id: impl Into<String>, host: impl Into<String>, pid: u32) -> Self {
        Self {
            agent_id: agent_id.into(),
            host: host.into(),
            pid,
            active_task: None,
            heartbeat_at: Utc::now().timestamp(),
        }
    }

    pub fn with_active_task(mut self, task: impl Into<String>) -> Self {
        self.active_task = Some(task.into());
        self
    }

    pub fn with_heartbeat(mut self, timestamp: i64) -> Self {
        self.heartbeat_at = timestamp;
        self
    }

    pub fn is_alive(&self, now: i64, timeout_seconds: i64) -> bool {
        now - self.heartbeat_at <= timeout_seconds
    }
}

/// Declarative capability card announced by an agent upon entering the local workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCard {
    pub agent_id: String,
    pub host: String, // "cursor", "claude-code", "gemini-cli", "antigravity", etc.
    pub memory_capabilities: Vec<String>, // ["episodic", "semantic", "procedural"]
    pub domain_scopes: Vec<String>, // ["crates/strata-memory", "crates/strata-core"]
    pub memory_persistence: String, // "longterm", "ephemeral", "session"
    pub tenant_scopes: Vec<String>, // ["project:strata", "global"]
    pub memory_privacy_mode: String, // "isolated", "shared", "read_only"
    pub confidence_score: f32, // Calibrated domain confidence [0.0, 1.0]
    pub updated_at: i64,
}

impl AgentCard {
    pub fn new(agent_id: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            host: host.into(),
            memory_capabilities: vec!["episodic".into(), "semantic".into(), "procedural".into()],
            domain_scopes: Vec::new(),
            memory_persistence: "longterm".into(),
            tenant_scopes: vec!["project:default".into()],
            memory_privacy_mode: "shared".into(),
            confidence_score: 1.0,
            updated_at: Utc::now().timestamp(),
        }
    }

    pub fn with_domain_scopes(mut self, scopes: Vec<String>) -> Self {
        self.domain_scopes = scopes;
        self
    }

    pub fn with_memory_capabilities(mut self, caps: Vec<String>) -> Self {
        self.memory_capabilities = caps;
        self
    }

    pub fn with_tenant_scopes(mut self, scopes: Vec<String>) -> Self {
        self.tenant_scopes = scopes;
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence_score = confidence.clamp(0.0, 1.0);
        self
    }
}

/// Authoritative mapping between a code domain/topic space and the specialist agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainAuthority {
    pub domain: String,              // e.g. "crates/strata-memory", "src/auth"
    pub authoritative_agent: String, // e.g. "agent-claude-1"
    pub confidence_score: f32,       // Calibrated confidence [0.0, 1.0]
    pub lease_held: bool,            // Whether the agent currently holds an exclusive lease
    pub active_tasks_count: u32,
    pub last_active_at: i64,
}

impl DomainAuthority {
    pub fn new(domain: impl Into<String>, agent: impl Into<String>, confidence: f32) -> Self {
        Self {
            domain: domain.into(),
            authoritative_agent: agent.into(),
            confidence_score: confidence.clamp(0.0, 1.0),
            lease_held: false,
            active_tasks_count: 0,
            last_active_at: Utc::now().timestamp(),
        }
    }

    pub fn with_lease(mut self, held: bool) -> Self {
        self.lease_held = held;
        self
    }

    pub fn with_active_tasks(mut self, count: u32) -> Self {
        self.active_tasks_count = count;
        self
    }
}

/// Query routed via the transactive memory directory to the domain authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactiveQuery {
    pub query_id: String,
    pub from_agent: String,
    pub domain_topic: String,
    pub query_text: String,
    pub timeout_ms: u64,
    pub timestamp_us: i64,
}

impl TransactiveQuery {
    pub fn new(
        query_id: impl Into<String>,
        from_agent: impl Into<String>,
        domain_topic: impl Into<String>,
        query_text: impl Into<String>,
    ) -> Self {
        Self {
            query_id: query_id.into(),
            from_agent: from_agent.into(),
            domain_topic: domain_topic.into(),
            query_text: query_text.into(),
            timeout_ms: 5000,
            timestamp_us: Utc::now().timestamp_micros(),
        }
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

/// Response returned by the domain authority agent to a transactive query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactiveResponse {
    pub query_id: String,
    pub responder_agent: String,
    pub confidence_score: f32,
    pub answer: String,
    pub references: Vec<String>,
    pub timestamp_us: i64,
}

impl TransactiveResponse {
    pub fn new(
        query_id: impl Into<String>,
        responder_agent: impl Into<String>,
        confidence_score: f32,
        answer: impl Into<String>,
    ) -> Self {
        Self {
            query_id: query_id.into(),
            responder_agent: responder_agent.into(),
            confidence_score: confidence_score.clamp(0.0, 1.0),
            answer: answer.into(),
            references: Vec::new(),
            timestamp_us: Utc::now().timestamp_micros(),
        }
    }

    pub fn with_references(mut self, references: Vec<String>) -> Self {
        self.references = references;
        self
    }
}

/// Lifecycle status of an asynchronous Agent-to-Agent task delegation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    Pending,
    Accepted,
    Completed,
    Rejected,
    TimedOut,
}

impl DelegationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Completed => "completed",
            Self::Rejected => "rejected",
            Self::TimedOut => "timed_out",
        }
    }
}

impl std::fmt::Display for DelegationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for DelegationStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            "completed" => Ok(Self::Completed),
            "rejected" => Ok(Self::Rejected),
            "timed_out" | "timedout" => Ok(Self::TimedOut),
            other => Err(format!("Unknown delegation status: {other}")),
        }
    }
}

/// Explicit verification-backed contract for delegating tasks between autonomous agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelegationContract {
    pub delegation_id: String,
    pub delegator_agent: String,
    pub delegatee_agent: Option<String>,
    pub domain_scope: Option<String>,
    pub task_id: String,
    pub objective: String,
    pub context: Option<String>,
    #[serde(default)]
    pub expected_artifacts: Vec<String>,
    pub verification_command: Option<String>,
    pub timeout_seconds: u64,
    pub status: DelegationStatus,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

impl DelegationContract {
    pub fn new(
        delegation_id: impl Into<String>,
        delegator_agent: impl Into<String>,
        task_id: impl Into<String>,
        objective: impl Into<String>,
    ) -> Self {
        Self {
            delegation_id: delegation_id.into(),
            delegator_agent: delegator_agent.into(),
            delegatee_agent: None,
            domain_scope: None,
            task_id: task_id.into(),
            objective: objective.into(),
            context: None,
            expected_artifacts: Vec::new(),
            verification_command: None,
            timeout_seconds: 60,
            status: DelegationStatus::Pending,
            result_summary: None,
            error: None,
            created_at: Utc::now().timestamp(),
            completed_at: None,
        }
    }

    pub fn with_delegatee(mut self, delegatee: impl Into<String>) -> Self {
        self.delegatee_agent = Some(delegatee.into());
        self
    }

    pub fn with_domain_scope(mut self, domain: impl Into<String>) -> Self {
        self.domain_scope = Some(domain.into());
        self
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub fn with_artifacts(mut self, artifacts: Vec<String>) -> Self {
        self.expected_artifacts = artifacts;
        self
    }

    pub fn with_verification(mut self, cmd: impl Into<String>) -> Self {
        self.verification_command = Some(cmd.into());
        self
    }

    pub fn with_timeout(mut self, timeout_seconds: u64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    pub fn is_expired(&self, now: i64) -> bool {
        (self.status == DelegationStatus::Pending || self.status == DelegationStatus::Accepted)
            && (now - self.created_at) > (self.timeout_seconds as i64)
    }
}

/// Represents an exclusive temporal lease on a resource (file, directory, or module).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLease {
    pub resource_id: String,
    pub agent_id: String,
    pub lease_expires_at: i64,
    pub metadata: Option<String>,
}

impl ResourceLease {
    pub fn new(
        resource_id: impl Into<String>,
        agent_id: impl Into<String>,
        lease_expires_at: i64,
    ) -> Self {
        Self {
            resource_id: resource_id.into(),
            agent_id: agent_id.into(),
            lease_expires_at,
            metadata: None,
        }
    }

    pub fn with_metadata(mut self, metadata: impl Into<String>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    pub fn is_expired(&self, now: i64) -> bool {
        self.lease_expires_at <= now
    }

    pub fn remaining_seconds(&self, now: i64) -> i64 {
        (self.lease_expires_at - now).max(0)
    }
}

/// Outcome of attempting to acquire or renew a resource lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LeaseAcquireResult {
    Acquired {
        resource_id: String,
        expires_at: i64,
    },
    Conflict {
        resource_id: String,
        held_by: String,
        remaining_seconds: i64,
    },
}

/// Real-time event transmitted across the local A2A IPC bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum IpcEvent {
    LeaseAcquired {
        resource_id: String,
        agent_id: String,
        expires_at: i64,
        metadata: Option<String>,
        timestamp_us: i64,
    },
    LeaseReleased {
        resource_id: String,
        agent_id: String,
        timestamp_us: i64,
    },
    AntiPatternDiscovered {
        pattern_id: String,
        category: String,
        pattern: String,
        remedy: String,
        timestamp_us: i64,
    },
    AgentPresenceChanged {
        agent_id: String,
        host: String,
        pid: u32,
        status: String, // "active", "idle", "disconnected"
        timestamp_us: i64,
    },
    MemoryPromoted {
        memory_id: String,
        tier: String,
        to_global: bool,
        timestamp_us: i64,
    },
    DirectMessage {
        message_id: String,
        from_agent: String,
        to_agent: String,
        task_id: Option<String>,
        content: String,
        timestamp_us: i64,
    },
    TaskBroadcast {
        message_id: String,
        from_agent: String,
        task_id: Option<String>,
        kind: String,
        content: String,
        timestamp_us: i64,
    },
    TaskStateChanged {
        task_id: String,
        plan_id: Option<String>,
        old_status: crate::blackboard::TaskStatus,
        new_status: crate::blackboard::TaskStatus,
        agent_id: Option<String>,
        timestamp_us: i64,
    },
    AgentAnnounced {
        card: AgentCard,
        timestamp_us: i64,
    },
    DomainAuthorityUpdated {
        authority: DomainAuthority,
        timestamp_us: i64,
    },
    TransactiveQueryRouted {
        query: TransactiveQuery,
        timestamp_us: i64,
    },
    TransactiveResponseReturned {
        response: TransactiveResponse,
        timestamp_us: i64,
    },
    TaskDelegated {
        contract: DelegationContract,
        timestamp_us: i64,
    },
    DelegationAccepted {
        delegation_id: String,
        delegatee_agent: String,
        timestamp_us: i64,
    },
    DelegationCompleted {
        delegation_id: String,
        delegatee_agent: String,
        success: bool,
        summary: String,
        artifacts: Vec<String>,
        timestamp_us: i64,
    },
    DelegationRejected {
        delegation_id: String,
        delegatee_agent: String,
        reason: String,
        timestamp_us: i64,
    },
}

impl IpcEvent {
    pub fn timestamp_us(&self) -> i64 {
        match self {
            Self::LeaseAcquired { timestamp_us, .. } => *timestamp_us,
            Self::LeaseReleased { timestamp_us, .. } => *timestamp_us,
            Self::AntiPatternDiscovered { timestamp_us, .. } => *timestamp_us,
            Self::AgentPresenceChanged { timestamp_us, .. } => *timestamp_us,
            Self::MemoryPromoted { timestamp_us, .. } => *timestamp_us,
            Self::DirectMessage { timestamp_us, .. } => *timestamp_us,
            Self::TaskBroadcast { timestamp_us, .. } => *timestamp_us,
            Self::TaskStateChanged { timestamp_us, .. } => *timestamp_us,
            Self::AgentAnnounced { timestamp_us, .. } => *timestamp_us,
            Self::DomainAuthorityUpdated { timestamp_us, .. } => *timestamp_us,
            Self::TransactiveQueryRouted { timestamp_us, .. } => *timestamp_us,
            Self::TransactiveResponseReturned { timestamp_us, .. } => *timestamp_us,
            Self::TaskDelegated { timestamp_us, .. } => *timestamp_us,
            Self::DelegationAccepted { timestamp_us, .. } => *timestamp_us,
            Self::DelegationCompleted { timestamp_us, .. } => *timestamp_us,
            Self::DelegationRejected { timestamp_us, .. } => *timestamp_us,
        }
    }
}

/// Persistent Agent-to-Agent message stored in SQLite and dispatched via IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2aMessage {
    pub message_id: String,
    pub from_agent: String,
    pub to_agent: Option<String>, // None = broadcast to all agents in workspace
    pub task_id: Option<String>,
    pub kind: String, // "direct", "broadcast", "blocker", "question", "handoff"
    pub content: String,
    pub read_at: Option<i64>,
    pub created_at: i64,
}

impl A2aMessage {
    pub fn direct(
        message_id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            from_agent: from.into(),
            to_agent: Some(to.into()),
            task_id: None,
            kind: "direct".to_string(),
            content: content.into(),
            read_at: None,
            created_at: Utc::now().timestamp(),
        }
    }

    pub fn broadcast(
        message_id: impl Into<String>,
        from: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            from_agent: from.into(),
            to_agent: None,
            task_id: None,
            kind: "broadcast".to_string(),
            content: content.into(),
            read_at: None,
            created_at: Utc::now().timestamp(),
        }
    }

    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = kind.into();
        self
    }
}

/// Top-level framed message on the local IPC transport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum IpcMessage {
    Event { event: IpcEvent },
    Ping { timestamp: i64 },
    Pong { timestamp: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_presence_lifecycle() {
        let now = 1700000000;
        let presence = AgentPresence::new("agent-cursor-1", "cursor", 12345)
            .with_active_task("refactoring core")
            .with_heartbeat(now);

        assert_eq!(presence.agent_id, "agent-cursor-1");
        assert_eq!(presence.host, "cursor");
        assert_eq!(presence.pid, 12345);
        assert_eq!(presence.active_task.as_deref(), Some("refactoring core"));
        assert!(presence.is_alive(now + 10, 30));
        assert!(!presence.is_alive(now + 31, 30));

        let json = serde_json::to_string(&presence).expect("serialize presence");
        let de: AgentPresence = serde_json::from_str(&json).expect("deserialize presence");
        assert_eq!(presence, de);
    }

    #[test]
    fn test_delegation_contract_lifecycle() {
        let contract = DelegationContract::new(
            "del-123",
            "agent-planner",
            "task-456",
            "Implement A2A delegation",
        )
        .with_domain_scope("crates/strata-core")
        .with_delegatee("agent-worker")
        .with_verification("cargo test")
        .with_artifacts(vec!["src/lib.rs".to_string()])
        .with_timeout(120);

        assert_eq!(contract.status, DelegationStatus::Pending);
        assert!(!contract.is_expired(contract.created_at + 10));
        assert!(contract.is_expired(contract.created_at + 121));

        let json = serde_json::to_string(&contract).expect("serialize contract");
        let de: DelegationContract = serde_json::from_str(&json).expect("deserialize contract");
        assert_eq!(contract, de);
    }

    #[test]
    fn test_resource_lease_expiration() {
        let now = 1700000000;
        let lease = ResourceLease::new("crate:strata-cli", "agent-claude", now + 60)
            .with_metadata("building release");

        assert!(!lease.is_expired(now));
        assert!(!lease.is_expired(now + 59));
        assert!(lease.is_expired(now + 60));
        assert_eq!(lease.remaining_seconds(now), 60);
        assert_eq!(lease.remaining_seconds(now + 70), 0);

        let acquired = LeaseAcquireResult::Acquired {
            resource_id: "crate:strata-cli".to_string(),
            expires_at: now + 60,
        };
        let conflict = LeaseAcquireResult::Conflict {
            resource_id: "crate:strata-cli".to_string(),
            held_by: "agent-claude".to_string(),
            remaining_seconds: 45,
        };

        let json_acq = serde_json::to_string(&acquired).unwrap();
        assert!(json_acq.contains("\"status\":\"acquired\""));
        let json_conf = serde_json::to_string(&conflict).unwrap();
        assert!(json_conf.contains("\"status\":\"conflict\""));
    }

    #[test]
    fn test_ipc_message_roundtrip() {
        let event = IpcEvent::AntiPatternDiscovered {
            pattern_id: "pat-123".to_string(),
            category: "compiler".to_string(),
            pattern: "borrow checker error on mutex guard".to_string(),
            remedy: "drop guard before await".to_string(),
            timestamp_us: 1700000000123456,
        };
        assert_eq!(event.timestamp_us(), 1700000000123456);

        let msg = IpcMessage::Event {
            event: event.clone(),
        };

        let json = serde_json::to_string(&msg).expect("serialize IPC message");
        assert!(json.contains("\"kind\":\"event\""));
        assert!(json.contains("\"type\":\"anti_pattern_discovered\""));
        assert!(json.contains("\"pattern_id\":\"pat-123\""));

        let de: IpcMessage = serde_json::from_str(&json).expect("deserialize IPC message");
        assert_eq!(msg, de);

        let ping = IpcMessage::Ping { timestamp: 123456 };
        let ping_json = serde_json::to_string(&ping).unwrap();
        let ping_de: IpcMessage = serde_json::from_str(&ping_json).unwrap();
        assert_eq!(ping, ping_de);
    }

    #[test]
    fn test_a2a_messaging_events() {
        let dm = IpcEvent::DirectMessage {
            message_id: "msg-1".to_string(),
            from_agent: "claude".to_string(),
            to_agent: "cursor".to_string(),
            task_id: Some("task-10".to_string()),
            content: "Schema created, ready for endpoints".to_string(),
            timestamp_us: 1700000000000000,
        };
        assert_eq!(dm.timestamp_us(), 1700000000000000);

        let json = serde_json::to_string(&dm).unwrap();
        assert!(json.contains("\"type\":\"direct_message\""));
        assert!(json.contains("\"from_agent\":\"claude\""));

        let de: IpcEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(dm, de);

        let task_event = IpcEvent::TaskStateChanged {
            task_id: "task-10".to_string(),
            plan_id: Some("plan-1".to_string()),
            old_status: crate::blackboard::TaskStatus::Running,
            new_status: crate::blackboard::TaskStatus::Completed,
            agent_id: Some("claude".to_string()),
            timestamp_us: 1700000000000001,
        };
        let task_json = serde_json::to_string(&task_event).unwrap();
        assert!(task_json.contains("\"type\":\"task_state_changed\""));
        assert!(task_json.contains("\"new_status\":\"completed\""));

        let a2a_msg = A2aMessage::direct("m-1", "agent-a", "agent-b", "hello")
            .with_task_id("task-1")
            .with_kind("handoff");
        assert_eq!(a2a_msg.kind, "handoff");
        assert_eq!(a2a_msg.to_agent.as_deref(), Some("agent-b"));
    }

    #[test]
    fn test_transactive_directory_schemas_and_roundtrip() {
        let card = AgentCard::new("agent-gemini-pro", "gemini-cli")
            .with_domain_scopes(vec![
                "crates/strata-memory".into(),
                "crates/strata-core".into(),
            ])
            .with_memory_capabilities(vec!["episodic".into(), "procedural".into()])
            .with_tenant_scopes(vec!["project:strata".into()])
            .with_confidence(0.95);

        assert_eq!(card.agent_id, "agent-gemini-pro");
        assert_eq!(card.host, "gemini-cli");
        assert_eq!(card.domain_scopes.len(), 2);
        assert_eq!(card.confidence_score, 0.95);

        let card_json = serde_json::to_string(&card).expect("serialize AgentCard");
        let card_de: AgentCard = serde_json::from_str(&card_json).expect("deserialize AgentCard");
        assert_eq!(card, card_de);

        let authority = DomainAuthority::new("crates/strata-memory", "agent-gemini-pro", 0.92)
            .with_lease(true)
            .with_active_tasks(1);

        assert!(authority.lease_held);
        assert_eq!(authority.active_tasks_count, 1);
        assert_eq!(authority.authoritative_agent, "agent-gemini-pro");

        let auth_json = serde_json::to_string(&authority).expect("serialize DomainAuthority");
        let auth_de: DomainAuthority =
            serde_json::from_str(&auth_json).expect("deserialize DomainAuthority");
        assert_eq!(authority, auth_de);

        let query = TransactiveQuery::new(
            "q-101",
            "agent-cursor-1",
            "crates/strata-memory",
            "How does JTMS belief revision invalidate code anchors?",
        )
        .with_timeout_ms(3000);

        assert_eq!(query.timeout_ms, 3000);

        let response = TransactiveResponse::new(
            "q-101",
            "agent-gemini-pro",
            0.96,
            "JTMS traverses non-monotonic dependency edges and triggers Blake3 Merkle diff checks.",
        )
        .with_references(vec!["crates/strata-memory/src/jtms.rs".into()]);

        assert_eq!(response.references.len(), 1);

        let event_q = IpcEvent::TransactiveQueryRouted {
            query: query.clone(),
            timestamp_us: 1700000000000100,
        };
        let event_r = IpcEvent::TransactiveResponseReturned {
            response: response.clone(),
            timestamp_us: 1700000000000200,
        };

        assert_eq!(event_q.timestamp_us(), 1700000000000100);
        assert_eq!(event_r.timestamp_us(), 1700000000000200);

        let q_json = serde_json::to_string(&event_q).unwrap();
        assert!(q_json.contains("\"type\":\"transactive_query_routed\""));
        let r_json = serde_json::to_string(&event_r).unwrap();
        assert!(r_json.contains("\"type\":\"transactive_response_returned\""));
    }
}
