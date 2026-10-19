use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;
use std::time::Instant;
use strata_core::config::StrataConfig;
use strata_memory::{EmbeddingProviderFactory, SqliteStore};

#[derive(Args, Debug, Clone)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub action: MemoryAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemoryAction {
    /// Inspect active embedding provider, model, dimension, and vector consistency stats
    #[command(name = "status", alias = "stats", alias = "info", alias = "diagnose")]
    Status {
        #[arg(long, help = "Output status and vector statistics as raw JSON")]
        json: bool,
    },

    /// Recompute embeddings across memories, semantic facts, and procedural skills using the active provider
    #[command(name = "reindex", alias = "re-embed")]
    Reindex {
        #[arg(
            long,
            default_value_t = 64,
            help = "Batch size for embedding generation"
        )]
        batch_size: usize,

        #[arg(
            long,
            help = "Re-embed records even if their embedding model already matches the active provider"
        )]
        force: bool,

        #[arg(long, help = "Simulate reindexing without writing changes to SQLite")]
        dry_run: bool,

        #[arg(long, help = "Output report as raw JSON")]
        json: bool,
    },
}

pub async fn run_memory(args: MemoryArgs, store: Arc<SqliteStore>) -> Result<()> {
    let config = StrataConfig::load();
    let resolved = EmbeddingProviderFactory::create_from_config(&config)
        .context("Failed to initialize embedding provider from configuration")?;
    let embedder = resolved.provider;

    match args.action {
        MemoryAction::Status { json } => {
            let stats = store
                .get_vector_consistency_stats(Some(embedder.model_name()))
                .context("Failed to query vector consistency statistics")?;

            if json {
                let json_output = serde_json::json!({
                    "provider": embedder.provider_name(),
                    "model": embedder.model_name(),
                    "version": embedder.model_version(),
                    "dimension": embedder.dimension(),
                    "is_fallback": resolved.is_fallback,
                    "fallback_warning": resolved.fallback_reason,
                    "stats": stats,
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            } else {
                println!("\n🧠 [Strata Memory & Vector Space Status]");
                println!("══════════════════════════════════════════════════════════════════");
                println!("Provider:           {}", embedder.provider_name());
                println!(
                    "Active Model:       {} (v{})",
                    embedder.model_name(),
                    embedder.model_version()
                );
                println!("Dimension:          {}", embedder.dimension());
                if resolved.is_fallback {
                    println!(
                        "Fallback Mode:      YES - {}",
                        resolved
                            .fallback_reason
                            .as_deref()
                            .unwrap_or("Fallback provider active")
                    );
                } else {
                    println!(
                        "Fallback Mode:      No (Active provider explicitly configured/detected)"
                    );
                }

                println!("\n📊 Vector Consistency:");
                if stats.is_consistent {
                    println!("  Status:           ✓ Consistent (Uniform semantic space)");
                } else {
                    println!("  Status:           ⚠️ Inconsistent (Multiple models or untagged legacy vectors detected)");
                }
                println!("  Total Vectors:    {}", stats.total_vectors);
                println!(
                    "  - Memories:       {} / {} with vectors",
                    stats.memories_with_embeddings, stats.total_memories
                );
                println!(
                    "  - Semantic Facts: {} / {} with vectors",
                    stats.facts_with_embeddings, stats.total_semantic_facts
                );
                println!(
                    "  - Skills:         {} / {} with vectors",
                    stats.skills_with_embeddings, stats.total_procedural_skills
                );
                println!(
                    "  - Episodes:       {} / {} with vectors",
                    stats.episodes_with_embeddings, stats.total_episodic_memories
                );

                if !stats.model_breakdown.is_empty() {
                    println!("\n📦 Model Distribution:");
                    for mb in &stats.model_breakdown {
                        println!(
                            "  • {:<18} | {:<20} (v{}) | count: {}",
                            mb.table, mb.model, mb.version, mb.count
                        );
                    }
                }

                if !stats.recommendations.is_empty() {
                    println!("\n💡 Recommendations:");
                    for rec in &stats.recommendations {
                        println!("  • {rec}");
                    }
                }
                println!("══════════════════════════════════════════════════════════════════\n");
            }
        }
        MemoryAction::Reindex {
            batch_size,
            force,
            dry_run,
            json,
        } => {
            if dry_run {
                let stats = store
                    .get_vector_consistency_stats(Some(embedder.model_name()))
                    .context("Failed to query vector consistency statistics")?;

                let to_reindex = if force {
                    stats.total_memories
                        + stats.total_semantic_facts
                        + stats.total_procedural_skills
                } else {
                    let active = embedder.model_name();
                    let current_matched: usize = stats
                        .model_breakdown
                        .iter()
                        .filter(|mb| mb.model == active)
                        .map(|mb| mb.count)
                        .sum();
                    (stats.total_memories
                        + stats.total_semantic_facts
                        + stats.total_procedural_skills)
                        .saturating_sub(current_matched)
                };

                if json {
                    let json_output = serde_json::json!({
                        "dry_run": true,
                        "target_provider": embedder.provider_name(),
                        "target_model": embedder.model_name(),
                        "target_version": embedder.model_version(),
                        "dimension": embedder.dimension(),
                        "records_to_reindex": to_reindex,
                        "force": force,
                        "batch_size": batch_size,
                    });
                    println!("{}", serde_json::to_string_pretty(&json_output)?);
                } else {
                    println!("\n🔄 [Strata Memory Vector Reindex (Simulated / Dry-Run)]");
                    println!("══════════════════════════════════════════════════════════════════");
                    println!("Target Provider:    {}", embedder.provider_name());
                    println!(
                        "Target Model:       {} (v{}, dim: {})",
                        embedder.model_name(),
                        embedder.model_version(),
                        embedder.dimension()
                    );
                    println!("Records to Reindex: {}", to_reindex);
                    println!("Force Mode:         {}", force);
                    println!("Batch Size:         {}", batch_size);
                    println!(
                        "ℹ️ No database modifications made. Run without --dry-run to execute."
                    );
                    println!(
                        "══════════════════════════════════════════════════════════════════\n"
                    );
                }
                return Ok(());
            }

            let start = Instant::now();
            let report = store
                .reindex_embeddings(embedder.as_ref(), force, batch_size)
                .await
                .context("Failed to reindex embeddings in SQLite database")?;
            let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

            if json {
                let json_output = serde_json::json!({
                    "provider": embedder.provider_name(),
                    "report": report,
                    "elapsed_ms": duration_ms,
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            } else {
                println!("\n🔄 [Strata Memory Vector Reindex]");
                println!("══════════════════════════════════════════════════════════════════");
                println!("Target Provider:    {}", embedder.provider_name());
                println!(
                    "Target Model:       {} (v{}, dim: {})",
                    report.target_model, report.target_version, report.dimension
                );
                println!("Total Reindexed:    {}", report.total_reindexed);
                println!("  - Memories:       {}", report.memories_reindexed);
                println!("  - Semantic Facts: {}", report.facts_reindexed);
                println!("  - Skills:         {}", report.skills_reindexed);
                println!("  - Episodes:       {}", report.episodes_reindexed);
                println!("⏱️ Elapsed Time:     {:.2}ms", duration_ms);
                println!("✓ Vector reindexing completed successfully.");
                println!("══════════════════════════════════════════════════════════════════\n");
            }
        }
    }

    Ok(())
}
