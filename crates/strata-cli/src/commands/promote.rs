use anyhow::{Context, Result};
use clap::Args;
use std::io::{self, Write};
use std::sync::Arc;
use uuid::Uuid;

use strata_core::schemas::SemanticFact;
use strata_core::state::{MemoryRecord, MemoryTier};
use strata_memory::SqliteMemoryEngine;

#[derive(Debug, Clone, Args)]
pub struct PromoteArgs {
    /// ID (UUID) of the memory record or semantic fact to promote to Core Tier
    #[arg(long, short = 'i', help = "UUID of the memory record or semantic fact")]
    pub id: String,

    /// Target entity type: 'memory' (default) or 'fact' (semantic fact)
    #[arg(
        long,
        default_value = "memory",
        help = "Entity type: 'memory' or 'fact'"
    )]
    pub entity_type: String,

    /// Optional justification or policy rationale for promotion
    #[arg(
        long,
        short = 'r',
        help = "Optional rationale or policy context for Core promotion"
    )]
    pub reason: Option<String>,

    /// Bypass interactive confirmation prompt (non-interactive CI/CD mode)
    #[arg(
        long,
        short = 'y',
        alias = "force",
        help = "Bypass interactive confirmation prompt"
    )]
    pub yes: bool,

    /// Output result as raw JSON
    #[arg(long, help = "Output as raw JSON")]
    pub json: bool,
}

pub async fn run_promote(args: PromoteArgs, engine: Arc<SqliteMemoryEngine>) -> Result<()> {
    let id = Uuid::parse_str(&args.id)
        .with_context(|| format!("Invalid UUID format for id: '{}'", args.id))?;

    let is_fact =
        args.entity_type.to_lowercase() == "fact" || args.entity_type.to_lowercase() == "semantic";

    if is_fact {
        promote_semantic_fact(&id, &args, engine).await
    } else {
        promote_memory_record(&id, &args, engine).await
    }
}

async fn promote_memory_record(
    id: &Uuid,
    args: &PromoteArgs,
    engine: Arc<SqliteMemoryEngine>,
) -> Result<()> {
    let store = engine.store();
    let mem = store
        .get_memory(id)?
        .with_context(|| format!("Memory record with ID '{}' not found", id))?;

    if mem.tier == MemoryTier::Core && mem.approved_by_human {
        if args.json {
            let res = serde_json::json!({
                "status": "already_promoted",
                "id": mem.id.to_string(),
                "tier": "core",
                "approved_by_human": true,
                "importance": mem.importance,
            });
            println!("{}", serde_json::to_string_pretty(&res)?);
        } else {
            println!(
                "ℹ️  Memory '{}' is already in permanent Core Tier (approved_by_human = true).",
                id
            );
        }
        return Ok(());
    }

    if !args.yes {
        render_memory_modal(&mem, args.reason.as_deref());
        print!(
            "\n  [?] Promote this memory to permanent Core Tier (frozen retention, R=1.0)? [y/N]: "
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_lowercase();

        if trimmed != "y" && trimmed != "yes" {
            if args.json {
                let res = serde_json::json!({
                    "status": "aborted",
                    "id": id.to_string(),
                    "reason": "Promotion cancelled by user",
                });
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                println!(
                    "\n❌ [ABORTED] Promotion cancelled by user. Memory remains in current tier.\n"
                );
            }
            return Ok(());
        }
    }

    let promoted = engine
        .promote_to_core(id, true, args.reason.as_deref())
        .await
        .with_context(|| format!("Failed to promote memory '{}' to Core Tier", id))?;

    if args.json {
        let res = serde_json::json!({
            "status": "success",
            "id": promoted.id.to_string(),
            "tier": "core",
            "approved_by_human": promoted.approved_by_human,
            "importance": promoted.importance,
            "promotion_reason": args.reason,
        });
        println!("{}", serde_json::to_string_pretty(&res)?);
    } else {
        println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║                      ✓ CORE TIER PROMOTION SUCCESSFUL                                ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");
        println!("  ID:                 {}", promoted.id);
        println!("  Tier:               Core (Permanent / Frozen)");
        println!("  Human Approval:     true (Explicitly Approved)");
        println!("  Retention (R):      1.0 (Exempt from ACT-R decay / never pruned)");
        if let Some(r) = &args.reason {
            println!("  Rationale:          {}", r);
        }
        println!("────────────────────────────────────────────────────────────────────────────────────────\n");
    }

    Ok(())
}

async fn promote_semantic_fact(
    id: &Uuid,
    args: &PromoteArgs,
    engine: Arc<SqliteMemoryEngine>,
) -> Result<()> {
    let store = engine.store();
    let fact = store
        .get_semantic_fact(id)?
        .with_context(|| format!("Semantic fact with ID '{}' not found", id))?;

    if fact.tier == MemoryTier::Core && fact.approved_by_human {
        if args.json {
            let res = serde_json::json!({
                "status": "already_promoted",
                "id": fact.id.to_string(),
                "tier": "core",
                "approved_by_human": true,
                "importance": fact.importance,
            });
            println!("{}", serde_json::to_string_pretty(&res)?);
        } else {
            println!("ℹ️  Semantic fact '{}' is already in permanent Core Tier (approved_by_human = true).", id);
        }
        return Ok(());
    }

    if !args.yes {
        render_fact_modal(&fact, args.reason.as_deref());
        print!("\n  [?] Promote this semantic fact to permanent Core Tier (frozen retention, R=1.0)? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_lowercase();

        if trimmed != "y" && trimmed != "yes" {
            if args.json {
                let res = serde_json::json!({
                    "status": "aborted",
                    "id": id.to_string(),
                    "reason": "Promotion cancelled by user",
                });
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                println!("\n❌ [ABORTED] Promotion cancelled by user. Semantic fact remains in current tier.\n");
            }
            return Ok(());
        }
    }

    let promoted = engine
        .promote_fact_to_core(id, true, args.reason.as_deref())
        .await
        .with_context(|| format!("Failed to promote semantic fact '{}' to Core Tier", id))?;

    if args.json {
        let res = serde_json::json!({
            "status": "success",
            "id": promoted.id.to_string(),
            "tier": "core",
            "approved_by_human": promoted.approved_by_human,
            "importance": promoted.importance,
            "promotion_reason": args.reason,
        });
        println!("{}", serde_json::to_string_pretty(&res)?);
    } else {
        println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║                      ✓ CORE TIER PROMOTION SUCCESSFUL                                ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");
        println!("  ID:                 {}", promoted.id);
        println!("  Tier:               Core (Permanent / Frozen)");
        println!("  Human Approval:     true (Explicitly Approved)");
        println!("  Retention (R):      1.0 (Exempt from ACT-R decay / never pruned)");
        if let Some(r) = &args.reason {
            println!("  Rationale:          {}", r);
        }
        println!("────────────────────────────────────────────────────────────────────────────────────────\n");
    }

    Ok(())
}

fn render_memory_modal(mem: &MemoryRecord, reason: Option<&str>) {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
    println!(
        "║                  🛡️  STRATA CORE TIER PROMOTION APPROVAL MODAL                       ║"
    );
    println!(
        "╚══════════════════════════════════════════════════════════════════════════════════════╝"
    );
    println!("  Memory ID:          {}", mem.id);
    println!("  Memory Type:        {}", mem.memory_type);
    println!("  Scope:              {}", mem.scope);
    println!("  Current Tier:       {:?}", mem.tier);
    println!("  Target Tier:        Core (Permanent / Frozen R=1.0)");
    if let Some(summary) = &mem.summary {
        println!("  Summary:            {}", summary);
    }
    if let Some(r) = reason {
        println!("  Rationale:          {}", r);
    }
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
    println!("  Content Preview:");
    for line in mem.content.lines().take(6) {
        println!("    {}", line);
    }
    if mem.content.lines().count() > 6 {
        println!("    [truncated]...");
    }
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
    println!(
        "  ⚠️  WARNING: Core Tier memories are never pruned and anchor all future agent reasoning."
    );
    println!("               Only promote verified architectural invariants and golden rules.");
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
}

fn render_fact_modal(fact: &SemanticFact, reason: Option<&str>) {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
    println!(
        "║               🛡️  STRATA CORE TIER PROMOTION APPROVAL MODAL (FACT)                   ║"
    );
    println!(
        "╚══════════════════════════════════════════════════════════════════════════════════════╝"
    );
    println!("  Fact ID:            {}", fact.id);
    println!("  Category:           {}", fact.category);
    println!("  Scope:              {}", fact.scope);
    println!("  Current Tier:       {:?}", fact.tier);
    println!("  Target Tier:        Core (Permanent / Frozen R=1.0)");
    if let Some(r) = reason {
        println!("  Rationale:          {}", r);
    }
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
    println!("  Statement:          {}", fact.statement);
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
    println!(
        "  ⚠️  WARNING: Core Tier facts permanently guide multi-agent decisions and cannot decay."
    );
    println!("               Only promote verified architectural invariants and golden rules.");
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
}
