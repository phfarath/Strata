use strata_core::events::{Event, EventPayload};
use strata_core::schemas::{ProceduralSkill, ProceduralStep};

/// Trajectory miner that extracts reusable procedural skills from execution sequences without an LLM.
pub struct TrajectoryMiner;

impl Default for TrajectoryMiner {
    fn default() -> Self {
        Self::new()
    }
}

impl TrajectoryMiner {
    pub fn new() -> Self {
        Self
    }

    /// Mines recovery trajectories: Error -> Corrective Action(s) -> Success.
    pub fn mine_recovery_trajectories(&self, events: &[Event]) -> Vec<ProceduralSkill> {
        let mut skills = Vec::new();
        let mut i = 0;

        while i < events.len() {
            // Check if events[i] is an error
            let error_info = match &events[i].payload {
                EventPayload::ErrorObserved(err) => {
                    Some((err.error_type.clone(), err.message.clone()))
                }
                EventPayload::ToolResultReceived(res) if res.is_error => Some((
                    format!("ToolExecutionError({})", res.tool_name),
                    format!("{:?}", res.result),
                )),
                _ => None,
            };

            if let Some((error_type, error_msg)) = error_info {
                // Scan forward for corrective tool invocations ending in success
                let mut corrective_steps = Vec::new();
                let mut found_success = false;
                let mut j = i + 1;

                while j < events.len() && (j - i) <= 10 {
                    match &events[j].payload {
                        EventPayload::ToolInvoked(inv) => {
                            let step_num = (corrective_steps.len() + 1) as u32;
                            corrective_steps.push(ProceduralStep::new(
                                step_num,
                                &inv.tool_name,
                                "execute",
                                inv.input.clone(),
                            ));
                        }
                        EventPayload::ToolResultReceived(res) if !res.is_error => {
                            if !corrective_steps.is_empty() {
                                found_success = true;
                                break;
                            }
                        }
                        EventPayload::TaskCompleted(task) if task.success => {
                            if !corrective_steps.is_empty() {
                                found_success = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }

                if found_success && !corrective_steps.is_empty() {
                    let sanitized_name = error_type.replace(|c: char| !c.is_alphanumeric(), "_");
                    let name = format!("Recover_{sanitized_name}");
                    let description = format!(
                        "Deterministic recovery procedure for error '{}': {}",
                        error_type, error_msg
                    );

                    let mut skill = ProceduralSkill::new(name, description)
                        .with_steps(corrective_steps)
                        .with_preconditions(vec![format!(
                            "Error pattern matches '{}'",
                            error_type
                        )]);
                    skill.importance = 0.85;
                    skill.tags = vec!["auto_mined".to_string(), "recovery".to_string(), error_type];

                    skills.push(skill);
                    i = j; // Advance pointer
                    continue;
                }
            }
            i += 1;
        }

        skills
    }

    /// Mines recurring ordered tool patterns (e.g., [read, edit, test]) that resulted in successful tasks.
    pub fn mine_successful_task_patterns(&self, events: &[Event]) -> Vec<ProceduralSkill> {
        let mut skills = Vec::new();
        let mut current_task_id: Option<String> = None;
        let mut current_task_title = String::new();
        let mut task_steps = Vec::new();

        for event in events {
            match &event.payload {
                EventPayload::TaskStarted(ts) => {
                    current_task_id = Some(ts.task_id.clone());
                    current_task_title = ts.title.clone();
                    task_steps.clear();
                }
                EventPayload::ToolInvoked(inv) => {
                    if current_task_id.is_some() {
                        let step_num = (task_steps.len() + 1) as u32;
                        task_steps.push(ProceduralStep::new(
                            step_num,
                            &inv.tool_name,
                            "execute",
                            inv.input.clone(),
                        ));
                    }
                }
                EventPayload::TaskCompleted(tc) if tc.success => {
                    if !task_steps.is_empty() {
                        let task_id = current_task_id.take().unwrap_or_else(|| "task".to_string());
                        let name = format!("Procedure_{}", task_id.replace('-', "_"));
                        let desc = format!(
                            "Workflow for '{}' ({})",
                            current_task_title, tc.outcome_summary
                        );
                        let mut skill = ProceduralSkill::new(name, desc)
                            .with_steps(task_steps.clone())
                            .with_preconditions(vec![format!(
                                "Task domain: {}",
                                current_task_title
                            )]);
                        skill.importance = 0.75;
                        skill.tags = vec!["workflow".to_string(), "task_pattern".to_string()];
                        skills.push(skill);
                    }
                    current_task_id = None;
                    task_steps.clear();
                }
                EventPayload::TaskCompleted(_) => {
                    current_task_id = None;
                    task_steps.clear();
                }
                _ => {}
            }
        }

        skills
    }
}
