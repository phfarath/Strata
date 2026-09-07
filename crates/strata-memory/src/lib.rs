pub mod alignment;
pub mod ast;
pub mod call_graph;
pub mod clustering;
pub mod community;
pub mod compiler;
pub mod consolidation;
pub mod decay;
pub mod embedding;
pub mod enricher;
pub mod jtms;
pub mod leases;
pub mod pipeline;
pub mod procedural_mining;
pub mod retrieval;
pub mod spreading_activation;
pub mod store;
pub mod subconscious;
pub mod sync;
pub mod workspace;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::events::{Event, EventId};
use strata_core::state::{
    DigestOutput, FailurePattern, MemoryHandle, MemoryRecord, MemoryTier, Scope,
};
use strata_core::traits::{EventStore, MemoryEngine};

pub use alignment::PreferenceMiner;
pub use ast::{
    AstDiffResult, AstParser, CodeAnchorEngine, ExtractedSymbol, LanguageKind, ReconciliationReport,
};
pub use call_graph::{CallEdge, CallGraph, CallGraphAnalyzer, CallType};
pub use clustering::{SpatialCluster, SpatialClusterer, VectorPoint};
pub use community::{
    ArchitectureCluster, ArchitectureGraphSummary, ClusterDependency, ClusterMember,
    ClusteringConfig, CommunityDetector, MemberType,
};
pub use compiler::{
    estimate_tokens, HostCompileResult, MultiHostCompileReport, MultiHostCompiler,
    STRATA_MARKER_END, STRATA_MARKER_START,
};
pub use consolidation::{Consolidator, NeuroSymbolicConsolidator};
pub use decay::{DecayCalculator, PruneReport};
pub use embedding::{
    bytes_to_embedding, cosine_similarity, embedding_to_bytes, EmbeddingProvider,
    FastEmbedProvider, MockEmbeddingProvider,
};
pub use enricher::{AsyncLlmEnricher, CanonicalTemplateEnricher, MemoryEnricher};
pub use jtms::{ConflictMatch, ConflictResolution, TruthMaintenanceSystem};
pub use leases::StigmergyCoordinator;
pub use pipeline::{ConsolidationPipeline, ConsolidationResult, PipelineConfig};
pub use procedural_mining::TrajectoryMiner;
pub use retrieval::{HybridRanker, HybridRankerConfig};
pub use spreading_activation::{
    EdgeKind, GraphEdge, GraphNode, KnowledgeGraph, SpreadingActivationConfig,
    SpreadingActivationEngine,
};
pub use store::SqliteStore;
pub use strata_core::schemas::{
    CodeAnchor, ContextBudgetConfig, ExportFormat, FeedbackEvent, FeedbackRating, HostTargetConfig,
    ImplicitSignal, KtoSample, MemoryFeedback, PreferencePair, SemanticFact, SftSample, SignalKind,
    SymbolType, TransferFilter, TransferReport,
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

        let global = self.global_store.as_ref().ok_or_else(|| {
            StrataError::Configuration(
                "No global store configured for federation (global_store is None)".to_string(),
            )
        })?;

        let mut record = self.store.get_memory(id)?.ok_or_else(|| {
            StrataError::NotFound(format!("Memory record {id} not found in local store"))
        })?;

        record.scope = Scope::Global;
        record.tier = MemoryTier::Core;
        record.approved_by_human = true;
        record.updated_at = chrono::Utc::now();

        let mut meta = record.metadata.clone();
        if let Some(r) = reason {
            meta["promotion_reason"] = serde_json::Value::String(r.to_string());
        }
        meta["promoted_to_global_at"] =
            serde_json::Value::String(chrono::Utc::now().to_rfc3339());
        record.metadata = meta;

        if record.embedding.is_none() && !record.content.trim().is_empty() {
            if let Ok(emb) = self.embedding_provider.embed_text(&record.content).await {
                record.embedding = Some(emb);
            }
        }

        global.insert_or_update_memory(&record)?;
        let _ = self.store.insert_or_update_memory(&record);

        Ok(record)
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
        let candidates = source_store.get_all_memories(
            None,
            types_slice.as_deref(),
            filter.limit,
        )?;

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
            meta["transferred_at"] =
                serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            memory.metadata = meta;

            self.store.insert_or_update_memory(&memory)?;
            report.transferred_memories += 1;
        }

        // 2. Transfer failure patterns
        if filter.include_failure_patterns {
            let failures = source_store.search_failures(filter.query.as_deref(), None, filter.limit)?;
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
            for mut skill in skills {
                if self.store.get_procedural_skill(&skill.id)?.is_some() {
                    report.skipped_duplicates += 1;
                    continue;
                }
                self.store.insert_or_update_procedural_skill(&mut skill)?;
                report.transferred_procedural_skills += 1;
            }
        }

        Ok(report)
    }

    /// Access the underlying HybridRanker.
    pub fn ranker(&self) -> &HybridRanker {
        &self.ranker
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
        let mut results = self
            .ranker
            .retrieve_federated(
                &self.store,
                self.global_store.as_deref(),
                Some(self.embedding_provider.as_ref()),
                query,
                scope,
                None,
                limit,
            )
            .await?;

        // Update access metrics in the corresponding store
        for memory in &mut results {
            memory.mark_accessed();
            if self.store.get_memory(&memory.id)?.is_some() {
                let _ = self.store.insert_or_update_memory(memory);
            } else if let Some(ref global) = self.global_store {
                let _ = global.insert_or_update_memory(memory);
            }
        }

        Ok(results)
    }

    async fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError> {
        if let Some(mut memory) = self.store.get_memory(id)? {
            memory.mark_accessed();
            let _ = self.store.insert_or_update_memory(&memory);
            return Ok(Some(memory));
        }

        if let Some(ref global) = self.global_store {
            if let Some(mut memory) = global.get_memory(id)? {
                memory.mark_accessed();
                let _ = global.insert_or_update_memory(&memory);
                return Ok(Some(memory));
            }
        }

        Ok(None)
    }

    async fn write(&self, record: &MemoryRecord) -> Result<MemoryHandle, StrataError> {
        let mut to_write = record.clone();

        // Strict Human-in-the-loop Invariant: Core Tier requires explicit human approval
        if to_write.tier == MemoryTier::Core && !to_write.approved_by_human {
            return Err(StrataError::Validation(
                "Cannot write memory directly to Core Tier without explicit human approval (approved_by_human=true)".to_string(),
            ));
        }

        // If embedding is missing, generate it automatically
        if to_write.embedding.is_none() && !to_write.content.trim().is_empty() {
            if let Ok(emb) = self.embedding_provider.embed_text(&to_write.content).await {
                to_write.embedding = Some(emb);
            }
        }

        self.store.insert_or_update_memory(&to_write)?;

        // Auto-enqueue CDC sync delta in outbox
        let workspace =
            std::env::var("STRATA_WORKSPACE_ID").unwrap_or_else(|_| "default".to_string());
        if let Ok(payload) = serde_json::to_value(&to_write) {
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

        Ok(to_write.to_handle(Some(to_write.importance)))
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
        let mut local_failures = self
            .consolidator
            .get_known_failures(&self.store, query, scope, limit)?;

        if let Some(ref global) = self.global_store {
            let global_failures = self
                .consolidator
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
