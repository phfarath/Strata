use async_trait::async_trait;
use strata_core::errors::StrataError;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, StrataError>;

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StrataError> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed_text(text).await?);
        }
        Ok(results)
    }

    fn dimension(&self) -> usize;

    fn provider_name(&self) -> &str {
        "custom"
    }

    fn model_name(&self) -> &str {
        "unknown"
    }

    fn model_version(&self) -> &str {
        "1"
    }
}

/// Compute cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (&x, &y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let norm = norm_a.sqrt() * norm_b.sqrt();
    if norm <= f32::EPSILON {
        0.0
    } else {
        (dot / norm).clamp(-1.0, 1.0)
    }
}

/// Convert embedding f32 slice to byte vector for SQLite BLOB storage.
pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Convert byte slice from SQLite BLOB storage back into f32 embedding.
pub fn bytes_to_embedding(bytes: &[u8]) -> Result<Vec<f32>, StrataError> {
    if bytes.len() % 4 != 0 {
        return Err(StrataError::Validation(format!(
            "Invalid embedding byte length: {}",
            bytes.len()
        )));
    }
    let mut embedding = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk
            .try_into()
            .map_err(|_| StrataError::Validation("Failed to read f32 chunk".to_string()))?;
        embedding.push(f32::from_le_bytes(arr));
    }
    Ok(embedding)
}

/// A deterministic hash-based embedding provider for offline operation, testing, and fast fallback.
/// Generates normalized dense vectors by hashing character n-grams and tokens.
#[derive(Debug, Clone)]
pub struct MockEmbeddingProvider {
    dimension: usize,
}

impl MockEmbeddingProvider {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension: if dimension == 0 { 384 } else { dimension },
        }
    }
}

impl Default for MockEmbeddingProvider {
    fn default() -> Self {
        Self::new(384)
    }
}

#[async_trait]
impl EmbeddingProvider for MockEmbeddingProvider {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, StrataError> {
        let dim = self.dimension;
        let mut vector = vec![0.0f32; dim];
        if text.trim().is_empty() {
            return Ok(vector);
        }

        let words: Vec<&str> = text.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            let lower = word.to_lowercase();
            let mut h: u64 = 5381;
            for b in lower.bytes() {
                h = ((h << 5).wrapping_add(h)).wrapping_add(b as u64);
            }
            let idx = (h as usize) % dim;
            let sign = if (h >> 32) % 2 == 0 { 1.0f32 } else { -1.0f32 };
            let pos_weight = 1.0 / (1.0 + (i as f32) * 0.05);
            vector[idx] += sign * pos_weight;

            // Bigram hashing
            if lower.len() >= 3 {
                for window in lower.as_bytes().windows(3) {
                    let mut wh: u64 = 2166136261;
                    for &wb in window {
                        wh = (wh ^ (wb as u64)).wrapping_mul(16777619);
                    }
                    let w_idx = (wh as usize) % dim;
                    vector[w_idx] += 0.5;
                }
            }
        }

        // L2 normalize
        let norm_sq: f32 = vector.iter().map(|x| x * x).sum();
        let norm = norm_sq.sqrt();
        if norm > f32::EPSILON {
            for val in &mut vector {
                *val /= norm;
            }
        }

        Ok(vector)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn provider_name(&self) -> &str {
        "mock"
    }

    fn model_name(&self) -> &str {
        "mock-hash"
    }

    fn model_version(&self) -> &str {
        "1"
    }
}

/// FastEmbed local embedding provider using ONNX models.
pub struct FastEmbedProvider {
    model: std::sync::Arc<std::sync::Mutex<fastembed::TextEmbedding>>,
    dimension: usize,
    model_name: String,
}

impl FastEmbedProvider {
    pub fn try_new() -> Result<Self, StrataError> {
        Self::try_new_with_model(fastembed::EmbeddingModel::AllMiniLML6V2)
    }

    pub fn try_new_with_model(model: fastembed::EmbeddingModel) -> Result<Self, StrataError> {
        let model_name = format!("fastembed:{:?}", model);
        let dimension = match model {
            fastembed::EmbeddingModel::BGEBaseENV15 => 768,
            _ => 384,
        };
        let mut options = fastembed::InitOptions::default();
        options.model_name = model;
        options.show_download_progress = false;
        let model_inst = fastembed::TextEmbedding::try_new(options)
            .map_err(|e| StrataError::Embedding(format!("Failed to initialize FastEmbed: {e}")))?;

        Ok(Self {
            model: std::sync::Arc::new(std::sync::Mutex::new(model_inst)),
            dimension,
            model_name,
        })
    }

    pub fn try_new_with_model_str(model_str: Option<&str>) -> Result<Self, StrataError> {
        match model_str.map(|s| s.trim()) {
            None | Some("") => Self::try_new(),
            Some(name) => {
                let clean = name.strip_prefix("fastembed:").unwrap_or(name).to_lowercase();
                let model = match clean.as_str() {
                    "allminilml6v2" | "all-minilm-l6-v2" | "minilm" => {
                        fastembed::EmbeddingModel::AllMiniLML6V2
                    }
                    "bgesmallenv15" | "bge-small-en-v1.5" | "bge-small" => {
                        fastembed::EmbeddingModel::BGESmallENV15
                    }
                    "bgebaseenv15" | "bge-base-en-v1.5" | "bge-base" => {
                        fastembed::EmbeddingModel::BGEBaseENV15
                    }
                    other => {
                        return Err(StrataError::Validation(format!(
                            "Unknown FastEmbed embedding model: '{other}'. Supported: 'AllMiniLML6V2', 'BGESmallENV15', 'BGEBaseENV15'"
                        )));
                    }
                };
                Self::try_new_with_model(model)
            }
        }
    }
}

#[async_trait]
impl EmbeddingProvider for FastEmbedProvider {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, StrataError> {
        let text_owned = text.to_string();
        let model = self.model.clone();
        let results = tokio::task::spawn_blocking(move || {
            let texts = vec![text_owned];
            let guard = model
                .lock()
                .map_err(|e| StrataError::Embedding(format!("Lock error: {e}")))?;
            guard
                .embed(texts, None)
                .map_err(|e| StrataError::Embedding(e.to_string()))
        })
        .await
        .map_err(|e| StrataError::Execution(e.to_string()))??;

        results
            .into_iter()
            .next()
            .ok_or_else(|| StrataError::Embedding("Empty embedding returned".to_string()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StrataError> {
        let texts_owned = texts.to_vec();
        let model = self.model.clone();
        tokio::task::spawn_blocking(move || {
            let guard = model
                .lock()
                .map_err(|e| StrataError::Embedding(format!("Lock error: {e}")))?;
            guard
                .embed(texts_owned, None)
                .map_err(|e| StrataError::Embedding(e.to_string()))
        })
        .await
        .map_err(|e| StrataError::Execution(e.to_string()))?
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn provider_name(&self) -> &str {
        "fastembed"
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn model_version(&self) -> &str {
        "1"
    }
}

/// Supported embedding provider kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingProviderKind {
    Mock,
    FastEmbed,
}

impl std::fmt::Display for EmbeddingProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingProviderKind::Mock => write!(f, "mock"),
            EmbeddingProviderKind::FastEmbed => write!(f, "fastembed"),
        }
    }
}

/// Metadata description of a resolved embedding provider.
#[derive(Clone)]
pub struct ResolvedEmbeddingProvider {
    pub provider: std::sync::Arc<dyn EmbeddingProvider>,
    pub kind: EmbeddingProviderKind,
    pub model_name: String,
    pub model_version: String,
    pub dimension: usize,
    pub is_fallback: bool,
    pub fallback_reason: Option<String>,
}

/// Centralized factory for initializing embedding providers across CLI, MCP, daemon, and consolidation.
pub struct EmbeddingProviderFactory;

impl EmbeddingProviderFactory {
    /// Create a provider given optional explicit provider name and model name.
    ///
    /// Rules:
    /// - If provider is explicitly Some("mock") / Some("mock-hash"):
    ///     Creates `MockEmbeddingProvider`, `is_fallback: false`.
    /// - If provider is explicitly Some("fastembed") / Some("onnx"):
    ///     Tries to create `FastEmbedProvider`. If it fails, returns `Err(StrataError::Embedding(...))`
    ///     (strict: explicit user configuration is not silently downgraded).
    /// - If provider is None (auto-detect / default):
    ///     Attempts to create `FastEmbedProvider`.
    ///     If successful -> `is_fallback: false`.
    ///     If it fails -> Falls back to `MockEmbeddingProvider`, `is_fallback: true`, logs warning.
    pub fn create(
        provider: Option<&str>,
        model: Option<&str>,
    ) -> Result<ResolvedEmbeddingProvider, StrataError> {
        let normalized_prov = provider.map(|p| p.trim().to_lowercase());
        match normalized_prov.as_deref() {
            Some("mock") | Some("mock-hash") => {
                let dim = 384;
                let mock = std::sync::Arc::new(MockEmbeddingProvider::new(dim));
                Ok(ResolvedEmbeddingProvider {
                    provider: mock,
                    kind: EmbeddingProviderKind::Mock,
                    model_name: "mock-hash".to_string(),
                    model_version: "1".to_string(),
                    dimension: dim,
                    is_fallback: false,
                    fallback_reason: None,
                })
            }
            Some("fastembed") | Some("onnx") => {
                let fast = FastEmbedProvider::try_new_with_model_str(model)?;
                let model_name = fast.model_name().to_string();
                let dim = fast.dimension();
                Ok(ResolvedEmbeddingProvider {
                    provider: std::sync::Arc::new(fast),
                    kind: EmbeddingProviderKind::FastEmbed,
                    model_name,
                    model_version: "1".to_string(),
                    dimension: dim,
                    is_fallback: false,
                    fallback_reason: None,
                })
            }
            Some(other) => {
                Err(StrataError::Validation(format!(
                    "Unsupported embedding provider: '{other}'. Supported providers: 'fastembed', 'mock'"
                )))
            }
            None => {
                match FastEmbedProvider::try_new_with_model_str(model) {
                    Ok(fast) => {
                        let model_name = fast.model_name().to_string();
                        let dim = fast.dimension();
                        Ok(ResolvedEmbeddingProvider {
                            provider: std::sync::Arc::new(fast),
                            kind: EmbeddingProviderKind::FastEmbed,
                            model_name,
                            model_version: "1".to_string(),
                            dimension: dim,
                            is_fallback: false,
                            fallback_reason: None,
                        })
                    }
                    Err(err) => {
                        let reason = format!("FastEmbed auto-initialization unavailable ({err})");
                        tracing::warn!("{reason}; falling back to MockEmbeddingProvider");
                        let dim = 384;
                        let mock = std::sync::Arc::new(MockEmbeddingProvider::new(dim));
                        Ok(ResolvedEmbeddingProvider {
                            provider: mock,
                            kind: EmbeddingProviderKind::Mock,
                            model_name: "mock-hash".to_string(),
                            model_version: "1".to_string(),
                            dimension: dim,
                            is_fallback: true,
                            fallback_reason: Some(reason),
                        })
                    }
                }
            }
        }
    }

    /// Creates an embedding provider from StrataConfig or environment variables (`STRATA_EMBEDDING_PROVIDER`, `STRATA_EMBEDDING_MODEL`).
    pub fn create_from_config(
        config: &strata_core::config::StrataConfig,
    ) -> Result<ResolvedEmbeddingProvider, StrataError> {
        let env_prov = std::env::var("STRATA_EMBEDDING_PROVIDER").ok();
        let env_model = std::env::var("STRATA_EMBEDDING_MODEL").ok();

        let provider = config
            .embedding_provider
            .as_deref()
            .or_else(|| env_prov.as_deref());
        let model = config
            .embedding_model
            .as_deref()
            .or_else(|| env_model.as_deref());

        Self::create(provider, model)
    }

    /// Helper to create a Mock provider for testing without disk/network dependencies.
    pub fn create_mock(dimension: usize) -> ResolvedEmbeddingProvider {
        let dim = if dimension == 0 { 384 } else { dimension };
        ResolvedEmbeddingProvider {
            provider: std::sync::Arc::new(MockEmbeddingProvider::new(dim)),
            kind: EmbeddingProviderKind::Mock,
            model_name: "mock-hash".to_string(),
            model_version: "1".to_string(),
            dimension: dim,
            is_fallback: false,
            fallback_reason: None,
        }
    }
}

/// Validate whether two embedding model identifiers belong to the same semantic space.
/// Returns false if either model is different or if a neural model is compared to mock.
pub fn are_embedding_models_compatible(model_a: &str, model_b: &str) -> bool {
    let a = model_a.trim();
    let b = model_b.trim();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    fn normalize_model(m: &str) -> String {
        let clean = m.trim().to_ascii_lowercase().replace('_', "-");
        let stripped = clean
            .strip_prefix("fastembed:")
            .or_else(|| clean.strip_prefix("fastembed/"))
            .unwrap_or(&clean);
        match stripped {
            "mock" | "mock-hash" => "mock-hash".to_string(),
            "all-minilm-l6-v2" | "allminilml6v2" | "minilm" => "all-minilm-l6-v2".to_string(),
            "bge-small-en-v1.5" | "bgesmallenv15" | "bge-small" | "bgesmall" => {
                "bge-small-en-v1.5".to_string()
            }
            "bge-base-en-v1.5" | "bgebaseenv15" | "bge-base" | "bgebase" => {
                "bge-base-en-v1.5".to_string()
            }
            other => other.to_string(),
        }
    }
    normalize_model(a) == normalize_model(b)
}

/// Check if a query embedding model can be matched against a stored embedding model.
/// If `stored_model` is None (legacy record created before model tagging),
/// only mock query models are permitted to match for backwards compatibility.
pub fn is_embedding_compatible_with_query(query_model: &str, stored_model: Option<&str>) -> bool {
    match stored_model {
        Some(stored) => are_embedding_models_compatible(query_model, stored),
        None => {
            let q = query_model.trim().to_lowercase();
            q == "mock" || q == "mock-hash"
        }
    }
}
