use std::collections::HashMap;
use strata_core::errors::StrataError;
use strata_core::state::{MemoryRecord, MemoryType, Scope};
use uuid::Uuid;

use crate::embedding::{cosine_similarity, EmbeddingProvider};
use crate::store::SqliteStore;

#[derive(Debug, Clone)]
pub struct HybridRankerConfig {
    /// RRF smoothing constant (default: 60.0)
    pub rrf_k: f32,
    /// Lexical BM25 ranker weight (default: 0.5)
    pub bm25_weight: f32,
    /// Vector cosine similarity ranker weight (default: 0.5)
    pub vector_weight: f32,
    /// Minimum cosine similarity threshold for vector candidates (default: 0.0)
    pub min_similarity: f32,
}

impl Default for HybridRankerConfig {
    fn default() -> Self {
        Self {
            rrf_k: 60.0,
            bm25_weight: 0.5,
            vector_weight: 0.5,
            min_similarity: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HybridRanker {
    config: HybridRankerConfig,
}

impl HybridRanker {
    pub fn new(config: HybridRankerConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(HybridRankerConfig::default())
    }

    /// Perform hybrid retrieval by combining FTS5 BM25 and Vector Cosine Similarity via RRF.
    pub async fn retrieve(
        &self,
        store: &SqliteStore,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        query: &str,
        scope: Option<&Scope>,
        memory_types: Option<&[MemoryType]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StrataError> {
        let candidate_limit = (limit * 3).max(20);

        // 1. Lexical retrieval via SQLite FTS5
        let fts_results = store.search_fts(query, scope, candidate_limit)?;

        // 2. Vector retrieval
        let mut vector_results: Vec<(MemoryRecord, f32)> = Vec::new();
        if let Some(embedder) = embedding_provider {
            if let Ok(query_embedding) = embedder.embed_text(query).await {
                // Fetch candidates in scope to score against
                let all_candidates =
                    store.get_all_memories(scope, memory_types, candidate_limit * 2)?;
                for memory in all_candidates {
                    if let Some(ref mem_emb) = memory.embedding {
                        let sim = cosine_similarity(&query_embedding, mem_emb);
                        if sim >= self.config.min_similarity {
                            vector_results.push((memory, sim));
                        }
                    }
                }
                // Sort vector results by similarity descending
                vector_results
                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            }
        }

        // If both FTS and vector results are empty, fall back to recent memories
        if fts_results.is_empty() && vector_results.is_empty() {
            return store.get_all_memories(scope, memory_types, limit);
        }

        // 3. Compute Reciprocal Rank Fusion (RRF)
        let fused = self.fuse_ranks(&fts_results, &vector_results, limit);
        Ok(fused)
    }

    /// Fuse BM25 and Vector rankings using Reciprocal Rank Fusion (RRF).
    pub fn fuse_ranks(
        &self,
        fts_ranked: &[(MemoryRecord, f32)],
        vector_ranked: &[(MemoryRecord, f32)],
        limit: usize,
    ) -> Vec<MemoryRecord> {
        let mut score_map: HashMap<Uuid, f32> = HashMap::new();
        let mut record_map: HashMap<Uuid, MemoryRecord> = HashMap::new();

        let k = self.config.rrf_k;
        let w_bm25 = self.config.bm25_weight;
        let w_vec = self.config.vector_weight;

        // Score FTS results
        for (rank_idx, (record, _bm25_score)) in fts_ranked.iter().enumerate() {
            let rrf_score = w_bm25 * (1.0 / (k + (rank_idx as f32) + 1.0));
            *score_map.entry(record.id).or_insert(0.0) += rrf_score;
            record_map
                .entry(record.id)
                .or_insert_with(|| record.clone());
        }

        // Score Vector results
        for (rank_idx, (record, _sim)) in vector_ranked.iter().enumerate() {
            let rrf_score = w_vec * (1.0 / (k + (rank_idx as f32) + 1.0));
            *score_map.entry(record.id).or_insert(0.0) += rrf_score;
            record_map
                .entry(record.id)
                .or_insert_with(|| record.clone());
        }

        // Apply quality weighting: importance & confidence
        for (id, score) in score_map.iter_mut() {
            if let Some(rec) = record_map.get(id) {
                let importance_factor = 0.8 + 0.4 * rec.importance;
                let confidence_factor = 0.5 + 0.5 * rec.confidence;
                *score *= importance_factor * confidence_factor;
            }
        }

        let mut ranked_ids: Vec<(Uuid, f32)> = score_map.into_iter().collect();
        ranked_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked_ids
            .into_iter()
            .take(limit)
            .filter_map(|(id, _)| record_map.remove(&id))
            .collect()
    }
}
