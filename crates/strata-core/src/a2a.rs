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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl IpcEvent {
    pub fn timestamp_us(&self) -> i64 {
        match self {
            Self::LeaseAcquired { timestamp_us, .. } => *timestamp_us,
            Self::LeaseReleased { timestamp_us, .. } => *timestamp_us,
            Self::AntiPatternDiscovered { timestamp_us, .. } => *timestamp_us,
            Self::AgentPresenceChanged { timestamp_us, .. } => *timestamp_us,
            Self::MemoryPromoted { timestamp_us, .. } => *timestamp_us,
        }
    }
}

/// Top-level framed message on the local IPC transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
}
