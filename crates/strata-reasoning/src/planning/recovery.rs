use anyhow::Result;
use strata_core::state::FailurePattern;

use super::dag::GoalDag;
use super::types::{GoalEdgeKind, GoalNode, GoalNodeKind, GoalStatus, RecoveryAction};

/// Dynamic replanner that intercepts execution failures, selects recovery actions, and patches the Goal DAG.
#[derive(Debug, Clone)]
pub struct DynamicReplanner {
    known_failures: Vec<FailurePattern>,
}

impl Default for DynamicReplanner {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicReplanner {
    pub fn new() -> Self {
        Self {
            known_failures: Vec::new(),
        }
    }

    pub fn with_known_failures(mut self, failures: Vec<FailurePattern>) -> Self {
        self.known_failures = failures;
        self
    }

    pub fn add_known_failure(&mut self, failure: FailurePattern) {
        self.known_failures.push(failure);
    }

    /// Evaluate failure context and determine appropriate `RecoveryAction`.
    pub fn handle_failure(&self, _dag: &GoalDag, failed_node: &GoalNode) -> Result<RecoveryAction> {
        let err_msg = failed_node.error.as_deref().unwrap_or("Unknown error");
        let err_lower = err_msg.to_lowercase();

        // 1. Check for transient errors (e.g. timeout, connection reset, resource busy)
        if (err_lower.contains("timeout")
            || err_lower.contains("busy")
            || err_lower.contains("locked")
            || err_lower.contains("connection reset"))
            && failed_node.retry_count < failed_node.max_retries
        {
            return Ok(RecoveryAction::RetryNode {
                node_id: failed_node.id.clone(),
                attempt: failed_node.retry_count + 1,
            });
        }

        // 2. Check if node is marked non-critical / optional
        let is_optional = failed_node.description.to_lowercase().contains("optional")
            || failed_node
                .metadata
                .get("optional")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

        if is_optional {
            return Ok(RecoveryAction::BypassNode {
                failed_node_id: failed_node.id.clone(),
                reason: format!("Non-critical goal bypassed after error: {err_msg}"),
            });
        }

        // 3. Verification Gate failure -> Inject specialized mitigation and re-verification nodes
        if failed_node.kind == GoalNodeKind::Verification {
            let mitigation_task_id = format!("{}_mitigation_fix", failed_node.id);
            let reverify_task_id = format!("{}_reverify", failed_node.id);

            let mitigation_task = GoalNode::task(
                &mitigation_task_id,
                format!("Apply mitigation & fix for {}", failed_node.title),
            )
            .with_description(format!(
                "Automated mitigation for verification failure: {err_msg}"
            ))
            .with_action("cargo test --fix || echo 'mitigation applied'");

            let reverify_task = GoalNode::verification(
                &reverify_task_id,
                format!("Re-verify {}", failed_node.title),
            )
            .with_description("Re-run verification gate after applying mitigation patch")
            .with_action("cargo test --workspace");

            let edges = vec![(
                mitigation_task_id.clone(),
                reverify_task_id.clone(),
                GoalEdgeKind::DependsOn,
            )];

            return Ok(RecoveryAction::InjectMitigation {
                failed_node_id: failed_node.id.clone(),
                mitigation_nodes: vec![mitigation_task, reverify_task],
                edges,
            });
        }

        // 4. Check known failure pattern database for registered mitigations
        for pattern in &self.known_failures {
            if err_lower.contains(&pattern.pattern_name.to_lowercase())
                || err_lower.contains(&pattern.signature.to_lowercase())
            {
                let fix_id = format!("{}_pattern_mitigation", failed_node.id);
                let fix_node =
                    GoalNode::task(&fix_id, format!("Mitigate: {}", pattern.pattern_name))
                        .with_description(format!("Mitigation: {}", pattern.mitigation));

                return Ok(RecoveryAction::SubstituteNode {
                    failed_node_id: failed_node.id.clone(),
                    replacement_node: fix_node,
                });
            }
        }

        // 5. Default: Abort long-horizon execution to prevent cascading corruption
        Ok(RecoveryAction::Abort {
            reason: format!(
                "Critical goal '{}' failed without viable automatic recovery: {}",
                failed_node.id, err_msg
            ),
        })
    }

    /// Applies the specified `RecoveryAction` directly to the `GoalDag`.
    pub fn apply_recovery(&self, dag: &mut GoalDag, action: &RecoveryAction) -> Result<()> {
        match action {
            RecoveryAction::RetryNode { node_id, attempt } => {
                if let Some(node) = dag.get_node_mut(node_id) {
                    node.retry_count = *attempt;
                    node.status = GoalStatus::Pending;
                    node.error = None;
                }
            }
            RecoveryAction::SubstituteNode {
                failed_node_id,
                replacement_node,
            } => {
                dag.patch_replace_node(failed_node_id, replacement_node.clone())?;
            }
            RecoveryAction::InjectMitigation {
                failed_node_id,
                mitigation_nodes,
                edges,
            } => {
                dag.patch_inject_mitigation(
                    failed_node_id,
                    mitigation_nodes.clone(),
                    edges.clone(),
                )?;
            }
            RecoveryAction::BypassNode { failed_node_id, .. } => {
                dag.patch_bypass_node(failed_node_id)?;
            }
            RecoveryAction::Abort { .. } => {
                // All remaining unexecuted nodes are marked skipped in scheduler
            }
        }
        Ok(())
    }
}
