use anyhow::Result;
use clap::{Args, Subcommand};
use strata_core::a2a::{AgentPresence, LeaseAcquireResult, ResourceLease};
use strata_memory::StigmergyCoordinator;

#[derive(Args, Debug)]
pub struct A2aArgs {
    #[command(subcommand)]
    pub action: Option<A2aAction>,

    /// Freshness window in seconds for active agent presence (default: 60)
    #[arg(long, default_value_t = 60)]
    pub ttl: i64,

    /// Output as raw JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum A2aAction {
    /// Show full status of active agents and resource leases in the workspace
    Status {
        #[arg(long, default_value_t = 60)]
        ttl: i64,
        #[arg(long)]
        json: bool,
    },

    /// List active agents in the workspace
    Who {
        #[arg(long, default_value_t = 60)]
        ttl: i64,
        #[arg(long)]
        json: bool,
    },

    /// List currently active unexpired resource leases
    Leases {
        #[arg(long)]
        json: bool,
    },

    /// Atomically acquire or renew a lease on a resource
    Acquire {
        #[arg(help = "Target resource ID (e.g. 'crate:strata-cli', 'file:src/main.rs')")]
        resource: String,

        #[arg(
            long,
            help = "Agent identifier acquiring the lease (e.g. 'cli', 'cursor')"
        )]
        agent: String,

        #[arg(long, default_value_t = 30, help = "TTL in seconds for the lease")]
        ttl: i64,

        #[arg(long, help = "Optional task metadata or description")]
        metadata: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Release an existing resource lease
    Release {
        #[arg(help = "Target resource ID to release")]
        resource: String,

        #[arg(long, help = "Agent identifier releasing the lease")]
        agent: String,

        #[arg(long)]
        json: bool,
    },

    /// Clean up expired leases
    Prune {
        #[arg(long)]
        json: bool,
    },
}

#[derive(serde::Serialize)]
struct A2aStatusOutput {
    agents: Vec<AgentPresence>,
    leases: Vec<ResourceLease>,
}

pub async fn run_a2a(args: A2aArgs, coordinator: StigmergyCoordinator) -> Result<()> {
    match args.action {
        None | Some(A2aAction::Status { .. }) => {
            let ttl = match &args.action {
                Some(A2aAction::Status { ttl, .. }) => *ttl,
                _ => args.ttl,
            };
            let json = match &args.action {
                Some(A2aAction::Status { json, .. }) => *json,
                _ => args.json,
            };

            let agents = coordinator.active_agents(ttl)?;
            let leases = coordinator.active_leases()?;

            if json {
                let out = A2aStatusOutput { agents, leases };
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("\n🌐 [Strata Stigmergic Workspace Status]");
                println!("═════════════════════════════════════════════════════════════");
                println!(
                    "Active Agents (Heartbeat within {}s): {}",
                    ttl,
                    agents.len()
                );
                if agents.is_empty() {
                    println!("  (No active agents currently registered)");
                } else {
                    for a in &agents {
                        let task = a.active_task.as_deref().unwrap_or("idle");
                        println!(
                            "  • {:<16} host={:<10} pid={:<6} task={}",
                            a.agent_id, a.host, a.pid, task
                        );
                    }
                }

                println!("\nActive Resource Leases: {}", leases.len());
                if leases.is_empty() {
                    println!("  (No active locks/leases held)");
                } else {
                    let now = chrono::Utc::now().timestamp();
                    for l in &leases {
                        let meta = l.metadata.as_deref().unwrap_or("-");
                        let rem = l.remaining_seconds(now);
                        println!(
                            "  🔒 {:<24} held_by={:<14} rem={}s meta={}",
                            l.resource_id, l.agent_id, rem, meta
                        );
                    }
                }
                println!();
            }
        }

        Some(A2aAction::Who { ttl, json }) => {
            let agents = coordinator.active_agents(ttl)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&agents)?);
            } else {
                println!("\n👥 Active Workspace Agents (TTL: {}s):", ttl);
                for a in &agents {
                    let task = a.active_task.as_deref().unwrap_or("idle");
                    println!(
                        "  • {:<16} host={:<12} pid={:<6} task={}",
                        a.agent_id, a.host, a.pid, task
                    );
                }
                println!();
            }
        }

        Some(A2aAction::Leases { json }) => {
            let leases = coordinator.active_leases()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&leases)?);
            } else {
                println!("\n🔒 Active Resource Leases:");
                let now = chrono::Utc::now().timestamp();
                for l in &leases {
                    let rem = l.remaining_seconds(now);
                    println!(
                        "  • {:<24} agent={:<14} rem={}s",
                        l.resource_id, l.agent_id, rem
                    );
                }
                println!();
            }
        }

        Some(A2aAction::Acquire {
            resource,
            agent,
            ttl,
            metadata,
            json,
        }) => {
            let res = coordinator.acquire_lease(&resource, &agent, ttl, metadata.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                match res {
                    LeaseAcquireResult::Acquired {
                        resource_id,
                        expires_at,
                    } => {
                        println!(
                            "✓ Lease acquired on '{}' by '{}' until timestamp {}.",
                            resource_id, agent, expires_at
                        );
                    }
                    LeaseAcquireResult::Conflict {
                        resource_id,
                        held_by,
                        remaining_seconds,
                    } => {
                        println!(
                            "✗ CONFLICT: Resource '{}' is already leased by '{}' ({}s remaining).",
                            resource_id, held_by, remaining_seconds
                        );
                    }
                }
            }
        }

        Some(A2aAction::Release {
            resource,
            agent,
            json,
        }) => {
            let released = coordinator.release_lease(&resource, &agent)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "resource_id": resource,
                        "agent_id": agent,
                        "released": released
                    })
                );
            } else if released {
                println!("✓ Lease on '{}' released by '{}'.", resource, agent);
            } else {
                println!(
                    "Notice: No active lease found on '{}' for agent '{}'.",
                    resource, agent
                );
            }
        }

        Some(A2aAction::Prune { json }) => {
            let pruned = coordinator.prune_expired()?;
            if json {
                println!("{}", serde_json::json!({ "pruned_count": pruned }));
            } else {
                println!("✓ Pruned {} expired lease(s).", pruned);
            }
        }
    }

    Ok(())
}
