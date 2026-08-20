use std::sync::Arc;
use anyhow::Result;
use chrono::Utc;
use tracing::info;

use strata_core::schemas::FactStatus;
use strata_core::state::MemoryTier;
use strata_memory::{DecayCalculator, SqliteStore};

pub struct PruneOptions {
    pub threshold: f32,
    pub scope: Option<String>,
    pub dry_run: bool,
    pub json: bool,
}

pub async fn run_prune(opts: PruneOptions, store: Arc<SqliteStore>) -> Result<()> {
    info!(
        threshold = opts.threshold,
        scope = ?opts.scope,
        dry_run = opts.dry_run,
        "Executing mathematical ACT-R / Ebbinghaus memory decay & Tri-Tier cold storage pruning"
    );

    let calculator = DecayCalculator::with_default_config();
    let now = Utc::now();

    let (report, cold_count) = if opts.dry_run {
        // In dry-run mode, simulate evaluation without writing changes
        let mut rep = strata_memory::PruneReport::default();
        let facts = store.get_all_semantic_facts(None, Some(FactStatus::Active), 10000)?;
        for fact in facts {
            if fact.tier == MemoryTier::Core || fact.importance >= calculator.config.invariant_threshold {
                rep.core_protected += 1;
            } else if fact.tier == MemoryTier::Working {
                rep.working_active += 1;
            } else {
                let logs = store.get_memory_access_logs(&fact.id)?;
                let metrics = calculator.evaluate_semantic_fact(&fact, &logs, now);
                if metrics.retention < opts.threshold {
                    rep.facts_pruned += 1;
                }
            }
        }

        let memories = store.get_all_memories(None, None, 10000)?;
        for mem in memories {
            if mem.tier == MemoryTier::Core || mem.importance >= calculator.config.invariant_threshold {
                rep.core_protected += 1;
            } else if mem.tier == MemoryTier::Working {
                rep.working_active += 1;
            } else {
                let logs = store.get_memory_access_logs(&mem.id)?;
                let metrics = calculator.evaluate_memory_record(&mem, &logs, now);
                if metrics.retention < opts.threshold {
                    rep.memories_archived += 1;
                    rep.memories_pruned += 1;
                }
            }
        }
        let cold = store.get_cold_storage_count().unwrap_or(0);
        (rep, cold)
    } else {
        let rep = calculator.prune_expired(&store, Some(opts.threshold), Some(now))?;
        let cold = store.get_cold_storage_count().unwrap_or(0);
        (rep, cold)
    };

    if opts.json {
        let json_report = serde_json::json!({
            "threshold": opts.threshold,
            "scope": opts.scope,
            "dry_run": opts.dry_run,
            "core_protected": report.core_protected,
            "working_active": report.working_active,
            "memories_archived_to_cold_storage": report.memories_archived,
            "memories_pruned": report.memories_pruned,
            "facts_pruned": report.facts_pruned,
            "skills_pruned": report.skills_pruned,
            "total_cold_storage_count": cold_count,
            "total_pruned": report.memories_pruned + report.facts_pruned + report.skills_pruned,
        });
        println!("{}", serde_json::to_string_pretty(&json_report)?);
    } else {
        let dry_prefix = if opts.dry_run { " [SIMULATION / DRY RUN]" } else { "" };
        println!("\n🧹 [Strata Tri-Tier Memory Decay & Prune Report{dry_prefix}]");
        println!("══════════════════════════════════════════════════════════");
        println!("Retention Threshold (θ_prune): {:.2}", opts.threshold);
        if let Some(ref s) = opts.scope {
            println!("Scope Filter:                  {s}");
        } else {
            println!("Scope Filter:                  [All Scopes]");
        }
        println!("──────────────────────────────────────────────────────────");
        println!("🛡️  Core Tier (Immune/Protected):    {} records", report.core_protected);
        println!("⚡  Working Tier (Active Session):    {} records", report.working_active);
        println!("📦  Peripheral -> Cold Storage:       {} records archived", report.memories_archived);
        println!("🏷️  Semantic Facts Pruned:            {} facts deprecated", report.facts_pruned);
        println!("💾  Total Cold Storage In-Disk:       {} records", cold_count);
        println!("──────────────────────────────────────────────────────────");
        println!("Total Pruned/Archived:               {}\n", report.memories_pruned + report.facts_pruned + report.skills_pruned);
    }

    Ok(())
}
