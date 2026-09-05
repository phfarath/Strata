use std::sync::Arc;
use strata_core::a2a::{AgentPresence, LeaseAcquireResult, ResourceLease};
use strata_core::errors::StrataError;

use crate::store::SqliteStore;

/// High-level stigmergic coordinator managing agent presence, heartbeats,
/// and atomic temporal leases over workspace resources.
#[derive(Clone)]
pub struct StigmergyCoordinator {
    store: Arc<SqliteStore>,
}

impl std::fmt::Debug for StigmergyCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StigmergyCoordinator").finish()
    }
}

impl StigmergyCoordinator {
    /// Creates a new `StigmergyCoordinator` wrapping the given SQLite store.
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }

    /// Records or updates an agent's presence and heartbeat.
    pub fn heartbeat(&self, presence: &AgentPresence) -> Result<(), StrataError> {
        self.store.record_agent_presence(presence)
    }

    /// Lists active agents whose last heartbeat was within `ttl_seconds`.
    pub fn active_agents(&self, ttl_seconds: i64) -> Result<Vec<AgentPresence>, StrataError> {
        self.store.get_active_agents(ttl_seconds)
    }

    /// Deregisters an agent from the active workspace presence table.
    pub fn deregister_agent(&self, agent_id: &str) -> Result<bool, StrataError> {
        self.store.remove_agent_presence(agent_id)
    }

    /// Atomically acquires or renews a lease on a resource.
    pub fn acquire_lease(
        &self,
        resource_id: &str,
        agent_id: &str,
        ttl_seconds: i64,
        metadata: Option<&str>,
    ) -> Result<LeaseAcquireResult, StrataError> {
        self.store
            .acquire_or_renew_lease(resource_id, agent_id, ttl_seconds, metadata)
    }

    /// Releases a lease if held by the calling agent.
    pub fn release_lease(&self, resource_id: &str, agent_id: &str) -> Result<bool, StrataError> {
        self.store.release_lease(resource_id, agent_id)
    }

    /// Checks the current lease on a resource, if any.
    pub fn get_lease(&self, resource_id: &str) -> Result<Option<ResourceLease>, StrataError> {
        self.store.get_lease(resource_id)
    }

    /// Lists all currently active, unexpired leases in the workspace.
    pub fn active_leases(&self) -> Result<Vec<ResourceLease>, StrataError> {
        self.store.list_active_leases()
    }

    /// Checks if a resource has an active conflicting lease held by *another* agent.
    pub fn check_conflict(
        &self,
        resource_id: &str,
        requesting_agent_id: &str,
    ) -> Result<Option<ResourceLease>, StrataError> {
        if let Some(lease) = self.store.get_lease(resource_id)? {
            let now = chrono::Utc::now().timestamp();
            if !lease.is_expired(now) && lease.agent_id != requesting_agent_id {
                return Ok(Some(lease));
            }
        }
        Ok(None)
    }

    /// Prunes expired leases from the database.
    pub fn prune_expired(&self) -> Result<usize, StrataError> {
        self.store.prune_expired_leases()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_presence_heartbeat_and_discovery() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);

        let now = Utc::now().timestamp();
        let agent1 = AgentPresence::new("agent-cursor", "cursor", 1001)
            .with_active_task("refactoring core")
            .with_heartbeat(now);
        let agent2 = AgentPresence::new("agent-claude", "claude-code", 1002)
            .with_active_task("running integration tests")
            .with_heartbeat(now - 120);

        coordinator.heartbeat(&agent1).expect("heartbeat agent1");
        coordinator.heartbeat(&agent2).expect("heartbeat agent2");

        // Query with TTL of 60 seconds: agent1 is active, agent2 is considered stale
        let active = coordinator.active_agents(60).expect("active agents 60s");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].agent_id, "agent-cursor");

        // Query with TTL of 300 seconds: both are returned
        let active_all = coordinator.active_agents(300).expect("active agents 300s");
        assert_eq!(active_all.len(), 2);

        // Explicit deregister
        let removed = coordinator
            .deregister_agent("agent-cursor")
            .expect("deregister");
        assert!(removed);
        let active_after = coordinator.active_agents(60).expect("active after");
        assert_eq!(active_after.len(), 0);
    }

    #[test]
    fn test_stigmergic_atomic_leases_and_conflict() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);

        let res_id = "crate:strata-cli";

        // Agent 1 acquires lease for 10 seconds
        let res1 = coordinator
            .acquire_lease(res_id, "agent-cursor", 10, Some("editing main.rs"))
            .expect("acquire lease");
        match res1 {
            LeaseAcquireResult::Acquired { expires_at, .. } => {
                assert!(expires_at > Utc::now().timestamp());
            }
            LeaseAcquireResult::Conflict { .. } => panic!("Expected acquired, got conflict"),
        }

        // Agent 2 attempts to acquire the same lease -> Conflict
        let res2 = coordinator
            .acquire_lease(res_id, "agent-claude", 10, None)
            .expect("acquire lease conflict");
        match res2 {
            LeaseAcquireResult::Conflict {
                held_by,
                remaining_seconds,
                ..
            } => {
                assert_eq!(held_by, "agent-cursor");
                assert!(remaining_seconds > 0);
            }
            LeaseAcquireResult::Acquired { .. } => {
                panic!("Expected conflict, got acquired!");
            }
        }

        // Check conflict helper
        let conflict = coordinator
            .check_conflict(res_id, "agent-claude")
            .expect("check conflict");
        assert!(conflict.is_some());
        assert_eq!(conflict.unwrap().agent_id, "agent-cursor");

        // Check conflict helper from the holder's perspective -> None
        let conflict_self = coordinator
            .check_conflict(res_id, "agent-cursor")
            .expect("check conflict self");
        assert!(conflict_self.is_none());

        // Agent 1 can renew its own lease
        let res_renew = coordinator
            .acquire_lease(res_id, "agent-cursor", 20, Some("still editing"))
            .expect("renew lease");
        assert!(matches!(res_renew, LeaseAcquireResult::Acquired { .. }));

        // Agent 1 explicitly releases the lease
        let released = coordinator
            .release_lease(res_id, "agent-cursor")
            .expect("release lease");
        assert!(released);

        // Agent 2 now succeeds immediately
        let res3 = coordinator
            .acquire_lease(res_id, "agent-claude", 10, None)
            .expect("acquire after release");
        assert!(matches!(res3, LeaseAcquireResult::Acquired { .. }));
    }

    #[test]
    fn test_expired_lease_auto_recovery_without_daemon() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);

        let res_id = "file:src/mcp/server.rs";

        // Agent 1 acquires a lease with negative / expired TTL (simulating a crash / elapsed time)
        let _ = coordinator.acquire_lease(res_id, "agent-crashed", -5, Some("crashed task"));

        // Agent 2 attempts to acquire -> should succeed because existing lease is expired
        let res = coordinator
            .acquire_lease(res_id, "agent-alive", 30, Some("taking over"))
            .expect("acquire expired lease");

        match res {
            LeaseAcquireResult::Acquired {
                expires_at,
                resource_id,
            } => {
                assert_eq!(resource_id, res_id);
                assert!(expires_at > Utc::now().timestamp());
            }
            LeaseAcquireResult::Conflict { .. } => {
                panic!("Failed to automatically take over expired lease!");
            }
        }

        let lease = coordinator
            .get_lease(res_id)
            .expect("get lease")
            .expect("lease exists");
        assert_eq!(lease.agent_id, "agent-alive");
    }
}
