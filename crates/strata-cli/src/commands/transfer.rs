use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use strata_core::schemas::{TransferFilter, TransferReport};
use strata_core::state::{MemoryTier, MemoryType};
use strata_memory::{SqliteMemoryEngine, SqliteStore};

#[derive(Debug, Clone, Args)]
pub struct TransferArgs {
    /// Path to the source repository or source SQLite database file (.strata/strata.db)
    #[arg(
        long = "from",
        short = 'f',
        help = "Source repository path or SQLite database file path"
    )]
    pub from: PathBuf,

    /// Filter by memory category: 'episodic', 'semantic', 'procedural', 'negative_pattern'
    #[arg(long, help = "Optional memory category filter")]
    pub memory_type: Option<String>,

    /// Filter by memory tier: 'peripheral', 'working', 'core'
    #[arg(long, help = "Optional memory tier filter")]
    pub tier: Option<String>,

    /// Substring search query to filter transferred content
    #[arg(long, short = 'q', help = "Filter transferred items matching query")]
    pub query: Option<String>,

    /// Exclude failure patterns from transfer
    #[arg(long, help = "Do not transfer failure patterns")]
    pub no_failures: bool,

    /// Exclude procedural skills from transfer
    #[arg(long, help = "Do not transfer procedural skills")]
    pub no_skills: bool,

    /// Maximum number of items to transfer per category
    #[arg(long, default_value_t = 100, help = "Maximum items to transfer")]
    pub limit: usize,

    /// Output report as raw JSON
    #[arg(long, help = "Output as raw JSON")]
    pub json: bool,
}

pub async fn run_transfer(args: TransferArgs, engine: Arc<SqliteMemoryEngine>) -> Result<()> {
    let source_db_path = resolve_source_db_path(&args.from)?;

    if !source_db_path.exists() {
        anyhow::bail!(
            "Source database does not exist at: {}",
            source_db_path.display()
        );
    }

    let source_store = SqliteStore::open(&source_db_path).with_context(|| {
        format!(
            "Failed to open source SQLite store at: {}",
            source_db_path.display()
        )
    })?;

    let parsed_type = match args.memory_type.as_deref() {
        Some(t) => match t.to_lowercase().as_str() {
            "episodic" => Some(MemoryType::Episodic),
            "semantic" => Some(MemoryType::Semantic),
            "procedural" => Some(MemoryType::Procedural),
            "negative_pattern" | "failure" | "anti_pattern" => Some(MemoryType::NegativePattern),
            other => anyhow::bail!("Invalid memory_type: '{other}'. Expected episodic, semantic, procedural, or negative_pattern"),
        },
        None => None,
    };

    let parsed_tier = match args.tier.as_deref() {
        Some(t) => match t.to_lowercase().as_str() {
            "peripheral" => Some(MemoryTier::Peripheral),
            "working" => Some(MemoryTier::Working),
            "core" => Some(MemoryTier::Core),
            other => {
                anyhow::bail!("Invalid tier: '{other}'. Expected peripheral, working, or core")
            }
        },
        None => None,
    };

    let filter = TransferFilter {
        memory_type: parsed_type,
        tier: parsed_tier,
        query: args.query.clone(),
        include_failure_patterns: !args.no_failures,
        include_procedural_skills: !args.no_skills,
        limit: args.limit,
    };

    let source_label = args.from.display().to_string();
    let report = engine.transfer_from(&source_store, &filter, &source_label)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print_transfer_report(&report, &source_db_path);

    Ok(())
}

fn resolve_source_db_path(input: &Path) -> Result<PathBuf> {
    if input.is_file() {
        return Ok(input.to_path_buf());
    }

    // Check input/.strata/strata.db
    let candidate = input.join(".strata").join("strata.db");
    if candidate.exists() {
        return Ok(candidate);
    }

    // Check input/strata.db
    let candidate2 = input.join("strata.db");
    if candidate2.exists() {
        return Ok(candidate2);
    }

    Ok(candidate)
}

fn print_transfer_report(report: &TransferReport, source_path: &Path) {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
    println!(
        "║                 📦 STRATA CROSS-PROJECT KNOWLEDGE TRANSFER                           ║"
    );
    println!(
        "╚══════════════════════════════════════════════════════════════════════════════════════╝"
    );
    println!(
        "  Source:                         {}",
        source_path.display()
    );
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
    println!(
        "  ✓ Transferred Memories:         {}",
        report.transferred_memories
    );
    println!(
        "  ✓ Transferred Failure Patterns: {}",
        report.transferred_failure_patterns
    );
    println!(
        "  ✓ Transferred Procedural Skills:{}",
        report.transferred_procedural_skills
    );
    if report.skipped_duplicates > 0 {
        println!(
            "  ⏭️  Skipped Existing Duplicates: {}",
            report.skipped_duplicates
        );
    }
    println!(
        "────────────────────────────────────────────────────────────────────────────────────────"
    );
    println!("✨ Transfer complete. Imported knowledge is active in local workspace search.\n");
}
