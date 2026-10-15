use std::sync::Arc;
use strata_core::a2a::{
    A2aMessage, AgentCard, AgentPresence, DomainAuthority, LeaseAcquireResult, ResourceLease,
};
use strata_core::blackboard::{TaskArtifact, TaskFilter, TaskRecord};
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

    // Blackboard & Tasks

    pub fn create_task(&self, task: &TaskRecord) -> Result<(), StrataError> {
        self.store.create_task(task)
    }

    pub fn create_tasks_batch(&self, tasks: &[TaskRecord]) -> Result<(), StrataError> {
        self.store.create_tasks_batch(tasks)
    }

    pub fn get_task(&self, task_id: &str) -> Result<Option<TaskRecord>, StrataError> {
        self.store.get_task(task_id)
    }

    pub fn list_tasks(&self, filter: &TaskFilter) -> Result<Vec<TaskRecord>, StrataError> {
        self.store.list_tasks(filter)
    }

    pub fn claim_task_atomic(
        &self,
        task_id_opt: Option<&str>,
        agent_id: &str,
        lease_ttl_secs: i64,
    ) -> Result<Option<TaskRecord>, StrataError> {
        self.store.claim_task_atomic(task_id_opt, agent_id, lease_ttl_secs)
    }

    pub fn complete_task(
        &self,
        task_id: &str,
        agent_id: &str,
        summary: Option<&str>,
        artifact: Option<&TaskArtifact>,
    ) -> Result<bool, StrataError> {
        self.store.complete_task(task_id, agent_id, summary, artifact)
    }

    pub fn fail_task(
        &self,
        task_id: &str,
        agent_id: &str,
        error_message: &str,
    ) -> Result<bool, StrataError> {
        self.store.fail_task(task_id, agent_id, error_message)
    }

    pub fn update_task_progress(
        &self,
        task_id: &str,
        agent_id: &str,
        message: &str,
    ) -> Result<bool, StrataError> {
        self.store.update_task_progress(task_id, agent_id, message)
    }

    pub fn reclaim_stale_tasks(&self, stale_secs: i64) -> Result<Vec<String>, StrataError> {
        self.store.reclaim_stale_tasks(stale_secs)
    }

    pub fn save_task_artifact(&self, artifact: &TaskArtifact) -> Result<(), StrataError> {
        self.store.save_task_artifact(artifact)
    }

    pub fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, StrataError> {
        self.store.get_task_artifacts(task_id)
    }

    // A2A Mailbox

    pub fn save_a2a_message(&self, msg: &A2aMessage) -> Result<(), StrataError> {
        self.store.save_a2a_message(msg)
    }

    pub fn get_a2a_inbox(&self, agent_id: &str, unread_only: bool) -> Result<Vec<A2aMessage>, StrataError> {
        self.store.get_a2a_inbox(agent_id, unread_only)
    }

    pub fn mark_a2a_read(&self, message_ids: &[String]) -> Result<usize, StrataError> {
        self.store.mark_a2a_read(message_ids)
    }

    // Transactive Memory Directory (A2A Frontier 2)

    /// Registers or updates an agent's capability card.
    pub fn register_agent_card(&self, card: &AgentCard) -> Result<(), StrataError> {
        self.store.record_agent_card(card)
    }

    /// Fetches an agent's capability card.
    pub fn get_agent_card(&self, agent_id: &str) -> Result<Option<AgentCard>, StrataError> {
        self.store.get_agent_card(agent_id)
    }

    /// Lists all registered agent capability cards.
    pub fn list_agent_cards(&self) -> Result<Vec<AgentCard>, StrataError> {
        self.store.list_agent_cards()
    }

    /// Explicitly registers or overrides authority over a code domain.
    pub fn set_domain_authority(&self, authority: &DomainAuthority) -> Result<(), StrataError> {
        self.store.set_domain_authority(authority)
    }

    /// Fetches the authority record for an exact domain.
    pub fn get_domain_authority(&self, domain: &str) -> Result<Option<DomainAuthority>, StrataError> {
        self.store.get_domain_authority(domain)
    }

    /// Lists all domain authorities currently registered in the workspace directory.
    pub fn list_domain_authorities(&self) -> Result<Vec<DomainAuthority>, StrataError> {
        self.store.list_domain_authorities()
    }

    /// Resolves the best specialist agent for a domain or file path using exact and hierarchical matching.
    pub fn resolve_domain_authority(&self, domain_or_path: &str) -> Result<Option<DomainAuthority>, StrataError> {
        self.store.find_best_authority_for_domain(domain_or_path)
    }

    /// Alias for `resolve_domain_authority`.
    pub fn find_best_authority_for_domain(&self, domain_or_path: &str) -> Result<Option<DomainAuthority>, StrataError> {
        self.resolve_domain_authority(domain_or_path)
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

    #[test]
    fn test_blackboard_task_lifecycle_and_work_stealing() {
        use strata_core::blackboard::TaskStatus;

        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);

        // Define two tasks: Task 1 and Task 2 (which depends on Task 1)
        let t1 = TaskRecord::new("subtask-1", "Design DB schema")
            .with_plan_id("plan-alpha")
            .with_required_resources(vec!["file:src/schema.rs".to_string()]);

        let t2 = TaskRecord::new("subtask-2", "Implement repository")
            .with_plan_id("plan-alpha")
            .with_dependencies(vec!["subtask-1".to_string()])
            .with_required_resources(vec!["file:src/repo.rs".to_string()])
            .with_status(TaskStatus::Blocked);

        coordinator.create_tasks_batch(&[t1, t2]).expect("batch insert");

        // Agent Claude claims next task: should receive subtask-1 (subtask-2 is blocked by dependency)
        let claimed_1 = coordinator
            .claim_task_atomic(None, "agent-claude", 60)
            .expect("claim next")
            .expect("t1 claimed");
        assert_eq!(claimed_1.task_id, "subtask-1");
        assert_eq!(claimed_1.assigned_agent.as_deref(), Some("agent-claude"));

        // Verify resource lease acquired for file:src/schema.rs
        let lease = coordinator.get_lease("file:src/schema.rs").unwrap().unwrap();
        assert_eq!(lease.agent_id, "agent-claude");

        // Agent Cursor attempts to claim next task: should be None because subtask-1 is claimed and subtask-2 is blocked
        let claimed_none = coordinator
            .claim_task_atomic(None, "agent-cursor", 60)
            .expect("claim none");
        assert!(claimed_none.is_none());

        // Agent Claude completes subtask-1 with an artifact
        let art = TaskArtifact::new(
            "art-1",
            "subtask-1",
            "agent-claude",
            "summary",
            "Created SQLite tables: tasks and messages",
        );
        let completed = coordinator
            .complete_task("subtask-1", "agent-claude", Some("Schema ready"), Some(&art))
            .expect("complete task");
        assert!(completed);

        // Verify lease was released on file:src/schema.rs
        let lease_after = coordinator.get_lease("file:src/schema.rs").unwrap();
        assert!(lease_after.is_none());

        // Artifact should be retrievable
        let artifacts = coordinator.get_task_artifacts("subtask-1").unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, "summary");

        // Now subtask-2 should be unblocked and claimable by Agent Cursor!
        let claimed_2 = coordinator
            .claim_task_atomic(None, "agent-cursor", 60)
            .expect("claim unblocked")
            .expect("t2 claimed");
        assert_eq!(claimed_2.task_id, "subtask-2");
        assert_eq!(claimed_2.assigned_agent.as_deref(), Some("agent-cursor"));
    }

    #[test]
    fn test_a2a_mailbox_send_and_inbox() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);

        let msg1 = A2aMessage::direct("m-1", "agent-claude", "agent-cursor", "Can you review my schema?")
            .with_task_id("subtask-1");
        let msg2 = A2aMessage::broadcast("m-2", "agent-claude", "All tests passing on crate core");

        coordinator.save_a2a_message(&msg1).unwrap();
        coordinator.save_a2a_message(&msg2).unwrap();

        // Agent Cursor inbox: should see direct message m-1 and broadcast m-2
        let inbox_cursor = coordinator.get_a2a_inbox("agent-cursor", false).unwrap();
        assert_eq!(inbox_cursor.len(), 2);

        // Mark m-1 as read
        let marked = coordinator.mark_a2a_read(&["m-1".to_string()]).unwrap();
        assert_eq!(marked, 1);

        // Unread for Cursor: only broadcast m-2
        let unread = coordinator.get_a2a_inbox("agent-cursor", true).unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].message_id, "m-2");
    }

    #[test]
    fn test_reclaim_stale_tasks_returns_orphans_to_pending() {
        use strata_core::blackboard::TaskStatus;

        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);
        let now = Utc::now().timestamp();

        // Orphan: running for 1000s without any progress heartbeat, holding a lease.
        let mut orphan = TaskRecord::new("orphan", "Dead agent task")
            .with_status(TaskStatus::Running)
            .with_required_resources(vec!["file:src/a.rs".to_string()]);
        orphan.assigned_agent = Some("agent-dead".to_string());
        orphan.updated_at = now - 1000;
        coordinator.create_task(&orphan).unwrap();
        coordinator
            .acquire_lease("file:src/a.rs", "agent-dead", 3600, Some("task:orphan"))
            .unwrap();

        // Healthy: claimed just now.
        let mut fresh = TaskRecord::new("fresh", "Live agent task").with_status(TaskStatus::Claimed);
        fresh.assigned_agent = Some("agent-live".to_string());
        coordinator.create_task(&fresh).unwrap();

        let reclaimed = coordinator.reclaim_stale_tasks(300).unwrap();
        assert_eq!(reclaimed, vec!["orphan".to_string()]);

        let orphan_after = coordinator.get_task("orphan").unwrap().unwrap();
        assert_eq!(orphan_after.status, TaskStatus::Pending);
        assert!(orphan_after.assigned_agent.is_none());
        assert!(coordinator.get_lease("file:src/a.rs").unwrap().is_none());

        let fresh_after = coordinator.get_task("fresh").unwrap().unwrap();
        assert_eq!(fresh_after.status, TaskStatus::Claimed);

        // Reclaimed task is claimable again by another agent.
        let again = coordinator
            .claim_task_atomic(Some("orphan"), "agent-new", 60)
            .unwrap()
            .expect("orphan re-claimable");
        assert_eq!(again.assigned_agent.as_deref(), Some("agent-new"));
    }

    #[test]
    fn test_claim_task_atomic_conflict_and_renewal() {
        use strata_core::blackboard::TaskStatus;

        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);

        let task = TaskRecord::new("task-active", "Active work");
        coordinator.create_task(&task).unwrap();

        // Agent 1 claims the task
        let claimed = coordinator
            .claim_task_atomic(Some("task-active"), "agent-1", 60)
            .unwrap()
            .expect("claimed by agent-1");
        assert_eq!(claimed.assigned_agent.as_deref(), Some("agent-1"));
        assert_eq!(claimed.status, TaskStatus::Claimed);

        // Agent 2 attempts to claim the same task by explicit ID -> should fail (None)
        let conflict = coordinator
            .claim_task_atomic(Some("task-active"), "agent-2", 60)
            .unwrap();
        assert!(conflict.is_none(), "peer agent cannot steal active task");

        // Agent 1 renews/re-claims the same task -> should succeed
        let renewed = coordinator
            .claim_task_atomic(Some("task-active"), "agent-1", 120)
            .unwrap()
            .expect("renewal by owner succeeds");
        assert_eq!(renewed.assigned_agent.as_deref(), Some("agent-1"));
    }

    #[test]
    fn test_transactive_directory_coordination() {
        let store = Arc::new(SqliteStore::open_in_memory().expect("in-memory db"));
        let coordinator = StigmergyCoordinator::new(store);

        // Register agent card for claude-specialist
        let card1 = AgentCard::new("claude-specialist", "claude-code")
            .with_domain_scopes(vec!["crates/strata-memory".into(), "crates/strata-core".into()])
            .with_confidence(0.95);
        coordinator.register_agent_card(&card1).expect("register card1");

        // Register agent card for cursor-generalist
        let card2 = AgentCard::new("cursor-generalist", "cursor")
            .with_domain_scopes(vec!["crates/strata-cli".into()])
            .with_confidence(0.80);
        coordinator.register_agent_card(&card2).expect("register card2");

        let cards = coordinator.list_agent_cards().expect("list cards");
        assert_eq!(cards.len(), 2);

        // Query domain authority for exact match
        let auth_mem = coordinator
            .resolve_domain_authority("crates/strata-memory")
            .expect("resolve authority")
            .expect("found authority");
        assert_eq!(auth_mem.authoritative_agent, "claude-specialist");
        assert_eq!(auth_mem.confidence_score, 0.95);
        assert!(!auth_mem.lease_held);

        // Query domain authority for sub-path (hierarchical prefix matching)
        let auth_sub = coordinator
            .resolve_domain_authority("crates/strata-memory/src/jtms.rs")
            .expect("resolve sub-path")
            .expect("found hierarchical match");
        assert_eq!(auth_sub.authoritative_agent, "claude-specialist");

        // When claude-specialist acquires a lease on crates/strata-memory, lease_held becomes true
        coordinator
            .acquire_lease("crates/strata-memory", "claude-specialist", 60, None)
            .expect("acquire lease");

        let auth_leased = coordinator
            .resolve_domain_authority("crates/strata-memory")
            .unwrap()
            .unwrap();
        assert!(auth_leased.lease_held);

        // Release lease -> lease_held becomes false
        coordinator
            .release_lease("crates/strata-memory", "claude-specialist")
            .expect("release lease");

        let auth_unleased = coordinator
            .resolve_domain_authority("crates/strata-memory")
            .unwrap()
            .unwrap();
        assert!(!auth_unleased.lease_held);
    }
}

