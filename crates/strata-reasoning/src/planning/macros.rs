use anyhow::Result;
use serde::{Deserialize, Serialize};
use strata_core::schemas::ProceduralSkill;

use super::dag::GoalDag;
use super::types::{GoalNode, GoalNodeKind, GoalStatus};

/// Detailed record of a single procedural macro substitution in the Goal DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroSubstitution {
    pub original_node_id: String,
    pub original_title: String,
    pub skill_name: String,
    pub skill_id: uuid::Uuid,
    pub generated_step_ids: Vec<String>,
    pub steps_count: usize,
    pub success_rate: f32,
}

/// Compilation report summarizing macro substitutions executed across a Goal DAG.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MacroCompilationReport {
    pub substitutions: Vec<MacroSubstitution>,
    pub total_steps_compiled: usize,
    pub replanning_bypassed: bool,
}

impl MacroCompilationReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_substitutions(&self) -> bool {
        !self.substitutions.is_empty()
    }
}

/// Deterministic compiler that matches Goal DAG subgoals against learned procedural skills,
/// substituting high-level goals with pre-verified action macros without LLM re-planning.
#[derive(Debug, Clone)]
pub struct ProceduralMacroCompiler {
    skills: Vec<ProceduralSkill>,
    min_success_rate: f32,
    auto_inject_verification: bool,
}

impl Default for ProceduralMacroCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProceduralMacroCompiler {
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            min_success_rate: 0.70,
            auto_inject_verification: true,
        }
    }

    pub fn with_skills(mut self, skills: Vec<ProceduralSkill>) -> Self {
        self.skills = skills;
        self
    }

    pub fn skills(&self) -> &[ProceduralSkill] {
        &self.skills
    }

    /// Matches a goal node against the procedural skill registry using precondition and invariant matching.
    pub fn find_best_macro_for_goal(&self, goal: &GoalNode) -> Option<&ProceduralSkill> {
        // Skip root nodes or verification gates unless explicitly targeted
        if matches!(
            goal.kind,
            GoalNodeKind::Root | GoalNodeKind::Verification | GoalNodeKind::Rollback
        ) {
            return None;
        }

        let goal_title_lower = goal.title.to_lowercase();
        let goal_desc_lower = goal.description.to_lowercase();
        let goal_action = goal.action.as_deref().unwrap_or("").to_lowercase();

        let mut best_match: Option<(&ProceduralSkill, f32)> = None;

        for skill in &self.skills {
            // Must meet minimum historical success rate
            if skill.success_rate < self.min_success_rate || skill.steps.is_empty() {
                continue;
            }

            let mut match_score = 0.0f32;
            let skill_name_clean = skill.name.replace('_', " ").to_lowercase();
            let skill_desc_clean = skill.description.to_lowercase();

            // 1. Precondition matching: check if goal title or metadata matches declared task domain
            for pre in &skill.preconditions {
                let pre_lower = pre.to_lowercase();
                if pre_lower.starts_with("task domain:") {
                    let domain = pre_lower.trim_start_matches("task domain:").trim();
                    if goal_title_lower.contains(domain) || domain.contains(&goal_title_lower) {
                        match_score += 0.8;
                    }
                } else if goal_title_lower.contains(&pre_lower)
                    || goal_desc_lower.contains(&pre_lower)
                {
                    match_score += 0.5;
                }
            }

            // 2. Direct name or description matching
            if goal_title_lower.contains(&skill_name_clean)
                || skill_name_clean.contains(&goal_title_lower)
            {
                match_score += 0.7;
            } else if goal_desc_lower.contains(&skill_name_clean)
                || skill_desc_clean.contains(&goal_title_lower)
            {
                match_score += 0.4;
            }

            // 3. Action match
            if !goal_action.is_empty()
                && skill
                    .steps
                    .iter()
                    .any(|s| s.tool.to_lowercase() == goal_action)
            {
                match_score += 0.3;
            }

            // 4. Tag overlap bonus
            for tag in &skill.tags {
                let tag_lower = tag.to_lowercase();
                if goal_title_lower.contains(&tag_lower) || goal_desc_lower.contains(&tag_lower) {
                    match_score += 0.2;
                }
            }

            // Quality weighting by success rate and importance
            let total_score = match_score * (skill.success_rate * 0.7 + skill.importance * 0.3);

            if total_score >= 0.5 {
                match best_match {
                    Some((_, prev_score)) if total_score > prev_score => {
                        best_match = Some((skill, total_score));
                    }
                    None => {
                        best_match = Some((skill, total_score));
                    }
                    _ => {}
                }
            }
        }

        best_match.map(|(s, _)| s)
    }

    /// Compiles all matching procedural macros into the Goal DAG in-place.
    ///
    /// Replaces matched subgoals with concrete sequential macro action steps,
    /// rewires incoming and outgoing dependency edges, and returns a detailed compilation report.
    pub fn compile_macros(&self, dag: &mut GoalDag) -> Result<MacroCompilationReport> {
        let mut report = MacroCompilationReport::new();

        // Identify candidate nodes to expand (only pending Task nodes)
        let candidate_nodes: Vec<GoalNode> = dag
            .all_nodes()
            .into_iter()
            .filter(|n| {
                n.status == GoalStatus::Pending
                    && matches!(n.kind, GoalNodeKind::Task | GoalNodeKind::Phase)
            })
            .cloned()
            .collect();

        for node in candidate_nodes {
            if let Some(skill) = self.find_best_macro_for_goal(&node) {
                let step_ids = dag.expand_macro(&node.id, skill, self.auto_inject_verification)?;
                if !step_ids.is_empty() {
                    let steps_len = step_ids.len();
                    report.substitutions.push(MacroSubstitution {
                        original_node_id: node.id.clone(),
                        original_title: node.title.clone(),
                        skill_name: skill.name.clone(),
                        skill_id: skill.id,
                        generated_step_ids: step_ids,
                        steps_count: steps_len,
                        success_rate: skill.success_rate,
                    });
                    report.total_steps_compiled += steps_len;
                    report.replanning_bypassed = true;
                }
            }
        }

        if report.has_substitutions() {
            dag.validate()?;
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::types::GoalEdge;
    use crate::planning::{DagScheduler, DynamicReplanner, RecoveryAction};
    use strata_core::schemas::ProceduralStep;

    fn sample_skill() -> ProceduralSkill {
        let mut skill = ProceduralSkill::new(
            "setup_database",
            "Creates database tables and indexes for auth",
        );
        skill.preconditions = vec!["Task domain: Database Setup".to_string()];
        skill.postconditions = vec!["Tables auth_users and sessions exist".to_string()];
        skill.success_rate = 0.95;
        skill.importance = 0.8;
        skill.tags = vec!["database".to_string(), "setup".to_string()];
        skill.steps = vec![
            ProceduralStep::new(
                1,
                "sqlite",
                "CREATE TABLE auth_users (id TEXT PRIMARY KEY, email TEXT);",
                serde_json::json!({ "mode": "write" }),
            ),
            ProceduralStep::new(
                2,
                "sqlite",
                "CREATE INDEX idx_auth_users_email ON auth_users(email);",
                serde_json::json!({ "mode": "write" }),
            ),
        ];
        skill
    }

    fn sample_recovery_skill() -> ProceduralSkill {
        let mut skill = ProceduralSkill::new(
            "Recover_DeadlockTimeout",
            "Mitigates SQLite deadlock by bumping busy timeout and retrying",
        );
        skill.preconditions = vec!["Error pattern matches 'DeadlockTimeout'".to_string()];
        skill.tags = vec![
            "recovery".to_string(),
            "auto_mined".to_string(),
            "DeadlockTimeout".to_string(),
        ];
        skill.success_rate = 0.90;
        skill.importance = 0.9;
        skill.steps = vec![
            ProceduralStep::new(
                1,
                "sqlite_pragma",
                "PRAGMA busy_timeout = 5000;",
                serde_json::json!({ "timeout": 5000 }),
            ),
            ProceduralStep::new(
                2,
                "transaction_retry",
                "Retry failed transaction block",
                serde_json::json!({ "retry": true }),
            ),
        ];
        skill
    }

    #[test]
    fn test_find_best_macro_for_goal_preconditions() {
        let compiler = ProceduralMacroCompiler::new().with_skills(vec![sample_skill()]);

        let matching_goal = GoalNode::task("goal-1", "Database Setup Task");
        let matched = compiler.find_best_macro_for_goal(&matching_goal);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "setup_database");

        let unrelated_goal = GoalNode::task("goal-2", "Send Welcome Email");
        let unmatch = compiler.find_best_macro_for_goal(&unrelated_goal);
        assert!(unmatch.is_none());
    }

    #[test]
    fn test_goal_dag_macro_expansion_and_topology() {
        let mut dag = GoalDag::new();
        let n1 = GoalNode::task("step_init", "Initialize environment");
        let n2 = GoalNode::task("step_db", "Database Setup Task");
        let n3 = GoalNode::task("step_server", "Start HTTP server");

        dag.add_node(n1);
        dag.add_node(n2);
        dag.add_node(n3);

        dag.add_edge("step_init", "step_db", GoalEdge::depends_on())
            .unwrap();
        dag.add_edge("step_db", "step_server", GoalEdge::depends_on())
            .unwrap();
        dag.validate().unwrap();

        let compiler = ProceduralMacroCompiler::new().with_skills(vec![sample_skill()]);
        let report = compiler.compile_macros(&mut dag).unwrap();

        assert!(report.has_substitutions());
        assert_eq!(report.substitutions.len(), 1);
        assert_eq!(report.substitutions[0].original_node_id, "step_db");
        assert_eq!(report.substitutions[0].skill_name, "setup_database");
        assert_eq!(report.substitutions[0].steps_count, 3); // 2 steps + 1 verify

        // Validate DAG topology after expansion
        dag.validate().unwrap();

        // Node step_db should be marked skipped
        assert_eq!(dag.get_node("step_db").unwrap().status, GoalStatus::Skipped);

        // Compute execution waves
        let waves = dag.compute_waves().unwrap();
        assert!(!waves.is_empty());

        // Check that initial node executes before macro steps, and macro steps execute before server
        let prereqs_server = dag.get_prerequisites("step_server");
        assert!(prereqs_server.iter().any(|p| p.contains("macro_verify")));
    }

    #[tokio::test]
    async fn test_scheduler_macro_compilation_and_execution() {
        let mut dag = GoalDag::new();
        let task = GoalNode::task("db_setup_goal", "Database Setup Task");
        dag.add_node(task);

        let compiler = ProceduralMacroCompiler::new().with_skills(vec![sample_skill()]);
        let scheduler = DagScheduler::new().with_macro_compiler(compiler);

        let (completed_dag, report) = scheduler.execute(dag).await.unwrap();
        assert_eq!(report.total_nodes, 4); // 1 skipped original + 2 macro tasks + 1 verify
        assert_eq!(report.failed_nodes, 0);

        // Verification node should be completed
        let verify_node = completed_dag
            .all_nodes()
            .into_iter()
            .find(|n| n.id.contains("macro_verify"));
        assert!(verify_node.is_some());
        assert_eq!(verify_node.unwrap().status, GoalStatus::Completed);
    }

    #[test]
    fn test_dynamic_replanner_mined_recovery_routine() {
        let mut dag = GoalDag::new();
        let mut failed_task = GoalNode::task("task_write", "Write transaction batch");
        failed_task.error = Some("DeadlockTimeout: SQLite writer queue saturated".to_string());
        failed_task.retry_count = failed_task.max_retries; // Transient retries exhausted
        dag.add_node(failed_task.clone());

        let replanner =
            DynamicReplanner::new().with_procedural_skills(vec![sample_recovery_skill()]);

        let action = replanner.handle_failure(&dag, &failed_task).unwrap();
        match action {
            RecoveryAction::InjectMitigation {
                failed_node_id,
                mitigation_nodes,
                edges,
            } => {
                assert_eq!(failed_node_id, "task_write");
                assert_eq!(mitigation_nodes.len(), 3); // 2 recovery steps + 1 reverify
                assert_eq!(edges.len(), 2);

                // Apply recovery patch
                replanner
                    .apply_recovery(
                        &mut dag,
                        &RecoveryAction::InjectMitigation {
                            failed_node_id,
                            mitigation_nodes,
                            edges,
                        },
                    )
                    .unwrap();

                // DAG must remain valid and acyclic
                dag.validate().unwrap();
                let waves = dag.compute_waves().unwrap();
                assert_eq!(waves.len(), 3); // step 1 -> step 2 -> reverify
            }
            other => panic!("Expected InjectMitigation, got {:?}", other),
        }
    }
}
