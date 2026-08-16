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
}

/// FastEmbed local embedding provider using ONNX models.
pub struct FastEmbedProvider {
    model: std::sync::Arc<std::sync::Mutex<fastembed::TextEmbedding>>,
    dimension: usize,
}

impl FastEmbedProvider {
    pub fn try_new() -> Result<Self, StrataError> {
        let mut options = fastembed::InitOptions::default();
        options.model_name = fastembed::EmbeddingModel::AllMiniLML6V2;
        options.show_download_progress = false;
        let model = fastembed::TextEmbedding::try_new(options)
            .map_err(|e| StrataError::Embedding(format!("Failed to initialize FastEmbed: {e}")))?;

        Ok(Self {
            model: std::sync::Arc::new(std::sync::Mutex::new(model)),
            dimension: 384,
        })
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
}
