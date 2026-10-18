use anyhow::{Context, Result};
use clap::Args;
use std::io::{self, Write};
use std::sync::Arc;
use uuid::Uuid;

use strata_core::schemas::SemanticFact;
use strata_core::state::{HumanApprovalRecord, MemoryRecord, MemoryTier};
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

    /// Promote to Developer-Global store (~/.strata/global.db) with Scope::Global
    #[arg(
        long = "to-global",
        alias = "global",
        help = "Promote to Developer-Global store (~/.strata/global.db) with Scope::Global"
    )]
    pub to_global: bool,

    /// Generate a single-use cryptographically secure approval token instead of promoting immediately
    #[arg(
        long = "generate-token",
        help = "Generate a single-use cryptographically secure approval token instead of promoting immediately"
    )]
    pub generate_token: bool,

    /// Verifiable approval token to consume for promotion
    #[arg(
        long = "token",
        help = "Verifiable approval token to consume for promotion"
    )]
    pub token: Option<String>,

    /// Approver identity (defaults to detected developer / system username)
    #[arg(
        long = "approver",
        help = "Approver identity (defaults to detected developer / system username)"
    )]
    pub approver: Option<String>,

    /// Output result as raw JSON
    #[arg(long, help = "Output as raw JSON")]
    pub json: bool,
}

pub fn detect_human_user(approver_arg: Option<&str>) -> String {
    if let Some(app) = approver_arg {
        if !app.trim().is_empty() {
            return app.trim().to_string();
        }
    }
    if let Ok(val) = std::env::var("STRATA_APPROVER") {
        if !val.trim().is_empty() {
            return val.trim().to_string();
        }
    }
    if let Ok(val) = std::env::var("USER") {
        if !val.trim().is_empty() {
            return val.trim().to_string();
        }
    }
    if let Ok(val) = std::env::var("USERNAME") {
        if !val.trim().is_empty() {
            return val.trim().to_string();
        }
    }
    "cli_user".to_string()
}

pub async fn run_promote(args: PromoteArgs, engine: Arc<SqliteMemoryEngine>) -> Result<()> {
    let id = Uuid::parse_str(&args.id)
        .with_context(|| format!("Invalid UUID format for id: '{}'", args.id))?;

    if args.generate_token {
        return generate_token_flow(&id, &args, engine).await;
    }

    if args.to_global {
        return promote_to_global_store(&id, &args, engine).await;
    }

    let is_fact =
        args.entity_type.to_lowercase() == "fact" || args.entity_type.to_lowercase() == "semantic";

    if is_fact {
        promote_semantic_fact(&id, &args, engine).await
    } else {
        promote_memory_record(&id, &args, engine).await
    }
}

async fn generate_token_flow(
    id: &Uuid,
    args: &PromoteArgs,
    engine: Arc<SqliteMemoryEngine>,
) -> Result<()> {
    let is_fact = args.entity_type.to_lowercase() == "fact" || args.entity_type.to_lowercase() == "semantic";
    let (content_hash, entity_desc) = if is_fact {
        let fact = engine.store().get_semantic_fact(id)?
            .with_context(|| format!("Semantic fact with ID '{}' not found", id))?;
        (
            blake3::hash(fact.statement.as_bytes()).to_hex().to_string(),
            format!("Semantic Fact: \"{}\"", fact.statement),
        )
    } else {
        let mem = engine.store().get_memory(id)?
            .with_context(|| format!("Memory record with ID '{}' not found", id))?;
        (
            blake3::hash(mem.content.as_bytes()).to_hex().to_string(),
            format!("Memory: \"{}\"", mem.summary.as_deref().unwrap_or(&mem.content)),
        )
    };

    let approver = detect_human_user(args.approver.as_deref());
    let ttl_seconds = 3600u64;

    let token = engine.generate_approval_token(
        id,
        if is_fact { "fact" } else { "memory" },
        &content_hash,
        &approver,
        ttl_seconds,
        args.reason.as_deref(),
    ).with_context(|| "Failed to generate verifiable approval token")?;

    if args.json {
        let res = serde_json::json!({
            "status": "token_generated",
            "id": id.to_string(),
            "entity_type": if is_fact { "fact" } else { "memory" },
            "token": token,
            "content_hash": content_hash,
            "approved_by": approver,
            "expires_in_seconds": ttl_seconds,
            "reason": args.reason,
        });
        println!("{}", serde_json::to_string_pretty(&res)?);
    } else {
        println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║               🔑  STRATA VERIFIABLE HUMAN APPROVAL TOKEN GENERATED                   ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");
        println!("  Target ID:          {}", id);
        println!("  Entity:             {}", entity_desc);
        println!("  BLAKE3 Hash:        {}", content_hash);
        println!("  Approver:           {}", approver);
        println!("  Expires In:         {} seconds (1 hour)", ttl_seconds);
        if let Some(r) = &args.reason {
            println!("  Rationale:          {}", r);
        }
        println!("────────────────────────────────────────────────────────────────────────────────────────");
        println!("  Token:              {}", token);
        println!("────────────────────────────────────────────────────────────────────────────────────────");
        println!("  ℹ️  AI agents may now promote this record via MCP tool `strata_promote`");
        println!("     passing `approval_token: \"{}\"`.", token);
        println!("────────────────────────────────────────────────────────────────────────────────────────\n");
    }

    Ok(())
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
                "human_approval": mem.human_approval,
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

    let content_hash = blake3::hash(mem.content.as_bytes()).to_hex().to_string();

    let approval = if let Some(ref tok) = args.token {
        let mut rec = engine
            .verify_and_consume_approval_token(tok, id, &content_hash)
            .with_context(|| "Failed to verify approval token")?;
        if let Some(ref r) = args.reason {
            if rec.rationale.is_none() {
                rec.rationale = Some(r.clone());
            }
        }
        rec
    } else {
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

        let channel = if args.yes {
            "cli_flag".to_string()
        } else {
            "cli_interactive".to_string()
        };
        let approver = detect_human_user(args.approver.as_deref());

        HumanApprovalRecord {
            approved_by: approver,
            approved_at: chrono::Utc::now(),
            channel,
            content_hash,
            rationale: args.reason.clone(),
        }
    };

    let promoted = engine
        .promote_to_core_with_approval(id, approval.clone())
        .await
        .with_context(|| format!("Failed to promote memory '{}' to Core Tier", id))?;

    let provenance = promoted.build_provenance(Some("Promoted to Core Tier with verified human approval"));

    if args.json {
        let res = serde_json::json!({
            "status": "success",
            "id": promoted.id.to_string(),
            "tier": "core",
            "approved_by_human": promoted.approved_by_human,
            "importance": promoted.importance,
            "promotion_reason": args.reason,
            "human_approval": promoted.human_approval,
            "provenance": provenance,
        });
        println!("{}", serde_json::to_string_pretty(&res)?);
    } else {
        println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║                      ✓ CORE TIER PROMOTION SUCCESSFUL                                ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");
        println!("  ID:                 {}", promoted.id);
        println!("  Tier:               Core (Permanent / Frozen)");
        println!("  Human Approval:     true (Explicitly Approved)");
        println!("  Approver:           {}", approval.approved_by);
        println!("  Channel:            {}", approval.channel);
        println!("  BLAKE3 Hash:        {}", approval.content_hash);
        println!("  Retention (R):      1.0 (Exempt from ACT-R decay / never pruned)");
        if let Some(r) = &approval.rationale {
            println!("  Rationale:          {}", r);
        }
        println!("────────────────────────────────────────────────────────────────────────────────────────\n");
    }

    Ok(())
}

async fn promote_to_global_store(
    id: &Uuid,
    args: &PromoteArgs,
    engine: Arc<SqliteMemoryEngine>,
) -> Result<()> {
    if engine.global_store().is_none() {
        anyhow::bail!(
            "Developer-Global store is not available or configured. Cannot promote to global."
        );
    }

    let mem = engine.store().get_memory(id)?.with_context(|| {
        format!(
            "Memory record with ID '{}' not found in local workspace store",
            id
        )
    })?;

    let content_hash = blake3::hash(mem.content.as_bytes()).to_hex().to_string();

    let approval = if let Some(ref tok) = args.token {
        let mut rec = engine
            .verify_and_consume_approval_token(tok, id, &content_hash)
            .with_context(|| "Failed to verify approval token")?;
        if let Some(ref r) = args.reason {
            if rec.rationale.is_none() {
                rec.rationale = Some(r.clone());
            }
        }
        rec
    } else {
        if !args.yes {
            render_memory_modal(&mem, args.reason.as_deref());
            print!(
                "\n  [?] Promote this memory to Developer-Global Store (~/.strata/global.db) with Scope::Global? [y/N]: "
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
                    println!("\n❌ [ABORTED] Global promotion cancelled by user.\n");
                }
                return Ok(());
            }
        }

        let channel = if args.yes {
            "cli_flag".to_string()
        } else {
            "cli_interactive".to_string()
        };
        let approver = detect_human_user(args.approver.as_deref());

        HumanApprovalRecord {
            approved_by: approver,
            approved_at: chrono::Utc::now(),
            channel,
            content_hash,
            rationale: args.reason.clone(),
        }
    };

    let promoted = engine
        .promote_to_global_with_approval(id, approval.clone())
        .await
        .with_context(|| format!("Failed to promote memory '{}' to Global Store", id))?;

    let provenance = promoted.build_provenance(Some("Promoted to Global Store with verified human approval"));

    if args.json {
        let res = serde_json::json!({
            "status": "success",
            "id": promoted.id.to_string(),
            "scope": "global",
            "tier": "core",
            "approved_by_human": promoted.approved_by_human,
            "promotion_reason": args.reason,
            "destination": "~/.strata/global.db",
            "human_approval": promoted.human_approval,
            "provenance": provenance,
        });
        println!("{}", serde_json::to_string_pretty(&res)?);
    } else {
        println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║                 ✓ DEVELOPER-GLOBAL KNOWLEDGE PROMOTION SUCCESSFUL                    ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");
        println!("  ID:                 {}", promoted.id);
        println!("  Scope:              Global (Accessible across all repositories)");
        println!("  Tier:               Core (Permanent / Frozen retention)");
        println!("  Destination:        ~/.strata/global.db");
        println!("  Approver:           {}", approval.approved_by);
        println!("  Channel:            {}", approval.channel);
        println!("  BLAKE3 Hash:        {}", approval.content_hash);
        if let Some(r) = &approval.rationale {
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
                "human_approval": fact.human_approval,
            });
            println!("{}", serde_json::to_string_pretty(&res)?);
        } else {
            println!("ℹ️  Semantic fact '{}' is already in permanent Core Tier (approved_by_human = true).", id);
        }
        return Ok(());
    }

    let content_hash = blake3::hash(fact.statement.as_bytes()).to_hex().to_string();

    let approval = if let Some(ref tok) = args.token {
        let mut rec = engine
            .verify_and_consume_approval_token(tok, id, &content_hash)
            .with_context(|| "Failed to verify approval token")?;
        if let Some(ref r) = args.reason {
            if rec.rationale.is_none() {
                rec.rationale = Some(r.clone());
            }
        }
        rec
    } else {
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

        let channel = if args.yes {
            "cli_flag".to_string()
        } else {
            "cli_interactive".to_string()
        };
        let approver = detect_human_user(args.approver.as_deref());

        HumanApprovalRecord {
            approved_by: approver,
            approved_at: chrono::Utc::now(),
            channel,
            content_hash,
            rationale: args.reason.clone(),
        }
    };

    let promoted = engine
        .promote_fact_to_core_with_approval(id, approval.clone())
        .await
        .with_context(|| format!("Failed to promote semantic fact '{}' to Core Tier", id))?;

    let provenance = promoted.build_provenance(Some("Promoted to Core Tier with verified human approval"));

    if args.json {
        let res = serde_json::json!({
            "status": "success",
            "id": promoted.id.to_string(),
            "tier": "core",
            "approved_by_human": promoted.approved_by_human,
            "importance": promoted.importance,
            "promotion_reason": args.reason,
            "human_approval": promoted.human_approval,
            "provenance": provenance,
        });
        println!("{}", serde_json::to_string_pretty(&res)?);
    } else {
        println!("\n╔══════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║                      ✓ CORE TIER PROMOTION SUCCESSFUL                                ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");
        println!("  ID:                 {}", promoted.id);
        println!("  Tier:               Core (Permanent / Frozen)");
        println!("  Human Approval:     true (Explicitly Approved)");
        println!("  Approver:           {}", approval.approved_by);
        println!("  Channel:            {}", approval.channel);
        println!("  BLAKE3 Hash:        {}", approval.content_hash);
        println!("  Retention (R):      1.0 (Exempt from ACT-R decay / never pruned)");
        if let Some(r) = &approval.rationale {
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

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::state::{MemoryRecord, MemoryTier, MemoryType, Scope};
    use strata_memory::SqliteMemoryEngine;

    #[tokio::test]
    pub async fn test_cli_promote_generate_token_and_consume() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        let mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Architectural Invariant: Authentication state is immutable",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);

        engine.store().insert_or_update_memory(&mem).unwrap();

        // 1. Generate token via promote flow
        let gen_args = PromoteArgs {
            id: mem.id.to_string(),
            entity_type: "memory".to_string(),
            reason: Some("Arch review approval".to_string()),
            yes: true,
            to_global: false,
            generate_token: true,
            token: None,
            approver: Some("alice".to_string()),
            json: true,
        };

        run_promote(gen_args, Arc::clone(&engine)).await.unwrap();

        // Retrieve token directly using engine's store to get the exact token string
        let hash = blake3::hash(mem.content.as_bytes()).to_hex().to_string();
        let token = engine
            .generate_approval_token(&mem.id, "memory", &hash, "alice", 3600, Some("Arch review approval"))
            .unwrap();

        // 2. Consume token to promote
        let promote_args = PromoteArgs {
            id: mem.id.to_string(),
            entity_type: "memory".to_string(),
            reason: Some("Arch review approval".to_string()),
            yes: true,
            to_global: false,
            generate_token: false,
            token: Some(token.clone()),
            approver: None,
            json: true,
        };

        run_promote(promote_args, Arc::clone(&engine)).await.unwrap();

        let promoted = engine.store().get_memory(&mem.id).unwrap().unwrap();
        assert_eq!(promoted.tier, MemoryTier::Core);
        assert!(promoted.approved_by_human);
        let appr = promoted.human_approval.expect("human_approval record");
        assert_eq!(appr.approved_by, "alice");
        assert_eq!(appr.channel, "approval_token");
        assert_eq!(appr.content_hash, hash);

        // Reusing same token must fail directly and via promote
        assert!(
            engine.verify_and_consume_approval_token(&token, &mem.id, &hash).is_err(),
            "Direct reuse of consumed token must fail"
        );

        let mem2 = MemoryRecord::new(
            MemoryType::Semantic,
            "Second memory with same content",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);
        engine.store().insert_or_update_memory(&mem2).unwrap();

        let reuse_args = PromoteArgs {
            id: mem2.id.to_string(),
            entity_type: "memory".to_string(),
            reason: None,
            yes: true,
            to_global: false,
            generate_token: false,
            token: Some(token),
            approver: None,
            json: true,
        };
        assert!(
            run_promote(reuse_args, Arc::clone(&engine)).await.is_err(),
            "CLI promotion with consumed token must fail"
        );
    }

    #[tokio::test]
    pub async fn test_cli_promote_interactive_and_flag_records() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        let mem = MemoryRecord::new(
            MemoryType::Procedural,
            "Always run database migrations in transactions",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);

        engine.store().insert_or_update_memory(&mem).unwrap();

        let args = PromoteArgs {
            id: mem.id.to_string(),
            entity_type: "memory".to_string(),
            reason: Some("Reliability baseline".to_string()),
            yes: true,
            to_global: false,
            generate_token: false,
            token: None,
            approver: Some("bob".to_string()),
            json: true,
        };

        run_promote(args, Arc::clone(&engine)).await.unwrap();

        let promoted = engine.store().get_memory(&mem.id).unwrap().unwrap();
        assert_eq!(promoted.tier, MemoryTier::Core);
        assert!(promoted.approved_by_human);
        let appr = promoted.human_approval.expect("human_approval record");
        assert_eq!(appr.approved_by, "bob");
        assert_eq!(appr.channel, "cli_flag");
    }

    #[tokio::test]
    pub async fn test_cli_promote_semantic_fact_with_token() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());

        let fact = SemanticFact::new(
            "Service port must be greater than 1024",
            "security",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);

        engine.store().insert_or_update_semantic_fact(&fact).unwrap();

        let hash = blake3::hash(fact.statement.as_bytes()).to_hex().to_string();
        let token = engine
            .generate_approval_token(&fact.id, "fact", &hash, "security-lead", 1800, Some("RFC-011"))
            .unwrap();

        let args = PromoteArgs {
            id: fact.id.to_string(),
            entity_type: "fact".to_string(),
            reason: Some("RFC-011".to_string()),
            yes: true,
            to_global: false,
            generate_token: false,
            token: Some(token),
            approver: None,
            json: true,
        };

        run_promote(args, Arc::clone(&engine)).await.unwrap();

        let promoted = engine.store().get_semantic_fact(&fact.id).unwrap().unwrap();
        assert_eq!(promoted.tier, MemoryTier::Core);
        assert!(promoted.approved_by_human);
        let appr = promoted.human_approval.expect("human_approval record");
        assert_eq!(appr.approved_by, "security-lead");
        assert_eq!(appr.channel, "approval_token");
        assert_eq!(appr.content_hash, hash);
    }
}
