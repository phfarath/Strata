use std::sync::Arc;
use anyhow::{Context, Result};
use tracing::info;

use strata_memory::{ConsolidationPipeline, MockEmbeddingProvider, SqliteStore};
use strata_reasoning::{AnthropicAdapter, MockReasoningEngine, OpenAiAdapter, OpenRouterAdapter};

pub struct ConsolidateOptions {
    pub session: Option<String>,
    pub all: bool,
    pub model: Option<String>,
    pub json: bool,
}

pub async fn run_consolidate(opts: ConsolidateOptions, store: Arc<SqliteStore>) -> Result<()> {
    let reasoning_engine = resolve_reasoning_engine(opts.model.as_deref());
    let pipeline = ConsolidationPipeline::with_default_config();
    let embedder = MockEmbeddingProvider::default();

    let events = if opts.all {
        info!("Running consolidation across all recorded sessions");
        store.get_all_events()?
    } else {
        let session_id = opts.session.clone().unwrap_or_else(|| "default".to_string());
        info!("Running consolidation for session '{session_id}'");
        store.get_events(&session_id, None, None)?
    };

    let result = pipeline
        .run_pipeline(&store, &embedder, &events, Some(reasoning_engine.as_ref()))
        .await
        .context("Failed to run consolidation pipeline")?;

    if opts.json {
        let json_report = serde_json::json!({
            "session": opts.session,
            "events_processed": result.events_processed,
            "episodic_created": result.episodic_memories.len(),
            "semantic_created": result.semantic_facts.len(),
            "procedural_created": result.procedural_skills.len(),
            "conflicts_resolved": result.conflicts_resolved,
            "memories_pruned": result.memories_pruned,
        });
        println!("{}", serde_json::to_string_pretty(&json_report)?);
    } else {
        println!("\n🧠 [Strata Memory Consolidation Report]");
        println!("═════════════════════════════════════════");
        if let Some(ref sid) = opts.session {
            println!("Target Session:          {sid}");
        } else {
            println!("Target Session:          [All Sessions]");
        }
        println!("📊 Events Processed:     {}", result.events_processed);
        println!("📖 Episodic Memories:    {}", result.episodic_memories.len());
        println!("💡 Semantic Facts:       {} ({} JTMS updates)", result.semantic_facts.len(), result.conflicts_resolved);
        println!("🛠️ Procedural Skills:    {}", result.procedural_skills.len());
        println!("🧹 Memories Pruned:      {}", result.memories_pruned);
        println!("─────────────────────────────────────────\n");
    }

    Ok(())
}

pub fn resolve_reasoning_engine(model_slug: Option<&str>) -> Arc<dyn strata_core::traits::ReasoningEngine> {
    // 1. Try OpenRouter (first priority per spec)
    if let Ok(adapter) = OpenRouterAdapter::from_env(model_slug.map(|s| s.to_string())) {
        return Arc::new(adapter);
    }

    // 2. Try OpenAI
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        let model = model_slug.unwrap_or("gpt-4o-mini");
        return Arc::new(OpenAiAdapter::new(key, model));
    }

    // 3. Try Anthropic
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        let model = model_slug.unwrap_or("claude-3-5-sonnet-20241022");
        return Arc::new(AnthropicAdapter::new(key, model));
    }

    // 4. Default: Deterministic Mock Engine
    Arc::new(MockReasoningEngine::new())
}
