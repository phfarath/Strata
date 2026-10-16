use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strata_core::errors::StrataError;
use strata_core::events::{Event, EventPayload};
use strata_core::schemas::{
    DecayConfig, EpisodicMemory, EvidenceRef, ProceduralExample, ProceduralSkill, ProceduralStep,
    SemanticFact, SignalScores,
};
use strata_core::state::Scope;
use strata_core::traits::ReasoningEngine;

use crate::ast::CodeAnchorEngine;
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
    /// Whether to run automatic code anchor reconciliation
    #[serde(default)]
    pub enable_code_reconciliation: bool,
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
            enable_code_reconciliation: false,
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
    pub facts_stale: usize,
    pub facts_suspicious: usize,
    pub events_processed: usize,
}

/// Multi-stage memory consolidation pipeline.
pub struct ConsolidationPipeline {
    pub config: PipelineConfig,
    pub jtms: TruthMaintenanceSystem,
    pub decay: DecayCalculator,
    pub anchor_engine: CodeAnchorEngine,
}

impl ConsolidationPipeline {
    pub fn new(config: PipelineConfig) -> Self {
        let jtms = TruthMaintenanceSystem::new(config.jtms_similarity_threshold);
        let decay = DecayCalculator::new(config.decay_config.clone());
        let anchor_engine = CodeAnchorEngine::new();
        Self {
            config,
            jtms,
            decay,
            anchor_engine,
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
    ) -> (
        Option<EpisodicMemory>,
        Vec<SemanticFact>,
        Option<ProceduralSkill>,
    ) {
        if let Some(engine) = reasoning_engine {
            let prompt = strata_reasoning::prompts::build_distillation_prompt(events);
            if let Ok(res_str) = engine.prompt(None, &prompt, None).await {
                if let Ok(distillation) =
                    strata_reasoning::prompts::parse_distillation_output(&res_str)
                {
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
    ) -> (
        Option<EpisodicMemory>,
        Vec<SemanticFact>,
        Option<ProceduralSkill>,
    ) {
        let time_start = events.first().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let time_end = events.last().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let actor = events
            .first()
            .map(|e| e.agent_id.clone())
            .unwrap_or_else(|| "agent".to_string());

        let raw_event_ids: Vec<i64> = events
            .iter()
            .enumerate()
            .map(|(idx, ev)| ev.sequence.map(|s| s as i64).unwrap_or(idx as i64))
            .collect();

        let episodic = if let Some(first_ep) = distillation.episodic_memories.first() {
            let mut ep =
                EpisodicMemory::new(session_id, &actor, &first_ep.summary, time_start, time_end)
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

        let procedural_skill =
            if let Some(skill) = distillation.procedural_skills.into_iter().next() {
                let steps: Vec<ProceduralStep> = skill
                    .steps
                    .into_iter()
                    .map(|s| {
                        ProceduralStep::new(
                            s.step_number,
                            s.tool_name.unwrap_or_else(|| "tool".to_string()),
                            s.action,
                            serde_json::json!({}),
                        )
                    })
                    .collect();

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
    ) -> (
        Option<EpisodicMemory>,
        Vec<SemanticFact>,
        Option<ProceduralSkill>,
    ) {
        if events.is_empty() {
            return (None, Vec::new(), None);
        }

        let time_start = events.first().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let time_end = events.last().map(|e| e.timestamp).unwrap_or_else(Utc::now);
        let actor = events
            .first()
            .map(|e| e.agent_id.clone())
            .unwrap_or_else(|| "agent".to_string());

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
                        obstacles
                            .push(format!("Tool '{}' failed: {:?}", res.tool_name, res.result));
                    }
                }
                EventPayload::ErrorObserved(err) => {
                    error_count += 1;
                    obstacles.push(format!("Error {}: {}", err.error_type, err.message));
                }
                EventPayload::ObservationReceived(obs) => {
                    let content_str = obs
                        .content
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| obs.content.to_string());
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
        let importance =
            (0.4 + 0.3 * success_rate + 0.3 * (error_count as f32).min(2.0) / 2.0).clamp(0.0, 1.0);

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
            format!(
                "Executed session with {} events and {} tools",
                events.len(),
                tools_used.len()
            )
        };

        let episodic = EpisodicMemory::new(session_id, actor, summary, time_start, time_end)
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
                format!(
                    "Sequential tool workflow with {} steps",
                    procedural_steps.len()
                ),
            )
            .with_steps(procedural_steps)
            .with_examples(vec![ProceduralExample::new(
                session_id,
                "Successfully completed",
            )]);
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
            let (episodic_opt, mut facts, skill_opt) = self
                .distill_session(&session_id, &session_events, reasoning_engine)
                .await;

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
            let prune_rep = self
                .decay
                .prune_expired(store, self.config.prune_threshold, None)?;
            result.memories_pruned =
                prune_rep.memories_pruned + prune_rep.facts_pruned + prune_rep.skills_pruned;
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
        let mut res = self
            .run_pipeline(store, embedder, &events, reasoning_engine)
            .await?;
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
            let res = self
                .consolidate_session(store, embedder, &sid, reasoning_engine)
                .await?;
            total.episodic_memories.extend(res.episodic_memories);
            total.semantic_facts.extend(res.semantic_facts);
            total.procedural_skills.extend(res.procedural_skills);
            total.conflicts_resolved += res.conflicts_resolved;
            total.memories_pruned += res.memories_pruned;
            total.events_processed += res.events_processed;
        }

        Ok(total)
    }

    /// Reconciles code-anchored semantic facts in the store against a workspace on disk.
    pub async fn reconcile_code_anchors(
        &self,
        store: &SqliteStore,
        workspace_root: &std::path::Path,
        git_commit_hash: Option<&str>,
        world_model: Option<&strata_reasoning::WorldModel>,
    ) -> Result<crate::ast::ReconciliationReport, StrataError> {
        self.anchor_engine
            .reconcile_workspace_dir(store, workspace_root, git_commit_hash, world_model)
            .await
    }
}

// ============================================================================
// AUDN Ingestion Pipeline (Add / Update / Delete / Noop Deduplicator)
// ============================================================================

/// Classification operation produced by the AUDN Ingestion Pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudnOperation {
    /// Genuinely new memory/fact; insert into store.
    Add,
    /// Modifies or supersedes an existing memory/fact on the same topic.
    Update { target_id: uuid::Uuid },
    /// Explicitly retracts or deletes an obsolete memory/fact.
    Delete { target_id: uuid::Uuid },
    /// Identical or near-duplicate of an existing memory/fact; silently discard.
    Noop { existing_id: uuid::Uuid },
}

/// Decision report returned by the AUDN Ingestion Pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudnDecision {
    pub operation: AudnOperation,
    pub reason: String,
    pub cosine_similarity: f32,
    pub minhash_similarity: f32,
    pub record: Option<strata_core::state::MemoryRecord>,
}

/// Configuration thresholds for the AUDN Ingestion Pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudnConfig {
    /// Cosine similarity threshold above which non-contradictory candidates are discarded as `Noop` (default `0.92`).
    pub cosine_noop_threshold: f32,
    /// MinHash Jaccard similarity threshold at or above which non-contradictory candidates are discarded as `Noop` (default `0.85`).
    pub minhash_noop_threshold: f32,
    /// Number of hash functions in each MinHash signature (default `64`).
    pub num_hashes: usize,
}

impl Default for AudnConfig {
    fn default() -> Self {
        Self {
            cosine_noop_threshold: 0.92,
            minhash_noop_threshold: 0.85,
            num_hashes: 64,
        }
    }
}

impl AudnConfig {
    pub fn new(
        cosine_noop_threshold: f32,
        minhash_noop_threshold: f32,
        num_hashes: usize,
    ) -> Self {
        Self {
            cosine_noop_threshold: cosine_noop_threshold.clamp(0.0, 1.0),
            minhash_noop_threshold: minhash_noop_threshold.clamp(0.0, 1.0),
            num_hashes: num_hashes.max(1),
        }
    }

    pub fn with_cosine_threshold(mut self, threshold: f32) -> Self {
        self.cosine_noop_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    pub fn with_minhash_threshold(mut self, threshold: f32) -> Self {
        self.minhash_noop_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    pub fn with_num_hashes(mut self, num_hashes: usize) -> Self {
        self.num_hashes = num_hashes.max(1);
        self
    }
}

/// Deterministically computes a MinHash signature of length `num_hashes` over
/// normalized word and character 3-gram shingles extracted from `text`.
pub fn compute_minhash_signature(text: &str, num_hashes: usize) -> Vec<u64> {
    if num_hashes == 0 {
        return Vec::new();
    }

    let shingles = extract_audn_shingles(text);
    if shingles.is_empty() {
        return vec![0u64; num_hashes];
    }

    let base_hashes: Vec<u64> = shingles.iter().map(|s| fnv1a_64(s.as_bytes())).collect();
    let mut signature = Vec::with_capacity(num_hashes);

    for i in 0..num_hashes {
        let seed = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut min_hash = u64::MAX;
        for &bh in &base_hashes {
            let h = splitmix64(bh ^ seed);
            if h < min_hash {
                min_hash = h;
            }
        }
        signature.push(min_hash);
    }

    signature
}

/// Estimates Jaccard similarity in `[0.0, 1.0]` between two MinHash signatures.
pub fn minhash_jaccard_similarity(sig_a: &[u64], sig_b: &[u64]) -> f32 {
    if sig_a.is_empty() || sig_b.is_empty() || sig_a.len() != sig_b.len() {
        return 0.0;
    }
    if sig_a.iter().all(|&x| x == 0) && sig_b.iter().all(|&x| x == 0) {
        return 1.0;
    }
    if sig_a.iter().all(|&x| x == 0) || sig_b.iter().all(|&x| x == 0) {
        return 0.0;
    }
    let matches = sig_a
        .iter()
        .zip(sig_b.iter())
        .filter(|(a, b)| a == b)
        .count();
    matches as f32 / sig_a.len() as f32
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn splitmix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn normalize_audn_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn canonical_audn_words(text: &str) -> Vec<String> {
    const ARTICLES: &[&str] = &["the", "a", "an"];
    let norm = normalize_audn_text(text);
    let all_words: Vec<String> = norm.split_whitespace().map(String::from).collect();
    let filtered: Vec<String> = all_words
        .iter()
        .filter(|w| !ARTICLES.contains(&w.as_str()))
        .cloned()
        .collect();
    if filtered.is_empty() {
        all_words
    } else {
        filtered
    }
}

fn extract_audn_shingles(text: &str) -> std::collections::HashSet<String> {
    let words = canonical_audn_words(text);
    let mut shingles = std::collections::HashSet::new();

    for w in &words {
        shingles.insert(format!("w:{w}"));
        if w.len() >= 3 {
            for window in w.as_bytes().windows(3) {
                if let Ok(tri) = std::str::from_utf8(window) {
                    shingles.insert(format!("c:{tri}"));
                }
            }
        }
    }

    for pair in words.windows(2) {
        shingles.insert(format!("b:{}_{}", pair[0], pair[1]));
    }

    shingles
}

fn tokenize_audn_meaningful_words(text: &str) -> std::collections::HashSet<String> {
    const STOPWORDS: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "to", "for", "in", "of", "on", "with",
        "and", "or", "by", "at", "from", "as", "be", "this", "that", "it", "we", "our", "use",
        "uses", "using", "used", "now", "new", "old", "retract", "retracted", "delete", "deleted",
        "obsolete", "remove", "removed", "drop", "dropped", "rule", "requirement", "valid",
        "longer", "no",
    ];
    normalize_audn_text(text)
        .split_whitespace()
        .filter(|w| w.len() >= 2 && !STOPWORDS.contains(w))
        .map(String::from)
        .collect()
}

fn audn_lexical_jaccard(a: &str, b: &str) -> f32 {
    let set_a = tokenize_audn_meaningful_words(a);
    let set_b = tokenize_audn_meaningful_words(b);
    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

fn has_audn_topic_overlap(
    text_a: &str,
    tags_a: &[String],
    text_b: &str,
    tags_b: &[String],
) -> bool {
    if !tags_a.is_empty() && tags_a.iter().any(|t| tags_b.contains(t)) {
        return true;
    }
    let words_a = tokenize_audn_meaningful_words(text_a);
    let words_b = tokenize_audn_meaningful_words(text_b);
    let shared = words_a.intersection(&words_b).count();
    shared >= 2 || (shared >= 1 && (words_a.len() <= 3 || words_b.len() <= 3))
}

fn is_audn_numeric_template_update(text_a: &str, text_b: &str) -> bool {
    let norm_a = normalize_audn_text(text_a);
    let norm_b = normalize_audn_text(text_b);
    if norm_a == norm_b {
        return false;
    }
    let nums_a: Vec<&str> = norm_a
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_ascii_digit()))
        .collect();
    let nums_b: Vec<&str> = norm_b
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_ascii_digit()))
        .collect();
    if nums_a.is_empty() || nums_b.is_empty() || nums_a == nums_b {
        return false;
    }

    let strip_digits = |s: &str| -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_num = false;
        for c in s.chars() {
            if c.is_ascii_digit() {
                if !in_num {
                    out.push('#');
                    in_num = true;
                }
            } else {
                in_num = false;
                out.push(c);
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    };

    let tpl_a = strip_digits(&norm_a);
    let tpl_b = strip_digits(&norm_b);
    tpl_a == tpl_b && tpl_a.len() >= 5
}

fn is_subject_predicate_value_update(text_a: &str, text_b: &str) -> bool {
    let norm_a = canonical_audn_words(text_a).join(" ");
    let norm_b = canonical_audn_words(text_b).join(" ");
    if norm_a == norm_b {
        return false;
    }

    for sep in [" is ", " are ", " uses ", " equals "] {
        if let (Some((subj_a, pred_a)), Some((subj_b, pred_b))) =
            (norm_a.split_once(sep), norm_b.split_once(sep))
        {
            let subj_words: Vec<&str> = subj_a.split_whitespace().collect();
            if subj_a == subj_b && subj_words.len() >= 2 {
                let pred_jaccard = audn_lexical_jaccard(pred_a, pred_b);
                if pred_jaccard < 0.5 {
                    return true;
                }
            }
        }
    }
    false
}

fn is_explicit_delete_directive(text: &str, metadata: Option<&serde_json::Value>) -> bool {
    if let Some(meta) = metadata {
        if let Some(action) = meta.get("audn_action").and_then(|v| v.as_str()) {
            if action.eq_ignore_ascii_case("delete")
                || action.eq_ignore_ascii_case("retract")
                || action.eq_ignore_ascii_case("remove")
            {
                return true;
            }
        }
    }

    let lower = text.trim().to_lowercase();

    let prefix_cues = [
        "retract:",
        "retracted:",
        "delete:",
        "deleted:",
        "obsolete:",
        "forget:",
        "remove:",
        "drop:",
    ];
    let has_prefix = prefix_cues.iter().any(|p| lower.starts_with(p));

    let inline_cues = [
        "no longer valid",
        "no longer applicable",
        "remove rule",
        "rule removed",
        "drop requirement",
        "requirement dropped",
        "deprecate rule",
        "obsolete rule",
    ];
    let has_inline = inline_cues.iter().any(|c| lower.contains(c));

    if has_prefix || has_inline {
        return true;
    }

    false
}

fn strip_delete_directive_prefix(text: &str) -> String {
    let trimmed = text.trim();
    let mut cleaned = trimmed.to_string();
    for prefix in [
        "retract:",
        "retracted:",
        "delete:",
        "deleted:",
        "obsolete:",
        "forget:",
        "remove:",
        "drop:",
        "remove rule:",
        "drop requirement:",
    ] {
        if cleaned.to_lowercase().starts_with(prefix) {
            cleaned = cleaned[prefix.len()..].trim().to_string();
            break;
        }
    }
    for suffix in [
        " is no longer valid",
        " is no longer applicable",
        " is obsolete",
        " is removed",
        " has been removed",
    ] {
        if cleaned.to_lowercase().ends_with(suffix) {
            cleaned = cleaned[..cleaned.len() - suffix.len()].trim().to_string();
            break;
        }
    }
    cleaned
}

fn is_audn_memory_deprecated(record: &strata_core::state::MemoryRecord) -> bool {
    record
        .metadata
        .get("status")
        .and_then(|v| v.as_str())
        .map_or(false, |s| {
            s.eq_ignore_ascii_case("deprecated") || s.eq_ignore_ascii_case("archived")
        })
}

fn audn_scopes_compatible(a: &Scope, b: &Scope) -> bool {
    a == b || a.is_compatible(b) || b.is_compatible(a)
}

/// AUDN (Add / Update / Delete / Noop) Ingestion Pipeline.
/// Intercepts candidate memories and semantic facts before SQLite persistence to prevent
/// semantic bloat, update superseded facts in place, and execute explicit retractions.
#[derive(Debug, Clone)]
pub struct AudnPipeline {
    pub config: AudnConfig,
    pub jtms: TruthMaintenanceSystem,
}

impl Default for AudnPipeline {
    fn default() -> Self {
        Self::with_default_config()
    }
}

impl AudnPipeline {
    pub fn new(config: AudnConfig) -> Self {
        Self {
            config,
            jtms: TruthMaintenanceSystem::with_default_threshold(),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AudnConfig::default())
    }

    /// Detect whether `candidate_text` represents an update/contradiction to `existing_text`.
    fn detect_update_cues(
        &self,
        existing_text: &str,
        candidate_text: &str,
        candidate_meta: Option<&serde_json::Value>,
    ) -> Option<String> {
        if let Some(meta) = candidate_meta {
            if let Some(action) = meta.get("audn_action").and_then(|v| v.as_str()) {
                if action.eq_ignore_ascii_case("update") {
                    return Some("Explicit metadata audn_action=update".to_string());
                }
            }
        }

        if normalize_audn_text(existing_text) == normalize_audn_text(candidate_text) {
            return None;
        }

        if self.jtms.is_orthogonal_coexistence(existing_text, candidate_text) {
            return None;
        }

        if is_audn_numeric_template_update(existing_text, candidate_text) {
            return Some("Numeric or version parameter update on matching template".to_string());
        }

        if is_subject_predicate_value_update(existing_text, candidate_text) {
            return Some("Predicate value update on matching subject".to_string());
        }

        let (has_contradiction, cues) = self
            .jtms
            .detect_lexical_contradiction(existing_text, candidate_text);

        if has_contradiction {
            // Verify that the contradiction is genuinely asymmetric between existing and candidate
            // (so a migration sentence compared against a near-duplicate of itself does not self-contradict).
            let a_lower = existing_text.to_lowercase();
            let b_lower = candidate_text.to_lowercase();
            let check_tokens = [
                "always",
                "never",
                "enable",
                "disable",
                "enabled",
                "disabled",
                "true",
                "false",
                "avoid",
                "do not use",
                "supported",
                "unsupported",
                "deprecated",
                "active",
                "required",
                "optional",
                "allow",
                "deny",
                "allowed",
                "forbidden",
                "disallowed",
                "increase",
                "decrease",
                "must not",
                "should not",
                "cannot",
                "is not",
                "valid",
                "invalid",
                "public",
                "private",
                "sync",
                "async",
                "synchronous",
                "asynchronous",
                "mutable",
                "immutable",
                "blocking",
                "non-blocking",
                "strict",
                "permissive",
                "mandatory",
                "rest",
                "grpc",
                "json",
                "protobuf",
                "mysql",
                "postgres",
                "postgresql",
                "sqlite",
                "monolith",
                "microservices",
                "event-driven",
                "hs256",
                "rs256",
                "tls 1.2",
                "tls 1.3",
                "http",
                "https",
                "session",
                "bearer token",
                "basic auth",
                "oauth2",
                "plaintext",
                "encrypted",
                "unencrypted",
                "not",
                "no longer",
                "removed",
                "migrated",
                "replaced",
                "supersedes",
                "instead of",
                "switched to",
            ];
            let is_asymmetric = check_tokens
                .iter()
                .any(|tok| a_lower.contains(tok) != b_lower.contains(tok));
            if is_asymmetric {
                return Some(format!(
                    "JTMS contradiction/update cues: {}",
                    cues.join("; ")
                ));
            }
        }

        None
    }

    /// Classifies a candidate `MemoryRecord` against `SqliteStore` into `Add`, `Update`, `Delete`, or `Noop`
    /// without mutating the database.
    pub fn classify_memory(
        &self,
        store: &SqliteStore,
        candidate: &strata_core::state::MemoryRecord,
    ) -> Result<AudnDecision, StrataError> {
        let mut existing_records = store.get_all_memories(None, None, 10_000)?;
        existing_records.retain(|r| {
            r.id != candidate.id
                && !is_audn_memory_deprecated(r)
                && audn_scopes_compatible(&r.scope, &candidate.scope)
        });

        let is_delete =
            is_explicit_delete_directive(&candidate.content, Some(&candidate.metadata));
        let cleaned_candidate_text = if is_delete {
            strip_delete_directive_prefix(&candidate.content)
        } else {
            candidate.content.clone()
        };

        let cand_sig = compute_minhash_signature(&cleaned_candidate_text, self.config.num_hashes);
        let cand_norm = canonical_audn_words(&cleaned_candidate_text).join(" ");

        // Explicit target_id override in metadata
        if let Some(target_str) = candidate
            .metadata
            .get("target_id")
            .and_then(|v| v.as_str())
        {
            if let Ok(target_uuid) = uuid::Uuid::parse_str(target_str) {
                if let Some(existing) = existing_records.iter().find(|r| r.id == target_uuid) {
                    let ex_sig =
                        compute_minhash_signature(&existing.content, self.config.num_hashes);
                    let mh_sim = minhash_jaccard_similarity(&cand_sig, &ex_sig);
                    let cos_sim = match (&candidate.embedding, &existing.embedding) {
                        (Some(ca), Some(ex)) => crate::embedding::cosine_similarity(ca, ex),
                        _ => 0.0,
                    };
                    if is_delete {
                        return Ok(AudnDecision {
                            operation: AudnOperation::Delete {
                                target_id: existing.id,
                            },
                            reason: format!(
                                "Explicit deletion of target memory {}",
                                existing.id
                            ),
                            cosine_similarity: cos_sim,
                            minhash_similarity: mh_sim,
                            record: None,
                        });
                    }
                    if let Some(reason) = self.detect_update_cues(
                        &existing.content,
                        &candidate.content,
                        Some(&candidate.metadata),
                    ) {
                        return Ok(AudnDecision {
                            operation: AudnOperation::Update {
                                target_id: existing.id,
                            },
                            reason,
                            cosine_similarity: cos_sim,
                            minhash_similarity: mh_sim,
                            record: Some(existing.clone()),
                        });
                    }
                }
            }
        }

        // 1. Check Delete directive against matching topic in store
        if is_delete {
            let mut best_target: Option<(&strata_core::state::MemoryRecord, f32, f32, f32)> = None;
            for existing in &existing_records {
                if !has_audn_topic_overlap(
                    &existing.content,
                    &existing.tags,
                    &cleaned_candidate_text,
                    &candidate.tags,
                ) {
                    continue;
                }
                let ex_sig = compute_minhash_signature(&existing.content, self.config.num_hashes);
                let mh_sim = minhash_jaccard_similarity(&cand_sig, &ex_sig);
                let cos_sim = match (&candidate.embedding, &existing.embedding) {
                    (Some(ca), Some(ex)) => crate::embedding::cosine_similarity(ca, ex),
                    _ => 0.0,
                };
                let lex_sim = audn_lexical_jaccard(&existing.content, &cleaned_candidate_text);
                let combined_score = mh_sim.max(cos_sim).max(lex_sim);
                if best_target.map_or(true, |(_, _, _, best)| combined_score > best) {
                    best_target = Some((existing, cos_sim, mh_sim, combined_score));
                }
            }

            if let Some((target, cos_sim, mh_sim, _)) = best_target {
                return Ok(AudnDecision {
                    operation: AudnOperation::Delete {
                        target_id: target.id,
                    },
                    reason: format!(
                        "Retraction/deletion directive matched existing memory {}",
                        target.id
                    ),
                    cosine_similarity: cos_sim,
                    minhash_similarity: mh_sim,
                    record: None,
                });
            }
        }

        // 2. Scan existing records for Noop (near-duplicate) or Update (contradiction/revision)
        let mut best_update: Option<(&strata_core::state::MemoryRecord, String, f32, f32, f32)> =
            None;
        let mut max_cos_sim = 0.0f32;
        let mut max_mh_sim = 0.0f32;

        for existing in &existing_records {
            let ex_sig = compute_minhash_signature(&existing.content, self.config.num_hashes);
            let mh_sim = minhash_jaccard_similarity(&cand_sig, &ex_sig);
            let cos_sim = match (&candidate.embedding, &existing.embedding) {
                (Some(ca), Some(ex)) => crate::embedding::cosine_similarity(ca, ex),
                _ => 0.0,
            };

            if cos_sim > max_cos_sim {
                max_cos_sim = cos_sim;
            }
            if mh_sim > max_mh_sim {
                max_mh_sim = mh_sim;
            }

            let ex_norm = canonical_audn_words(&existing.content).join(" ");
            let norm_equal = !cand_norm.is_empty() && cand_norm == ex_norm;

            let update_reason_opt = self.detect_update_cues(
                &existing.content,
                &candidate.content,
                Some(&candidate.metadata),
            );

            if let Some(update_reason) = update_reason_opt {
                if has_audn_topic_overlap(
                    &existing.content,
                    &existing.tags,
                    &candidate.content,
                    &candidate.tags,
                ) {
                    let lex_sim = audn_lexical_jaccard(&existing.content, &candidate.content);
                    let score = mh_sim.max(cos_sim).max(lex_sim);
                    if best_update.as_ref().map_or(true, |(_, _, _, _, b)| score > *b) {
                        best_update = Some((existing, update_reason, cos_sim, mh_sim, score));
                    }
                    continue;
                }
            }

            // Not a contradiction/update: check Noop thresholds
            if norm_equal
                || mh_sim >= self.config.minhash_noop_threshold
                || cos_sim > self.config.cosine_noop_threshold
            {
                let reason = if norm_equal {
                    format!(
                        "Normalized lexical match with existing memory {}",
                        existing.id
                    )
                } else if cos_sim > self.config.cosine_noop_threshold {
                    format!(
                        "Embedding cosine similarity {:.3} > {:.3} threshold with memory {}",
                        cos_sim, self.config.cosine_noop_threshold, existing.id
                    )
                } else {
                    format!(
                        "MinHash Jaccard similarity {:.3} >= {:.3} threshold with memory {}",
                        mh_sim, self.config.minhash_noop_threshold, existing.id
                    )
                };

                return Ok(AudnDecision {
                    operation: AudnOperation::Noop {
                        existing_id: existing.id,
                    },
                    reason,
                    cosine_similarity: cos_sim,
                    minhash_similarity: mh_sim,
                    record: Some(existing.clone()),
                });
            }
        }

        if let Some((target, reason, cos_sim, mh_sim, _)) = best_update {
            return Ok(AudnDecision {
                operation: AudnOperation::Update {
                    target_id: target.id,
                },
                reason,
                cosine_similarity: cos_sim,
                minhash_similarity: mh_sim,
                record: Some(target.clone()),
            });
        }

        Ok(AudnDecision {
            operation: AudnOperation::Add,
            reason: "Novel memory with no existing duplicate or conflict in scope".to_string(),
            cosine_similarity: max_cos_sim,
            minhash_similarity: max_mh_sim,
            record: Some(candidate.clone()),
        })
    }

    /// Ingests a candidate `MemoryRecord` through the AUDN pipeline and applies the resulting
    /// `Add`, `Update`, `Delete`, or `Noop` operation to `SqliteStore`.
    pub async fn ingest_memory(
        &self,
        store: &SqliteStore,
        embedder: Option<&dyn EmbeddingProvider>,
        candidate: &strata_core::state::MemoryRecord,
    ) -> Result<AudnDecision, StrataError> {
        let mut prepared = candidate.clone();

        if prepared.tier == strata_core::state::MemoryTier::Core && !prepared.approved_by_human {
            return Err(StrataError::Validation(
                "Cannot write memory directly to Core Tier without explicit human approval (approved_by_human=true)".to_string(),
            ));
        }

        if prepared.embedding.is_none() && !prepared.content.trim().is_empty() {
            if let Some(emb_provider) = embedder {
                if let Ok(emb) = emb_provider.embed_text(&prepared.content).await {
                    prepared.embedding = Some(emb);
                }
            }
        }

        // Backfill missing embeddings on existing active records in compatible scopes if embedder is provided
        if let Some(emb_provider) = embedder {
            let existing = store.get_all_memories(None, None, 10_000)?;
            for mut mem in existing {
                if mem.embedding.is_none()
                    && !is_audn_memory_deprecated(&mem)
                    && audn_scopes_compatible(&mem.scope, &prepared.scope)
                    && !mem.content.trim().is_empty()
                {
                    if let Ok(emb) = emb_provider.embed_text(&mem.content).await {
                        mem.embedding = Some(emb);
                        let _ = store.insert_or_update_memory(&mem);
                    }
                }
            }
        }

        let mut decision = self.classify_memory(store, &prepared)?;

        match decision.operation {
            AudnOperation::Noop { .. } => {
                // Silent discard: do not write a new row to SQLite
            }
            AudnOperation::Delete { target_id } => {
                store.delete_memory(&target_id)?;
                decision.record = None;
            }
            AudnOperation::Update { target_id } => {
                if let Some(mut existing) = store.get_memory(&target_id)? {
                    existing.content = prepared.content.clone();
                    if prepared.summary.is_some() {
                        existing.summary = prepared.summary.clone();
                    }
                    existing.importance = existing.importance.max(prepared.importance);
                    existing.confidence = prepared.confidence;
                    existing.approved_by_human =
                        existing.approved_by_human || prepared.approved_by_human;
                    if prepared.embedding.is_some() {
                        existing.embedding = prepared.embedding.clone();
                    }
                    for tag in &prepared.tags {
                        if !existing.tags.contains(tag) {
                            existing.tags.push(tag.clone());
                        }
                    }
                    for ev in &prepared.evidence_ids {
                        if !existing.evidence_ids.contains(ev) {
                            existing.evidence_ids.push(*ev);
                        }
                    }

                    let mut meta = existing.metadata.clone();
                    if !meta.is_object() {
                        meta = serde_json::json!({});
                    }
                    let prev_ver = meta.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
                    meta["version"] = serde_json::json!(prev_ver + 1);
                    meta["last_audn_op"] = serde_json::json!("update");
                    existing.metadata = meta;
                    existing.updated_at = Utc::now();

                    store.insert_or_update_memory(&existing)?;
                    decision.record = Some(existing);
                }
            }
            AudnOperation::Add => {
                let mut meta = prepared.metadata.clone();
                if !meta.is_object() {
                    meta = serde_json::json!({});
                }
                if meta.get("version").is_none() {
                    meta["version"] = serde_json::json!(1);
                }
                prepared.metadata = meta;
                store.insert_or_update_memory(&prepared)?;
                decision.record = Some(prepared);
            }
        }

        Ok(decision)
    }

    /// Ingests a candidate `SemanticFact` through the AUDN pipeline and applies the resulting
    /// `Add`, `Update`, `Delete`, or `Noop` operation to `SqliteStore`.
    pub async fn ingest_fact(
        &self,
        store: &SqliteStore,
        embedder: Option<&dyn EmbeddingProvider>,
        candidate: &SemanticFact,
    ) -> Result<AudnDecision, StrataError> {
        let cand_emb = if let Some(emb_provider) = embedder {
            emb_provider.embed_text(&candidate.statement).await.ok()
        } else {
            None
        };

        let existing_with_embs = store.get_semantic_facts_with_embeddings(
            None,
            Some(strata_core::schemas::FactStatus::Active),
        )?;

        let is_delete = is_explicit_delete_directive(&candidate.statement, None);
        let cleaned_stmt = if is_delete {
            strip_delete_directive_prefix(&candidate.statement)
        } else {
            candidate.statement.clone()
        };

        let cand_sig = compute_minhash_signature(&cleaned_stmt, self.config.num_hashes);
        let cand_norm = canonical_audn_words(&cleaned_stmt).join(" ");

        let eligible: Vec<&(SemanticFact, Option<Vec<f32>>)> = existing_with_embs
            .iter()
            .filter(|(f, _)| {
                f.id != candidate.id && audn_scopes_compatible(&f.scope, &candidate.scope)
            })
            .collect();

        // 1. Delete directive check
        if is_delete {
            let mut best_target: Option<(&SemanticFact, f32, f32, f32)> = None;
            for (existing, ex_emb) in &eligible {
                if !has_audn_topic_overlap(
                    &existing.statement,
                    &existing.tags,
                    &cleaned_stmt,
                    &candidate.tags,
                ) {
                    continue;
                }
                let ex_sig = compute_minhash_signature(&existing.statement, self.config.num_hashes);
                let mh_sim = minhash_jaccard_similarity(&cand_sig, &ex_sig);
                let cos_sim = match (&cand_emb, ex_emb) {
                    (Some(ca), Some(ex)) => crate::embedding::cosine_similarity(ca, ex),
                    _ => 0.0,
                };
                let lex_sim = audn_lexical_jaccard(&existing.statement, &cleaned_stmt);
                let score = mh_sim.max(cos_sim).max(lex_sim);
                if best_target.map_or(true, |(_, _, _, b)| score > b) {
                    best_target = Some((existing, cos_sim, mh_sim, score));
                }
            }

            if let Some((target, cos_sim, mh_sim, _)) = best_target {
                let mut deprecated = target.clone();
                deprecated.status = strata_core::schemas::FactStatus::Deprecated;
                deprecated.last_updated_at = Utc::now();
                store.insert_or_update_semantic_fact(&deprecated)?;
                let _ = self.jtms.propagate_downstream_invalidation(
                    store,
                    &target.id,
                    "Fact retracted via AUDN Delete directive",
                );
                return Ok(AudnDecision {
                    operation: AudnOperation::Delete {
                        target_id: target.id,
                    },
                    reason: format!(
                        "Retraction/deletion directive deprecated semantic fact {}",
                        target.id
                    ),
                    cosine_similarity: cos_sim,
                    minhash_similarity: mh_sim,
                    record: None,
                });
            }
        }

        // 2. Check Noop and Update against active facts
        let mut best_update: Option<(&SemanticFact, String, f32, f32, f32)> = None;
        let mut max_cos_sim = 0.0f32;
        let mut max_mh_sim = 0.0f32;

        for (existing, ex_emb) in &eligible {
            let ex_sig = compute_minhash_signature(&existing.statement, self.config.num_hashes);
            let mh_sim = minhash_jaccard_similarity(&cand_sig, &ex_sig);
            let cos_sim = match (&cand_emb, ex_emb) {
                (Some(ca), Some(ex)) => crate::embedding::cosine_similarity(ca, ex),
                _ => 0.0,
            };

            if cos_sim > max_cos_sim {
                max_cos_sim = cos_sim;
            }
            if mh_sim > max_mh_sim {
                max_mh_sim = mh_sim;
            }

            let ex_norm = canonical_audn_words(&existing.statement).join(" ");
            let norm_equal = !cand_norm.is_empty() && cand_norm == ex_norm;

            if let Some(update_reason) =
                self.detect_update_cues(&existing.statement, &candidate.statement, None)
            {
                if has_audn_topic_overlap(
                    &existing.statement,
                    &existing.tags,
                    &candidate.statement,
                    &candidate.tags,
                ) {
                    let lex_sim = audn_lexical_jaccard(&existing.statement, &candidate.statement);
                    let score = mh_sim.max(cos_sim).max(lex_sim);
                    if best_update.as_ref().map_or(true, |(_, _, _, _, b)| score > *b) {
                        best_update = Some((existing, update_reason, cos_sim, mh_sim, score));
                    }
                    continue;
                }
            }

            if norm_equal
                || mh_sim >= self.config.minhash_noop_threshold
                || cos_sim > self.config.cosine_noop_threshold
            {
                let reason = if norm_equal {
                    format!("Normalized lexical match with existing fact {}", existing.id)
                } else if cos_sim > self.config.cosine_noop_threshold {
                    format!(
                        "Embedding cosine similarity {:.3} > {:.3} threshold with fact {}",
                        cos_sim, self.config.cosine_noop_threshold, existing.id
                    )
                } else {
                    format!(
                        "MinHash Jaccard similarity {:.3} >= {:.3} threshold with fact {}",
                        mh_sim, self.config.minhash_noop_threshold, existing.id
                    )
                };

                return Ok(AudnDecision {
                    operation: AudnOperation::Noop {
                        existing_id: existing.id,
                    },
                    reason,
                    cosine_similarity: cos_sim,
                    minhash_similarity: mh_sim,
                    record: Some(semantic_fact_to_memory_record(existing, ex_emb.clone())),
                });
            }
        }

        if let Some((target, reason, cos_sim, mh_sim, _)) = best_update {
            let mut updated = target.clone();
            updated.statement = candidate.statement.clone();
            updated.version = target.version + 1;
            updated.importance = target.importance.max(candidate.importance);
            updated.confidence = candidate.confidence;
            updated.last_updated_at = Utc::now();
            for tag in &candidate.tags {
                if !updated.tags.contains(tag) {
                    updated.tags.push(tag.clone());
                }
            }
            for ev in &candidate.evidence {
                if !updated.evidence.iter().any(|e| e.source_id == ev.source_id) {
                    updated.evidence.push(ev.clone());
                }
            }

            store.insert_or_update_semantic_fact(&updated)?;
            if let Some(ref emb) = cand_emb {
                store.update_semantic_fact_embedding(&updated.id, emb)?;
            }
            let _ = self.jtms.propagate_downstream_invalidation(
                store,
                &updated.id,
                "Prerequisite fact updated in place via AUDN pipeline",
            );

            return Ok(AudnDecision {
                operation: AudnOperation::Update {
                    target_id: updated.id,
                },
                reason,
                cosine_similarity: cos_sim,
                minhash_similarity: mh_sim,
                record: Some(semantic_fact_to_memory_record(&updated, cand_emb)),
            });
        }

        // 3. Add novel fact
        let mut to_write = candidate.clone();
        to_write.status = strata_core::schemas::FactStatus::Active;
        store.insert_or_update_semantic_fact(&to_write)?;
        if let Some(ref emb) = cand_emb {
            store.update_semantic_fact_embedding(&to_write.id, emb)?;
        }

        Ok(AudnDecision {
            operation: AudnOperation::Add,
            reason: "Novel semantic fact with no existing duplicate or conflict in scope"
                .to_string(),
            cosine_similarity: max_cos_sim,
            minhash_similarity: max_mh_sim,
            record: Some(semantic_fact_to_memory_record(&to_write, cand_emb)),
        })
    }
}

fn semantic_fact_to_memory_record(
    fact: &SemanticFact,
    embedding: Option<Vec<f32>>,
) -> strata_core::state::MemoryRecord {
    let mut record = strata_core::state::MemoryRecord::new(
        strata_core::state::MemoryType::Semantic,
        fact.statement.clone(),
        fact.scope.clone(),
    )
    .with_tier(fact.tier)
    .with_human_approval(fact.approved_by_human)
    .with_importance(fact.importance)
    .with_confidence(fact.confidence)
    .with_tags(fact.tags.clone());
    record.id = fact.id;
    record.created_at = fact.created_at;
    record.updated_at = fact.last_updated_at;
    record.embedding = embedding;
    record.metadata = serde_json::json!({
        "category": fact.category,
        "version": fact.version,
        "status": fact.status.to_string(),
    });
    record
}

#[cfg(test)]
mod audn_tests {
    use super::*;
    use crate::embedding::MockEmbeddingProvider;
    use strata_core::schemas::FactStatus;
    use strata_core::state::{MemoryRecord, MemoryType, Scope};

    #[test]
    fn test_minhash_signature_determinism_and_near_duplicate_similarity() {
        let text_a = "The authentication service uses Ed25519 signatures for verifying agent tokens.";
        let text_b = "Authentication service uses Ed25519 signatures for verifying agent tokens!";
        let text_c = "Frontend UI components use Tailwind CSS utility classes for responsive layout.";

        let sig_a1 = compute_minhash_signature(text_a, 64);
        let sig_a2 = compute_minhash_signature(text_a, 64);
        assert_eq!(sig_a1.len(), 64);
        assert_eq!(sig_a1, sig_a2, "MinHash signatures must be deterministic");

        let sig_b = compute_minhash_signature(text_b, 64);
        let sim_ab = minhash_jaccard_similarity(&sig_a1, &sig_b);
        assert!(
            sim_ab >= 0.85,
            "Near-identical statements should have MinHash similarity >= 0.85, got {sim_ab}"
        );

        let sig_c = compute_minhash_signature(text_c, 64);
        let sim_ac = minhash_jaccard_similarity(&sig_a1, &sig_c);
        assert!(
            sim_ac < 0.25,
            "Unrelated statements should have low MinHash similarity, got {sim_ac}"
        );
    }

    #[tokio::test]
    async fn test_audn_noop_silently_discards_identical_and_near_duplicate_memories() {
        let store = SqliteStore::open_in_memory().expect("in-memory store");
        let embedder = MockEmbeddingProvider::default();
        let audn = AudnPipeline::with_default_config();
        let scope = Scope::Project("strata".to_string());

        let m1 = MemoryRecord::new(
            MemoryType::Semantic,
            "SQLite WAL mode with synchronous NORMAL ensures ACID safety and concurrent reads.",
            scope.clone(),
        )
        .with_tags(vec!["sqlite".to_string(), "storage".to_string()]);

        let dec1 = audn
            .ingest_memory(&store, Some(&embedder), &m1)
            .await
            .expect("ingest m1");
        assert_eq!(dec1.operation, AudnOperation::Add);
        assert_eq!(store.get_all_memories(None, None, 100).unwrap().len(), 1);

        // 1. Exact duplicate
        let m2_exact = MemoryRecord::new(
            MemoryType::Semantic,
            "SQLite WAL mode with synchronous NORMAL ensures ACID safety and concurrent reads.",
            scope.clone(),
        );
        let dec2 = audn
            .ingest_memory(&store, Some(&embedder), &m2_exact)
            .await
            .expect("ingest m2_exact");
        assert_eq!(
            dec2.operation,
            AudnOperation::Noop {
                existing_id: m1.id
            }
        );
        assert_eq!(
            store.get_all_memories(None, None, 100).unwrap().len(),
            1,
            "SQLite row count must not grow on exact duplicate Noop"
        );

        // 2. Near-duplicate via MinHash (punctuation/article difference)
        let m3_minhash = MemoryRecord::new(
            MemoryType::Semantic,
            "The SQLite WAL mode with synchronous NORMAL ensures ACID safety and concurrent reads!",
            scope.clone(),
        );
        let dec3 = audn
            .ingest_memory(&store, None, &m3_minhash)
            .await
            .expect("ingest m3_minhash");
        assert_eq!(
            dec3.operation,
            AudnOperation::Noop {
                existing_id: m1.id
            }
        );
        assert!(dec3.minhash_similarity >= 0.85);
        assert_eq!(
            store.get_all_memories(None, None, 100).unwrap().len(),
            1,
            "SQLite row count must not grow on MinHash near-duplicate Noop"
        );

        // 3. Near-duplicate via embedding cosine similarity > 0.92 even when lexical phrasing differs
        let mut base_vec = vec![0.0f32; 384];
        base_vec[0] = 0.98;
        base_vec[1] = 0.20;
        let mut m_base = MemoryRecord::new(
            MemoryType::Semantic,
            "All background tasks must acquire an exclusive lease before modifying workspace files.",
            scope.clone(),
        );
        m_base.embedding = Some(base_vec);
        let dec_base = audn
            .ingest_memory(&store, None, &m_base)
            .await
            .expect("ingest m_base");
        assert_eq!(dec_base.operation, AudnOperation::Add);
        assert_eq!(store.get_all_memories(None, None, 100).unwrap().len(), 2);

        let mut near_vec = vec![0.0f32; 384];
        near_vec[0] = 0.96;
        near_vec[1] = 0.28; // cosine similarity with base_vec ~ 0.996 > 0.92
        let mut m_cos_dup = MemoryRecord::new(
            MemoryType::Semantic,
            "Concurrent worker agents are required to hold a resource lock prior to editing code.",
            scope.clone(),
        );
        m_cos_dup.embedding = Some(near_vec);

        let dec_cos = audn
            .ingest_memory(&store, None, &m_cos_dup)
            .await
            .expect("ingest m_cos_dup");
        assert_eq!(
            dec_cos.operation,
            AudnOperation::Noop {
                existing_id: m_base.id
            }
        );
        assert!(
            dec_cos.cosine_similarity > 0.92,
            "Expected cosine_similarity > 0.92, got {}",
            dec_cos.cosine_similarity
        );
        assert_eq!(
            store.get_all_memories(None, None, 100).unwrap().len(),
            2,
            "SQLite row count must not grow on cosine > 0.92 near-duplicate Noop"
        );
    }

    #[tokio::test]
    async fn test_audn_update_modifies_existing_memory_in_place() {
        let store = SqliteStore::open_in_memory().expect("in-memory store");
        let embedder = MockEmbeddingProvider::default();
        let audn = AudnPipeline::with_default_config();
        let scope = Scope::Project("backend".to_string());

        let m_initial = MemoryRecord::new(
            MemoryType::Semantic,
            "Primary database uses SQLite for storage",
            scope.clone(),
        )
        .with_tags(vec!["database".to_string()]);

        let dec1 = audn
            .ingest_memory(&store, Some(&embedder), &m_initial)
            .await
            .expect("ingest initial");
        assert_eq!(dec1.operation, AudnOperation::Add);

        // Superseding technology migration on same topic
        let m_update = MemoryRecord::new(
            MemoryType::Semantic,
            "Primary database migrated to PostgreSQL instead of SQLite for storage",
            scope.clone(),
        )
        .with_tags(vec!["database".to_string(), "postgres".to_string()]);

        let dec2 = audn
            .ingest_memory(&store, Some(&embedder), &m_update)
            .await
            .expect("ingest update");
        assert_eq!(
            dec2.operation,
            AudnOperation::Update {
                target_id: m_initial.id
            }
        );

        let all = store.get_all_memories(None, None, 100).unwrap();
        assert_eq!(all.len(), 1, "Update must modify existing record in place");
        let updated = &all[0];
        assert_eq!(updated.id, m_initial.id);
        assert_eq!(
            updated.content,
            "Primary database migrated to PostgreSQL instead of SQLite for storage"
        );
        assert_eq!(
            updated.metadata.get("version").and_then(|v| v.as_u64()),
            Some(2)
        );
        assert!(updated.tags.contains(&"postgres".to_string()));

        // Numeric parameter update on same template
        let m_pool_v1 = MemoryRecord::new(
            MemoryType::Semantic,
            "Database connection pool max size is 10",
            scope.clone(),
        );
        let dec_p1 = audn
            .ingest_memory(&store, Some(&embedder), &m_pool_v1)
            .await
            .expect("ingest pool v1");
        assert_eq!(dec_p1.operation, AudnOperation::Add);

        let m_pool_v2 = MemoryRecord::new(
            MemoryType::Semantic,
            "Database connection pool max size is 50",
            scope.clone(),
        );
        let dec_p2 = audn
            .ingest_memory(&store, Some(&embedder), &m_pool_v2)
            .await
            .expect("ingest pool v2");
        assert_eq!(
            dec_p2.operation,
            AudnOperation::Update {
                target_id: m_pool_v1.id
            }
        );
        let loaded_pool = store.get_memory(&m_pool_v1.id).unwrap().unwrap();
        assert_eq!(loaded_pool.content, "Database connection pool max size is 50");
        assert_eq!(
            loaded_pool.metadata.get("version").and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn test_audn_delete_retracts_obsolete_memory() {
        let store = SqliteStore::open_in_memory().expect("in-memory store");
        let embedder = MockEmbeddingProvider::default();
        let audn = AudnPipeline::with_default_config();
        let scope = Scope::Project("api".to_string());

        let m_legacy = MemoryRecord::new(
            MemoryType::Semantic,
            "Legacy XML SOAP endpoint requires Basic Auth headers",
            scope.clone(),
        )
        .with_tags(vec!["soap".to_string()]);

        let dec_add = audn
            .ingest_memory(&store, Some(&embedder), &m_legacy)
            .await
            .expect("ingest legacy");
        assert_eq!(dec_add.operation, AudnOperation::Add);
        assert_eq!(store.get_all_memories(None, None, 100).unwrap().len(), 1);

        let m_retract = MemoryRecord::new(
            MemoryType::Semantic,
            "Retract: Legacy XML SOAP endpoint requires Basic Auth headers",
            scope.clone(),
        );

        let dec_del = audn
            .ingest_memory(&store, Some(&embedder), &m_retract)
            .await
            .expect("ingest retract");
        assert_eq!(
            dec_del.operation,
            AudnOperation::Delete {
                target_id: m_legacy.id
            }
        );
        assert!(dec_del.record.is_none());
        assert!(
            store.get_memory(&m_legacy.id).unwrap().is_none(),
            "Deleted memory must be removed from SQLite store"
        );
        assert_eq!(store.get_all_memories(None, None, 100).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_audn_fact_ingestion_lifecycle() {
        let store = SqliteStore::open_in_memory().expect("in-memory store");
        let embedder = MockEmbeddingProvider::default();
        let audn = AudnPipeline::with_default_config();
        let scope = Scope::Project("core".to_string());

        // 1. Add
        let f1 = SemanticFact::new(
            "Internal RPC transport uses REST over JSON",
            "architecture",
            scope.clone(),
        );
        let d1 = audn
            .ingest_fact(&store, Some(&embedder), &f1)
            .await
            .expect("add f1");
        assert_eq!(d1.operation, AudnOperation::Add);

        // 2. Noop on identical fact
        let f1_dup = SemanticFact::new(
            "Internal RPC transport uses REST over JSON!",
            "architecture",
            scope.clone(),
        );
        let d2 = audn
            .ingest_fact(&store, Some(&embedder), &f1_dup)
            .await
            .expect("noop f1_dup");
        assert_eq!(
            d2.operation,
            AudnOperation::Noop {
                existing_id: f1.id
            }
        );
        assert_eq!(
            store
                .get_all_semantic_facts(None, Some(FactStatus::Active), 100)
                .unwrap()
                .len(),
            1
        );

        // 3. Update on migration
        let f1_upd = SemanticFact::new(
            "Internal RPC transport switched to gRPC with Protobuf",
            "architecture",
            scope.clone(),
        );
        let d3 = audn
            .ingest_fact(&store, Some(&embedder), &f1_upd)
            .await
            .expect("update f1");
        assert_eq!(
            d3.operation,
            AudnOperation::Update {
                target_id: f1.id
            }
        );
        let updated_fact = store.get_semantic_fact(&f1.id).unwrap().unwrap();
        assert_eq!(
            updated_fact.statement,
            "Internal RPC transport switched to gRPC with Protobuf"
        );
        assert_eq!(updated_fact.version, 2);

        // 4. Delete on retraction
        let f1_del = SemanticFact::new(
            "Obsolete: Internal RPC transport switched to gRPC with Protobuf is no longer valid",
            "architecture",
            scope.clone(),
        );
        let d4 = audn
            .ingest_fact(&store, Some(&embedder), &f1_del)
            .await
            .expect("delete f1");
        assert_eq!(
            d4.operation,
            AudnOperation::Delete {
                target_id: f1.id
            }
        );
        assert_eq!(
            store
                .get_all_semantic_facts(None, Some(FactStatus::Active), 100)
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn test_sqlite_memory_engine_ingest_audn_and_classify_memory() {
        let engine = crate::SqliteMemoryEngine::open_in_memory(None).expect("open engine");
        let audn = AudnPipeline::with_default_config();
        let scope = Scope::Project("engine-test".to_string());

        let m1 = MemoryRecord::new(
            MemoryType::Semantic,
            "Cache layer uses Redis for distributed session storage",
            scope.clone(),
        )
        .with_tags(vec!["cache".to_string()]);

        // Classify before insert -> Add
        let pre_class = audn.classify_memory(engine.store(), &m1).expect("classify");
        assert_eq!(pre_class.operation, AudnOperation::Add);

        // Ingest via engine -> Add
        let dec1 = engine.ingest_audn(&m1).await.expect("ingest m1");
        assert_eq!(dec1.operation, AudnOperation::Add);
        assert_eq!(
            engine.store().get_all_memories(None, None, 100).unwrap().len(),
            1
        );

        // Ingest identical via engine -> Noop
        let m1_dup = MemoryRecord::new(
            MemoryType::Semantic,
            "The cache layer uses Redis for distributed session storage.",
            scope.clone(),
        );
        let dec_noop = engine.ingest_audn(&m1_dup).await.expect("ingest m1_dup");
        assert_eq!(
            dec_noop.operation,
            AudnOperation::Noop {
                existing_id: m1.id
            }
        );
        assert_eq!(
            engine.store().get_all_memories(None, None, 100).unwrap().len(),
            1
        );

        // Ingest update via engine -> Update
        let m1_upd = MemoryRecord::new(
            MemoryType::Semantic,
            "Cache layer uses Memcached",
            scope.clone(),
        )
        .with_tags(vec!["cache".to_string()]);
        let dec_upd = engine.ingest_audn(&m1_upd).await.expect("ingest m1_upd");
        assert_eq!(
            dec_upd.operation,
            AudnOperation::Update {
                target_id: m1.id
            }
        );
        let stored = engine.store().get_memory(&m1.id).unwrap().unwrap();
        assert_eq!(stored.content, "Cache layer uses Memcached");

        // Ingest delete via engine -> Delete
        let m1_del = MemoryRecord::new(
            MemoryType::Semantic,
            "Delete: Cache layer uses Memcached",
            scope.clone(),
        );
        let dec_del = engine.ingest_audn(&m1_del).await.expect("ingest m1_del");
        assert_eq!(
            dec_del.operation,
            AudnOperation::Delete {
                target_id: m1.id
            }
        );
        assert_eq!(
            engine.store().get_all_memories(None, None, 100).unwrap().len(),
            0
        );
    }
}


