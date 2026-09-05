use anyhow::{bail, Context, Result};
use clap::Args;
use std::sync::Arc;
use strata_core::schemas::MemoryFeedback;
use strata_memory::SqliteStore;
use uuid::Uuid;

#[derive(Debug, Clone, Args)]
pub struct FeedbackArgs {
    #[arg(long, help = "UUID of the memory record to provide feedback on")]
    pub id: Option<String>,

    #[arg(
        long,
        default_value = "positive",
        help = "Feedback rating: positive or negative"
    )]
    pub rating: String,

    #[arg(long, help = "Optional feedback comment or correction rationale")]
    pub comment: Option<String>,

    #[arg(long, help = "Optional feedback source (user, test, agent)")]
    pub source: Option<String>,

    #[arg(long, help = "Output as raw JSON")]
    pub json: bool,
}

pub async fn run_feedback(args: FeedbackArgs, store: Arc<SqliteStore>) -> Result<()> {
    let id_str = match &args.id {
        Some(s) if !s.trim().is_empty() => s.clone(),
        _ => bail!("Memory record ID is required (specify with --id <uuid>)"),
    };

    let mem_uuid = Uuid::parse_str(&id_str)
        .with_context(|| format!("Invalid memory UUID format: '{id_str}'"))?;

    let rating_clean = args.rating.trim().to_lowercase();
    if rating_clean != "positive" && rating_clean != "negative" {
        bail!(
            "Rating must be 'positive' or 'negative', got: '{}'",
            args.rating
        );
    }

    let mut fb = if rating_clean == "positive" {
        MemoryFeedback::positive(mem_uuid)
    } else {
        MemoryFeedback::negative(mem_uuid, args.comment.clone())
    };

    if let Some(c) = &args.comment {
        fb = fb.with_comment(c);
    }

    store.record_memory_feedback(&fb).with_context(|| {
        format!("Failed to record feedback for memory '{id_str}' in SQLite store")
    })?;

    let updated_memory = store.get_memory(&mem_uuid)?;

    if args.json {
        let output = serde_json::json!({
            "status": "feedback_recorded",
            "memory_id": id_str,
            "rating": rating_clean,
            "comment": args.comment,
            "source": args.source,
            "updated_record": updated_memory,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("\n⭐ [Strata Memory Feedback Recorded]");
        println!("═════════════════════════════════════════");
        println!("Memory ID:   {}", id_str);
        println!("Rating:      {}", rating_clean);
        if let Some(c) = &args.comment {
            println!("Comment:     {}", c);
        }
        if let Some(s) = &args.source {
            println!("Source:      {}", s);
        }
        if let Some(mem) = updated_memory {
            println!("New Importance: {:.2}", mem.importance);
            println!("New Confidence: {:.2}", mem.confidence);
            println!("Access Count:   {}", mem.access_count);
        }
        println!("\n✓ Reinforcement feedback applied to memory ranking and recall.\n");
    }

    Ok(())
}
