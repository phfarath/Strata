use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;
use uuid::Uuid;

use strata_core::schemas::{EpisodeRecord, EpisodeStep};
use strata_memory::SqliteStore;

#[derive(Debug, Clone, Args)]
pub struct EpisodeArgs {
    #[command(subcommand)]
    pub command: EpisodeCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum EpisodeCommand {
    /// Record a multi-step episodic execution trace
    Record {
        /// High-level summary of the episode
        #[arg(long, short = 'm', help = "Summary of the episodic execution")]
        summary: String,

        /// Session ID
        #[arg(long, default_value = "default", help = "Session ID")]
        session: String,

        /// Task ID on the multi-agent blackboard
        #[arg(long, help = "Task ID on the multi-agent blackboard")]
        task_id: Option<String>,

        /// Agent ID that generated the trace
        #[arg(long, default_value = "local-agent", help = "Agent ID")]
        agent: String,

        /// Salience score (0.0 to 1.0)
        #[arg(long, default_value_t = 0.5, help = "Salience score (0.0 to 1.0)")]
        salience: f32,

        /// Outcomes / assertions observed (comma-separated)
        #[arg(long, help = "Comma-separated outcomes observed")]
        outcomes: Option<String>,

        /// Steps trace in JSON format: [{"action": "...", "observation": "...", "reward": 1.0, "error": null}]
        #[arg(long, help = "JSON array of EpisodeStep objects")]
        steps_json: Option<String>,

        /// Optional causal provenance or environment context
        #[arg(long, help = "Causal provenance or context note")]
        provenance: Option<String>,

        /// Output as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// List recorded episodic traces
    List {
        /// Filter by session ID
        #[arg(long, help = "Filter by session ID")]
        session: Option<String>,

        /// Maximum episodes to retrieve
        #[arg(long, default_value_t = 50, help = "Maximum episodes to return")]
        limit: usize,

        /// Output as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Replay and evaluate an episodic trace against expectations
    Replay {
        /// UUID of the episode to replay
        #[arg(help = "UUID of the episode record")]
        id: String,

        /// Output as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Run hippocampal consolidation over pending episodic traces
    Consolidate {
        /// Filter by session ID
        #[arg(long, help = "Filter by session ID")]
        session: Option<String>,

        /// Output as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },
}

pub async fn run_episode(args: EpisodeArgs, store: Arc<SqliteStore>) -> Result<()> {
    match args.command {
        EpisodeCommand::Record {
            summary,
            session,
            task_id,
            agent,
            salience,
            outcomes,
            steps_json,
            provenance,
            json,
        } => {
            let steps = if let Some(raw) = steps_json {
                serde_json::from_str::<Vec<EpisodeStep>>(&raw)
                    .with_context(|| format!("Invalid JSON format for steps: '{raw}'"))?
            } else {
                vec![EpisodeStep::new(0, summary.clone())
                    .with_output(serde_json::json!({ "result": "Action executed successfully" }))]
            };

            let outcome_list: Vec<String> = outcomes
                .map(|o| {
                    o.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            let mut ep = EpisodeRecord::new(session, agent, summary)
                .with_salience(salience)
                .with_steps(steps)
                .with_outcomes(outcome_list);

            if let Some(tid) = task_id {
                ep = ep.with_task_id(tid);
            }
            if let Some(prov) = provenance {
                ep = ep.with_provenance(prov);
            }

            store
                .write_episode(&ep)
                .map_err(|e| anyhow::anyhow!("Failed to record episode: {e}"))?;

            if json {
                println!("{}", serde_json::to_string_pretty(&ep)?);
            } else {
                println!("\n🧠 [Strata Episodic Memory Trace Recorded]");
                println!("══════════════════════════════════════════════════");
                println!("Episode ID:    {}", ep.id);
                println!("Session:       {}", ep.session_id);
                println!("Agent:         {}", ep.agent_id);
                if let Some(ref tid) = ep.task_id {
                    println!("Task ID:       {}", tid);
                }
                println!("Salience:      {:.2}", ep.salience_score);
                println!("Steps Count:   {}", ep.event_trace.len());
                println!("Summary:       {}", ep.summary);
            }
        }

        EpisodeCommand::List {
            session,
            limit,
            json,
        } => {
            let episodes = store
                .list_episodes(session.as_deref(), None, limit)
                .map_err(|e| anyhow::anyhow!("Failed to list episodes: {e}"))?;

            if json {
                println!("{}", serde_json::to_string_pretty(&episodes)?);
            } else {
                println!("\n🧠 [Strata Episodic Memory Traces ({} found)]", episodes.len());
                println!("════════════════════════════════════════════════════════════════════════════════════");
                println!("{:<36}  {:<12}  {:<6}  {:<8}  {:<8}  SUMMARY", "ID", "AGENT", "STEPS", "SALIENCE", "STATUS");
                println!("────────────────────────────────────────────────────────────────────────────────────");
                for ep in &episodes {
                    let status = if ep.consolidated { "CONSOLID" } else { "PENDING" };
                    let summary_truncated = if ep.summary.len() > 30 {
                        format!("{}...", &ep.summary[..27])
                    } else {
                        ep.summary.clone()
                    };
                    println!(
                        "{:<36}  {:<12}  {:<6}  {:<8.2}  {:<8}  {}",
                        ep.id,
                        if ep.agent_id.len() > 12 { &ep.agent_id[..12] } else { &ep.agent_id },
                        ep.event_trace.len(),
                        ep.salience_score,
                        status,
                        summary_truncated
                    );
                }
            }
        }

        EpisodeCommand::Replay { id, json } => {
            let uuid = Uuid::parse_str(&id)
                .with_context(|| format!("Invalid episode UUID: '{id}'"))?;

            let replay = store
                .replay_episode(&uuid)
                .map_err(|e| anyhow::anyhow!("Failed to replay episode: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("Episode with ID '{id}' not found"))?;

            if json {
                println!("{}", serde_json::to_string_pretty(&replay)?);
            } else {
                println!("\n🔁 [Strata Episodic Trace Replay]");
                println!("══════════════════════════════════════════════════");
                println!("Episode ID:       {}", replay.episode_id);
                println!("Session:          {}", replay.session_id);
                println!("Agent:            {}", replay.agent_id);
                println!("Total Steps:      {}", replay.total_steps);
                println!("Successful Steps: {}", replay.successful_steps);
                println!("Failed Steps:     {}", replay.failed_steps);
                println!("Simulated Reward: {:.2}", replay.simulated_reward);

                if !replay.extracted_insights.is_empty() {
                    println!("\n🔍 Extracted Insights:");
                    for ins in &replay.extracted_insights {
                        println!("  • {ins}");
                    }
                }

                if !replay.candidate_facts.is_empty() {
                    println!("\n💡 Candidate Semantic Facts:");
                    for fact in &replay.candidate_facts {
                        println!("  • {fact}");
                    }
                }

                if !replay.candidate_anti_patterns.is_empty() {
                    println!("\n⚠️ Candidate Anti-Patterns:");
                    for ap in &replay.candidate_anti_patterns {
                        println!("  • {ap}");
                    }
                }
            }
        }

        EpisodeCommand::Consolidate { session, json } => {
            let result = store
                .consolidate_episodes(session.as_deref())
                .map_err(|e| anyhow::anyhow!("Failed to consolidate episodes: {e}"))?;

            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("\n🧠 [Strata Hippocampal Consolidation Completed]");
                println!("══════════════════════════════════════════════════");
                println!("Episodes Processed:       {}", result.episodes_processed);
                println!("Semantic Facts Created:   {}", result.facts_consolidated);
                println!("Promoted to Core Tier:    {}", result.promoted_to_core);
                println!("Anti-Patterns Learned:    {}", result.anti_patterns_learned);
                println!("Contradictions Resolved:  {}", result.contradictions_resolved);
            }
        }
    }

    Ok(())
}
