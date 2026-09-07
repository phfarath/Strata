use async_trait::async_trait;
use std::sync::Arc;
use strata_core::traits::ReasoningEngine;

/// Trait defining the enrichment layer for consolidated memories.
/// The core memory engine functions 100% deterministically with `CanonicalTemplateEnricher` (0 tokens, offline).
/// When an LLM provider is available, `AsyncLlmEnricher` can be attached to optionally polish text in the background.
#[async_trait]
pub trait MemoryEnricher: Send + Sync {
    async fn enrich_summary(
        &self,
        title: &str,
        raw_summary: &str,
        context: &serde_json::Value,
    ) -> String;
}

/// Default canonical enricher: 100% offline, 0 tokens, zero latency.
#[derive(Debug, Clone, Default)]
pub struct CanonicalTemplateEnricher;

impl CanonicalTemplateEnricher {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MemoryEnricher for CanonicalTemplateEnricher {
    async fn enrich_summary(
        &self,
        title: &str,
        raw_summary: &str,
        _context: &serde_json::Value,
    ) -> String {
        let trimmed = raw_summary.trim();
        if trimmed.is_empty() {
            title.to_string()
        } else if trimmed.starts_with(title) {
            trimmed.to_string()
        } else {
            format!("{title}: {trimmed}")
        }
    }
}

/// Optional asynchronous LLM enricher: polishes text in background without blocking memory insertion.
pub struct AsyncLlmEnricher {
    engine: Arc<dyn ReasoningEngine>,
    fallback: CanonicalTemplateEnricher,
}

impl AsyncLlmEnricher {
    pub fn new(engine: Arc<dyn ReasoningEngine>) -> Self {
        Self {
            engine,
            fallback: CanonicalTemplateEnricher::new(),
        }
    }
}

#[async_trait]
impl MemoryEnricher for AsyncLlmEnricher {
    async fn enrich_summary(
        &self,
        title: &str,
        raw_summary: &str,
        context: &serde_json::Value,
    ) -> String {
        let prompt = format!(
            "Rewrite the following technical memory into one concise, clear, fluent summary sentence for a software agent.\nTitle: {}\nRaw content: {}\nContext: {}\nRespond with ONLY the single summary sentence.",
            title, raw_summary, context
        );

        match self
            .engine
            .prompt(
                Some("You are a concise technical summarizer."),
                &prompt,
                None,
            )
            .await
        {
            Ok(polished) => {
                let p = polished.trim();
                if p.is_empty() {
                    self.fallback
                        .enrich_summary(title, raw_summary, context)
                        .await
                } else {
                    p.to_string()
                }
            }
            Err(_) => {
                self.fallback
                    .enrich_summary(title, raw_summary, context)
                    .await
            }
        }
    }
}
