use std::collections::HashMap;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use strata_core::errors::StrataError;
use strata_core::events::{Event, EventPayload};
use strata_core::schemas::{
    DecayConfig, EpisodicMemory, EvidenceRef, ProceduralExample,
    ProceduralSkill, ProceduralStep, SemanticFact, SignalScores,
};
use strata_core::state::Scope;
use strata_core::traits::ReasoningEngine;

use crate::decay::DecayCalculator;
use crate::embedding::EmbeddingProvider;
use crate::jtms::TruthMaintenanceSystem;
use crate::store::SqliteStore;

/// Configuration for multi-stage consolidation pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Minimum salience score required to distill a session
    pub min_salience_threshold: f32,
    /// Whether to run JTMS truth maintenance during consolidation
    pub enable_jtms: bool,
    /// Whether to run mathematical decay pruning after consolidation
    pub enable_decay_pruning: bool,
    /// Optional custom pruning threshold override
    pub prune_threshold: Option<f32>,
    /// Decay calculator configuration
    pub decay_config: DecayConfig,
    /// Cosine similarity threshold for JTMS conflict detection
    pub jtms_similarity_threshold: f32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            min_salience_threshold: 0.2,
            enable_jtms: true,
            enable_decay_pruning: true,
            prune_threshold: None,
            decay_config: DecayConfig::default(),
            jtms_similarity_threshold: 0.85,
        }
    }
}

/// Consolidated outputs from a pipeline run.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationResult {
    #[serde(default)]
    pub session_id: Option<String>,
    pub episodic_memories: Vec<EpisodicMemory>,
    pub semantic_facts: Vec<SemanticFact>,
    pub procedural_skills: Vec<ProceduralSkill>,
    pub conflicts_resolved: usize,
    pub memories_pruned: usize,
    pub events_processed: usize,
}

/// Multi-stage memory consolidation pipeline.
pub struct ConsolidationPipeline {
    pub config: PipelineConfig,
    pub jtms: TruthMaintenanceSystem,
    pub decay: DecayCalculator,
}

impl ConsolidationPipeline {
    pub fn new(config: PipelineConfig) -> Self {
        let jtms = TruthMaintenanceSystem::new(config.jtms_similarity_threshold);
        let decay = DecayCalculator::new(config.decay_config.clone());
        Self {
            config,
            jtms,
            decay,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(PipelineConfig::default())
    }

    /// Stage 1: Event Filtering and grouping by session.
    /// Filters out low-signal status pings, empty payloads, and heartbeats.
    pub fn filter_and_group_events(&self, events: &[Event]) -> HashMap<String, Vec<Event>> {
        let mut grouped: HashMap<String, Vec<Event>> = HashMap::new();

        for event in events {
            if self.is_low_signal_event(event) {
                continue;
            }
            grouped
                .entry(event.session_id.clone())
                .or_default()
                .push(event.clone());
        }

        grouped
    }

    fn is_low_signal_event(&self, event: &Event) -> bool {
        match &event.payload {
            EventPayload::ObservationReceived(obs) => {
                let content_str = obs.content.as_str().unwrap_or_default();
                let content_trimmed = content_str.trim().to_lowercase();
                content_trimmed.is_empty()
                    || content_trimmed == "ping"
                    || content_trimmed == "pong"
                    || content_trimmed == "heartbeat"
                    || content_trimmed == "ok"
                    || content_trimmed == "status: ok"
            }
            _ => false,
        }
    }

    /// Stage 2: Salience Scoring.
    /// Evaluates error frequency, tool execution complexity, diff/output volume, and task outcomes.
    pub fn compute_session_salience(&self, events: &[Event]) -> f32 {
        if events.is_empty() {
            return 0.0;
        }

        let mut error_score = 0.0f32;
        let mut tool_score = 0.0f32;
        let mut task_score = 0.0f32;
        let mut payload_bytes = 0usize;

        for ev in events {
            match &ev.payload {
                EventPayload::ErrorObserved(_) => {
                    error_score += 0.35;
                }
                EventPayload::ToolResultReceived(res) => {
                    if res.is_error {
                        error_score += 0.30;
                    } else {
                        tool_score += 0.15;
                    }
                    payload_bytes += res.result.to_string().len();
                }
                EventPayload::ToolInvoked(inv) => {
                    tool_score += 0.10;
                    payload_bytes += inv.input.to_string().len();
                }
                EventPayload::TaskCompleted(task) => {
                    task_score += if task.success { 0.4 } else { 0.5 };
                }
                EventPayload::SessionEnded(_) => {
                    task_score += 0.2;
                }
                _ => {}
            }
        }

        let size_score = ((payload_bytes as f32) / 5000.0).min(0.25);
        let event_count_factor = ((events.len() as f32) / 10.0).min(0.2);

        (error_score + tool_score + task_score + size_score + event_count_factor).clamp(0.0, 1.0)
    }

    /// Stage 3: LLM Distillation (with structured JSON prompt or rule-based fallback).
    pub async fn distill_session(
        &self,
        session_id: &str,
        events: &[Event],
        reasoning_engine: Option<&dyn ReasoningEngine>,
    ) -> (Option<EpisodicMemory>, Vec<SemanticFact>, Option<ProceduralSkill>) {
        if let Some(engine) = reasoning_engine {
            let prompt = strata_reasoning::prompts::build_distillation_prompt(events);
            if let Ok(res_str) = engine.prompt(None, &prompt, None).await {
                if let Ok(distillation) = strata_reasoning::prompts::parse_distillation_output(&res_str) {
                    return self.convert_distillation_output(session_id, events, distillation);
                }
            }
        }

        self.distill_session_rule_based(session_id, events)
    }

    fn convert_distillation_output(
        &self,
        session_id: &str,
        events: &[Event],
        distillation: strata_reasoning::prompts::DistillationOutput,
    ) -> (Option<EpisodicMemory>, Vec<SemanticFact>, Option<ProceduralSkill>) {
        let time_start = events.first().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let time_end = events.last().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let actor = events.first().map(|e| e.agent_id.clone()).unwrap_or_else(|| "agent".to_string());

        let raw_event_ids: Vec<i64> = events.iter().enumerate().map(|(idx, ev)| {
            ev.sequence.map(|s| s as i64).unwrap_or(idx as i64)
        }).collect();

        let episodic = if let Some(first_ep) = distillation.episodic_memories.first() {
            let mut ep = EpisodicMemory::new(
                session_id,
                &actor,
                &first_ep.summary,
                time_start,
                time_end,
            )
            .with_raw_events(raw_event_ids);
            ep.signals.importance = first_ep.importance;
            ep.tags = first_ep.tags.clone();
            Some(ep)
        } else {
            None
        };

        let mut semantic_facts = Vec::new();
        for fact in distillation.semantic_facts {
            let sf = SemanticFact::new(
                fact.statement,
                "architectural_decision",
                Scope::Session(session_id.to_string()),
            )
            .with_importance(fact.importance)
            .with_confidence(fact.confidence)
            .with_tags(fact.tags);
            semantic_facts.push(sf);
        }

        let procedural_skill = if let Some(skill) = distillation.procedural_skills.into_iter().next() {
            let steps: Vec<ProceduralStep> = skill.steps.into_iter().map(|s| {
                ProceduralStep::new(
                    s.step_number,
                    s.tool_name.unwrap_or_else(|| "tool".to_string()),
                    s.action,
                    serde_json::json!({}),
                )
            }).collect();

            let mut ps = ProceduralSkill::new(skill.name, skill.description)
                .with_steps(steps)
                .with_preconditions(skill.preconditions);
            ps.importance = skill.importance;
            ps.tags = skill.tags;
            Some(ps)
        } else {
            None
        };

        (episodic, semantic_facts, procedural_skill)
    }

    /// Rule-based fallback extraction if LLM is unavailable or offline.
    pub fn distill_session_rule_based(
        &self,
        session_id: &str,
        events: &[Event],
    ) -> (Option<EpisodicMemory>, Vec<SemanticFact>, Option<ProceduralSkill>) {
        if events.is_empty() {
            return (None, Vec::new(), None);
        }

        let time_start = events.first().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let time_end = events.last().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let actor = events.first().map(|e| e.agent_id.clone()).unwrap_or_else(|| "agent".to_string());

        let mut tools_used = Vec::new();
        let mut files = Vec::new();
        let mut goals = Vec::new();
        let mut obstacles = Vec::new();
        let mut outcomes = Vec::new();
        let mut semantic_facts = Vec::new();
        let mut procedural_steps = Vec::new();
        let mut success_count = 0usize;
        let mut error_count = 0usize;
        let mut raw_event_ids: Vec<i64> = Vec::new();

        for (idx, ev) in events.iter().enumerate() {
            if let Some(seq) = ev.sequence {
                raw_event_ids.push(seq as i64);
            } else {
                raw_event_ids.push(idx as i64);
            }

            match &ev.payload {
                EventPayload::TaskStarted(task) => {
                    if let Some(ref desc) = task.description {
                        goals.push(desc.clone());
                    } else if !task.title.is_empty() {
                        goals.push(task.title.clone());
                    }
                }
                EventPayload::TaskCompleted(task) => {
                    if task.success {
                        success_count += 1;
                        outcomes.push(format!("Task {}: {}", task.task_id, task.outcome_summary));
                    } else {
                        error_count += 1;
                        obstacles.push(format!("Task failed: {}", task.outcome_summary));
                    }
                }
                EventPayload::ToolInvoked(inv) => {
                    if !tools_used.contains(&inv.tool_name) {
                        tools_used.push(inv.tool_name.clone());
                    }
                    if let Some(path) = inv.input.get("path").and_then(|p| p.as_str()) {
                        if !files.contains(&path.to_string()) {
                            files.push(path.to_string());
                        }
                    }
                    if let Some(file) = inv.input.get("file").and_then(|p| p.as_str()) {
                        if !files.contains(&file.to_string()) {
                            files.push(file.to_string());
                        }
                    }
                    let step = ProceduralStep::new(
                        procedural_steps.len() as u32 + 1,
                        &inv.tool_name,
                        "execute",
                        inv.input.clone(),
                    );
                    procedural_steps.push(step);
                }
                EventPayload::ToolResultReceived(res) => {
                    if res.is_error {
                        error_count += 1;
                        obstacles.push(format!("Tool '{}' failed: {:?}", res.tool_name, res.result));
                    }
                }
                EventPayload::ErrorObserved(err) => {
                    error_count += 1;
                    obstacles.push(format!("Error {}: {}", err.error_type, err.message));
                }
                EventPayload::ObservationReceived(obs) => {
                    let content_str = obs.content.as_str().map(|s| s.to_string()).unwrap_or_else(|| obs.content.to_string());
                    if content_str.len() > 10 {
                        let fact = SemanticFact::new(
                            content_str,
                            obs.observation_type.clone(),
                            Scope::Session(session_id.to_string()),
                        )
                        .with_importance(0.7)
                        .with_confidence(0.9)
                        .with_evidence(vec![EvidenceRef::new("observation", &obs.source, 0.9)
                            .with_session(session_id)]);
                        semantic_facts.push(fact);
                    }
                }
                EventPayload::SessionEnded(ended) => {
                    if let Some(ref sum) = ended.summary {
                        outcomes.push(sum.clone());
                    }
                }
                _ => {}
            }
        }

        // Build Signals
        let total_ops = (success_count + error_count).max(1) as f32;
        let success_rate = (success_count as f32) / total_ops;
        let frustration = ((error_count as f32) / total_ops).clamp(0.0, 1.0);
        let novelty = if semantic_facts.is_empty() { 0.3 } else { 0.8 };
        let importance = (0.4 + 0.3 * success_rate + 0.3 * (error_count as f32).min(2.0) / 2.0).clamp(0.0, 1.0);

        let signals = SignalScores {
            success: success_rate,
            frustration,
            novelty,
            importance,
        };

        let summary = if !outcomes.is_empty() {
            outcomes.join("; ")
        } else if !goals.is_empty() {
            format!("Addressed goal: {}", goals.join("; "))
        } else {
            format!("Executed session with {} events and {} tools", events.len(), tools_used.len())
        };

        let episodic = EpisodicMemory::new(
            session_id,
            actor,
            summary,
            time_start,
            time_end,
        )
        .with_goals(goals)
        .with_obstacles(obstacles)
        .with_outcomes(outcomes)
        .with_tools(tools_used)
        .with_files(files)
        .with_signals(signals)
        .with_raw_events(raw_event_ids);

        // Procedural skill if >= 2 sequential tool steps succeeded
        let procedural_skill = if procedural_steps.len() >= 2 && error_count == 0 {
            let mut skill = ProceduralSkill::new(
                format!("workflow_{session_id}"),
                format!("Sequential tool workflow with {} steps", procedural_steps.len()),
            )
            .with_steps(procedural_steps)
            .with_examples(vec![ProceduralExample::new(session_id, "Successfully completed")]);
            skill.importance = 0.7;
            Some(skill)
        } else {
            None
        };

        (Some(episodic), semantic_facts, procedural_skill)
    }

    /// Master pipeline execution across all 6 stages.
    pub async fn run_pipeline(
        &self,
        store: &SqliteStore,
        embedder: &dyn EmbeddingProvider,
        events: &[Event],
        reasoning_engine: Option<&dyn ReasoningEngine>,
    ) -> Result<ConsolidationResult, StrataError> {
        let mut result = ConsolidationResult::default();
        result.events_processed = events.len();

        if events.is_empty() {
            return Ok(result);
        }

        // 1. Filter and Group
        let sessions = self.filter_and_group_events(events);

        for (session_id, session_events) in sessions {
            // 2. Salience Scoring
            let salience = self.compute_session_salience(&session_events);
            if salience < self.config.min_salience_threshold && session_events.len() < 3 {
                continue;
            }

            // 3. LLM / Fallback Distillation
            let (episodic_opt, mut facts, skill_opt) =
                self.distill_session(&session_id, &session_events, reasoning_engine).await;

            // 4. Embedding Generation
            let mut fact_embeddings: Vec<Vec<f32>> = Vec::with_capacity(facts.len());
            for fact in &facts {
                let emb = embedder.embed_text(&fact.statement).await?;
                fact_embeddings.push(emb);
            }

            // 5. JTMS Conflict Check and SQLite Upsert
            if let Some(ep) = episodic_opt {
                store.insert_episodic_memory(&ep)?;
                result.episodic_memories.push(ep);
            }

            for (fact, emb) in facts.iter_mut().zip(fact_embeddings.iter()) {
                if self.config.enable_jtms {
                    let conflicts = self.jtms.resolve_and_upsert(store, fact, emb)?;
                    result.conflicts_resolved += conflicts.len();
                } else {
                    store.insert_or_update_semantic_fact(fact)?;
                    store.update_semantic_fact_embedding(&fact.id, emb)?;
                }
                result.semantic_facts.push(fact.clone());
            }

            if let Some(skill) = skill_opt {
                let skill_text = format!("{} {}", skill.name, skill.description);
                if let Ok(skill_emb) = embedder.embed_text(&skill_text).await {
                    // Update skill with embedding
                    store.insert_or_update_procedural_skill(&skill)?;
                    let _ = store.update_procedural_skill_embedding(&skill.id, &skill_emb);
                } else {
                    store.insert_or_update_procedural_skill(&skill)?;
                }
                result.procedural_skills.push(skill);
            }
        }

        // 6. Decay Pruning
        if self.config.enable_decay_pruning {
            let prune_rep = self.decay.prune_expired(store, self.config.prune_threshold, None)?;
            result.memories_pruned = prune_rep.memories_pruned + prune_rep.facts_pruned + prune_rep.skills_pruned;
        }

        Ok(result)
    }

    pub async fn consolidate_session(
        &self,
        store: &SqliteStore,
        embedder: &dyn EmbeddingProvider,
        session_id: &str,
        reasoning_engine: Option<&dyn ReasoningEngine>,
    ) -> Result<ConsolidationResult, StrataError> {
        let events = store.get_events(session_id, None, None)?;
        let mut res = self.run_pipeline(store, embedder, &events, reasoning_engine).await?;
        res.session_id = Some(session_id.to_string());
        Ok(res)
    }

    pub async fn consolidate_all(
        &self,
        store: &SqliteStore,
        embedder: &dyn EmbeddingProvider,
        reasoning_engine: Option<&dyn ReasoningEngine>,
    ) -> Result<ConsolidationResult, StrataError> {
        let session_ids = store.get_session_ids()?;
        let mut total = ConsolidationResult::default();

        for sid in session_ids {
            let res = self.consolidate_session(store, embedder, &sid, reasoning_engine).await?;
            total.episodic_memories.extend(res.episodic_memories);
            total.semantic_facts.extend(res.semantic_facts);
            total.procedural_skills.extend(res.procedural_skills);
            total.conflicts_resolved += res.conflicts_resolved;
            total.memories_pruned += res.memories_pruned;
            total.events_processed += res.events_processed;
        }

        Ok(total)
    }
}
