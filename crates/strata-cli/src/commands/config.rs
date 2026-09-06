use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use strata_core::config::StrataConfig;
use strata_reasoning::probe_provider;

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Set a configuration value (e.g. provider, model, api_key, ollama_url, temperature)
    Set {
        /// Key name (provider, model, api_key, ollama_url, temperature)
        key: String,

        /// Key value
        value: String,

        #[arg(
            long,
            help = "Save to user-global configuration (~/.strata/config.toml) instead of workspace"
        )]
        global: bool,
    },

    /// Get a configuration value by key name
    Get {
        /// Key name (provider, model, api_key, ollama_url, temperature)
        key: String,

        #[arg(
            long,
            help = "Read from user-global configuration (~/.strata/config.toml)"
        )]
        global: bool,
    },

    /// List all active configuration values and their sources
    List {
        #[arg(long, help = "Display user-global configuration only")]
        global: bool,
    },

    /// Probe connectivity and verify API keys or local daemon for a provider
    Test {
        #[arg(
            long,
            help = "Provider to probe: 'ollama', 'gemini', 'anthropic', 'openai', 'openrouter', 'mock'"
        )]
        provider: Option<String>,

        #[arg(long, help = "Specific model slug to test")]
        model: Option<String>,
    },
}

pub async fn run_config(args: ConfigArgs) -> Result<()> {
    match args.action {
        ConfigAction::Set { key, value, global } => {
            let target_path = if global {
                StrataConfig::global_config_path()
                    .context("Failed to resolve user home directory")?
            } else {
                StrataConfig::workspace_config_path(None)
            };

            let mut config = if target_path.exists() {
                StrataConfig::load_from_file(&target_path)?
            } else {
                StrataConfig::default()
            };

            config
                .set_key(&key, &value)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            config.save_to_file(&target_path)?;

            let scope = if global { "global" } else { "workspace" };
            println!(
                "✓ Saved [{scope}] config: '{}' = '{}' ({})",
                key.to_lowercase(),
                if key.to_lowercase().contains("key") {
                    "***"
                } else {
                    &value
                },
                target_path.display()
            );

            if !global && key.to_lowercase().contains("key") {
                eprintln!(
                    "⚠️  Warning: Storing an API key in workspace-local config ({})",
                    target_path.display()
                );
                eprintln!("   Ensure .strata/ is included in .gitignore to prevent committing secrets to git.");
            }
        }

        ConfigAction::Get { key, global } => {
            let config = if global {
                let path = StrataConfig::global_config_path()
                    .context("Failed to resolve user home directory")?;
                if path.exists() {
                    StrataConfig::load_from_file(&path)?
                } else {
                    StrataConfig::default()
                }
            } else {
                StrataConfig::load()
            };

            match config.get_key(&key) {
                Some(val) => println!("{val}"),
                None => {
                    eprintln!("Configuration key '{}' is not set.", key);
                }
            }
        }

        ConfigAction::List { global } => {
            let (config, source) = if global {
                let path = StrataConfig::global_config_path()
                    .context("Failed to resolve user home directory")?;
                let loaded = if path.exists() {
                    StrataConfig::load_from_file(&path)?
                } else {
                    StrataConfig::default()
                };
                (loaded, format!("Global ({})", path.display()))
            } else {
                let loaded = StrataConfig::load();
                (loaded, "Layered (Workspace + Global)".to_string())
            };

            println!("\n⚙️ [Strata Runtime & Provider Configuration]");
            println!("══════════════════════════════════════════════════");
            println!("Source: {source}");
            println!("──────────────────────────────────────────────────");
            for (k, v) in config.list_keys() {
                println!("{:<14} : {}", k, v);
            }
            println!("══════════════════════════════════════════════════\n");
        }

        ConfigAction::Test { provider, model } => {
            let config = StrataConfig::load();
            let prov = provider
                .or_else(|| config.provider.clone())
                .unwrap_or_else(|| "ollama".to_string());

            println!("🔍 Probing provider '{}'...", prov);
            match probe_provider(&prov, model.as_deref(), &config).await {
                Ok(msg) => {
                    println!("✓ Success: {}", msg);
                }
                Err(err) => {
                    eprintln!("✗ Check Failed: {}", err);
                    if prov == "ollama" {
                        eprintln!("💡 Tip: To use Ollama offline for free, install from https://ollama.com and run 'ollama serve'.");
                    } else if prov == "gemini" {
                        eprintln!("💡 Tip: Set GEMINI_API_KEY environment variable or run 'strata config set api_key <key>'.");
                    } else if prov == "anthropic" {
                        eprintln!("💡 Tip: Set ANTHROPIC_API_KEY environment variable or run 'strata config set api_key <key>'.");
                    } else if prov == "openai" {
                        eprintln!("💡 Tip: Set OPENAI_API_KEY environment variable or run 'strata config set api_key <key>'.");
                    } else if prov == "openrouter" {
                        eprintln!("💡 Tip: Set OPENROUTER_API_KEY environment variable or run 'strata config set api_key <key>'.");
                    }
                }
            }
        }
    }

    Ok(())
}
