use std::sync::Arc;
use strata_core::errors::StrataError;
use strata_core::events::{Event, EventPayload};
use strata_core::schemas::{
    ExportFormat, KtoSample, PreferencePair, SftSample, SignalKind,
};
use crate::store::SqliteStore;

/// Extracts alignment datasets (DPO pairs, KTO binary feedback, SFT instruction data)
/// from persistent memory, failure patterns, episodic signals, and procedural skills.
pub struct PreferenceMiner {
    store: Arc<SqliteStore>,
}

impl PreferenceMiner {
    /// Create a new `PreferenceMiner` backed by the given `SqliteStore`.
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }

    /// Mine DPO preference pairs (prompt, chosen, rejected) matching failure patterns,
    /// anti-patterns with mitigations, and successful vs failed episodic trajectories.
    pub fn mine_dpo_pairs(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<PreferencePair>, StrataError> {
        let mut pairs = self.store.get_preference_pairs(session_id)?;

        // 1. Mine pairs from Failure Patterns (Anti-pattern -> Rejected vs Mitigation -> Chosen)
        let failures = self.store.search_failures(None, None, 100)?;
        for failure in failures {
            let prompt = format!(
                "Context: Agent encountered obstacle.\nProblem / Trigger: {}\nFailure Signature: {}",
                failure.trigger_condition, failure.signature
            );
            let chosen = format!(
                "Mitigation Strategy:\n{}\nDetails: {}",
                failure.mitigation, failure.description
            );
            let rejected = format!(
                "Anti-pattern approach leading to error '{}': {}",
                failure.error_type, failure.description
            );
            let source_sid = session_id.unwrap_or("failure-pattern").to_string();
            pairs.push(PreferencePair::new(prompt, chosen, rejected, source_sid));
        }

        // 2. Mine pairs from Episodic Memories (high success vs failed episodes)
        let episodes = if let Some(sid) = session_id {
            self.store.get_episodic_memories_by_session(sid)?
        } else {
            self.store.get_all_episodic_memories(None, 100)?
        };

        for ep in episodes {
            if ep.signals.success >= 0.7 && !ep.outcomes.is_empty() && !ep.obstacles.is_empty() {
                let prompt = if !ep.goals.is_empty() {
                    format!("Goal: {}", ep.goals.join(", "))
                } else {
                    format!("Task Summary: {}", ep.summary)
                };
                let chosen = format!("Resolved via: {}", ep.outcomes.join("; "));
                let rejected = format!("Stuck at obstacle: {}", ep.obstacles.join("; "));
                pairs.push(PreferencePair::new(prompt, chosen, rejected, ep.session_id));
            }
        }

        // 3. Implicit signals from Event Stream (ToolLoop & CommandFix)
        let events = if let Some(sid) = session_id {
            self.store.get_events(sid, None, Some(5000))?
        } else {
            self.store.get_all_events()?
        };
        pairs.extend(self.mine_implicit_signals_from_events(&events));

        // 4. Explicit feedback pairs (Memory records with negative rating vs positive corrections)
        let feedback_events = self.store.get_feedback_events(session_id)?;
        for fb in feedback_events {
            if fb.rating == strata_core::schemas::FeedbackRating::Negative {
                if let (Some(mem_id), Some(comment)) = (fb.memory_id, fb.comment) {
                    if let Ok(Some(mem)) = self.store.get_memory(&mem_id) {
                        let prompt = mem.summary.clone().unwrap_or_else(|| "Memory context retrieval".to_string());
                        let rejected = mem.content.clone();
                        let chosen = comment;
                        let sid = session_id.unwrap_or("feedback").to_string();
                        pairs.push(PreferencePair::new(prompt, chosen, rejected, sid));
                    }
                }
            }
        }

        Ok(pairs)
    }

    /// Extract implicit ToolLoop and CommandFix pairs from an ordered sequence of events.
    pub fn mine_implicit_signals_from_events(&self, events: &[Event]) -> Vec<PreferencePair> {
        let mut pairs = Vec::new();
        if events.is_empty() {
            return pairs;
        }

        let mut session_map: std::collections::HashMap<String, Vec<&Event>> =
            std::collections::HashMap::new();
        for ev in events {
            session_map.entry(ev.session_id.clone()).or_default().push(ev);
        }

        for (sid, s_events) in session_map {
            let mut last_tool_inv: Option<(String, String)> = None;
            let mut failed_tool_inv: Option<(String, String, String)> = None;
            let mut current_task: Option<String> = None;

            for ev in s_events {
                match &ev.payload {
                    EventPayload::TaskStarted(t) => {
                        current_task = Some(format!("{}: {}", t.title, t.description.as_deref().unwrap_or("")));
                    }
                    EventPayload::ToolInvoked(inv) => {
                        let cmd = inv.input.get("command")
                            .and_then(|c| c.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| inv.input.to_string());
                        last_tool_inv = Some((inv.tool_name.clone(), cmd));
                    }
                    EventPayload::ToolResultReceived(res) => {
                        if res.is_error {
                            let err_str = res.result.to_string();
                            if let Some((tool_name, input_cmd)) = last_tool_inv.take() {
                                failed_tool_inv = Some((tool_name, input_cmd, err_str));
                            }
                        } else if let Some((f_tool, f_input, f_err)) = failed_tool_inv.take() {
                            let (c_tool, c_input) = last_tool_inv.take().unwrap_or_else(|| (res.tool_name.clone(), String::new()));
                            let prompt = current_task.clone().unwrap_or_else(|| {
                                format!("Resolve tool execution failure in session '{sid}'")
                            });
                            let rejected = format!(
                                "Failing Execution (Tool: {}):\nCommand / Input: {}\nError:\n{}",
                                f_tool, f_input, f_err
                            );
                            let chosen = format!(
                                "Corrected Execution (Tool: {}):\nCommand / Input: {}\nResult:\n{}",
                                c_tool, c_input, res.result
                            );

                            pairs.push(PreferencePair::new(prompt, chosen, rejected, &sid));
                        }
                    }
                    EventPayload::ErrorObserved(err) => {
                        if let Some((tool_name, input_cmd, _)) = failed_tool_inv.take() {
                            failed_tool_inv = Some((tool_name, input_cmd, err.message.clone()));
                        }
                    }
                    EventPayload::TaskCompleted(task) => {
                        if task.success {
                            if let Some((f_tool, f_input, f_err)) = failed_tool_inv.take() {
                                let prompt = format!("Task '{}' resolution", task.task_id);
                                let rejected = format!("Failing trajectory (Tool: {}):\nInput: {}\nError: {}", f_tool, f_input, f_err);
                                let chosen = format!("Successful resolution: {}", task.outcome_summary);

                                pairs.push(PreferencePair::new(prompt, chosen, rejected, &sid));
                            }
                        } else {
                            failed_tool_inv = None;
                        }
                    }
                    _ => {}
                }
            }
        }

        pairs
    }

    /// Mine KTO samples (prompt, completion, label: bool) from positive/negative feedback,
    /// episodic outcome signals, and implicit behavioral telemetry.
    pub fn mine_kto_samples(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<KtoSample>, StrataError> {
        let mut samples = Vec::new();

        // 1. From mined preference pairs (chosen = true, rejected = false)
        let pairs = self.mine_dpo_pairs(session_id)?;
        for pair in pairs {
            samples.push(KtoSample::new(
                &pair.prompt,
                &pair.chosen,
                true,
                &pair.source_session_id,
            ));
            samples.push(KtoSample::new(
                &pair.prompt,
                &pair.rejected,
                false,
                &pair.source_session_id,
            ));
        }

        // 2. From Episodic memories
        let episodes = if let Some(sid) = session_id {
            self.store.get_episodic_memories_by_session(sid)?
        } else {
            self.store.get_all_episodic_memories(None, 100)?
        };

        for ep in episodes {
            let prompt = if !ep.goals.is_empty() {
                format!("Goal: {}", ep.goals.join(", "))
            } else {
                format!("Task Summary: {}", ep.summary)
            };

            if ep.signals.success >= 0.6 && !ep.outcomes.is_empty() {
                samples.push(KtoSample::new(
                    &prompt,
                    &ep.outcomes.join("; "),
                    true,
                    &ep.session_id,
                ));
            }

            if (ep.signals.success < 0.4 || ep.signals.frustration >= 0.6) && !ep.obstacles.is_empty() {
                samples.push(KtoSample::new(
                    &prompt,
                    &ep.obstacles.join("; "),
                    false,
                    &ep.session_id,
                ));
            }
        }

        // 3. From Implicit Signals
        let signals = self.store.get_implicit_signals(session_id)?;
        for sig in signals {
            let prompt = format!(
                "Action in session {} by agent {} (tool: {:?}, file: {:?})",
                sig.session_id, sig.agent_id, sig.tool_name, sig.file_path
            );
            match sig.kind {
                SignalKind::TestRerunSuccess | SignalKind::ExplicitRating => {
                    let completion = sig
                        .extra
                        .unwrap_or_else(|| "Successful resolution / verified test run".to_string());
                    samples.push(KtoSample::new(&prompt, &completion, true, &sig.session_id));
                }
                SignalKind::ToolLoop | SignalKind::GitRevert | SignalKind::TestRerunFail => {
                    let completion = sig
                        .extra
                        .unwrap_or_else(|| "Failed execution / tool loop repetition".to_string());
                    samples.push(KtoSample::new(&prompt, &completion, false, &sig.session_id));
                }
                _ => {}
            }
        }

        Ok(samples)
    }

    /// Turn procedural skills and examples into SFT format `(instruction, input, output)`.
    pub fn mine_sft_samples(&self) -> Result<Vec<SftSample>, StrataError> {
        let skills = self.store.get_all_procedural_skills(None, 100)?;
        let mut samples = Vec::new();

        for skill in skills {
            let instruction = format!("Execute skill: {} - {}", skill.name, skill.description);
            let input = format!(
                "Preconditions: {}\nParameters: {}",
                skill.preconditions.join(", "),
                serde_json::to_string(&skill.parameters).unwrap_or_default()
            );

            let steps_str = skill
                .steps
                .iter()
                .map(|s| format!("{}. [{}] {}: {:?}", s.order, s.tool, s.action, s.expected_result))
                .collect::<Vec<_>>()
                .join("\n");

            let source_sid = skill.project.unwrap_or_else(|| "global".to_string());

            samples.push(SftSample::new(&instruction, &input, &steps_str, &source_sid));

            // Also mine execution examples
            for ex in skill.examples {
                let ex_instruction = format!("Perform skill execution: {}", skill.name);
                let ex_input = format!("Session context: {}", ex.session_id);
                samples.push(SftSample::new(
                    &ex_instruction,
                    &ex_input,
                    &ex.outcome,
                    &ex.session_id,
                ));
            }
        }

        Ok(samples)
    }

    /// Export mined alignment dataset in specified format (DPO, KTO, SFT, Markdown, or JSONL).
    pub fn export(&self, format: ExportFormat, scope: Option<&str>) -> Result<String, StrataError> {
        match format {
            ExportFormat::Dpo => {
                let pairs = self.mine_dpo_pairs(scope)?;
                let mut lines = Vec::new();
                for p in pairs {
                    lines.push(serde_json::to_string(&p)?);
                }
                Ok(lines.join("\n"))
            }
            ExportFormat::Kto => {
                let samples = self.mine_kto_samples(scope)?;
                let mut lines = Vec::new();
                for s in samples {
                    lines.push(serde_json::to_string(&s)?);
                }
                Ok(lines.join("\n"))
            }
            ExportFormat::Sft => {
                let samples = self.mine_sft_samples()?;
                let mut lines = Vec::new();
                for s in samples {
                    lines.push(serde_json::to_string(&s)?);
                }
                Ok(lines.join("\n"))
            }
            ExportFormat::Jsonl => {
                let pairs = self.mine_dpo_pairs(scope)?;
                let sft = self.mine_sft_samples()?;
                let mut lines = Vec::new();
                for p in pairs {
                    lines.push(serde_json::to_string(&p)?);
                }
                for s in sft {
                    lines.push(serde_json::to_string(&s)?);
                }
                Ok(lines.join("\n"))
            }
            ExportFormat::Markdown => {
                let pairs = self.mine_dpo_pairs(scope)?;
                let kto = self.mine_kto_samples(scope)?;
                let sft = self.mine_sft_samples()?;

                let mut md = String::new();
                md.push_str("# Strata Alignment & Preference Dataset\n\n");
                md.push_str(&format!("- Total DPO Pairs: {}\n", pairs.len()));
                md.push_str(&format!("- Total KTO Samples: {}\n", kto.len()));
                md.push_str(&format!("- Total SFT Samples: {}\n\n", sft.len()));

                if !pairs.is_empty() {
                    md.push_str("## DPO Preference Pairs\n\n");
                    for (i, p) in pairs.iter().enumerate() {
                        md.push_str(&format!("### Pair {}\n", i + 1));
                        md.push_str(&format!("**Prompt**:\n```\n{}\n```\n\n", p.prompt));
                        md.push_str(&format!("**Chosen**:\n```\n{}\n```\n\n", p.chosen));
                        md.push_str(&format!("**Rejected**:\n```\n{}\n```\n\n", p.rejected));
                    }
                }

                if !sft.is_empty() {
                    md.push_str("## SFT Demonstrations\n\n");
                    for (i, s) in sft.iter().enumerate() {
                        md.push_str(&format!("### Sample {}\n", i + 1));
                        md.push_str(&format!("**Instruction**: {}\n\n", s.instruction));
                        md.push_str(&format!("**Input**:\n```\n{}\n```\n\n", s.input));
                        md.push_str(&format!("**Output**:\n```\n{}\n```\n\n", s.output));
                    }
                }

                Ok(md)
            }
        }
    }
}
