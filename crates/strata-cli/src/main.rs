use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;

use strata_core::{
    state::{MemoryRecord, MemoryType, Scope},
    traits::MemoryEngine,
};
use strata_memory::SqliteMemoryEngine;

use strata_cli::{
    commands::{
        a2a::{run_a2a, A2aArgs},
        architecture::{run_architecture, ArchitectureArgs},
        auth::{run_auth, AuthArgs},
        blast_radius::{run_blast_radius, BlastRadiusArgs},
        callgraph::run_callgraph,
        causal_cmd::{run_causal, CausalArgs},
        config::{run_config, ConfigArgs},
        consolidate::{run_consolidate, ConsolidateOptions},
        daemon::{run_daemon, DaemonArgs},
        distill::{run_distill, DistillArgs},
        doctor::run_doctor,
        episode::{run_episode, EpisodeArgs},
        export::{run_export, ExportArgs},
        feedback::{run_feedback, FeedbackArgs},
        hook::{handle_hook, HookCommand},
        init::{run_init, InitOptions},
        key::{run_key, KeyArgs},
        login::{run_login, run_logout, LoginArgs},
        macro_cmd::{run_macro, MacroArgs},
        mcp_install::{run_mcp_install, run_mcp_uninstall, McpInstallOptions, McpUninstallOptions},
        memory::{run_memory, MemoryArgs},
        observe::{run_observe, ObserveArgs},
        plan::{run_plan, PlanArgs},
        promote::{run_promote, PromoteArgs},
        prune::{run_prune, PruneOptions},
        reconcile::{run_reconcile, ReconcileArgs},
        search::{run_search, SearchOptions},
        sync::{run_sync, SyncArgs},
        sync_hosts::{run_sync_hosts, SyncHostsArgs},
        task::{run_task, TaskArgs},
        train::{run_train, TrainArgs},
        transfer::{run_transfer, TransferArgs},
        watch::{run_watch, WatchArgs},
        worker::{run_worker, WorkerArgs},
        workspace::{run_workspace, WorkspaceArgs},
    },
    mcp::server::McpServer,
};

#[derive(Parser, Debug)]
#[command(name = "strata", author, version, about = "Strata — Portable Persistent Memory Layer & Cognitive Runtime", long_about = None)]
struct Cli {
    #[arg(long, global = true, help = "Path to SQLite database file")]
    db_path: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "Disable federated Developer-Global store (~/.strata/global.db)"
    )]
    no_global: bool,

    #[arg(short, long, global = true, help = "Enable verbose debug logging")]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize Strata integrations for installed hosts (Cursor, Claude Code, Codex, Gemini)
    Init {
        #[arg(
            long,
            help = "Specific host to configure: 'cursor', 'claude', 'codex', 'gemini', or 'all'"
        )]
        host: Option<String>,

        #[arg(long, default_value = ".", help = "Target workspace directory")]
        workspace: PathBuf,

        #[arg(long, help = "Overwrite existing configurations")]
        force: bool,
    },

    /// Run the Stdio JSON-RPC MCP Server, or manage MCP integration for host editors
    Mcp {
        #[command(subcommand)]
        action: Option<McpAction>,
    },

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

        #[arg(
            long,
            help = "Optional scope filter: 'global', 'project:<name>', 'session:<id>'"
        )]
        scope: Option<String>,

        #[arg(
            long,
            help = "Filter by memory type: 'episodic', 'semantic', 'procedural', 'negative_pattern'"
        )]
        memory_type: Option<String>,

        #[arg(
            long,
            help = "Optional ISO 8601 / RFC 3339 timestamp for historical bi-temporal time-travel queries (e.g. '2026-08-01T12:00:00Z')"
        )]
        as_of_time: Option<String>,

        #[arg(
            long,
            help = "Optional Git commit hash (SHA) to anchor historical playback queries"
        )]
        as_of_commit: Option<String>,

        #[arg(
            long = "branch",
            alias = "as-of-branch",
            help = "Optional Git branch namespace filter (e.g. 'main', 'feat/xyz'). Defaults to current branch if detected."
        )]
        as_of_branch: Option<String>,

        #[arg(long, help = "Output as raw JSON")]
        json: bool,

        #[arg(
            long,
            help = "Enable associative Knowledge Graph spreading activation search"
        )]
        graph: bool,
    },

    /// Run diagnostic health check on SQLite database and host integrations
    Doctor,

    /// Write a memory record directly from the command line
    Write {
        content: String,

        #[arg(long, help = "Short headline or mnemonic summary (< 60 chars)")]
        summary: Option<String>,

        #[arg(
            long,
            default_value = "semantic",
            help = "Memory type: episodic, semantic, procedural, negative_pattern"
        )]
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

        #[arg(
            long,
            help = "LLM model slug for reasoning distillation (default: OpenRouter free tier)"
        )]
        model: Option<String>,

        #[arg(
            long,
            help = "Reasoning provider: 'ollama', 'gemini', 'anthropic', 'openai', 'openrouter', 'mock'"
        )]
        provider: Option<String>,

        #[arg(long, help = "Output report as raw JSON")]
        json: bool,

        #[arg(
            long = "hippocampus",
            short = 'H',
            help = "Run hippocampal consolidation over recorded episodic traces"
        )]
        hippocampus: bool,
    },

    /// Run mathematical ACT-R decay engine to prune expired low-importance memories
    Prune {
        #[arg(
            long,
            default_value_t = 0.2,
            help = "Activation threshold below which memories are pruned"
        )]
        threshold: f32,

        #[arg(long, help = "Optional scope filter")]
        scope: Option<String>,

        #[arg(long, help = "Simulate decay without writing changes to the database")]
        dry_run: bool,

        #[arg(long, help = "Output report as raw JSON")]
        json: bool,
    },

    /// Synchronize persistent memory spaces (push deltas, pull remote updates, view status)
    Sync(SyncArgs),

    /// Run background auto-dreaming & synchronization daemon loop (< 5MB RAM, KAIROS-style)
    Daemon(DaemonArgs),

    /// Export mined cognitive preference pairs (DPO), alignment signals (KTO), procedural skills (SFT), and markdown
    Export(ExportArgs),

    /// Compile and synchronize persistent memory & alignment rules across host instruction files within token budget
    #[command(name = "sync-hosts", alias = "sync_hosts")]
    SyncHosts(SyncHostsArgs),

    /// Provide explicit reinforcement feedback on a persistent memory record
    Feedback(FeedbackArgs),

    /// View cognitive observability dashboard, Ebbinghaus decay curves, ACT-R activations, and anti-patterns
    #[command(name = "observe", alias = "stats", alias = "decay")]
    Observe(ObserveArgs),

    /// Interactive terminal dashboard (TUI) with real-time cognitive metrics, JTMS graph, and AST anchors
    #[command(name = "ui", alias = "tui", alias = "dashboard")]
    Ui,

    /// Analyze architectural causal blast radius, ripple effects, and breaking risk before editing code
    #[command(
        name = "blast-radius",
        alias = "causal",
        alias = "impact",
        alias = "world-model"
    )]
    BlastRadius(BlastRadiusArgs),

    /// Hierarchical goal planning, topological wave decomposition, and DAG execution scheduler
    #[command(name = "plan", alias = "dag", alias = "schedule")]
    Plan(PlanArgs),

    /// Manage Strata Cloud developer accounts and authentication
    Auth(AuthArgs),

    /// Manage Strata Cloud machine API keys
    Key(KeyArgs),

    /// Log in to Strata Cloud via interactive browser window
    Login(LoginArgs),

    /// Log out from Strata Cloud and clear stored credentials
    Logout,

    /// Self-supervised distillation & LoRA curriculum fine-tuning pipeline
    #[command(name = "distill", alias = "curriculum")]
    Distill(DistillArgs),

    /// One-click local LoRA fine-tuning via Unsloth and Ollama deployment
    #[command(name = "train", alias = "lora", alias = "finetune")]
    Train(TrainArgs),

    /// Manage and execute compiled procedural macros
    #[command(name = "macro", alias = "macros")]
    Macro(MacroArgs),

    /// Causal reasoning, explanation, and counterfactual diagnosis
    #[command(name = "causal", alias = "why")]
    Causal(CausalArgs),

    /// Deterministic native call graph and import dependency analyzer
    #[command(name = "callgraph", alias = "calls", alias = "hierarchy")]
    CallGraph {
        #[arg(help = "Path to source file or directory to analyze")]
        path: String,

        #[arg(long, help = "Filter callers or callees of a specific symbol/function")]
        symbol: Option<String>,

        #[arg(
            long,
            default_value = "all",
            help = "Hierarchy direction (callers, callees, both, imports, all)"
        )]
        direction: String,

        #[arg(long, default_value_t = 50, help = "Maximum call edges to display")]
        limit: usize,

        #[arg(long, help = "Output as raw JSON")]
        json: bool,

        #[arg(
            long,
            help = "Deep heuristic index of AST symbols and call edges into SQLite"
        )]
        index: bool,
    },

    /// Deep heuristic index of workspace AST symbols and call edges into SQLite
    #[command(name = "index", alias = "reindex")]
    Index {
        #[arg(
            default_value = ".",
            help = "Path to workspace directory or file to index"
        )]
        path: String,

        #[arg(long, help = "Output report as raw JSON")]
        json: bool,
    },

    /// Multi-package monorepo workspace detector and boundary isolator
    #[command(name = "workspace", alias = "monorepo", alias = "packages")]
    Workspace(WorkspaceArgs),

    /// Graph community extraction and high-level architectural clustering
    #[command(
        name = "architecture",
        alias = "cluster",
        alias = "communities",
        alias = "macro"
    )]
    Architecture(ArchitectureArgs),

    /// Human-in-the-loop approval and promotion of memories/facts to permanent Core Tier (frozen retention, R=1.0)
    #[command(
        name = "promote",
        alias = "memory-promote",
        alias = "approve",
        alias = "freeze"
    )]
    Promote(PromoteArgs),

    /// Reconcile code-anchored semantic facts bi-temporally against current code ASTs and Git commits
    #[command(
        name = "reconcile",
        alias = "reconcile-anchors",
        alias = "sync-anchors"
    )]
    Reconcile(ReconcileArgs),

    /// Watch workspace files in real-time for AST code drift and invalidate JTMS beliefs
    #[command(name = "watch", alias = "drift-watch")]
    Watch(WatchArgs),

    /// Agent-to-Agent (A2A) stigmergic coordination and temporal resource leases
    #[command(name = "a2a", alias = "stigmergy", alias = "leases")]
    A2a(A2aArgs),

    /// Multi-agent shared blackboard task management and work-stealing
    #[command(name = "task", alias = "tasks", alias = "board", alias = "subtasks")]
    Task(TaskArgs),

    /// Episodic memory trace recording, replay, and hippocampal consolidation
    #[command(
        name = "episode",
        alias = "episodes",
        alias = "trace",
        alias = "replay"
    )]
    Episode(EpisodeArgs),

    /// Run autonomous blackboard task worker daemon (work-stealing, execution, heartbeats)
    #[command(name = "worker", alias = "runner")]
    Worker(WorkerArgs),

    /// Cross-project knowledge transfer: import memories, failure patterns, and procedural skills from another repository
    #[command(name = "transfer", alias = "import-from", alias = "cross-project")]
    Transfer(TransferArgs),

    /// View or modify Strata runtime configuration and local/cloud reasoning providers
    #[command(name = "config", alias = "cfg", alias = "settings")]
    Config(ConfigArgs),

    /// Manage persistent memory embeddings, reindexing, and vector consistency
    #[command(name = "memory", alias = "mem")]
    Memory(MemoryArgs),
}

#[derive(Subcommand, Debug)]
pub enum McpAction {
    /// Install Strata MCP server configuration into host editors (Cursor, Claude Desktop, Windsurf)
    #[command(name = "install", alias = "setup")]
    Install {
        #[arg(long, help = "Target editor: 'cursor', 'claude', 'windsurf', or 'all'")]
        client: Option<String>,

        #[arg(long, help = "Install into global user configuration directory")]
        global: bool,

        #[arg(long, default_value = ".", help = "Target workspace directory")]
        workspace: PathBuf,
    },

    /// Uninstall Strata MCP server configuration from host editors
    #[command(name = "uninstall", alias = "remove")]
    Uninstall {
        #[arg(long, help = "Target editor: 'cursor', 'claude', 'windsurf', or 'all'")]
        client: Option<String>,

        #[arg(long, help = "Uninstall from global user configuration directory")]
        global: bool,

        #[arg(long, default_value = ".", help = "Target workspace directory")]
        workspace: PathBuf,
    },
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
    if let Commands::Init {
        host,
        workspace,
        force,
    } = &cli.command
    {
        return run_init(InitOptions {
            workspace_dir: workspace.clone(),
            target_host: host.clone(),
            force: *force,
        });
    }

    // Fast-path for MCP installation / uninstallation subcommands (does not require SQLite DB)
    if let Commands::Mcp {
        action: Some(ref action),
    } = cli.command
    {
        match action {
            McpAction::Install {
                client,
                global,
                workspace,
            } => {
                return run_mcp_install(McpInstallOptions {
                    client: client.clone(),
                    global: *global,
                    workspace_dir: workspace.clone(),
                });
            }
            McpAction::Uninstall {
                client,
                global,
                workspace,
            } => {
                return run_mcp_uninstall(McpUninstallOptions {
                    client: client.clone(),
                    global: *global,
                    workspace_dir: workspace.clone(),
                });
            }
        }
    }

    // Fast-path for Plan (without --persist, can execute standalone or with scheduler)
    if let Commands::Plan(ref args) = cli.command {
        if !args.persist {
            return run_plan(args.clone(), None).await;
        }
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

    // Fast-path for CallGraph without --index (analyzes code AST directly on-the-fly)
    if let Commands::CallGraph {
        path,
        symbol,
        direction,
        limit,
        json,
        index,
    } = &cli.command
    {
        if !*index {
            return run_callgraph(
                path,
                symbol.as_deref(),
                direction,
                *json,
                *limit,
                false,
                None,
            )
            .await
            .map_err(Into::into);
        }
    }

    // Fast-path for Workspace (detects package boundaries directly on disk)
    if let Commands::Workspace(args) = cli.command {
        return run_workspace(args).await.map_err(Into::into);
    }

    // Fast-path for Architecture (clusters graph communities directly on-the-fly)
    if let Commands::Architecture(args) = cli.command {
        return run_architecture(args).await.map_err(Into::into);
    }

    let resolved_ws = strata_cli::config::StrataConfig::resolve_workspace(None);
    if std::env::var("STRATA_WORKSPACE_ID").is_err() && resolved_ws != "default" {
        std::env::set_var("STRATA_WORKSPACE_ID", &resolved_ws);
    }

    let stores = resolve_stores(cli.db_path, cli.no_global);
    let db_path = stores.local_path.clone();

    let config = strata_cli::config::StrataConfig::load();
    let resolved_emb = strata_memory::EmbeddingProviderFactory::create_from_config(&config)
        .with_context(|| "Failed to initialize embedding provider from configuration")?;
    let embedding_provider = resolved_emb.provider;

    let engine = Arc::new(
        SqliteMemoryEngine::open_federated(
            &stores.local_path,
            stores.global_path.as_deref(),
            Some(embedding_provider),
        )
        .with_context(|| {
            format!(
                "Failed to open Strata SQLite database at: {}",
                stores.local_path.display()
            )
        })?,
    );

    match cli.command {
        Commands::Init { .. }
        | Commands::Auth(_)
        | Commands::Key(_)
        | Commands::Login(_)
        | Commands::Logout
        | Commands::Workspace(_)
        | Commands::Architecture(_) => unreachable!(),

        Commands::CallGraph {
            path,
            symbol,
            direction,
            limit,
            json,
            index,
        } => {
            run_callgraph(
                &path,
                symbol.as_deref(),
                &direction,
                json,
                limit,
                index,
                Some(engine.store_arc()),
            )
            .await?;
        }

        Commands::Index { path, json } => {
            run_callgraph(&path, None, "all", json, 50, true, Some(engine.store_arc())).await?;
        }

        Commands::Mcp { action: None } => {
            let mut server = McpServer::new_with_engine(Arc::clone(&engine));
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            server.init_workspace_ipc(&cwd).await;
            server.run_stdio().await?;
        }
        Commands::Mcp { action: Some(_) } => unreachable!(),

        Commands::Hook { hook } => {
            handle_hook(hook, engine).await?;
        }

        Commands::Search {
            query,
            limit,
            scope,
            memory_type,
            as_of_time,
            as_of_commit,
            as_of_branch,
            json,
            graph,
        } => {
            run_search(
                SearchOptions {
                    query,
                    limit,
                    scope,
                    memory_type,
                    as_of_time,
                    as_of_commit,
                    as_of_branch,
                    json,
                    graph,
                },
                engine,
            )
            .await?;
        }

        Commands::Doctor => {
            run_doctor(&db_path, engine).await?;
        }

        Commands::Write {
            content,
            summary,
            memory_type,
            scope,
            tags,
            importance,
            confidence,
        } => {
            let m_type = memory_type
                .parse::<MemoryType>()
                .unwrap_or(MemoryType::Semantic);
            let m_scope = scope.parse::<Scope>().unwrap_or(Scope::Global);
            let tag_list = tags
                .map(|t| {
                    t.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
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
            let uuid =
                Uuid::parse_str(&id).with_context(|| format!("Invalid UUID format: {id}"))?;
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

        Commands::Digest {
            session_id,
            tokens,
            json,
        } => {
            let digest = engine.digest(&session_id, Some(tokens)).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&digest)?);
            } else {
                println!(
                    "\n🧠 [Strata Memory Digest - Session '{}']",
                    digest.session_id
                );
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
                        println!(
                            "  • [{}] {}: {}",
                            f.error_type, f.pattern_name, f.mitigation
                        );
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

        Commands::Consolidate {
            session,
            all,
            model,
            provider,
            json,
            hippocampus,
        } => {
            let store = engine.store_arc();
            run_consolidate(
                ConsolidateOptions {
                    session,
                    all,
                    model,
                    provider,
                    json,
                    hippocampus,
                },
                store,
            )
            .await?;
        }

        Commands::Prune {
            threshold,
            scope,
            dry_run,
            json,
        } => {
            let store = engine.store_arc();
            run_prune(
                PruneOptions {
                    threshold,
                    scope,
                    dry_run,
                    json,
                },
                store,
            )
            .await?;
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

        Commands::Ui => {
            strata_cli::tui::run_tui(engine, &db_path).await?;
        }

        Commands::BlastRadius(args) => {
            let store = engine.store_arc();
            run_blast_radius(args, store.as_ref()).await?;
        }

        Commands::Distill(args) => {
            let store = engine.store_arc();
            run_distill(args, store).await?;
        }

        Commands::Train(args) => {
            let store = engine.store_arc();
            run_train(args, store).await?;
        }

        Commands::Macro(args) => {
            let store = engine.store_arc();
            run_macro(args, store).await?;
        }

        Commands::Causal(args) => {
            let store = engine.store_arc();
            run_causal(args, store).await?;
        }

        Commands::Promote(args) => {
            run_promote(args, engine).await?;
        }

        Commands::Transfer(args) => {
            run_transfer(args, engine).await?;
        }

        Commands::Reconcile(args) => {
            let store = engine.store_arc();
            run_reconcile(args, store).await?;
        }

        Commands::Watch(args) => {
            run_watch(args, engine.store_arc()).await?;
        }

        Commands::A2a(args) => {
            let coordinator = engine.stigmergy();
            run_a2a(args, coordinator).await?;
        }

        Commands::Task(args) => {
            let coordinator = engine.stigmergy();
            run_task(args, coordinator).await?;
        }

        Commands::Episode(args) => {
            let store = engine.store_arc();
            run_episode(args, store).await?;
        }

        Commands::Worker(args) => {
            let coordinator = engine.stigmergy();
            run_worker(args, coordinator).await?;
        }

        Commands::Plan(args) => {
            let coordinator = engine.stigmergy();
            run_plan(args, Some(coordinator)).await?;
        }

        Commands::Config(args) => {
            run_config(args).await?;
        }

        Commands::Memory(args) => {
            let store = engine.store_arc();
            run_memory(args, store).await?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct ResolvedStores {
    pub local_path: PathBuf,
    pub global_path: Option<PathBuf>,
}

pub fn resolve_stores(explicit_local: Option<PathBuf>, no_global: bool) -> ResolvedStores {
    // 1. Local workspace database resolution
    let local_path = if let Some(p) = explicit_local {
        p
    } else if let Ok(env_path) = std::env::var("STRATA_DB_PATH") {
        PathBuf::from(env_path)
    } else {
        detect_workspace_db_path()
    };

    // 2. Global developer database resolution (~/.strata/global.db)
    let global_path = if no_global || std::env::var("STRATA_NO_GLOBAL").is_ok() {
        None
    } else if let Some(home) = dirs::home_dir() {
        let global_dir = home.join(".strata");
        let _ = std::fs::create_dir_all(&global_dir);
        let global = global_dir.join("global.db");
        // Don't federate with self if local_path points to the exact same file
        if local_path == global {
            None
        } else {
            Some(global)
        }
    } else {
        None
    };

    ResolvedStores {
        local_path,
        global_path,
    }
}

fn detect_workspace_db_path() -> PathBuf {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut curr = current_dir.as_path();
    loop {
        if curr.join(".strata").exists() || curr.join(".git").exists() {
            let strata_dir = curr.join(".strata");
            let _ = std::fs::create_dir_all(&strata_dir);
            return strata_dir.join("strata.db");
        }
        match curr.parent() {
            Some(p) => curr = p,
            None => break,
        }
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

pub fn resolve_db_path(explicit: Option<PathBuf>) -> PathBuf {
    resolve_stores(explicit, false).local_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_cli::commands::promote::{run_promote, PromoteArgs};
    use strata_cli::mcp::server::McpServer;
    use strata_core::state::{MemoryRecord, MemoryTier, MemoryType, Scope};
    use strata_memory::SqliteMemoryEngine;

    #[tokio::test]
    async fn test_cli_promote_rejection_without_token_mcp() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let server = McpServer::new_with_engine(Arc::clone(&engine));

        let mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Auth tokens must expire within 15 minutes",
            Scope::Global,
        );
        let handle = engine.write(&mem).await.unwrap();

        // 1. Calling strata_promote with bare boolean approved_by_human MUST fail
        let call = server
            .execute_tool(
                "strata_promote",
                serde_json::json!({
                    "id": handle.id.to_string(),
                    "approved_by_human": true,
                }),
            )
            .await;
        assert!(
            call.is_error.unwrap_or(false),
            "MCP promote without approval_token must fail"
        );

        // 2. Calling with empty token MUST fail
        let empty_call = server
            .execute_tool(
                "strata_promote",
                serde_json::json!({
                    "id": handle.id.to_string(),
                    "approval_token": "",
                }),
            )
            .await;
        assert!(
            empty_call.is_error.unwrap_or(false),
            "MCP promote with empty token must fail"
        );
    }

    #[tokio::test]
    async fn test_cli_promote_with_valid_token_and_provenance() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let server = McpServer::new_with_engine(Arc::clone(&engine));

        let mem = MemoryRecord::new(
            MemoryType::Procedural,
            "All SQL queries in transactions must be serialized",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);
        let handle = engine.write(&mem).await.unwrap();

        let hash = blake3::hash(mem.content.as_bytes()).to_hex().to_string();
        let token = engine
            .generate_approval_token(
                &handle.id,
                "memory",
                &hash,
                "security_team",
                3600,
                Some("Approved via audit"),
            )
            .unwrap();

        // Promotion with valid token MUST succeed
        let call = server
            .execute_tool(
                "strata_promote",
                serde_json::json!({
                    "id": handle.id.to_string(),
                    "approval_token": token,
                    "reason": "Approved via audit",
                }),
            )
            .await;
        assert!(
            !call.is_error.unwrap_or(false),
            "MCP promote with valid token must succeed"
        );

        // Verify stored state
        let fetched = engine.get(&handle.id).await.unwrap().unwrap();
        assert_eq!(fetched.tier, MemoryTier::Core);
        assert!(fetched.approved_by_human);
        let appr = fetched.human_approval.expect("human_approval");
        assert_eq!(appr.approved_by, "security_team");
        assert_eq!(appr.channel, "approval_token");
        assert_eq!(appr.content_hash, hash);

        // Verify provenance via memory_get
        let get_res = server
            .execute_tool(
                "memory_get",
                serde_json::json!({ "id": handle.id.to_string() }),
            )
            .await;
        assert!(!get_res.is_error.unwrap_or(false));
        let get_text = &get_res.content[0].text;
        assert!(get_text.contains("provenance"));
        assert!(get_text.contains("selection_reason"));
        assert!(get_text.contains("human_approval"));

        // Verify provenance via memory_search
        let search_res = server
            .execute_tool(
                "memory_search",
                serde_json::json!({ "query": "SQL queries" }),
            )
            .await;
        assert!(!search_res.is_error.unwrap_or(false));
        let search_text = &search_res.content[0].text;
        assert!(search_text.contains("provenance"));
        assert!(search_text.contains("human_approval"));
    }

    #[tokio::test]
    async fn test_cli_promote_token_tampering_and_reuse_rejection() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let server = McpServer::new_with_engine(Arc::clone(&engine));

        let mem = MemoryRecord::new(MemoryType::Semantic, "Original safe content", Scope::Global);
        let handle = engine.write(&mem).await.unwrap();

        let hash = blake3::hash(mem.content.as_bytes()).to_hex().to_string();
        let token = engine
            .generate_approval_token(&handle.id, "memory", &hash, "admin", 3600, None)
            .unwrap();

        // 1. Tamper content in memory
        let mut tampered = mem.clone();
        tampered.content = "Malicious modified content".to_string();
        engine.store().insert_or_update_memory(&tampered).unwrap();

        // 2. Attempt promotion with token generated for original content -> MUST FAIL
        let fail_call = server
            .execute_tool(
                "strata_promote",
                serde_json::json!({
                    "id": handle.id.to_string(),
                    "approval_token": token,
                }),
            )
            .await;
        assert!(
            fail_call.is_error.unwrap_or(false),
            "Tampered content must fail token verification"
        );
    }

    #[tokio::test]
    async fn test_cli_promote_command_flags() {
        let engine = Arc::new(SqliteMemoryEngine::open_in_memory(None).unwrap());
        let mem = MemoryRecord::new(
            MemoryType::Semantic,
            "Security rule: invariant validation",
            Scope::Global,
        )
        .with_tier(MemoryTier::Working);
        let handle = engine.write(&mem).await.unwrap();

        let args = PromoteArgs {
            id: handle.id.to_string(),
            entity_type: "memory".to_string(),
            reason: Some("CLI flag promotion".to_string()),
            yes: true,
            to_global: false,
            generate_token: false,
            token: None,
            approver: Some("ci_user".to_string()),
            json: true,
        };

        run_promote(args, Arc::clone(&engine)).await.unwrap();

        let fetched = engine.get(&handle.id).await.unwrap().unwrap();
        assert_eq!(fetched.tier, MemoryTier::Core);
        assert!(fetched.approved_by_human);
        let appr = fetched.human_approval.expect("human_approval");
        assert_eq!(appr.approved_by, "ci_user");
        assert_eq!(appr.channel, "cli_flag");
    }
}
