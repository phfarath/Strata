use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use strata_core::events::{Event, EventPayload};
use strata_core::schemas::{EpisodeRecord, ProceduralSkill, ProceduralStep};
use strata_reasoning::{CompiledMacro, MacroParameter, MacroStep};

/// A recurring sequential pattern of tool invocations mined from high-salience episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrequentToolSequence {
    pub tools: Vec<String>,
    pub occurrence_count: usize,
    pub average_salience: f32,
    pub sample_episodes: Vec<Uuid>,
    pub sample_step_inputs: Vec<Vec<serde_json::Value>>,
}

/// Macro Generalizer and Synthesizer (Phase 4.3).
///
/// Induces abstract template slots (e.g. `{{target_path}}`, `{{command}}`) from concrete execution
/// arguments, synthesizing executable multi-step procedural macros.
pub struct MacroSynthesizer;

impl MacroSynthesizer {
    /// Generalizes a recurring tool sequence into an executable, parameterized CompiledMacro.
    pub fn generalize(seq: &FrequentToolSequence) -> CompiledMacro {
        let macro_name = format!("Macro_{}", seq.tools.join("_"));
        let description = format!(
            "Autonomous compiled macro for tool sequence [{}] (mined across {} episodes, avg salience {:.2})",
            seq.tools.join(" -> "),
            seq.occurrence_count,
            seq.average_salience
        );

        let mut parameters = Vec::new();
        let mut steps = Vec::new();

        // 1. Analyze inputs across step positions to detect parameters
        for (step_idx, tool_name) in seq.tools.iter().enumerate() {
            let mut template_obj = serde_json::Map::new();

            // Collect all inputs seen at this step across samples
            let inputs_at_step: Vec<&serde_json::Value> = seq
                .sample_step_inputs
                .iter()
                .filter_map(|inputs| inputs.get(step_idx))
                .collect();

            if let Some(first_input) = inputs_at_step.first() {
                if let Some(map) = first_input.as_object() {
                    for (k, v) in map {
                        let param_name = format!("{}_{}", tool_name, k);

                        // Detect if variable is dynamic across episodes or is a known path/command/target
                        let is_dynamic_slot = k == "path"
                            || k == "file"
                            || k == "target"
                            || k == "cmd"
                            || k == "command"
                            || k == "symbol"
                            || k == "query"
                            || inputs_at_step.iter().any(|other| other.get(k) != Some(v));

                        if is_dynamic_slot {
                            let placeholder = format!("{{{{{param_name}}}}}");
                            template_obj.insert(k.clone(), serde_json::Value::String(placeholder));

                            if !parameters.iter().any(|p: &MacroParameter| p.name == param_name) {
                                parameters.push(MacroParameter::new(
                                    param_name,
                                    format!("Dynamic argument for step {} ({}.{})", step_idx + 1, tool_name, k),
                                    true,
                                ).with_default(v.clone()));
                            }
                        } else {
                            template_obj.insert(k.clone(), v.clone());
                        }
                    }
                } else {
                    template_obj.insert("input".to_string(), (*first_input).clone());
                }
            }

            steps.push(MacroStep::new(
                (step_idx + 1) as u32,
                tool_name.clone(),
                "execute",
                serde_json::Value::Object(template_obj),
            ));
        }

        let mut compiled = CompiledMacro::new(macro_name, description)
            .with_steps(steps)
            .with_parameters(parameters)
            .with_preconditions(vec![format!("Tool dependencies available: {}", seq.tools.join(", "))])
            .with_postconditions(vec![format!("Sequence completed: {}", seq.tools.join(" -> "))]);

        compiled.support_count = seq.occurrence_count;
        compiled.success_rate = (seq.average_salience).clamp(0.5, 1.0);
        compiled.created_at = Utc::now();
        compiled
    }
}

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

    /// Mines recurring tool sequences across episodes with high salience (score >= min_salience).
    pub fn mine_frequent_tool_sequences(
        &self,
        episodes: &[EpisodeRecord],
        min_support: usize,
        min_salience: f32,
    ) -> Vec<FrequentToolSequence> {
        let mut sequence_instances: HashMap<Vec<String>, (Vec<Uuid>, Vec<f32>, Vec<Vec<serde_json::Value>>)> =
            HashMap::new();

        for ep in episodes {
            if ep.salience_score < min_salience || ep.event_trace.is_empty() {
                continue;
            }

            // Extract tool sequence and inputs from event trace
            let trace: Vec<(String, serde_json::Value)> = ep
                .event_trace
                .iter()
                .filter_map(|step| {
                    step.tool.as_ref().map(|t| {
                        (t.clone(), step.input.clone().unwrap_or(serde_json::json!({})))
                    })
                })
                .collect();

            if trace.len() < 2 {
                continue;
            }

            // Mine n-grams from length 2 to 4
            let max_n = trace.len().min(4);
            for n in 2..=max_n {
                for i in 0..=trace.len() - n {
                    let window = &trace[i..i + n];
                    let tool_tuple: Vec<String> = window.iter().map(|(t, _)| t.clone()).collect();
                    let step_inputs: Vec<serde_json::Value> =
                        window.iter().map(|(_, inp)| inp.clone()).collect();

                    let entry = sequence_instances
                        .entry(tool_tuple)
                        .or_insert_with(|| (Vec::new(), Vec::new(), Vec::new()));

                    if !entry.0.contains(&ep.id) {
                        entry.0.push(ep.id);
                        entry.1.push(ep.salience_score);
                        entry.2.push(step_inputs);
                    }
                }
            }
        }

        // Filter by min_support and sort by occurrence count
        let mut results: Vec<FrequentToolSequence> = sequence_instances
            .into_iter()
            .filter(|(_, (eps, _, _))| eps.len() >= min_support)
            .map(|(tools, (eps, scores, sample_inputs))| {
                let occurrence_count = eps.len();
                let average_salience = scores.iter().sum::<f32>() / scores.len() as f32;
                FrequentToolSequence {
                    tools,
                    occurrence_count,
                    average_salience,
                    sample_episodes: eps,
                    sample_step_inputs: sample_inputs,
                }
            })
            .collect();

        results.sort_by(|a, b| b.occurrence_count.cmp(&a.occurrence_count));
        results
    }

    /// Autonomous Procedural Skill Mining & Macro Synthesis (Phase 4.3).
    ///
    /// Scans high-salience episodes, discovers recurring tool sequences, and synthesizes
    /// parameterized executable macros.
    pub fn mine_and_compile_macros(
        &self,
        episodes: &[EpisodeRecord],
        min_support: usize,
        min_salience: f32,
    ) -> Vec<CompiledMacro> {
        let frequent_seqs = self.mine_frequent_tool_sequences(episodes, min_support, min_salience);
        frequent_seqs
            .iter()
            .map(MacroSynthesizer::generalize)
            .collect()
    }

    /// Mines recovery trajectories: Error -> Corrective Action(s) -> Success.
    pub fn mine_recovery_trajectories(&self, events: &[Event]) -> Vec<ProceduralSkill> {
        let mut skills = Vec::new();
        let mut i = 0;

        while i < events.len() {
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
                    i = j;
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
