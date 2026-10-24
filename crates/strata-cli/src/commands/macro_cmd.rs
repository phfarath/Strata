use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use strata_memory::SqliteStore;
use strata_reasoning::planning::macros::{
    CallbackToolInvoker, CompiledMacro, MacroExecutionEngine,
};

#[derive(Debug, Clone, Args)]
pub struct MacroArgs {
    #[command(subcommand)]
    pub command: MacroCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum MacroCommand {
    /// List compiled procedural macros synthesized from recurrent execution patterns
    List {
        /// Optional tag filter (e.g. 'compiled_macro', 'refactor', 'test')
        #[arg(long, help = "Filter by specific tag")]
        tag: Option<String>,

        /// Maximum number of macros to list
        #[arg(long, default_value_t = 20, help = "Maximum items to display")]
        limit: usize,

        /// Output report as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Run a compiled procedural macro in 1 single native step with parameter interpolation
    Run {
        /// Macro ID (UUID) or unique name
        #[arg(help = "Macro ID (UUID) or macro name")]
        macro_id_or_name: String,

        /// Dynamic execution parameters as a JSON object: '{"path": "src/main.rs", ...}'
        #[arg(
            long,
            default_value = "{}",
            help = "JSON dictionary of execution parameters"
        )]
        params: String,

        /// Output execution report as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },
}

pub async fn run_macro(args: MacroArgs, store: Arc<SqliteStore>) -> Result<()> {
    match args.command {
        MacroCommand::List { tag, limit, json } => {
            let all_skills = store
                .get_all_procedural_skills(None, limit)
                .context("Failed to load procedural skills from database")?;

            let macros: Vec<_> = all_skills
                .into_iter()
                .filter(|s| {
                    let has_macro_tag =
                        s.tags.iter().any(|t| t == "compiled_macro" || t == "macro");
                    if let Some(ref t) = tag {
                        has_macro_tag && s.tags.iter().any(|st| st == t)
                    } else {
                        has_macro_tag
                    }
                })
                .take(limit)
                .map(|s| CompiledMacro::from_procedural_skill(&s))
                .collect();

            if json {
                let out = serde_json::to_string_pretty(&macros)?;
                println!("{out}");
                return Ok(());
            }

            println!("\n========================================================");
            println!("⚙️  STRATA COMPILED PROCEDURAL MACROS ({})", macros.len());
            println!("========================================================");

            if macros.is_empty() {
                println!("  No compiled macros found. Use 'strata distill' or autonomous mining to synthesize macros from episodes.");
                return Ok(());
            }

            for m in &macros {
                println!(
                    "\n• [{}] {} ({} steps, ~{} tokens saved)",
                    m.id,
                    m.name,
                    m.steps.len(),
                    m.estimated_tokens_saved
                );
                println!("  Description: {}", m.description);
                println!("  Sequence:    {}", m.tool_sequence.join(" -> "));
                let param_names: Vec<_> = m.parameters.iter().map(|p| p.name.as_str()).collect();
                println!("  Parameters:  [{}]", param_names.join(", "));
                println!("  Success:     {:.1}%", m.success_rate * 100.0);
            }
            println!("\nRun a macro with: strata macro run <id|name> --params '{{\"param\": \"value\"}}'\n");
            Ok(())
        }

        MacroCommand::Run {
            macro_id_or_name,
            params,
            json,
        } => {
            let skill = if let Ok(u) = Uuid::parse_str(&macro_id_or_name) {
                store.get_procedural_skill(&u)?.or_else(|| {
                    store
                        .get_procedural_skill_by_name(&macro_id_or_name)
                        .ok()
                        .flatten()
                })
            } else {
                store.get_procedural_skill_by_name(&macro_id_or_name)?
            };

            let skill = match skill {
                Some(s) => s,
                None => bail!("Macro '{}' not found in store", macro_id_or_name),
            };

            let compiled = CompiledMacro::from_procedural_skill(&skill);

            let parsed_params: serde_json::Value =
                serde_json::from_str(&params).context("Failed to parse --params as JSON object")?;

            let raw_obj = match parsed_params.as_object() {
                Some(obj) => obj.clone(),
                None => bail!("--params must be a valid JSON object: '{{\"key\": \"value\"}}'"),
            };

            let mut execution_params = HashMap::new();
            for (k, v) in raw_obj {
                execution_params.insert(k, v);
            }

            // Workspace-aware native tool invoker
            let invoker = CallbackToolInvoker::new(
                |tool_name: &str, input: &serde_json::Value| -> Result<serde_json::Value> {
                    match tool_name {
                        "read_file" => {
                            let path_str = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            let content = std::fs::read_to_string(path_str)
                                .with_context(|| format!("Failed to read file: {path_str}"))?;
                            Ok(json!({ "status": "ok", "path": path_str, "content": content }))
                        }
                        "write_file" | "edit_file" => {
                            let path_str = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            let content =
                                input.get("content").and_then(|v| v.as_str()).unwrap_or("");
                            std::fs::write(path_str, content)
                                .with_context(|| format!("Failed to write file: {path_str}"))?;
                            Ok(
                                json!({ "status": "ok", "path": path_str, "bytes_written": content.len() }),
                            )
                        }
                        "cargo_check" => {
                            let flags = input.get("flags").and_then(|v| v.as_str()).unwrap_or("");
                            let pkg = input.get("package").and_then(|v| v.as_str());
                            let mut cmd = std::process::Command::new("cargo");
                            cmd.arg("check");
                            if let Some(p) = pkg {
                                cmd.arg("-p").arg(p);
                            }
                            for f in flags.split_whitespace() {
                                cmd.arg(f);
                            }
                            let output = cmd.output().context("Failed to execute cargo check")?;
                            if output.status.success() {
                                Ok(json!({ "status": "ok", "exit_code": 0 }))
                            } else {
                                let err = String::from_utf8_lossy(&output.stderr);
                                bail!("cargo check failed: {err}");
                            }
                        }
                        other => bail!("Unsupported tool for local macro execution: {other}"),
                    }
                },
            );

            let engine = MacroExecutionEngine::new();
            let report = engine
                .execute_macro_1step(&compiled, &execution_params, &invoker)
                .await?;

            if json {
                let out = serde_json::to_string_pretty(&report)?;
                println!("{out}");
                return Ok(());
            }

            println!("\n========================================================");
            if report.success {
                println!("🚀 MACRO EXECUTION COMPLETED: {}", report.macro_name);
            } else {
                println!("❌ MACRO EXECUTION HALTED: {}", report.macro_name);
            }
            println!("========================================================");
            println!("  Success:         {}", report.success);
            println!(
                "  Steps Executed:  {}/{}",
                report.steps_completed, report.total_steps
            );
            println!("  Tokens Saved:    ~{}", report.tokens_saved);
            println!("  Wall-clock:      {}ms", report.total_duration_ms);

            if let Some(ref err) = report.error {
                println!("\n  Error: {}", err);
            }

            println!("\n  Step Telemetry:");
            for s in &report.step_results {
                let status_icon = if s.success { "✅" } else { "❌" };
                println!(
                    "    {} Step {}: {} ({}ms)",
                    status_icon, s.step_index, s.tool_name, s.duration_ms
                );
            }
            println!();

            Ok(())
        }
    }
}
