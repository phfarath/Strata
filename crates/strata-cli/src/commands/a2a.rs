use anyhow::Result;
use clap::{Args, Subcommand};
use std::time::Duration;
use uuid::Uuid;

use strata_core::a2a::{
    A2aMessage, AgentCard, AgentPresence, DomainAuthority, IpcEvent, LeaseAcquireResult,
    ResourceLease, TransactiveQuery, TransactiveResponse,
};
use strata_memory::StigmergyCoordinator;

use crate::agent_env::{is_json_mode, resolve_agent_id, resolve_task_id};

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

    /// Send a direct message to another agent
    Send {
        #[arg(help = "Destination agent identifier")]
        to: String,

        #[arg(help = "Message payload")]
        content: String,

        #[arg(long, help = "Task ID this message relates to")]
        task: Option<String>,

        #[arg(long, help = "Sender agent identifier")]
        agent: Option<String>,

        #[arg(
            long,
            default_value = "direct",
            help = "Message kind (direct, handoff, question, blocker)"
        )]
        kind: String,

        #[arg(long)]
        json: bool,
    },

    /// Broadcast a message to all agents in the workspace
    Broadcast {
        #[arg(help = "Message payload")]
        content: String,

        #[arg(long, help = "Task ID this message relates to")]
        task: Option<String>,

        #[arg(long, help = "Sender agent identifier")]
        agent: Option<String>,

        #[arg(long, default_value = "broadcast", help = "Message kind")]
        kind: String,

        #[arg(long)]
        json: bool,
    },

    /// Query inbox messages for an agent
    Inbox {
        #[arg(long, help = "Agent identifier (defaults to STRATA_AGENT_ID)")]
        agent: Option<String>,

        #[arg(long, help = "Show only unread messages")]
        unread: bool,

        #[arg(long, help = "Automatically mark fetched messages as read")]
        mark_read: bool,

        #[arg(long)]
        json: bool,
    },

    /// Synchronously wait for a matching event on the local IPC bus (zero polling)
    Wait {
        #[arg(long, help = "Filter by task ID")]
        task: Option<String>,

        #[arg(long, help = "Filter by sender agent ID")]
        from: Option<String>,

        #[arg(
            long,
            default_value_t = 30,
            help = "Timeout in seconds before exiting (default: 30s)"
        )]
        timeout: u64,

        #[arg(long)]
        json: bool,
    },

    /// Inspect or resolve domains in the transactive memory directory
    Directory {
        #[arg(long, help = "Resolve the best specialist agent for this domain/path")]
        resolve: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Register or update agent specialization and domain authority
    Announce {
        #[arg(long, help = "Agent identifier (defaults to STRATA_AGENT_ID)")]
        agent: Option<String>,

        #[arg(
            long,
            help = "Agent host runner (defaults to STRATA_AGENT_HOST or auto-detected)"
        )]
        host: Option<String>,

        #[arg(
            long,
            help = "Comma-separated domains or code partitions (e.g. 'crates/strata-memory,db')"
        )]
        domains: Option<String>,

        #[arg(long, default_value_t = 1.0, help = "Confidence score (0.0 - 1.0)")]
        confidence: f32,

        #[arg(long)]
        json: bool,
    },

    /// Route a transactive query to the specialist agent for a domain
    Query {
        #[arg(help = "Domain or code partition to query")]
        domain: String,

        #[arg(help = "Question or context request")]
        question: String,

        #[arg(long, help = "Sender agent identifier")]
        agent: Option<String>,

        #[arg(
            long,
            default_value_t = 0,
            help = "Synchronous wait timeout in ms (0 for async)"
        )]
        timeout_ms: u64,

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
                                        strata_core::a2a::IpcEvent::DirectMessage { from_agent, to_agent, task_id, content, .. } => {
                                            let task_str = task_id.as_deref().unwrap_or("-");
                                            println!("💬 [DIRECT MSG] from='{}' to='{}' task='{}': {}", from_agent, to_agent, task_str, content);
                                        }
                                        strata_core::a2a::IpcEvent::TaskBroadcast { from_agent, task_id, kind, content, .. } => {
                                            let task_str = task_id.as_deref().unwrap_or("-");
                                            println!("📢 [BROADCAST] from='{}' task='{}' kind='{}': {}", from_agent, task_str, kind, content);
                                        }
                                        strata_core::a2a::IpcEvent::TaskStateChanged { task_id, old_status, new_status, agent_id, .. } => {
                                            let agent_str = agent_id.as_deref().unwrap_or("-");
                                            println!("📋 [TASK STATE] id='{}' status: {} -> {} (by {})", task_id, old_status, new_status, agent_str);
                                        }
                                        strata_core::a2a::IpcEvent::AgentAnnounced { card, .. } => {
                                            println!("🪪 [AGENT ANNOUNCED] agent='{}' host='{}' caps={:?} domains={:?}", card.agent_id, card.host, card.memory_capabilities, card.domain_scopes);
                                        }
                                        strata_core::a2a::IpcEvent::DomainAuthorityUpdated { authority, .. } => {
                                            println!("🏛️ [DOMAIN AUTHORITY] domain='{}' agent='{}' conf={:.2} lease={}", authority.domain, authority.authoritative_agent, authority.confidence_score, authority.lease_held);
                                        }
                                        strata_core::a2a::IpcEvent::TransactiveQueryRouted { query, .. } => {
                                            println!("🔍 [TRANSACTIVE QUERY] from='{}' domain='{}': {}", query.from_agent, query.domain_topic, query.query_text);
                                        }
                                        strata_core::a2a::IpcEvent::TransactiveResponseReturned { response, .. } => {
                                            println!("💡 [TRANSACTIVE RESPONSE] responder='{}' conf={:.2}: {}", response.responder_agent, response.confidence_score, response.answer);
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

        Some(A2aAction::Send {
            to,
            content,
            task,
            agent,
            kind,
            json,
        }) => {
            let json_mode = is_json_mode(json);
            let from_agent = resolve_agent_id(agent.as_deref());
            let task_id = resolve_task_id(task.as_deref());
            let message_id = format!("msg-{}", &Uuid::new_v4().simple().to_string()[..12]);

            let mut msg =
                A2aMessage::direct(&message_id, &from_agent, &to, &content).with_kind(kind);
            if let Some(t) = &task_id {
                msg = msg.with_task_id(t);
            }

            coordinator.save_a2a_message(&msg)?;

            // Broadcast over IPC bus
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
            if let Ok(ipc) = strata_memory::IpcClient::connect(&endpoint).await {
                let _ = ipc
                    .publish(IpcEvent::DirectMessage {
                        message_id: message_id.clone(),
                        from_agent: from_agent.clone(),
                        to_agent: to.clone(),
                        task_id: task_id.clone(),
                        content: content.clone(),
                        timestamp_us: chrono::Utc::now().timestamp_micros(),
                    })
                    .await;
            }

            if json_mode {
                println!("{}", serde_json::to_string_pretty(&msg)?);
            } else {
                println!(
                    "✓ Message '{}' sent to '{}' by '{}'.",
                    message_id, to, from_agent
                );
            }
        }

        Some(A2aAction::Broadcast {
            content,
            task,
            agent,
            kind,
            json,
        }) => {
            let json_mode = is_json_mode(json);
            let from_agent = resolve_agent_id(agent.as_deref());
            let task_id = resolve_task_id(task.as_deref());
            let message_id = format!("msg-{}", &Uuid::new_v4().simple().to_string()[..12]);

            let mut msg =
                A2aMessage::broadcast(&message_id, &from_agent, &content).with_kind(kind.clone());
            if let Some(t) = &task_id {
                msg = msg.with_task_id(t);
            }

            coordinator.save_a2a_message(&msg)?;

            // Broadcast over IPC bus
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
            if let Ok(ipc) = strata_memory::IpcClient::connect(&endpoint).await {
                let _ = ipc
                    .publish(IpcEvent::TaskBroadcast {
                        message_id: message_id.clone(),
                        from_agent: from_agent.clone(),
                        task_id: task_id.clone(),
                        kind,
                        content: content.clone(),
                        timestamp_us: chrono::Utc::now().timestamp_micros(),
                    })
                    .await;
            }

            if json_mode {
                println!("{}", serde_json::to_string_pretty(&msg)?);
            } else {
                println!(
                    "✓ Broadcast message '{}' dispatched by '{}'.",
                    message_id, from_agent
                );
            }
        }

        Some(A2aAction::Inbox {
            agent,
            unread,
            mark_read,
            json,
        }) => {
            let json_mode = is_json_mode(json);
            let agent_id = resolve_agent_id(agent.as_deref());
            let messages = coordinator.get_a2a_inbox(&agent_id, unread)?;

            if mark_read && !messages.is_empty() {
                let ids: Vec<String> = messages.iter().map(|m| m.message_id.clone()).collect();
                let _ = coordinator.mark_a2a_read(&ids);
            }

            if json_mode {
                println!("{}", serde_json::to_string_pretty(&messages)?);
            } else {
                println!("\n📬 [A2A Mailbox Inbox: {}]", agent_id);
                println!("═════════════════════════════════════════════════════════════");
                if messages.is_empty() {
                    println!("  (Inbox is empty)");
                } else {
                    for m in &messages {
                        let read_str = if m.read_at.is_some() {
                            "read"
                        } else {
                            "UNREAD"
                        };
                        let task_str = m.task_id.as_deref().unwrap_or("-");
                        println!(
                            "  • [{:<6}] from={:<14} task={:<12} kind={:<10} content='{}'",
                            read_str, m.from_agent, task_str, m.kind, m.content
                        );
                    }
                }
                println!();
            }
        }

        Some(A2aAction::Wait {
            task,
            from,
            timeout,
            json,
        }) => {
            let json_mode = is_json_mode(json);
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (client_opt, _server) =
                strata_memory::IpcBrokerManager::start_or_connect(&cwd).await;
            let client = match client_opt {
                Some(c) => c,
                None => {
                    anyhow::bail!("Could not connect to A2A IPC bus.");
                }
            };

            let filter_task = resolve_task_id(task.as_deref());
            let filter_from = from;

            let event_opt = client
                .wait_for_event(
                    move |event| match event {
                        IpcEvent::DirectMessage {
                            from_agent,
                            task_id,
                            ..
                        } => {
                            if let Some(expected_from) = &filter_from {
                                if from_agent != expected_from {
                                    return false;
                                }
                            }
                            if let Some(expected_task) = &filter_task {
                                if task_id.as_deref() != Some(expected_task.as_str()) {
                                    return false;
                                }
                            }
                            true
                        }
                        IpcEvent::TaskBroadcast {
                            from_agent,
                            task_id,
                            ..
                        } => {
                            if let Some(expected_from) = &filter_from {
                                if from_agent != expected_from {
                                    return false;
                                }
                            }
                            if let Some(expected_task) = &filter_task {
                                if task_id.as_deref() != Some(expected_task.as_str()) {
                                    return false;
                                }
                            }
                            true
                        }
                        IpcEvent::TaskStateChanged {
                            task_id, agent_id, ..
                        } => {
                            if let Some(expected_task) = &filter_task {
                                if task_id != expected_task {
                                    return false;
                                }
                            }
                            if let Some(expected_from) = &filter_from {
                                if agent_id.as_deref() != Some(expected_from.as_str()) {
                                    return false;
                                }
                            }
                            true
                        }
                        _ => false,
                    },
                    Duration::from_secs(timeout),
                )
                .await?;

            match event_opt {
                Some(event) => {
                    if json_mode {
                        println!(
                            "{}",
                            serde_json::json!({ "status": "event_received", "event": event })
                        );
                    } else {
                        println!("✓ Event received: {:?}", event);
                    }
                }
                None => {
                    if json_mode {
                        println!(
                            "{}",
                            serde_json::json!({ "status": "timeout", "waited_seconds": timeout })
                        );
                    } else {
                        println!("⏱️ Timeout reached ({}s) without matching event.", timeout);
                    }
                }
            }
        }

        Some(A2aAction::Directory { resolve, json }) => {
            let json_mode = is_json_mode(json);
            if let Some(domain) = resolve {
                let auth_opt = coordinator.resolve_domain_authority(&domain)?;
                if json_mode {
                    println!("{}", serde_json::to_string_pretty(&auth_opt)?);
                } else {
                    println!("\n🔍 [Transactive Directory Resolution: '{}']", domain);
                    println!("═════════════════════════════════════════════════════════════");
                    match auth_opt {
                        Some(auth) => {
                            println!("  Specialist Agent : {}", auth.authoritative_agent);
                            println!("  Matched Domain   : {}", auth.domain);
                            println!("  Confidence Score : {:.2}", auth.confidence_score);
                            println!(
                                "  Active Lease     : {}",
                                if auth.lease_held {
                                    "YES (exclusive lock)"
                                } else {
                                    "no"
                                }
                            );
                            println!("  Active Tasks     : {}", auth.active_tasks_count);
                        }
                        None => {
                            println!("  (No specialist agent registered for this domain)");
                        }
                    }
                    println!();
                }
            } else {
                let agents = coordinator.list_agent_cards()?;
                let authorities = coordinator.list_domain_authorities()?;
                if json_mode {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "agents": agents,
                            "authorities": authorities,
                        }))?
                    );
                } else {
                    println!("\n📚 [Transactive Memory Directory]");
                    println!("═════════════════════════════════════════════════════════════");
                    println!("Registered Agents: {}", agents.len());
                    if agents.is_empty() {
                        println!("  (No agent capability cards announced)");
                    } else {
                        for a in &agents {
                            let domains_str = if a.domain_scopes.is_empty() {
                                "-".to_string()
                            } else {
                                a.domain_scopes.join(", ")
                            };
                            println!(
                                "  • {:<16} host={:<12} confidence={:.2} domains=[{}]",
                                a.agent_id, a.host, a.confidence_score, domains_str
                            );
                        }
                    }

                    println!("\nDomain Authorities: {}", authorities.len());
                    if authorities.is_empty() {
                        println!("  (No domain authorities registered)");
                    } else {
                        for auth in &authorities {
                            let lease_str = if auth.lease_held { " [LEASE HELD]" } else { "" };
                            println!(
                                "  • {:<28} -> {:<16} (conf: {:.2}){}",
                                auth.domain,
                                auth.authoritative_agent,
                                auth.confidence_score,
                                lease_str
                            );
                        }
                    }
                    println!();
                }
            }
        }

        Some(A2aAction::Announce {
            agent,
            host,
            domains,
            confidence,
            json,
        }) => {
            let json_mode = is_json_mode(json);
            let agent_id = resolve_agent_id(agent.as_deref());
            let host_name = host.unwrap_or_else(|| {
                std::env::var("STRATA_AGENT_HOST").unwrap_or_else(|_| "generic".to_string())
            });

            let domain_list: Vec<String> = domains
                .as_deref()
                .unwrap_or("")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let card = AgentCard::new(&agent_id, &host_name)
                .with_domain_scopes(domain_list.clone())
                .with_confidence(confidence);

            coordinator.register_agent_card(&card)?;

            let mut registered = Vec::new();
            for d in &domain_list {
                let auth = DomainAuthority::new(d, &agent_id, confidence);
                coordinator.set_domain_authority(&auth)?;
                registered.push(auth);
            }

            // Publish IPC events
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
            if let Ok(ipc) = strata_memory::IpcClient::connect(&endpoint).await {
                let now_us = chrono::Utc::now().timestamp_micros();
                let _ = ipc
                    .publish(IpcEvent::AgentAnnounced {
                        card: card.clone(),
                        timestamp_us: now_us,
                    })
                    .await;
                for auth in &registered {
                    let _ = ipc
                        .publish(IpcEvent::DomainAuthorityUpdated {
                            authority: auth.clone(),
                            timestamp_us: now_us,
                        })
                        .await;
                }
            }

            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "announced",
                        "agent_card": card,
                        "authorities": registered,
                    }))?
                );
            } else {
                println!(
                    "✓ Agent '{}' announced capability card with {} domain authorities.",
                    agent_id,
                    registered.len()
                );
                for a in &registered {
                    println!(
                        "  • Domain: {:<24} (conf: {:.2})",
                        a.domain, a.confidence_score
                    );
                }
            }
        }

        Some(A2aAction::Query {
            domain,
            question,
            agent,
            timeout_ms,
            json,
        }) => {
            let json_mode = is_json_mode(json);
            let from_agent = resolve_agent_id(agent.as_deref());
            let query_id = format!("qry-{}", &Uuid::new_v4().simple().to_string()[..12]);

            let best_auth = coordinator.resolve_domain_authority(&domain)?;
            let target_agent = best_auth.as_ref().map(|a| a.authoritative_agent.clone());

            let mut query = TransactiveQuery::new(&query_id, &from_agent, &domain, &question);
            if timeout_ms > 0 {
                query = query.with_timeout_ms(timeout_ms);
            }

            // Persist as A2A message in inbox
            let message_id = format!("msg-{}", &Uuid::new_v4().simple().to_string()[..12]);
            let a2a_msg = match &target_agent {
                Some(to) => A2aMessage::direct(&message_id, &from_agent, to, &question)
                    .with_kind("transactive_query"),
                None => A2aMessage::broadcast(&message_id, &from_agent, &question)
                    .with_kind("transactive_query"),
            };
            coordinator.save_a2a_message(&a2a_msg)?;

            // Publish IPC event
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (endpoint, _) = strata_memory::endpoint_for_workspace(&cwd);
            let mut ipc_client = None;
            if let Ok(ipc) = strata_memory::IpcClient::connect(&endpoint).await {
                let _ = ipc
                    .publish(IpcEvent::TransactiveQueryRouted {
                        query: query.clone(),
                        timestamp_us: chrono::Utc::now().timestamp_micros(),
                    })
                    .await;
                ipc_client = Some(ipc);
            }

            // If synchronous wait requested:
            let mut response_received: Option<TransactiveResponse> = None;
            if timeout_ms > 0 {
                if let Some(ipc) = ipc_client {
                    let expected_query_id = query_id.clone();
                    let event_opt = ipc
                        .wait_for_event(
                            move |event| match event {
                                IpcEvent::TransactiveResponseReturned { response, .. } => {
                                    response.query_id == expected_query_id
                                }
                                _ => false,
                            },
                            Duration::from_millis(timeout_ms),
                        )
                        .await?;

                    if let Some(IpcEvent::TransactiveResponseReturned { response, .. }) = event_opt
                    {
                        response_received = Some(response);
                    }
                }
            }

            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "query_id": query_id,
                        "domain": domain,
                        "target_agent": target_agent,
                        "authority": best_auth,
                        "response": response_received,
                    }))?
                );
            } else {
                println!("\n📬 [Transactive Query Routed]");
                println!("═════════════════════════════════════════════════════════════");
                println!("  Query ID     : {}", query_id);
                println!("  Domain       : {}", domain);
                match &target_agent {
                    Some(tgt) => println!("  Routed To    : {} (Specialist)", tgt),
                    None => println!("  Routed To    : (Broadcast - No specialist registered)"),
                }
                if let Some(resp) = response_received {
                    println!(
                        "  Response     : {} (from: {}, confidence: {:.2})",
                        resp.answer, resp.responder_agent, resp.confidence_score
                    );
                } else if timeout_ms > 0 {
                    println!(
                        "  Response     : Timed out waiting for response ({} ms)",
                        timeout_ms
                    );
                } else {
                    println!("  Status       : Routed asynchronously. Target agent notified.");
                }
                println!();
            }
        }
    }

    Ok(())
}
