use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::json;
use std::sync::Arc;

use strata_core::schemas::FactStatus;
use strata_memory::SqliteStore;

#[derive(Debug, Clone, Args)]
pub struct CausalArgs {
    #[command(subcommand)]
    pub command: CausalCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum CausalCommand {
    /// Explain causal preconditions, failure dependencies, and counterfactual interventions for an action
    Explain {
        /// Action or tool name (e.g. 'cargo_test', 'edit_file')
        #[arg(help = "Action or tool name to diagnose causally")]
        action: String,

        /// Optional observed error message
        #[arg(long, help = "Observed error text or failure symptom")]
        error: Option<String>,

        /// Output report as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },
}

pub async fn run_causal(args: CausalArgs, store: Arc<SqliteStore>) -> Result<()> {
    match args.command {
        CausalCommand::Explain {
            action,
            error,
            json,
        } => {
            let error_observed = error.unwrap_or_default();

            // 1. Look up failure patterns related to action or error symptom
            let patterns = store
                .search_failures(Some(&action), None, 50)
                .unwrap_or_default();
            let relevant_patterns: Vec<_> = patterns
                .into_iter()
                .filter(|p| {
                    p.pattern_name.to_lowercase().contains(&action.to_lowercase())
                        || p.error_type.to_lowercase().contains(&action.to_lowercase())
                        || (!error_observed.is_empty()
                            && p.description
                                .to_lowercase()
                                .contains(&error_observed.to_lowercase()))
                })
                .collect();

            // 2. Look up semantic facts related to action pre/post-conditions
            let facts = store
                .get_all_semantic_facts(None, None, 100)
                .unwrap_or_default();
            let relevant_facts: Vec<_> = facts
                .into_iter()
                .filter(|f| {
                    f.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&action.to_lowercase()))
                        || f.statement
                            .to_lowercase()
                            .contains(&action.to_lowercase())
                })
                .collect();

            let mut active_preconditions = Vec::new();
            let mut invalidated_assumptions = Vec::new();
            let mut counterfactual_recommendations = Vec::new();

            for p in &relevant_patterns {
                counterfactual_recommendations.push(format!(
                    "[{}] {} -> Mitigation: {}",
                    p.error_type, p.pattern_name, p.mitigation
                ));
            }

            for f in &relevant_facts {
                if f.status == FactStatus::Active {
                    active_preconditions.push(f.statement.clone());
                } else {
                    invalidated_assumptions.push(f.statement.clone());
                }
            }

            let report = json!({
                "action": action,
                "error_observed": if error_observed.is_empty() { None } else { Some(&error_observed) },
                "active_preconditions": active_preconditions,
                "invalidated_assumptions": invalidated_assumptions,
                "counterfactual_recommendations": counterfactual_recommendations,
            });

            if json {
                let out = serde_json::to_string_pretty(&report)?;
                println!("{out}");
                return Ok(());
            }

            println!("\n========================================================");
            println!("🧠 CAUSAL BELIEF & COUNTERFACTUAL EXPLANATION: {}", action);
            println!("========================================================");

            if !error_observed.is_empty() {
                println!("  Observed Symptom: \"{}\"", error_observed);
            }

            println!("\n  Active Preconditions & Invariants (JTMS Validated):");
            if active_preconditions.is_empty() {
                println!("    (None explicitly registered for this action)");
            } else {
                for prec in &active_preconditions {
                    println!("    ✅ {}", prec);
                }
            }

            println!("\n  Invalidated Beliefs / Stale Assumptions:");
            if invalidated_assumptions.is_empty() {
                println!("    (None invalidated)");
            } else {
                for inv in &invalidated_assumptions {
                    println!("    ⚠️  {}", inv);
                }
            }

            println!("\n  Counterfactual Interventions & Remedies:");
            if counterfactual_recommendations.is_empty() {
                println!("    (No historical anti-pattern remedies recorded)");
            } else {
                for rec in &counterfactual_recommendations {
                    println!("    💡 {}", rec);
                }
            }
            println!();

            Ok(())
        }
    }
}
