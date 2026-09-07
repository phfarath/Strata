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

    /// Listen to real-time events on the local A2A IPC event bus
    Listen {
        #[arg(long, help = "Output raw JSON lines")]
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

            // Broadcast over IPC if broker is active
            if let LeaseAcquireResult::Acquired {
                ref resource_id,
                expires_at,
            } = res
            {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let (endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
                if let Ok(ipc) = strata_memory::IpcClient::connect(&endpoint).await {
                    let _ = ipc
                        .publish(strata_core::a2a::IpcEvent::LeaseAcquired {
                            resource_id: resource_id.clone(),
                            agent_id: agent.clone(),
                            expires_at,
                            metadata: metadata.clone(),
                            timestamp_us: chrono::Utc::now().timestamp_micros(),
                        })
                        .await;
                }
            }

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

            // Broadcast over IPC if broker is active
            if released {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let (endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
                if let Ok(ipc) = strata_memory::IpcClient::connect(&endpoint).await {
                    let _ = ipc
                        .publish(strata_core::a2a::IpcEvent::LeaseReleased {
                            resource_id: resource.clone(),
                            agent_id: agent.clone(),
                            timestamp_us: chrono::Utc::now().timestamp_micros(),
                        })
                        .await;
                }
            }

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

        Some(A2aAction::Listen { json }) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
            println!("📡 Connecting to A2A IPC event bus at {endpoint}...");

            let (client_opt, _server) =
                strata_memory::IpcBrokerManager::start_or_connect(&cwd).await;
            let client = match client_opt {
                Some(c) => c,
                None => {
                    eprintln!("✗ Could not initialize A2A IPC bus. Ensure permissions are valid.");
                    return Ok(());
                }
            };

            println!("✓ Connected! Listening for real-time stigmergic events (Press Ctrl+C to exit)...\n");
            let mut sub = client.subscribe();

            loop {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        println!("\nDisconnected from A2A IPC event bus.");
                        break;
                    }
                    recv_res = sub.recv() => {
                        match recv_res {
                            Ok(event) => {
                                if json {
                                    if let Ok(line) = serde_json::to_string(&event) {
                                        println!("{line}");
                                    }
                                } else {
                                    match &event {
                                        strata_core::a2a::IpcEvent::LeaseAcquired { resource_id, agent_id, expires_at, metadata, .. } => {
                                            let meta = metadata.as_deref().unwrap_or("-");
                                            println!("🔒 [LEASE ACQUIRED] resource='{}' agent='{}' expires={} meta='{}'", resource_id, agent_id, expires_at, meta);
                                        }
                                        strata_core::a2a::IpcEvent::LeaseReleased { resource_id, agent_id, .. } => {
                                            println!("🔓 [LEASE RELEASED] resource='{}' agent='{}'", resource_id, agent_id);
                                        }
                                        strata_core::a2a::IpcEvent::AntiPatternDiscovered { category, pattern, remedy, .. } => {
                                            println!("⚡ [ANTI-PATTERN ALERT] category='{}' pattern='{}' remedy='{}'", category, pattern, remedy);
                                        }
                                        strata_core::a2a::IpcEvent::AgentPresenceChanged { agent_id, host, status, .. } => {
                                            println!("👥 [AGENT PRESENCE] agent='{}' host='{}' status='{}'", agent_id, host, status);
                                        }
                                        strata_core::a2a::IpcEvent::MemoryPromoted { memory_id, tier, to_global, .. } => {
                                            println!("⭐ [MEMORY PROMOTED] id='{}' tier='{}' global={}", memory_id, tier, to_global);
                                        }
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                                eprintln!("⚠️ [IPC Alert] Lagged behind by {count} messages");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                println!("\nA2A IPC server closed connection.");
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
