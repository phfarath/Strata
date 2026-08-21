pub mod alignment;
pub mod ast;
pub mod call_graph;
pub mod community;
pub mod compiler;
pub mod consolidation;
pub mod decay;
pub mod embedding;
pub mod jtms;
pub mod pipeline;
pub mod retrieval;
pub mod store;
pub mod sync;
pub mod workspace;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;
use async_trait::async_trait;
use uuid::Uuid;

use strata_core::errors::StrataError;
use strata_core::events::{Event, EventId};
use strata_core::state::{DigestOutput, FailurePattern, MemoryHandle, MemoryRecord, MemoryTier, Scope};
use strata_core::traits::{EventStore, MemoryEngine};

pub use alignment::PreferenceMiner;
pub use ast::{
    AstDiffResult, AstParser, CodeAnchorEngine, ExtractedSymbol, LanguageKind, ReconciliationReport,
};
pub use call_graph::{CallEdge, CallGraph, CallGraphAnalyzer, CallType};
pub use community::{
    ArchitectureCluster, ArchitectureGraphSummary, ClusterDependency, ClusterMember,
    ClusteringConfig, CommunityDetector, MemberType,
};
pub use compiler::{
    estimate_tokens, HostCompileResult, MultiHostCompileReport, MultiHostCompiler,
    STRATA_MARKER_END, STRATA_MARKER_START,
};
pub use consolidation::Consolidator;
pub use decay::{DecayCalculator, PruneReport};
pub use embedding::{
    bytes_to_embedding, cosine_similarity, embedding_to_bytes, EmbeddingProvider,
    FastEmbedProvider, MockEmbeddingProvider,
};
pub use jtms::{ConflictMatch, ConflictResolution, TruthMaintenanceSystem};
pub use pipeline::{ConsolidationPipeline, ConsolidationResult, PipelineConfig};
pub use retrieval::{HybridRanker, HybridRankerConfig};
pub use store::SqliteStore;
pub use workspace::{MonorepoPackage, PackageType, WorkspaceBoundary, WorkspaceBoundaryDetector};
pub use sync::{calculate_exponential_backoff, compute_version_hash, SyncEngine};
pub use strata_core::schemas::{
    CodeAnchor, ContextBudgetConfig, ExportFormat, FeedbackEvent, FeedbackRating, HostTargetConfig,
    ImplicitSignal, KtoSample, MemoryFeedback, PreferencePair, SemanticFact, SftSample, SignalKind, SymbolType,
};
pub type DpoPair = PreferencePair;




/// SQLite-backed persistent memory engine implementing `MemoryEngine` and `EventStore`.
pub struct SqliteMemoryEngine {
    store: Arc<SqliteStore>,
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
        let embedder: Arc<dyn EmbeddingProvider> = embedding_provider
            .unwrap_or_else(|| Arc::new(MockEmbeddingProvider::default()));

        Ok(Self {
            store,
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
        let embedder: Arc<dyn EmbeddingProvider> = embedding_provider
            .unwrap_or_else(|| Arc::new(MockEmbeddingProvider::default()));

        Ok(Self {
            store,
            embedding_provider: embedder,
            ranker: HybridRanker::with_default_config(),
            consolidator: Consolidator::new(),
        })
    }

    /// Get a reference to the underlying `SqliteStore`.
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// Get an Arc reference to the underlying `SqliteStore`.
    pub fn store_arc(&self) -> Arc<SqliteStore> {
        Arc::clone(&self.store)
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
    pub fn detect_workspace_boundaries(&self, root_dir: &Path) -> Result<WorkspaceBoundary, StrataError> {
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
        self.store.promote_memory_to_core(id, approved_by_human, reason)
    }

    /// Promotes a semantic fact to Core Tier with explicit human approval.
    pub async fn promote_fact_to_core(
        &self,
        id: &Uuid,
        approved_by_human: bool,
        reason: Option<&str>,
    ) -> Result<SemanticFact, StrataError> {
        self.store.promote_semantic_fact_to_core(id, approved_by_human, reason)
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
            .retrieve(
                &self.store,
                Some(self.embedding_provider.as_ref()),
                query,
                scope,
                None,
                limit,
            )
            .await?;

        // Update access metrics
        for memory in &mut results {
            memory.mark_accessed();
            let _ = self.store.insert_or_update_memory(memory);
        }

        Ok(results)
    }

    async fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>, StrataError> {
        let mem = self.store.get_memory(id)?;
        if let Some(mut memory) = mem {
            memory.mark_accessed();
            let _ = self.store.insert_or_update_memory(&memory);
            Ok(Some(memory))
        } else {
            Ok(None)
        }
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
        let workspace = std::env::var("STRATA_WORKSPACE_ID").unwrap_or_else(|_| "default".to_string());
        if let Ok(payload) = serde_json::to_value(&to_write) {
            let version_hash = sync::compute_version_hash(&payload);
            let delta = strata_core::schemas::SyncDelta::new(&workspace, 0, "memory_record", payload, version_hash);
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
        self.store.upsert_failure_pattern(failure)
    }

    async fn get_known_failures(
        &self,
        query: Option<&str>,
        scope: Option<&Scope>,
        limit: usize,
    ) -> Result<Vec<FailurePattern>, StrataError> {
        self.consolidator
            .get_known_failures(&self.store, query, scope, limit)
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
