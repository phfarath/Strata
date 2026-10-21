pub mod alignment;
pub mod ast;
pub mod call_graph;
pub mod clustering;
pub mod community;
pub mod compiler;
pub mod consolidation;
pub mod context_rot;
pub mod decay;
pub mod embedding;
pub mod enricher;
pub mod futility;
pub mod gating;
pub mod ipc;
pub mod jtms;
pub mod leases;
pub mod metaplasticity;
pub mod pipeline;
pub mod procedural_mining;
pub mod pyramid;
pub mod retrieval;
pub mod spreading_activation;
pub mod store;
pub mod subconscious;
pub mod sync;
pub mod workspace;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::events::{Event, EventId};
use strata_core::state::{
    DigestOutput, FailurePattern, MemoryHandle, MemoryRecord, MemoryTier, Scope,
};
pub use strata_core::state::{HumanApprovalRecord, ProvenanceBlock};
use strata_core::traits::{EventStore, MemoryEngine};

pub use alignment::{AlignmentDatasetReport, PreferenceMiner};
pub use ast::{
    AstDiffResult, AstParser, CodeAnchorEngine, ExtractedSymbol, LanguageKind, ReconciliationReport,
};
pub use call_graph::{
    CallEdge, CallGraph, CallGraphAnalyzer, CallType, CrossFileResolver, HeuristicIndexReport,
    HeuristicIndexer,
};
pub use clustering::{SpatialCluster, SpatialClusterer, VectorPoint};
pub use community::{
    ArchitectureCluster, ArchitectureGraphSummary, ClusterAssignmentCache, ClusterDependency,
    ClusterMember, ClusterMigrationPlan, ClusteringConfig, CommunityDetector,
    CommunityMemoryRouter, CommunityScopedRecallResult, MemberType,
};
pub use compiler::{
    estimate_tokens, glob_to_slug, matches_path_glob, CoreRule, HostCompileResult,
    MultiHostCompileReport, MultiHostCompiler, PathScopedExt, SemanticMemory, STRATA_MARKER_END,
    STRATA_MARKER_START,
};
pub use consolidation::{
    parse_dream_duration, Consolidator, DreamConfig, DreamEngine, DreamReport, DreamTriggerReason,
    NeuroSymbolicConsolidator,
};
pub use context_rot::{
    ContextRotConfig, ContextRotDetector, InjectionOutcome, MemoryEfficacyMetric,
    MemoryInjectionRecord,
};
pub use decay::{DecayCalculator, PruneReport};
pub use embedding::{
    are_embedding_models_compatible, bytes_to_embedding, cosine_similarity, embedding_to_bytes,
    is_embedding_compatible_with_query, EmbeddingProvider, EmbeddingProviderFactory,
    EmbeddingProviderKind, FastEmbedProvider, MockEmbeddingProvider, ResolvedEmbeddingProvider,
};
pub use enricher::{AsyncLlmEnricher, CanonicalTemplateEnricher, MemoryEnricher};
pub use futility::{
    record_shadow_failure, record_shadow_success, FutilityAccumulator, FutilityTraceRow,
    Signal as FutilitySignal, Verdict as FutilityVerdict,
};
pub use gating::{should_recall, IntentKind, RecallDecision, RecallGatingClassifier, TaskType};
pub use ipc::{
    endpoint_for_workspace, read_bounded_line, IpcBrokerManager, IpcClient, IpcServer,
    MAX_FRAME_LENGTH,
};
pub use jtms::{ConflictMatch, ConflictResolution, TruthMaintenanceSystem};
pub use leases::StigmergyCoordinator;
pub use metaplasticity::{
    calculate_adapted_bypass_threshold, calculate_adapted_decay_stability,
    calculate_adapted_prune_threshold, MetaplasticityConfig, MetaplasticityState,
};
pub use pipeline::{
    compute_minhash_signature, minhash_jaccard_similarity, AudnConfig, AudnDecision, AudnOperation,
    AudnPipeline, ConsolidationPipeline, ConsolidationResult, PipelineConfig,
};
pub use procedural_mining::TrajectoryMiner;
pub use pyramid::{
    DrillDownTree, EpisodicPyramidEngine, EpisodicPyramidStore, PyramidConfig,
    PyramidDrillDownReport, PyramidFilter, RollupSummary, SlmBridgeConfig, SlmInferenceResult,
    SlmPyramidBridge,
};
pub use retrieval::{
    import_distance_decay_factor, HybridRanker, HybridRankerConfig, ProximityBiasConfig,
    ProximityBiasEngine,
};
pub use spreading_activation::{
    EdgeKind, GraphEdge, GraphNode, KnowledgeGraph, SpreadingActivationConfig,
    SpreadingActivationEngine,
};
pub use store::{
    MemoryEdgeRecord, ModelVectorCount, ReindexReport, SqliteStore, VectorConsistencyStats,
};
pub use strata_core::schemas::{
    CodeAnchor, ContextBudgetConfig, ExportFormat, FeedbackEvent, FeedbackRating, HostTargetConfig,
    ImplicitSignal, KtoSample, MemoryFeedback, PreferencePair, SemanticFact, SftSample, SignalKind,
    SymbolType, TransferFilter, TransferReport,
};
pub use strata_reasoning::{
    ProceduralOptimizerConfig, ProceduralRule, ProceduralRuleVersion, ProceduralSelfEditor,
    TextGradientFeedback,
};
pub use subconscious::{GatingDecision, SubconsciousBuffer, SubconsciousConfig};
pub use sync::{calculate_exponential_backoff, compute_version_hash, SyncEngine};
pub use workspace::{MonorepoPackage, PackageType, WorkspaceBoundary, WorkspaceBoundaryDetector};
pub type DpoPair = PreferencePair;

/// SQLite-backed persistent memory engine implementing `MemoryEngine` and `EventStore`.
/// Supports optional two-tier federation across a local workspace store and a developer-global store.
pub struct SqliteMemoryEngine {
    store: Arc<SqliteStore>,
    global_store: Option<Arc<SqliteStore>>,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    ranker: HybridRanker,
    consolidator: Consolidator,
}

impl SqliteMemoryEngine {
    /// Create a new `SqliteMemoryEngine` with an SQLite database file.
    pub fn open<P: AsRef<Path>>(
        path: P,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self, StrataError> {
        let store = Arc::new(SqliteStore::open(path)?);
        let embedder: Arc<dyn EmbeddingProvider> =
            embedding_provider.unwrap_or_else(|| Arc::new(MockEmbeddingProvider::default()));

        Ok(Self {
            store,
            global_store: None,
            embedding_provider: embedder,
            ranker: HybridRanker::with_default_config(),
            consolidator: Consolidator::new(),
        })
    }

    /// Create an in-memory `SqliteMemoryEngine` for testing or temporary execution.
    pub fn open_in_memory(
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self, StrataError> {
        let store = Arc::new(SqliteStore::open_in_memory()?);
        let embedder: Arc<dyn EmbeddingProvider> =
            embedding_provider.unwrap_or_else(|| Arc::new(MockEmbeddingProvider::default()));

        Ok(Self {
            store,
            global_store: None,
            embedding_provider: embedder,
            ranker: HybridRanker::with_default_config(),
            consolidator: Consolidator::new(),
        })
    }

    /// Create a two-tier federated `SqliteMemoryEngine` connecting a local workspace store
    /// and an optional developer-global store (`~/.strata/global.db`).
    pub fn open_federated<P1: AsRef<Path>, P2: AsRef<Path>>(
        local_path: P1,
        global_path: Option<P2>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self, StrataError> {
        let store = Arc::new(SqliteStore::open(local_path)?);
        let global_store = if let Some(gp) = global_path {
            Some(Arc::new(SqliteStore::open(gp)?))
        } else {
            None
        };
        let embedder: Arc<dyn EmbeddingProvider> =
            embedding_provider.unwrap_or_else(|| Arc::new(MockEmbeddingProvider::default()));

        Ok(Self {
            store,
            global_store,
            embedding_provider: embedder,
            ranker: HybridRanker::with_default_config(),
            consolidator: Consolidator::new(),
        })
    }

    /// Attach or replace the global federated store.
    pub fn with_global_store(mut self, global_store: Arc<SqliteStore>) -> Self {
        self.global_store = Some(global_store);
        self
    }

    /// Get a reference to the primary local `SqliteStore`.
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// Get an Arc reference to the primary local `SqliteStore`.
    pub fn store_arc(&self) -> Arc<SqliteStore> {
        Arc::clone(&self.store)
    }

    /// Get a reference to the optional federated global `SqliteStore`.
    pub fn global_store(&self) -> Option<&SqliteStore> {
        self.global_store.as_deref()
    }

    /// Get an Arc reference to the optional federated global `SqliteStore`.
    pub fn global_store_arc(&self) -> Option<Arc<SqliteStore>> {
        self.global_store.as_ref().map(Arc::clone)
    }

    /// Access the StigmergyCoordinator for agent presence, heartbeats, and atomic leases.
    pub fn stigmergy(&self) -> StigmergyCoordinator {
        StigmergyCoordinator::new(Arc::clone(&self.store))
    }

    /// Get the active embedding provider.
    pub fn embedding_provider(&self) -> Arc<dyn EmbeddingProvider> {
        Arc::clone(&self.embedding_provider)
    }

    /// Record a tool failure and consolidate into known failure patterns.
    pub async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_msg: &str,
        context: &str,
        scope: Option<&Scope>,
    ) -> Result<FailurePattern, StrataError> {
        self.consolidator
            .record_tool_failure(&self.store, tool_name, error_msg, context, scope)
    }

    /// Queries active anti-patterns anchored to a specific file and optional symbol.
    pub async fn get_symbol_anti_patterns(
        &self,
        file: &str,
        symbol: Option<&str>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        let mut results = self.store.find_symbol_anti_patterns(file, symbol, limit)?;
        if let Some(ref global) = self.global_store {
            let global_results = global.find_symbol_anti_patterns(file, symbol, limit)?;
            let mut seen_signatures: std::collections::HashSet<String> =
                results.iter().map(|f| f.signature.clone()).collect();
            for gf in global_results {
                if seen_signatures.insert(gf.signature.clone()) {
                    results.push(gf);
                }
            }
            results.truncate(limit);
        }
        Ok(results)
    }

    /// Automatically detects monorepo workspace boundaries in the repository.
    pub fn detect_workspace_boundaries(
        &self,
        root_dir: &Path,
    ) -> Result<WorkspaceBoundary, StrataError> {
        WorkspaceBoundaryDetector::detect(root_dir)
    }

    /// Performs hierarchical, package-isolated memory search for a specific file.
    pub async fn search_scoped_to_file(
        &self,
        query: &str,
        file_path: &str,
        boundary: Option<&WorkspaceBoundary>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let hierarchical_scopes = if let Some(b) = boundary {
            b.get_hierarchical_scopes(file_path)
        } else {
            vec![Scope::Global]
        };

        let mut combined_results = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        // Search in hierarchical order (package scope first, then internal deps, then global)
        for scope in &hierarchical_scopes {
            let res = self.search(query, Some(scope), limit).await?;
            for r in res {
                if seen_ids.insert(r.id) {
                    combined_results.push(r);
                    if combined_results.len() >= limit {
                        break;
                    }
                }
            }
            if combined_results.len() >= limit {
                break;
            }
        }

        Ok(combined_results)
    }

    /// Promotes a memory record to Core Tier with verified human approval metadata.
    pub async fn promote_to_core_with_approval(
        &self,
        id: &Uuid,
        approval: HumanApprovalRecord,
    ) -> Result<MemoryRecord, StrataError> {
        self.store
            .promote_memory_to_core_with_approval(id, approval)
    }

    /// Promotes a memory record to Core Tier with explicit human approval.
    pub async fn promote_to_core(
        &self,
        id: &Uuid,
        approved_by_human: bool,
        reason: Option<&str>,
    ) -> Result<MemoryRecord, StrataError> {
        self.store
            .promote_memory_to_core(id, approved_by_human, reason)
    }

    /// Promotes a semantic fact to Core Tier with verified human approval metadata.
    pub async fn promote_fact_to_core_with_approval(
        &self,
        id: &Uuid,
        approval: HumanApprovalRecord,
    ) -> Result<SemanticFact, StrataError> {
        self.store
            .promote_semantic_fact_to_core_with_approval(id, approval)
    }

    /// Promotes a semantic fact to Core Tier with explicit human approval.
    pub async fn promote_fact_to_core(
        &self,
        id: &Uuid,
        approved_by_human: bool,
        reason: Option<&str>,
    ) -> Result<SemanticFact, StrataError> {
        self.store
            .promote_semantic_fact_to_core(id, approved_by_human, reason)
    }

    /// Performs associative Knowledge Graph retrieval via Spreading Activation (HippoRAG / ACT-R style).
    /// Discovers multi-hop connected memories, files, and root causes with zero LLM tokens.
    pub fn search_graph_associative(
        &self,
        query: &str,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<(MemoryRecord, f32)>, StrataError> {
        let spreading_engine = self.ranker.spreading_engine();
        let graph = spreading_engine.build_graph_from_store(&self.store, scope)?;
        let seeds = spreading_engine.seed_from_query(&graph, query);
        if seeds.is_empty() {
            return Ok(Vec::new());
        }

        let activations = spreading_engine.propagate(&graph, &seeds);
        let top_memories = spreading_engine.extract_top_memories(&activations, limit);

        let mut results = Vec::new();
        for (id, score) in top_memories {
            if let Some(mem) = self.store.get_memory(&id)? {
                results.push((mem, score));
            }
        }
        Ok(results)
    }

    /// Promotes a memory record to the Developer-Global store (~/.strata/global.db) with verified human approval.
    pub async fn promote_to_global_with_approval(
        &self,
        id: &Uuid,
        approval: HumanApprovalRecord,
    ) -> Result<MemoryRecord, StrataError> {
        let global = self.global_store.as_ref().ok_or_else(|| {
            StrataError::Configuration(
                "No global store configured for federation (global_store is None)".to_string(),
            )
        })?;

        let mut record = self.store.get_memory(id)?.ok_or_else(|| {
            StrataError::NotFound(format!("Memory record {id} not found in local store"))
        })?;

        if !approval.matches_content(&record.content) {
            return Err(StrataError::Validation(format!(
                "Content hash mismatch for memory '{id}': expected approval hash '{}', but actual content hash is '{}'. Memory content may have changed.",
                approval.content_hash,
                HumanApprovalRecord::compute_content_hash(&record.content),
            )));
        }

        record.scope = Scope::Global;
        record.tier = MemoryTier::Core;
        record.approved_by_human = true;
        record.human_approval = Some(approval.clone());
        record.updated_at = chrono::Utc::now();

        let mut meta = record.metadata.clone();
        if let Some(ref r) = approval.rationale {
            meta["promotion_reason"] = serde_json::Value::String(r.clone());
        }
        meta["promoted_to_global_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
        meta["human_approval"] = serde_json::to_value(&approval).unwrap_or(serde_json::json!({}));
        record.metadata = meta;

        if record.embedding.is_none() && !record.content.trim().is_empty() {
            if let Ok(emb) = self.embedding_provider.embed_text(&record.content).await {
                record.embedding = Some(emb);
                record.embedding_model = Some(self.embedding_provider.model_name().to_string());
                record.embedding_version =
                    Some(self.embedding_provider.model_version().to_string());
            }
        }

        global.insert_or_update_memory(&record)?;
        let _ = self.store.insert_or_update_memory(&record);

        Ok(record)
    }

    /// Promotes a memory record to the Developer-Global store (~/.strata/global.db) with Scope::Global.
    /// Requires explicit human approval.
    pub async fn promote_to_global(
        &self,
        id: &Uuid,
        approved_by_human: bool,
        reason: Option<&str>,
    ) -> Result<MemoryRecord, StrataError> {
        if !approved_by_human {
            return Err(StrataError::Validation(
                "Cannot promote memory to Global Tier without explicit human approval (approved_by_human=true)".to_string(),
            ));
        }

        let mem = self.store.get_memory(id)?.ok_or_else(|| {
            StrataError::NotFound(format!("Memory record {id} not found in local store"))
        })?;

        let approval = HumanApprovalRecord::new(
            "cli_user",
            "cli_direct",
            &mem.content,
            reason.map(String::from),
        );

        self.promote_to_global_with_approval(id, approval).await
    }

    /// Generates a single-use verifiable approval token for an entity.
    pub fn generate_approval_token(
        &self,
        entity_id: &Uuid,
        entity_type: &str,
        content_hash: &str,
        approved_by: &str,
        ttl_seconds: u64,
        rationale: Option<&str>,
    ) -> Result<String, StrataError> {
        self.store.generate_approval_token(
            entity_id,
            entity_type,
            content_hash,
            approved_by,
            ttl_seconds,
            rationale,
        )
    }

    /// Verifies and consumes a verifiable approval token.
    pub fn verify_and_consume_approval_token(
        &self,
        token_str: &str,
        expected_entity_id: &Uuid,
        expected_content_hash: &str,
    ) -> Result<HumanApprovalRecord, StrataError> {
        match self.store.verify_and_consume_approval_token(
            token_str,
            expected_entity_id,
            expected_content_hash,
        ) {
            Ok(rec) => Ok(rec),
            Err(e) => {
                if let Some(ref global) = self.global_store {
                    global.verify_and_consume_approval_token(
                        token_str,
                        expected_entity_id,
                        expected_content_hash,
                    )
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Promotes a FailurePattern to the Developer-Global store (~/.strata/global.db).
    pub fn promote_failure_to_global(&self, id: &Uuid) -> Result<FailurePattern, StrataError> {
        let global = self.global_store.as_ref().ok_or_else(|| {
            StrataError::Configuration("No global store configured for federation".to_string())
        })?;

        let mut failure = self.store.get_failure_pattern(id)?.ok_or_else(|| {
            StrataError::NotFound(format!("Failure pattern {id} not found in local store"))
        })?;

        failure.scope = Scope::Global;
        global.upsert_failure_pattern(&failure)?;
        let _ = self.store.upsert_failure_pattern(&failure);

        Ok(failure)
    }

    /// Directly transfers memories, failure patterns, and procedural skills from another repository's store.
    pub fn transfer_from(
        &self,
        source_store: &SqliteStore,
        filter: &TransferFilter,
        source_label: &str,
    ) -> Result<TransferReport, StrataError> {
        let mut report = TransferReport {
            source_path: source_label.to_string(),
            ..Default::default()
        };

        // 1. Transfer memories
        let types_slice = filter.memory_type.clone().map(|t| vec![t]);
        let candidates =
            source_store.get_all_memories(None, types_slice.as_deref(), filter.limit)?;

        for mut memory in candidates {
            if let Some(tier) = filter.tier {
                if memory.tier != tier {
                    continue;
                }
            }
            if let Some(ref q) = filter.query {
                let q_lower = q.to_lowercase();
                let matches_content = memory.content.to_lowercase().contains(&q_lower);
                let matches_summary = memory
                    .summary
                    .as_deref()
                    .map_or(false, |s| s.to_lowercase().contains(&q_lower));
                if !matches_content && !matches_summary {
                    continue;
                }
            }

            if self.store.get_memory(&memory.id)?.is_some() {
                report.skipped_duplicates += 1;
                continue;
            }

            let mut meta = memory.metadata.clone();
            meta["transferred_from"] = serde_json::Value::String(source_label.to_string());
            meta["transferred_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            memory.metadata = meta;

            self.store.insert_or_update_memory(&memory)?;
            report.transferred_memories += 1;
        }

        // 2. Transfer failure patterns
        if filter.include_failure_patterns {
            let failures =
                source_store.search_failures(filter.query.as_deref(), None, filter.limit)?;
            for mut failure in failures {
                let mut meta = failure.metadata.clone();
                meta["transferred_from"] = serde_json::Value::String(source_label.to_string());
                failure.metadata = meta;

                self.store.upsert_failure_pattern(&failure)?;
                report.transferred_failure_patterns += 1;
            }
        }

        // 3. Transfer procedural skills
        if filter.include_procedural_skills {
            let skills = source_store.get_all_procedural_skills(None, filter.limit)?;
            for skill in skills {
                if self.store.get_procedural_skill(&skill.id)?.is_some() {
                    report.skipped_duplicates += 1;
                    continue;
                }
                self.store.insert_or_update_procedural_skill(&skill)?;
                report.transferred_procedural_skills += 1;
            }
        }

        Ok(report)
    }

    /// Evaluates whether a user prompt and task type require memory recall using the deterministic gating classifier (< 1ms).
    pub fn should_recall(&self, prompt: &str, task_type: TaskType) -> bool {
        crate::gating::should_recall(prompt, task_type)
    }

    /// Produces a full deterministic gating classification report for a prompt and task type (< 1ms).
    pub fn classify_recall(&self, prompt: &str, task_type: TaskType) -> RecallDecision {
        crate::gating::RecallGatingClassifier::classify(prompt, task_type)
    }

    /// Access the underlying HybridRanker.
    pub fn ranker(&self) -> &HybridRanker {
        &self.ranker
    }

    /// Performs hybrid memory recall biased by the developer's active files and their
    /// 1-hop (`1.0`) and 2-hop (`0.5`) import graph neighbors.
    pub async fn recall_with_active_files(
        &self,
        query: &str,
        scope: Option<&Scope>,
        active_files: &[String],
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let mut results = self
            .ranker
            .retrieve_with_active_files(
                &self.store,
                Some(self.embedding_provider.as_ref()),
                query,
                scope,
                None,
                active_files,
                limit,
            )
            .await?;

        for memory in &mut results {
            memory.mark_accessed();
            let now = memory.last_accessed_at.unwrap_or_else(Utc::now);
            if !self.store.update_access_metrics(&memory.id, now)? {
                if let Some(ref global) = self.global_store {
                    let _ = global.update_access_metrics(&memory.id, now);
                }
            }
        }

        Ok(results)
    }

    /// Generates a working session digest (`DigestOutput`) whose `key_pointers` and `failure_warnings`
    /// are ranked with active file and import distance proximity biasing.
    pub async fn memory_digest_with_active_files(
        &self,
        session_id: &str,
        max_tokens: Option<usize>,
        active_files: &[String],
    ) -> Result<DigestOutput, StrataError> {
        self.consolidator.generate_digest_with_active_files(
            &self.store,
            session_id,
            max_tokens,
            active_files,
        )
    }

    /// Ingests a `MemoryRecord` through the AUDN (Add / Update / Delete / Noop) deduplication
    /// and belief-revision pipeline.
    pub async fn ingest_audn(&self, record: &MemoryRecord) -> Result<AudnDecision, StrataError> {
        let mut to_ingest = record.clone();
        if to_ingest.git_branch().is_none() {
            if let Ok(curr_dir) = std::env::current_dir() {
                if let Some(branch) = crate::workspace::detect_git_branch(&curr_dir) {
                    to_ingest = to_ingest.with_branch(branch);
                }
            }
        }

        let pipeline = AudnPipeline::with_default_config();
        let decision = pipeline
            .ingest_memory(
                &self.store,
                Some(self.embedding_provider.as_ref()),
                &to_ingest,
            )
            .await?;

        if matches!(
            decision.operation,
            AudnOperation::Add | AudnOperation::Update { .. }
        ) {
            if let Some(ref written) = decision.record {
                let workspace =
                    std::env::var("STRATA_WORKSPACE_ID").unwrap_or_else(|_| "default".to_string());
                if let Ok(payload) = serde_json::to_value(written) {
                    let version_hash = sync::compute_version_hash(&payload);
                    let delta = strata_core::schemas::SyncDelta::new(
                        &workspace,
                        0,
                        "memory_record",
                        payload,
                        version_hash,
                    );
                    let _ = self.store.enqueue_delta(&delta);
                }
            }
        }

        Ok(decision)
    }
}

#[async_trait]
impl MemoryEngine for SqliteMemoryEngine {
    async fn search(
        &self,
        query: &str,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        self.search_as_of(query, scope, None, None, None, limit)
            .await
    }

    async fn search_as_of(
        &self,
        query: &str,
        scope: Option<&Scope>,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let detected_branch = if as_of_branch.is_none() {
            if let Ok(curr) = std::env::current_dir() {
                crate::workspace::detect_git_branch(&curr)
            } else {
                None
            }
        } else {
            None
        };
        let effective_branch = as_of_branch.or(detected_branch.as_deref());

        let mut results = self
            .ranker
            .retrieve_federated_as_of(
                &self.store,
                self.global_store.as_deref(),
                Some(self.embedding_provider.as_ref()),
                query,
                scope,
                None,
                as_of_time,
                as_of_commit,
                effective_branch,
                limit,
            )
            .await?;

        // Update access metrics in the corresponding store
        for memory in &mut results {
            memory.mark_accessed();
            let now = memory.last_accessed_at.unwrap_or_else(Utc::now);
            if !self.store.update_access_metrics(&memory.id, now)? {
                if let Some(ref global) = self.global_store {
                    let _ = global.update_access_metrics(&memory.id, now);
                }
            }
        }

        Ok(results)
    }

    async fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError> {
        self.get_as_of(id, None, None, None).await
    }

    async fn get_as_of(
        &self,
        id: &Uuid,
        as_of_time: Option<DateTime<Utc>>,
        as_of_commit: Option<&str>,
        as_of_branch: Option<&str>,
    ) -> Result<Option<MemoryRecord>, StrataError> {
        if let Some(mut memory) =
            self.store
                .get_memory_as_of(id, as_of_time, as_of_commit, as_of_branch)?
        {
            memory.mark_accessed();
            let now = memory.last_accessed_at.unwrap_or_else(Utc::now);
            let _ = self.store.update_access_metrics(&memory.id, now);
            return Ok(Some(memory));
        }

        if let Some(ref global) = self.global_store {
            if let Some(mut memory) =
                global.get_memory_as_of(id, as_of_time, as_of_commit, as_of_branch)?
            {
                memory.mark_accessed();
                let now = memory.last_accessed_at.unwrap_or_else(Utc::now);
                let _ = global.update_access_metrics(&memory.id, now);
                return Ok(Some(memory));
            }
        }

        Ok(None)
    }

    async fn write(&self, record: &MemoryRecord) -> Result<MemoryHandle, StrataError> {
        // Strict Human-in-the-loop Invariant: Core Tier requires explicit human approval
        if record.tier == MemoryTier::Core && !record.approved_by_human {
            return Err(StrataError::Validation(
                "Cannot write memory directly to Core Tier without explicit human approval (approved_by_human=true)".to_string(),
            ));
        }

        // Route writes through AUDN pipeline for deduplication, canonical updates, or addition
        let decision = self.ingest_audn(record).await?;
        match decision.operation {
            AudnOperation::Noop { existing_id } => {
                if let Ok(Some(existing)) = self.store.get_memory(&existing_id) {
                    Ok(existing.to_handle(Some(existing.importance)))
                } else {
                    Ok(record.to_handle(Some(record.importance)))
                }
            }
            AudnOperation::Update { target_id } => {
                if let Some(ref updated) = decision.record {
                    Ok(updated.to_handle(Some(updated.importance)))
                } else if let Ok(Some(existing)) = self.store.get_memory(&target_id) {
                    Ok(existing.to_handle(Some(existing.importance)))
                } else {
                    Ok(record.to_handle(Some(record.importance)))
                }
            }
            AudnOperation::Add => {
                let rec = decision.record.as_ref().unwrap_or(record);
                Ok(rec.to_handle(Some(rec.importance)))
            }
            AudnOperation::Delete { .. } => Ok(record.to_handle(Some(record.importance))),
        }
    }

    async fn digest(
        &self,
        session_id: &str,
        max_tokens: Option<usize>,
    ) -> Result<DigestOutput, StrataError> {
        self.consolidator
            .generate_digest(&self.store, session_id, max_tokens)
    }

    async fn record_failure(&self, failure: &FailurePattern) -> Result<(), StrataError> {
        self.store.upsert_failure_pattern(failure)?;
        if failure.scope == Scope::Global {
            if let Some(ref global) = self.global_store {
                let _ = global.upsert_failure_pattern(failure);
            }
        }
        Ok(())
    }

    async fn get_known_failures(
        &self,
        query: Option<&str>,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        let mut local_failures =
            self.consolidator
                .get_known_failures(&self.store, query, scope, limit)?;

        if let Some(ref global) = self.global_store {
            let global_failures =
                self.consolidator
                    .get_known_failures(global, query, Some(&Scope::Global), limit)?;

            let mut seen_signatures = std::collections::HashSet::new();
            for f in &local_failures {
                seen_signatures.insert(f.signature.clone());
            }

            for gf in global_failures {
                if seen_signatures.insert(gf.signature.clone()) {
                    local_failures.push(gf);
                }
            }
            local_failures.truncate(limit);
        }

        Ok(local_failures)
    }

    async fn get_symbol_anti_patterns(
        &self,
        file: &str,
        symbol: Option<&str>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        self.get_symbol_anti_patterns(file, symbol, limit).await
    }
}

#[async_trait]
impl EventStore for SqliteMemoryEngine {
    async fn append(&self, event: &Event) -> Result<EventId, StrataError> {
        let event_id = self.store.insert_event(event)?;

        // Automatically consolidate derived memory if applicable
        if let Some(extracted_mem) = self.consolidator.extract_from_event(event) {
            let _ = self.write(&extracted_mem).await;
        }

        Ok(event_id)
    }

    async fn read_stream(
        &self,
        session_id: &str,
        from_seq: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<Event>, StrataError> {
        self.store.get_events(session_id, from_seq, limit)
    }
}
