use crate::store::SqliteStore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use strata_core::a2a::DelegationStatus;
use strata_core::errors::StrataError;
use strata_core::events::{Event, EventPayload};
use strata_core::schemas::{ExportFormat, KtoSample, PreferencePair, SftSample, SignalKind};
use strata_core::state::MemoryTier;

/// Detailed summary report of mined alignment dataset statistics across various memory subsystems.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AlignmentDatasetReport {
    pub total_dpo_pairs: usize,
    pub oracle_verified_dpo_pairs: usize,
    pub delegation_dpo_pairs: usize,
    pub hitl_approval_dpo_pairs: usize,
    pub failure_pattern_dpo_pairs: usize,
    pub episodic_dpo_pairs: usize,
    pub implicit_signal_dpo_pairs: usize,
    pub total_kto_samples: usize,
    pub oracle_verified_kto_samples: usize,
    pub delegation_kto_samples: usize,
    pub hitl_approval_kto_samples: usize,
    pub total_sft_samples: usize,
    pub oracle_verified_sft_samples: usize,
}

/// Extracts alignment datasets (DPO pairs, KTO binary feedback, SFT instruction data)
/// from persistent memory, failure patterns, episodic signals, procedural skills,
/// A2A delegations, and HITL/council approved core memories, with support for oracle-verified gating.
pub struct PreferenceMiner {
    store: Arc<SqliteStore>,
}

impl PreferenceMiner {
    /// Create a new `PreferenceMiner` backed by the given `SqliteStore`.
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }

    /// Mine DPO preference pairs (prompt, chosen, rejected) matching failure patterns,
    /// anti-patterns with mitigations, successful vs failed episodic trajectories,
    /// verified A2A task delegations, and HITL/council approved core memories.
    pub fn mine_dpo_pairs(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<PreferencePair>, StrataError> {
        let mut pairs = self.store.get_preference_pairs(session_id)?;

        // 1. Mine pairs from Failure Patterns (Anti-pattern -> Rejected vs Mitigation -> Chosen)
        let failures = self.store.search_failures(None, None, 10_000)?;
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
            let is_verified = !failure.mitigation.trim().is_empty();
            let v_source = if is_verified {
                Some("failure_pattern_mitigation".to_string())
            } else {
                None
            };
            pairs.push(
                PreferencePair::new(prompt, chosen, rejected, source_sid)
                    .with_verification(is_verified, v_source),
            );
        }

        // 2. Mine pairs from Episodic Memories (high success vs failed episodes)
        let episodes = if let Some(sid) = session_id {
            self.store.get_episodic_memories_by_session(sid)?
        } else {
            self.store.get_all_episodic_memories(None, 10_000)?
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
                let is_verified = ep.signals.success >= 0.8;
                let v_source = if is_verified {
                    Some("episodic_high_success".to_string())
                } else {
                    None
                };
                pairs.push(
                    PreferencePair::new(prompt, chosen, rejected, ep.session_id)
                        .with_verification(is_verified, v_source),
                );
            }
        }

        // 3. Implicit signals from Event Stream (ToolLoop & CommandFix)
        let events = if let Some(sid) = session_id {
            self.store.get_events(sid, None, Some(10_000))?
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
                        let prompt = mem
                            .summary
                            .clone()
                            .unwrap_or_else(|| "Memory context retrieval".to_string());
                        let rejected = mem.content.clone();
                        let chosen = comment;
                        let sid = session_id.unwrap_or("feedback").to_string();
                        pairs.push(
                            PreferencePair::new(prompt, chosen, rejected, sid).with_verification(
                                true,
                                Some("human_feedback_verified".to_string()),
                            ),
                        );
                    }
                }
            }
        }

        // 5. Mine pairs from A2A Delegations (verified completed vs rejected)
        pairs.extend(self.mine_delegation_dpo_pairs(session_id)?);

        // 6. Mine pairs from HITL & Council Approved Core Memories
        pairs.extend(self.mine_hitl_approval_pairs(session_id)?);

        Ok(pairs)
    }

    /// Mine DPO preference pairs from completed vs failed A2A heterogeneous task delegations.
    ///
    /// Contracts completed with an explicit `verification_command` are treated as oracle-verified
    /// ground truth. Only explicitly rejected or timed-out contracts in the same domain scope serve as hard negative samples
    /// (in-flight Pending/Accepted tasks are never treated as failures).
    pub fn mine_delegation_dpo_pairs(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<PreferencePair>, StrataError> {
        let contracts = self.store.list_delegations(None, None, None)?;
        let mut pairs = Vec::new();

        let completed: Vec<_> = contracts
            .iter()
            .filter(|c| c.status == DelegationStatus::Completed)
            .cloned()
            .collect();

        // In-flight tasks (Pending, Accepted) must NOT be categorized as failed
        let failed: Vec<_> = contracts
            .iter()
            .filter(|c| {
                matches!(
                    c.status,
                    DelegationStatus::Rejected | DelegationStatus::TimedOut
                )
            })
            .cloned()
            .collect();

        for c in completed {
            if let Some(sid) = session_id {
                if c.task_id != sid && c.domain_scope.as_deref() != Some(sid) {
                    continue;
                }
            }

            let prompt = format!(
                "Task Delegation Objective: {}\nDomain Scope: {}\nTask ID: {}\nContext / Constraints: {}",
                c.objective,
                c.domain_scope.as_deref().unwrap_or("general"),
                c.task_id,
                c.context.as_deref().unwrap_or("")
            );

            let mut chosen_parts = Vec::new();
            chosen_parts.push("Status: Completed".to_string());
            if let Some(ref summary) = c.result_summary {
                chosen_parts.push(format!("Summary: {}", summary));
            }
            if !c.expected_artifacts.is_empty() {
                chosen_parts.push(format!("Artifacts: {}", c.expected_artifacts.join(", ")));
            }
            if let Some(ref cmd) = c.verification_command {
                chosen_parts.push(format!("Verification Command: {}", cmd));
            }
            let chosen = chosen_parts.join("\n");

            let rejected = if let Some(neg) =
                failed.iter().find(|f| f.domain_scope == c.domain_scope)
            {
                format!(
                    "Status: {}\nError / Obstacle: {}\nIncomplete Artifacts: {}",
                    neg.status.as_str(),
                    neg.error
                        .as_deref()
                        .unwrap_or("Task delegation failed without satisfying constraints"),
                    neg.expected_artifacts.join(", ")
                )
            } else if let Some(neg) = failed.first() {
                format!(
                    "Status: {}\nError / Obstacle: {}",
                    neg.status.as_str(),
                    neg.error.as_deref().unwrap_or("Task delegation failed")
                )
            } else {
                "Status: Failed\nError: Task execution timed out or failed verification criteria without producing verified artifacts.\nArtifacts: None".to_string()
            };

            let src_session = c.task_id.clone();
            // Oracle verification requires an actual verification_command; completed without verification is unverified
            let is_verified = c.verification_command.is_some();
            let v_source = c
                .verification_command
                .as_ref()
                .map(|cmd| format!("delegation_contract_oracle:{}", cmd))
                .or_else(|| Some("delegation_contract_unverified".to_string()));

            pairs.push(
                PreferencePair::new(prompt, chosen, rejected, src_session)
                    .with_verification(is_verified, v_source),
            );
        }

        Ok(pairs)
    }

    /// Mine DPO preference pairs from HITL and council approved Core memories and semantic facts.
    ///
    /// Core tier memories vetted by a human or council represent ground-truth invariants.
    pub fn mine_hitl_approval_pairs(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<PreferencePair>, StrataError> {
        let mut pairs = Vec::new();

        // 1. Core memories with human approval
        let memories = self.store.get_all_memories(None, None, 10_000)?;
        for mem in memories {
            if mem.tier == MemoryTier::Core
                && (mem.approved_by_human || mem.human_approval.is_some())
            {
                if let Some(sid) = session_id {
                    if mem.scope.to_string() != sid && !mem.tags.iter().any(|t| t == sid) {
                        continue;
                    }
                }

                let summary = mem
                    .summary
                    .clone()
                    .unwrap_or_else(|| "Core Architectural Invariant".to_string());
                let prompt = format!(
                    "System Invariant & Core Architecture Directive\nSummary: {}\nScope: {}",
                    summary, mem.scope
                );

                let approved_by = mem
                    .human_approval
                    .as_ref()
                    .map(|a| a.approved_by.clone())
                    .unwrap_or_else(|| "human_council".to_string());

                let chosen = format!(
                    "Verified Directive (Approved by: {}):\n{}\nImportance: {:.2}",
                    approved_by, mem.content, mem.importance
                );

                let rejected = format!(
                    "Anti-pattern Violation: Disregard core invariant '{}' and apply unvetted modifications without consensus.",
                    summary
                );

                let v_source = Some(format!("hitl_approved_core_memory:{}", approved_by));
                let src_sid = session_id.unwrap_or("core_invariant").to_string();

                pairs.push(
                    PreferencePair::new(prompt, chosen, rejected, src_sid)
                        .with_verification(true, v_source),
                );
            }
        }

        // 2. Core semantic facts with human approval
        let facts = self.store.get_all_semantic_facts(None, None, 10_000)?;
        for fact in facts {
            if fact.tier == MemoryTier::Core
                && (fact.approved_by_human || fact.human_approval.is_some())
            {
                if let Some(sid) = session_id {
                    if fact.scope.to_string() != sid && fact.category != sid {
                        continue;
                    }
                }

                let prompt = format!(
                    "Core Semantic Invariant Fact\nCategory: {}\nScope: {}",
                    fact.category, fact.scope
                );

                let approved_by = fact
                    .human_approval
                    .as_ref()
                    .map(|a| a.approved_by.clone())
                    .unwrap_or_else(|| "human_council".to_string());

                let chosen = format!(
                    "Verified Fact Statement (Approved by: {}):\n{}\nConfidence: {:.2}",
                    approved_by, fact.statement, fact.confidence
                );

                let rejected = format!(
                    "Falsified Assumption: Contradict core invariant in category '{}' with unverified assertions.",
                    fact.category
                );

                let v_source = Some(format!("hitl_approved_core_fact:{}", approved_by));
                let src_sid = session_id.unwrap_or("core_fact").to_string();

                pairs.push(
                    PreferencePair::new(prompt, chosen, rejected, src_sid)
                        .with_verification(true, v_source),
                );
            }
        }

        Ok(pairs)
    }

    /// Mine DPO pairs with optional oracle verification gating.
    pub fn mine_dpo_pairs_filtered(
        &self,
        session_id: Option<&str>,
        require_verified: bool,
    ) -> Result<Vec<PreferencePair>, StrataError> {
        let mut pairs = self.mine_dpo_pairs(session_id)?;
        if require_verified {
            pairs.retain(|p| p.oracle_verified);
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
            session_map
                .entry(ev.session_id.clone())
                .or_default()
                .push(ev);
        }

        for (sid, s_events) in session_map {
            let mut last_tool_inv: Option<(String, String)> = None;
            let mut failed_tool_inv: Option<(String, String, String)> = None;
            let mut current_task: Option<String> = None;

            for ev in s_events {
                match &ev.payload {
                    EventPayload::TaskStarted(t) => {
                        current_task = Some(format!(
                            "{}: {}",
                            t.title,
                            t.description.as_deref().unwrap_or("")
                        ));
                    }
                    EventPayload::ToolInvoked(inv) => {
                        let cmd = inv
                            .input
                            .get("command")
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
                            let (c_tool, c_input) = last_tool_inv
                                .take()
                                .unwrap_or_else(|| (res.tool_name.clone(), String::new()));
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
                            let v_source = format!("tool_execution_success:{}", c_tool);

                            pairs.push(
                                PreferencePair::new(prompt, chosen, rejected, &sid)
                                    .with_verification(true, Some(v_source)),
                            );
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
                                let rejected = format!(
                                    "Failing trajectory (Tool: {}):\nInput: {}\nError: {}",
                                    f_tool, f_input, f_err
                                );
                                let chosen =
                                    format!("Successful resolution: {}", task.outcome_summary);

                                pairs.push(
                                    PreferencePair::new(prompt, chosen, rejected, &sid)
                                        .with_verification(
                                            true,
                                            Some("task_completed_success".to_string()),
                                        ),
                                );
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
    /// episodic outcome signals, implicit telemetry, A2A delegations, and HITL approvals.
    pub fn mine_kto_samples(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<KtoSample>, StrataError> {
        let mut samples = Vec::new();

        // 1. From mined preference pairs (chosen = true, rejected = false)
        // Note: mine_dpo_pairs already aggregates failure patterns, episodes, feedback, delegations, and HITL.
        let pairs = self.mine_dpo_pairs(session_id)?;
        for pair in pairs {
            samples.push(
                KtoSample::new(&pair.prompt, &pair.chosen, true, &pair.source_session_id)
                    .with_verification(pair.oracle_verified, pair.verification_source.clone()),
            );
            samples.push(
                KtoSample::new(&pair.prompt, &pair.rejected, false, &pair.source_session_id)
                    .with_verification(pair.oracle_verified, pair.verification_source.clone()),
            );
        }

        // 2. From Episodic memories (single-turn outcomes)
        let episodes = if let Some(sid) = session_id {
            self.store.get_episodic_memories_by_session(sid)?
        } else {
            self.store.get_all_episodic_memories(None, 10_000)?
        };

        for ep in episodes {
            let prompt = if !ep.goals.is_empty() {
                format!("Goal: {}", ep.goals.join(", "))
            } else {
                format!("Task Summary: {}", ep.summary)
            };

            if ep.signals.success >= 0.6 && !ep.outcomes.is_empty() {
                let is_verified = ep.signals.success >= 0.8;
                let v_source = if is_verified {
                    Some("episodic_high_success".to_string())
                } else {
                    None
                };
                samples.push(
                    KtoSample::new(&prompt, ep.outcomes.join("; "), true, &ep.session_id)
                        .with_verification(is_verified, v_source),
                );
            }

            if (ep.signals.success < 0.4 || ep.signals.frustration >= 0.6)
                && !ep.obstacles.is_empty()
            {
                let is_verified = ep.signals.frustration >= 0.8 || ep.signals.success <= 0.2;
                let v_source = if is_verified {
                    Some("episodic_verified_failure".to_string())
                } else {
                    None
                };
                samples.push(
                    KtoSample::new(&prompt, ep.obstacles.join("; "), false, &ep.session_id)
                        .with_verification(is_verified, v_source),
                );
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
                SignalKind::TestRerunSuccess => {
                    let completion = sig
                        .extra
                        .unwrap_or_else(|| "Successful resolution / verified test run".to_string());
                    samples.push(
                        KtoSample::new(&prompt, &completion, true, &sig.session_id)
                            .with_verification(true, Some("test_rerun_oracle".to_string())),
                    );
                }
                SignalKind::ExplicitRating => {
                    let completion = sig
                        .extra
                        .unwrap_or_else(|| "Explicit rating confirmation".to_string());
                    samples.push(
                        KtoSample::new(&prompt, &completion, true, &sig.session_id)
                            .with_verification(true, Some("explicit_rating_oracle".to_string())),
                    );
                }
                SignalKind::TestRerunFail => {
                    let completion = sig
                        .extra
                        .unwrap_or_else(|| "Failed execution / test rerun failure".to_string());
                    samples.push(
                        KtoSample::new(&prompt, &completion, false, &sig.session_id)
                            .with_verification(true, Some("test_rerun_fail_oracle".to_string())),
                    );
                }
                SignalKind::ToolLoop | SignalKind::GitRevert => {
                    let completion = sig
                        .extra
                        .unwrap_or_else(|| "Failed execution / tool loop repetition".to_string());
                    samples.push(
                        KtoSample::new(&prompt, &completion, false, &sig.session_id)
                            .with_verification(true, Some(format!("{}_oracle", sig.kind))),
                    );
                }
                _ => {}
            }
        }

        // 4. Add isolated failed/timed-out delegations not already present in samples
        let contracts = self.store.list_delegations(None, None, None)?;
        for c in contracts {
            if matches!(
                c.status,
                DelegationStatus::Rejected | DelegationStatus::TimedOut
            ) {
                if let Some(sid) = session_id {
                    if c.task_id != sid && c.domain_scope.as_deref() != Some(sid) {
                        continue;
                    }
                }
                let prompt = format!(
                    "Task Delegation Objective: {}\nDomain Scope: {}\nTask ID: {}\nContext / Constraints: {}",
                    c.objective,
                    c.domain_scope.as_deref().unwrap_or("general"),
                    c.task_id,
                    c.context.as_deref().unwrap_or("")
                );
                if !samples.iter().any(|s| !s.label && s.prompt == prompt) {
                    let completion = format!(
                        "Status: {}\nError: {}",
                        c.status.as_str(),
                        c.error
                            .as_deref()
                            .unwrap_or("Delegation failed without resolution")
                    );
                    let v_source = Some(format!("delegation_failure:{}", c.status.as_str()));
                    samples.push(
                        KtoSample::new(&prompt, &completion, false, &c.task_id)
                            .with_verification(true, v_source),
                    );
                }
            }
        }

        Ok(samples)
    }

    /// Mine binary KTO samples from A2A delegation outcomes (completed = true, rejected/failed = false).
    pub fn mine_delegation_kto_samples(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<KtoSample>, StrataError> {
        let contracts = self.store.list_delegations(None, None, None)?;
        let mut samples = Vec::new();

        for c in contracts {
            if let Some(sid) = session_id {
                if c.task_id != sid && c.domain_scope.as_deref() != Some(sid) {
                    continue;
                }
            }

            let prompt = format!(
                "Task Delegation Objective: {}\nDomain Scope: {}\nTask ID: {}\nContext / Constraints: {}",
                c.objective,
                c.domain_scope.as_deref().unwrap_or("general"),
                c.task_id,
                c.context.as_deref().unwrap_or("")
            );
            let src_session = c.task_id.clone();

            match c.status {
                DelegationStatus::Completed => {
                    let mut comp_parts = Vec::new();
                    comp_parts.push("Status: Completed".to_string());
                    if let Some(ref summary) = c.result_summary {
                        comp_parts.push(format!("Summary: {}", summary));
                    }
                    if !c.expected_artifacts.is_empty() {
                        comp_parts.push(format!("Artifacts: {}", c.expected_artifacts.join(", ")));
                    }
                    let completion = comp_parts.join("\n");
                    let is_verified = c.verification_command.is_some();
                    let v_source = c
                        .verification_command
                        .as_ref()
                        .map(|cmd| format!("delegation_contract_oracle:{}", cmd))
                        .or_else(|| Some("delegation_contract_unverified".to_string()));
                    samples.push(
                        KtoSample::new(&prompt, &completion, true, &src_session)
                            .with_verification(is_verified, v_source),
                    );
                }
                DelegationStatus::Rejected | DelegationStatus::TimedOut => {
                    let completion = format!(
                        "Status: {}\nError: {}",
                        c.status.as_str(),
                        c.error
                            .as_deref()
                            .unwrap_or("Delegation failed without resolution")
                    );
                    let v_source = Some(format!("delegation_failure:{}", c.status.as_str()));
                    samples.push(
                        KtoSample::new(&prompt, &completion, false, &src_session)
                            .with_verification(true, v_source),
                    );
                }
                _ => {}
            }
        }

        Ok(samples)
    }

    /// Mine KTO binary feedback samples from HITL and council approved Core tier records.
    pub fn mine_hitl_approval_kto_samples(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<KtoSample>, StrataError> {
        let mut samples = Vec::new();

        let memories = self.store.get_all_memories(None, None, 10_000)?;
        for mem in memories {
            if mem.tier == MemoryTier::Core
                && (mem.approved_by_human || mem.human_approval.is_some())
            {
                if let Some(sid) = session_id {
                    if mem.scope.to_string() != sid && !mem.tags.iter().any(|t| t == sid) {
                        continue;
                    }
                }

                let summary = mem
                    .summary
                    .clone()
                    .unwrap_or_else(|| "Core Architectural Invariant".to_string());
                let prompt = format!(
                    "System Invariant & Core Architecture Directive\nSummary: {}\nScope: {}",
                    summary, mem.scope
                );

                let approved_by = mem
                    .human_approval
                    .as_ref()
                    .map(|a| a.approved_by.clone())
                    .unwrap_or_else(|| "human_council".to_string());

                let completion =
                    format!("Verified Invariant: {}\nContent: {}", summary, mem.content);
                let v_source = Some(format!("hitl_approved_core_memory:{}", approved_by));
                let src_sid = session_id.unwrap_or("core_invariant").to_string();

                samples.push(
                    KtoSample::new(&prompt, &completion, true, &src_sid)
                        .with_verification(true, v_source),
                );
            }
        }

        let facts = self.store.get_all_semantic_facts(None, None, 10_000)?;
        for fact in facts {
            if fact.tier == MemoryTier::Core
                && (fact.approved_by_human || fact.human_approval.is_some())
            {
                if let Some(sid) = session_id {
                    if fact.scope.to_string() != sid && fact.category != sid {
                        continue;
                    }
                }

                let prompt = format!(
                    "Core Semantic Invariant Fact\nCategory: {}\nScope: {}",
                    fact.category, fact.scope
                );
                let approved_by = fact
                    .human_approval
                    .as_ref()
                    .map(|a| a.approved_by.clone())
                    .unwrap_or_else(|| "human_council".to_string());

                let completion = format!("Verified Fact Statement: {}", fact.statement);
                let v_source = Some(format!("hitl_approved_core_fact:{}", approved_by));
                let src_sid = session_id.unwrap_or("core_fact").to_string();

                samples.push(
                    KtoSample::new(&prompt, &completion, true, &src_sid)
                        .with_verification(true, v_source),
                );
            }
        }

        Ok(samples)
    }

    /// Mine KTO samples with optional oracle verification gating.
    pub fn mine_kto_samples_filtered(
        &self,
        session_id: Option<&str>,
        require_verified: bool,
    ) -> Result<Vec<KtoSample>, StrataError> {
        let mut samples = self.mine_kto_samples(session_id)?;
        if require_verified {
            samples.retain(|s| s.oracle_verified);
        }
        Ok(samples)
    }

    /// Turn procedural skills and examples into SFT format `(instruction, input, output)`.
    pub fn mine_sft_samples(&self) -> Result<Vec<SftSample>, StrataError> {
        let skills = self.store.get_all_procedural_skills(None, 10_000)?;
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
                .map(|s| {
                    format!(
                        "{}. [{}] {}: {:?}",
                        s.order, s.tool, s.action, s.expected_result
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let source_sid = skill.project.unwrap_or_else(|| "global".to_string());
            let is_verified = skill.success_rate >= 0.8;
            let v_source = if is_verified {
                Some("procedural_skill_success_rate".to_string())
            } else {
                None
            };

            samples.push(
                SftSample::new(&instruction, &input, &steps_str, &source_sid)
                    .with_verification(is_verified, v_source),
            );

            // Also mine execution examples
            for ex in skill.examples {
                let ex_instruction = format!("Perform skill execution: {}", skill.name);
                let ex_input = format!("Session context: {}", ex.session_id);
                samples.push(
                    SftSample::new(&ex_instruction, &ex_input, &ex.outcome, &ex.session_id)
                        .with_verification(true, Some("skill_execution_example".to_string())),
                );
            }
        }

        Ok(samples)
    }

    /// Mine SFT samples with optional oracle verification gating.
    pub fn mine_sft_samples_filtered(
        &self,
        require_verified: bool,
    ) -> Result<Vec<SftSample>, StrataError> {
        let mut samples = self.mine_sft_samples()?;
        if require_verified {
            samples.retain(|s| s.oracle_verified);
        }
        Ok(samples)
    }

    /// Generates a structured breakdown of mined alignment pairs and samples without redundant database scans.
    pub fn get_alignment_report(&self) -> Result<AlignmentDatasetReport, StrataError> {
        let all_dpo = self.mine_dpo_pairs(None)?;
        let oracle_dpo = all_dpo.iter().filter(|p| p.oracle_verified).count();

        let delegation_dpo = all_dpo
            .iter()
            .filter(|p| {
                p.verification_source
                    .as_deref()
                    .unwrap_or("")
                    .starts_with("delegation_contract_oracle")
            })
            .count();
        let hitl_dpo = all_dpo
            .iter()
            .filter(|p| {
                let src = p.verification_source.as_deref().unwrap_or("");
                src.starts_with("hitl_approved_core_memory")
                    || src.starts_with("hitl_approved_core_fact")
            })
            .count();

        let failure_dpo = all_dpo
            .iter()
            .filter(|p| p.verification_source.as_deref() == Some("failure_pattern_mitigation"))
            .count();
        let episodic_dpo = all_dpo
            .iter()
            .filter(|p| p.verification_source.as_deref() == Some("episodic_high_success"))
            .count();
        let implicit_dpo = all_dpo
            .iter()
            .filter(|p| {
                let src = p.verification_source.as_deref().unwrap_or("");
                src.starts_with("tool_execution_success") || src == "task_completed_success"
            })
            .count();

        let all_kto = self.mine_kto_samples(None)?;
        let oracle_kto = all_kto.iter().filter(|s| s.oracle_verified).count();
        let delegation_kto = self.mine_delegation_kto_samples(None)?.len();
        let hitl_kto = self.mine_hitl_approval_kto_samples(None)?.len();

        let all_sft = self.mine_sft_samples()?;
        let oracle_sft = all_sft.iter().filter(|s| s.oracle_verified).count();

        Ok(AlignmentDatasetReport {
            total_dpo_pairs: all_dpo.len(),
            oracle_verified_dpo_pairs: oracle_dpo,
            delegation_dpo_pairs: delegation_dpo,
            hitl_approval_dpo_pairs: hitl_dpo,
            failure_pattern_dpo_pairs: failure_dpo,
            episodic_dpo_pairs: episodic_dpo,
            implicit_signal_dpo_pairs: implicit_dpo,
            total_kto_samples: all_kto.len(),
            oracle_verified_kto_samples: oracle_kto,
            delegation_kto_samples: delegation_kto,
            hitl_approval_kto_samples: hitl_kto,
            total_sft_samples: all_sft.len(),
            oracle_verified_sft_samples: oracle_sft,
        })
    }

    /// Export mined alignment dataset in specified format (DPO, KTO, SFT, Markdown, or JSONL),
    /// optionally gated to oracle-verified records only.
    pub fn export_with_gating(
        &self,
        format: ExportFormat,
        scope: Option<&str>,
        require_verified: bool,
    ) -> Result<String, StrataError> {
        match format {
            ExportFormat::Dpo => {
                let pairs = self.mine_dpo_pairs_filtered(scope, require_verified)?;
                let mut lines = Vec::new();
                for p in pairs {
                    lines.push(serde_json::to_string(&p)?);
                }
                Ok(lines.join("\n"))
            }
            ExportFormat::Kto => {
                let samples = self.mine_kto_samples_filtered(scope, require_verified)?;
                let mut lines = Vec::new();
                for s in samples {
                    lines.push(serde_json::to_string(&s)?);
                }
                Ok(lines.join("\n"))
            }
            ExportFormat::Sft => {
                let samples = self.mine_sft_samples_filtered(require_verified)?;
                let mut lines = Vec::new();
                for s in samples {
                    lines.push(serde_json::to_string(&s)?);
                }
                Ok(lines.join("\n"))
            }
            ExportFormat::Jsonl => {
                let pairs = self.mine_dpo_pairs_filtered(scope, require_verified)?;
                let sft = self.mine_sft_samples_filtered(require_verified)?;
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
                let pairs = self.mine_dpo_pairs_filtered(scope, require_verified)?;
                let kto = self.mine_kto_samples_filtered(scope, require_verified)?;
                let sft = self.mine_sft_samples_filtered(require_verified)?;

                let mut md = String::new();
                md.push_str("# Strata Alignment & Preference Dataset\n\n");
                if require_verified {
                    md.push_str(
                        "> **Filter**: Oracle-Verified Only (`require_verified = true`)\n\n",
                    );
                }
                md.push_str(&format!("- Total DPO Pairs: {}\n", pairs.len()));
                md.push_str(&format!("- Total KTO Samples: {}\n", kto.len()));
                md.push_str(&format!("- Total SFT Samples: {}\n\n", sft.len()));

                if !pairs.is_empty() {
                    md.push_str("## DPO Preference Pairs\n\n");
                    for (i, p) in pairs.iter().enumerate() {
                        md.push_str(&format!("### Pair {}\n", i + 1));
                        if let Some(ref src) = p.verification_source {
                            md.push_str(&format!(
                                "*Oracle Verification*: `{}` (verified: {})\n\n",
                                src, p.oracle_verified
                            ));
                        }
                        md.push_str(&format!("**Prompt**:\n```\n{}\n```\n\n", p.prompt));
                        md.push_str(&format!("**Chosen**:\n```\n{}\n```\n\n", p.chosen));
                        md.push_str(&format!("**Rejected**:\n```\n{}\n```\n\n", p.rejected));
                    }
                }

                if !sft.is_empty() {
                    md.push_str("## SFT Demonstrations\n\n");
                    for (i, s) in sft.iter().enumerate() {
                        md.push_str(&format!("### Sample {}\n", i + 1));
                        if let Some(ref src) = s.verification_source {
                            md.push_str(&format!(
                                "*Oracle Verification*: `{}` (verified: {})\n\n",
                                src, s.oracle_verified
                            ));
                        }
                        md.push_str(&format!("**Instruction**: {}\n\n", s.instruction));
                        md.push_str(&format!("**Input**:\n```\n{}\n```\n\n", s.input));
                        md.push_str(&format!("**Output**:\n```\n{}\n```\n\n", s.output));
                    }
                }

                Ok(md)
            }
        }
    }

    /// Export mined alignment dataset in specified format (DPO, KTO, SFT, Markdown, or JSONL)
    /// without gating (unrestricted).
    pub fn export(&self, format: ExportFormat, scope: Option<&str>) -> Result<String, StrataError> {
        self.export_with_gating(format, scope, false)
    }
}
