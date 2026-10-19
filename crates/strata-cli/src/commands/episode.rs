use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;
use uuid::Uuid;

use strata_core::schemas::{EpisodeRecord, EpisodeStep};
use strata_memory::{
    EpisodicPyramidEngine, PyramidConfig, SlmBridgeConfig, SlmPyramidBridge, SqliteStore,
};

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

    /// Hierarchical rollup of Tier 1 episodic traces into Tier 2 milestones and Tier 3 session narratives
    Rollup {
        /// Filter by session ID
        #[arg(long, help = "Filter by session ID")]
        session: Option<String>,

        /// Minimum cluster size required to form a rollup
        #[arg(long, default_value_t = 2, help = "Minimum cluster size")]
        min_cluster: usize,

        /// Cosine similarity threshold for grouping
        #[arg(
            long,
            default_value_t = 0.70,
            help = "Similarity threshold for clustering"
        )]
        similarity: f32,

        /// Generate Tier 3 session narrative via local SLM SmolLM2 adapter
        #[arg(
            long,
            help = "Generate Tier 3 session narrative via local SLM SmolLM2 adapter"
        )]
        slm_narrative: bool,

        /// Custom path to SmolLM2 LoRA adapter directory
        #[arg(long, help = "Custom path to SmolLM2 LoRA adapter directory")]
        adapter_path: Option<std::path::PathBuf>,

        /// Custom path to python binary for SLM inference
        #[arg(long, help = "Custom path to python binary")]
        python_bin: Option<String>,

        /// Output as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Drill down into a milestone rollup to inspect underlying execution traces
    DrillDown {
        /// UUID of the rollup episode record
        #[arg(help = "UUID of the rollup episode record")]
        id: String,

        /// Output as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,

        /// Output hierarchical ASCII tree representation
        #[arg(long, help = "Output hierarchical ASCII tree representation")]
        ascii: bool,
    },

    /// Zoom out and view the hierarchical tree of milestone rollups
    Tree {
        /// Filter by session ID
        #[arg(long, help = "Filter by session ID")]
        session: Option<String>,

        /// Maximum rollups to inspect
        #[arg(long, default_value_t = 20, help = "Maximum rollups to return")]
        limit: usize,

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
                println!(
                    "\n🧠 [Strata Episodic Memory Traces ({} found)]",
                    episodes.len()
                );
                println!("════════════════════════════════════════════════════════════════════════════════════");
                println!(
                    "{:<36}  {:<12}  {:<6}  {:<8}  {:<8}  SUMMARY",
                    "ID", "AGENT", "STEPS", "SALIENCE", "STATUS"
                );
                println!("────────────────────────────────────────────────────────────────────────────────────");
                for ep in &episodes {
                    let status = if ep.consolidated {
                        "CONSOLID"
                    } else {
                        "PENDING"
                    };
                    let summary_truncated = if ep.summary.len() > 30 {
                        format!("{}...", &ep.summary[..27])
                    } else {
                        ep.summary.clone()
                    };
                    println!(
                        "{:<36}  {:<12}  {:<6}  {:<8.2}  {:<8}  {}",
                        ep.id,
                        if ep.agent_id.len() > 12 {
                            &ep.agent_id[..12]
                        } else {
                            &ep.agent_id
                        },
                        ep.event_trace.len(),
                        ep.salience_score,
                        status,
                        summary_truncated
                    );
                }
            }
        }

        EpisodeCommand::Replay { id, json } => {
            let uuid =
                Uuid::parse_str(&id).with_context(|| format!("Invalid episode UUID: '{id}'"))?;

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
                println!(
                    "Contradictions Resolved:  {}",
                    result.contradictions_resolved
                );
            }
        }

        EpisodeCommand::Rollup {
            session,
            min_cluster,
            similarity,
            slm_narrative,
            adapter_path,
            python_bin,
            json,
        } => {
            let config = PyramidConfig::new()
                .with_min_cluster_size(min_cluster)
                .with_cluster_similarity_threshold(similarity);

            let rollups = EpisodicPyramidEngine::aggregate_session_rollups(
                store.as_ref(),
                session.as_deref(),
                &config,
                None,
            )
            .map_err(|e| anyhow::anyhow!("Failed to aggregate episodic rollups: {e}"))?;

            let mut tier3_record = None;
            if slm_narrative {
                let mut bridge_config = SlmBridgeConfig::default();
                if let Some(ap) = adapter_path {
                    bridge_config.adapter_path = ap;
                }
                if let Some(py) = python_bin {
                    bridge_config.python_binary = py;
                }
                let bridge = SlmPyramidBridge::new(bridge_config);
                tier3_record = EpisodicPyramidEngine::aggregate_session_narrative(
                    store.as_ref(),
                    session.as_deref(),
                    &bridge,
                )
                .map_err(|e| anyhow::anyhow!("Failed to generate Tier 3 session narrative: {e}"))?;
            }

            if json {
                let result = serde_json::json!({
                    "tier2_rollups": rollups,
                    "tier3_narrative": tier3_record,
                });
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("\n🏛️ [Strata Episodic Memory Pyramid Rollup Completed]");
                println!("══════════════════════════════════════════════════");
                println!("Generated Tier 2 Rollups: {}", rollups.len());
                for r in &rollups {
                    println!("\n  Rollup ID: {}", r.id);
                    println!("  Tier:      {}", r.tier);
                    println!("  Session:   {}", r.session_id);
                    println!("  Salience:  {:.2}", r.salience_score);
                    println!("  Children:  {}", r.child_ids.len());
                    println!("  Summary:   {}", r.summary);
                    if !r.outcomes.is_empty() {
                        println!("  Outcomes:  {}", r.outcomes.join(", "));
                    }
                }

                if let Some(t3) = tier3_record {
                    println!("\n📜 [Tier 3 Session Narrative (SmolLM2 SLM Bridge)]");
                    println!("──────────────────────────────────────────────────");
                    println!("  Epoch ID:   {}", t3.id);
                    println!("  Session:    {}", t3.session_id);
                    println!("  Milestones: {}", t3.child_ids.len());
                    println!("  Salience:   {:.2}", t3.salience_score);
                    println!("  Narrative:  {}", t3.summary);
                    if !t3.outcomes.is_empty() {
                        println!("  Outcomes:   {}", t3.outcomes.join(", "));
                    }
                }
            }
        }

        EpisodeCommand::DrillDown { id, json, ascii: _ } => {
            let uuid = Uuid::parse_str(&id)
                .with_context(|| format!("Invalid rollup episode UUID: '{id}'"))?;

            let tree = EpisodicPyramidEngine::drill_down(store.as_ref(), &uuid)
                .map_err(|e| anyhow::anyhow!("Failed to drill down: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("Rollup episode with ID '{id}' not found"))?;

            if json {
                println!("{}", serde_json::to_string_pretty(&tree)?);
            } else {
                print!("{}", tree.render_ascii());
            }
        }

        EpisodeCommand::Tree {
            session,
            limit,
            json,
        } => {
            let rollups =
                EpisodicPyramidEngine::zoom_out(store.as_ref(), session.as_deref(), limit)
                    .map_err(|e| anyhow::anyhow!("Failed to zoom out rollups: {e}"))?;

            let mut trees = Vec::new();
            for r in &rollups {
                if let Some(tree) = EpisodicPyramidEngine::drill_down(store.as_ref(), &r.id)
                    .map_err(|e| anyhow::anyhow!("Failed to drill down rollup '{}': {e}", r.id))?
                {
                    trees.push(tree);
                }
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&trees)?);
            } else {
                if trees.is_empty() {
                    println!("No Tier 2 rollups found.");
                } else {
                    println!(
                        "\n🌲 [Strata Episodic Memory Pyramid Trees ({} rollups)]",
                        trees.len()
                    );
                    println!(
                        "═══════════════════════════════════════════════════════════════════════"
                    );
                    for tree in &trees {
                        print!("{}", tree.render_ascii());
                    }
                }
            }
        }
    }

    Ok(())
}
