use std::sync::Arc;
use anyhow::Result;
use chrono::Utc;
use tracing::info;

use strata_memory::{DecayCalculator, SqliteStore};

pub struct PruneOptions {
    pub threshold: f32,
    pub scope: Option<String>,
    pub json: bool,
}

pub async fn run_prune(opts: PruneOptions, store: Arc<SqliteStore>) -> Result<()> {
    info!(
        threshold = opts.threshold,
        scope = ?opts.scope,
        "Executing mathematical ACT-R / Ebbinghaus memory decay & pruning"
    );

    let calculator = DecayCalculator::with_default_config();
    let report = calculator.prune_expired(&store, Some(opts.threshold), Some(Utc::now()))?;

    if opts.json {
        let json_report = serde_json::json!({
            "threshold": opts.threshold,
            "scope": opts.scope,
            "memories_pruned": report.memories_pruned,
            "facts_pruned": report.facts_pruned,
            "skills_pruned": report.skills_pruned,
            "total_pruned": report.memories_pruned + report.facts_pruned + report.skills_pruned,
        });
        println!("{}", serde_json::to_string_pretty(&json_report)?);
    } else {
        println!("\n🧹 [Strata Memory Decay & Prune Report]");
        println!("═════════════════════════════════════════");
        println!("Retention Threshold:     {:.2}", opts.threshold);
        if let Some(ref s) = opts.scope {
            println!("Scope Filter:            {s}");
        } else {
            println!("Scope Filter:            [All Scopes]");
        }
        println!("General Memories Pruned: {}", report.memories_pruned);
        println!("Semantic Facts Pruned:   {}", report.facts_pruned);
        println!("Procedural Skills Pruned:{}", report.skills_pruned);
        println!("─────────────────────────────────────────");
        println!("Total Pruned:            {}\n", report.memories_pruned + report.facts_pruned + report.skills_pruned);
    }

    Ok(())
}
