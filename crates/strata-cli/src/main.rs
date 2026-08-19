use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;

use strata_core::{
    state::{MemoryRecord, MemoryType, Scope},
    traits::MemoryEngine,
};
use strata_memory::SqliteMemoryEngine;

use strata_cli::{
    commands::{
        auth::{run_auth, AuthArgs},
        blast_radius::{run_blast_radius, BlastRadiusArgs},
        consolidate::{run_consolidate, ConsolidateOptions},
        daemon::{run_daemon, DaemonArgs},
        doctor::run_doctor,
        export::{run_export, ExportArgs},
        feedback::{run_feedback, FeedbackArgs},
        hook::{handle_hook, HookCommand},
        init::{run_init, InitOptions},
        key::{run_key, KeyArgs},
        login::{run_login, run_logout, LoginArgs},
        observe::{run_observe, ObserveArgs},
        prune::{run_prune, PruneOptions},
        search::{run_search, SearchOptions},
        sync::{run_sync, SyncArgs},
        sync_hosts::{run_sync_hosts, SyncHostsArgs},
    },
    mcp::server::McpServer,
};


#[derive(Parser, Debug)]
#[command(name = "strata", author, version, about = "Strata — Portable Persistent Memory Layer & Cognitive Runtime", long_about = None)]
struct Cli {
    #[arg(long, global = true, help = "Path to SQLite database file")]
    db_path: Option<PathBuf>,

    #[arg(short, long, global = true, help = "Enable verbose debug logging")]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize Strata integrations for installed hosts (Cursor, Claude Code, Codex, Gemini)
    Init {
        #[arg(long, help = "Specific host to configure: 'cursor', 'claude', 'codex', 'gemini', or 'all'")]
        host: Option<String>,

        #[arg(long, default_value = ".", help = "Target workspace directory")]
        workspace: PathBuf,

        #[arg(long, help = "Overwrite existing configurations")]
        force: bool,
    },

    /// Run the Stdio JSON-RPC MCP Server serving memory tools
    Mcp,

    /// Execute lifecycle hooks for agent integrations
    Hook {
        #[command(subcommand)]
        hook: HookCommand,
    },

    /// Search persistent memory records using hybrid semantic/lexical search
    Search {
        query: String,

        #[arg(long, default_value_t = 5, help = "Maximum results to return")]
        limit: usize,

        #[arg(long, help = "Optional scope filter: 'global', 'project:<name>', 'session:<id>'")]
        scope: Option<String>,

        #[arg(long, help = "Filter by memory type: 'episodic', 'semantic', 'procedural', 'negative_pattern'")]
        memory_type: Option<String>,

        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Run diagnostic health check on SQLite database and host integrations
    Doctor,

    /// Write a memory record directly from the command line
    Write {
        content: String,

        #[arg(long, help = "Short headline or mnemonic summary (< 60 chars)")]
        summary: Option<String>,

        #[arg(long, default_value = "semantic", help = "Memory type: episodic, semantic, procedural, negative_pattern")]
        memory_type: String,

        #[arg(long, default_value = "global", help = "Memory scope")]
        scope: String,

        #[arg(long, help = "Comma-separated category tags")]
        tags: Option<String>,

        #[arg(long, default_value_t = 0.5, help = "Importance score (0.0 to 1.0)")]
        importance: f32,

        #[arg(long, default_value_t = 1.0, help = "Confidence score (0.0 to 1.0)")]
        confidence: f32,
    },

    /// Retrieve full memory record by UUID
    Get {
        id: String,

        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Generate a compact project/session digest (~300-500 tokens)
    Digest {
        #[arg(long, default_value = "default", help = "Session ID or project scope")]
        session_id: String,

        #[arg(long, default_value_t = 500, help = "Maximum token budget estimate")]
        tokens: usize,

        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Consolidate episodic event stream into semantic facts, procedural skills, and negative patterns
    Consolidate {
        #[arg(long, help = "Specific session ID to consolidate")]
        session: Option<String>,

        #[arg(long, help = "Consolidate all unconsolidated sessions")]
        all: bool,

        #[arg(long, help = "LLM model slug for reasoning distillation (default: OpenRouter free tier)")]
        model: Option<String>,

        #[arg(long, help = "Output report as raw JSON")]
        json: bool,
    },

    /// Run mathematical ACT-R decay engine to prune expired low-importance memories
    Prune {
        #[arg(long, default_value_t = 0.2, help = "Activation threshold below which memories are pruned")]
        threshold: f32,

        #[arg(long, help = "Optional scope filter")]
        scope: Option<String>,

        #[arg(long, help = "Output report as raw JSON")]
        json: bool,
    },

    /// Synchronize persistent memory spaces (push deltas, pull remote updates, view status)
    Sync(SyncArgs),

    /// Run background synchronization daemon loop (< 10MB RAM)
    Daemon(DaemonArgs),

    /// Export mined cognitive preference pairs (DPO), alignment signals (KTO), procedural skills (SFT), and markdown
    Export(ExportArgs),

    /// Compile and synchronize persistent memory & alignment rules across host instruction files within token budget
    #[command(name = "sync-hosts", alias = "sync_hosts")]
    SyncHosts(SyncHostsArgs),

    /// Provide explicit reinforcement feedback on a persistent memory record
    Feedback(FeedbackArgs),

    /// View cognitive observability dashboard, Ebbinghaus decay curves, ACT-R activations, and anti-patterns
    #[command(name = "observe", alias = "stats", alias = "decay", alias = "dashboard", alias = "tui")]
    Observe(ObserveArgs),

    /// Analyze architectural causal blast radius, ripple effects, and breaking risk before editing code
    #[command(name = "blast-radius", alias = "causal", alias = "impact", alias = "world-model")]
    BlastRadius(BlastRadiusArgs),

    /// Manage Strata Cloud developer accounts and authentication
    Auth(AuthArgs),

    /// Manage Strata Cloud machine API keys
    Key(KeyArgs),

    /// Log in to Strata Cloud via interactive browser window
    Login(LoginArgs),

    /// Log out from Strata Cloud and clear stored credentials
    Logout,
}


#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // In MCP mode, send all tracing logs exclusively to stderr to keep stdout pure JSON-RPC
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    // Fast-path for init (doesn't need SQLite DB opened)
    if let Commands::Init { host, workspace, force } = &cli.command {
        return run_init(InitOptions {
            workspace_dir: workspace.clone(),
            target_host: host.clone(),
            force: *force,
        });
    }

    // Fast-path for Auth, Key & Login commands (communicate over HTTP with Strata Cloud)
    if let Commands::Login(args) = cli.command {
        return run_login(args).await;
    }
    if let Commands::Logout = cli.command {
        return run_logout().await;
    }
    if let Commands::Auth(args) = cli.command {
        return run_auth(args).await;
    }
    if let Commands::Key(args) = cli.command {
        return run_key(args).await;
    }
    let resolved_ws = strata_cli::config::StrataConfig::resolve_workspace(None);
    if std::env::var("STRATA_WORKSPACE_ID").is_err() && resolved_ws != "default" {
        std::env::set_var("STRATA_WORKSPACE_ID", &resolved_ws);
    }

    let db_path = resolve_db_path(cli.db_path);

    let engine = Arc::new(
        SqliteMemoryEngine::open(&db_path, None)
            .with_context(|| format!("Failed to open Strata SQLite database at: {}", db_path.display()))?,
    );

    match cli.command {
        Commands::Init { .. } | Commands::Auth(_) | Commands::Key(_) | Commands::Login(_) | Commands::Logout => unreachable!(),

        Commands::Mcp => {
            let server = McpServer::new(engine);
            server.run_stdio().await?;
        }

        Commands::Hook { hook } => {
            handle_hook(hook, engine).await?;
        }

        Commands::Search { query, limit, scope, memory_type, json } => {
            run_search(SearchOptions { query, limit, scope, memory_type, json }, engine).await?;
        }

        Commands::Doctor => {
            run_doctor(&db_path, engine).await?;
        }

        Commands::Write { content, summary, memory_type, scope, tags, importance, confidence } => {
            let m_type = memory_type.parse::<MemoryType>().unwrap_or(MemoryType::Semantic);
            let m_scope = scope.parse::<Scope>().unwrap_or(Scope::Global);
            let tag_list = tags
                .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();

            let mut record = MemoryRecord::new(m_type, content, m_scope)
                .with_importance(importance)
                .with_confidence(confidence)
                .with_tags(tag_list);

            if let Some(s) = summary {
                record = record.with_summary(s);
            }

            let handle = engine.write(&record).await?;
            println!("✓ Memory recorded successfully!");
            println!("  ID: {}", handle.id);
            println!("  Title: {}", handle.title);
            println!("  Type: {}", handle.memory_type);
            println!("  Scope: {}", handle.scope);
        }

        Commands::Get { id, json } => {
            let uuid = Uuid::parse_str(&id).with_context(|| format!("Invalid UUID format: {id}"))?;
            match engine.get(&uuid).await? {
                Some(record) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&record)?);
                    } else {
                        println!("\n📖 Memory Record [{}]", record.id);
                        println!("═════════════════════════════════════════");
                        println!("Type:       {}", record.memory_type);
                        println!("Scope:      {}", record.scope);
                        if let Some(s) = &record.summary {
                            println!("Summary:    {s}");
                        }
                        println!("Importance: {:.2}", record.importance);
                        println!("Confidence: {:.2}", record.confidence);
                        println!("Created:    {}", record.created_at);
                        if !record.tags.is_empty() {
                            println!("Tags:       {}", record.tags.join(", "));
                        }
                        println!("\nContent:\n{}", record.content);
                    }
                }
                None => {
                    eprintln!("Memory record with ID '{id}' not found.");
                    std::process::exit(1);
                }
            }
        }

        Commands::Digest { session_id, tokens, json } => {
            let digest = engine.digest(&session_id, Some(tokens)).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&digest)?);
            } else {
                println!("\n🧠 [Strata Memory Digest - Session '{}']", digest.session_id);
                println!("═════════════════════════════════════════");
                if !digest.summary.is_empty() {
                    println!("Summary: {}", digest.summary);
                }
                if !digest.recent_decisions.is_empty() {
                    println!("\nRecent Decisions:");
                    for d in &digest.recent_decisions {
                        println!("  • {d}");
                    }
                }
                if !digest.failure_warnings.is_empty() {
                    println!("\n⚠️ Known Failure Warnings:");
                    for f in &digest.failure_warnings {
                        println!("  • [{}] {}: {}", f.error_type, f.pattern_name, f.mitigation);
                    }
                }
                if !digest.key_pointers.is_empty() {
                    println!("\nPointers:");
                    for p in &digest.key_pointers {
                        println!("  • ({}) {} [id: {}]", p.memory_type, p.title, p.id);
                    }
                }
                println!("\nEstimated tokens: ~{}", digest.estimated_tokens);
            }
        }

        Commands::Consolidate { session, all, model, json } => {
            let store = engine.store_arc();
            run_consolidate(ConsolidateOptions { session, all, model, json }, store).await?;
        }

        Commands::Prune { threshold, scope, json } => {
            let store = engine.store_arc();
            run_prune(PruneOptions { threshold, scope, json }, store).await?;
        }

        Commands::Sync(args) => {
            let store = engine.store_arc();
            run_sync(args, store).await?;
        }

        Commands::Daemon(args) => {
            let store = engine.store_arc();
            run_daemon(args, store).await?;
        }

        Commands::Export(args) => {
            let store = engine.store_arc();
            run_export(args, store).await?;
        }

        Commands::SyncHosts(args) => {
            let store = engine.store_arc();
            run_sync_hosts(args, store).await?;
        }

        Commands::Feedback(args) => {
            let store = engine.store_arc();
            run_feedback(args, store).await?;
        }

        Commands::Observe(args) => {
            let store = engine.store_arc();
            run_observe(args, store).await?;
        }

        Commands::BlastRadius(args) => {
            let store = engine.store_arc();
            run_blast_radius(args, store.as_ref()).await?;
        }
    }


    Ok(())
}

fn resolve_db_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(env_path) = std::env::var("STRATA_DB_PATH") {
        return PathBuf::from(env_path);
    }
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".strata");
        let _ = std::fs::create_dir_all(&dir);
        return dir.join("strata.db");
    }
    let local = PathBuf::from(".strata");
    let _ = std::fs::create_dir_all(&local);
    local.join("strata.db")
}
