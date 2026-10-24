//! MCP tools for the multi-agent blackboard (`task_*`) and A2A mailbox (`a2a_*`).
//!
//! Thin adapters over `StigmergyCoordinator`, mirroring the `strata task` / `strata a2a`
//! CLI so that MCP-only hosts (Cursor, Claude Code, Antigravity) can coordinate too.

use std::sync::Arc;

use serde_json::{json, Value};
use uuid::Uuid;

use strata_core::a2a::{
    A2aMessage, AgentCard, DelegationContract, DelegationStatus, DomainAuthority, IpcEvent,
    TransactiveQuery,
};
use strata_core::blackboard::{TaskArtifact, TaskFilter, TaskStatus};
use strata_memory::{IpcClient, StigmergyCoordinator};

use super::protocol::{CallToolResult, ToolDefinition};
use crate::agent_env::resolve_agent_id;

const AGENT_ID: (&str, &str) = (
    "agent_id",
    "Identity of the calling agent (defaults to STRATA_AGENT_ID or host-pid)",
);

fn tool(
    name: &str,
    description: &str,
    props: &[(&str, &str, &str)],
    required: &[&str],
) -> ToolDefinition {
    let properties: serde_json::Map<String, Value> = props
        .iter()
        .chain(std::iter::once(&(AGENT_ID.0, "string", AGENT_ID.1)))
        .map(|(k, ty, desc)| (k.to_string(), json!({ "type": ty, "description": desc })))
        .collect();
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: json!({ "type": "object", "properties": properties, "required": required }),
    }
}

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        tool(
            "task_list",
            "List tasks on the multi-agent blackboard. With task_id, returns that task plus its artifacts (inspect).",
            &[
                ("task_id", "string", "Inspect one task (with artifacts) instead of listing"),
                ("plan", "string", "Filter by plan ID"),
                ("status", "string", "pending, claimed, running, blocked, completed, failed"),
                ("agent", "string", "Filter by assigned agent"),
                ("limit", "integer", "Max tasks (default 50)"),
            ],
            &[],
        ),
        tool(
            "task_claim",
            "Atomically claim a task (work-stealing: next ready, unblocked task when task_id is omitted) and lock its required resources. Returns upstream artifacts for context handoff.",
            &[
                ("task_id", "string", "Specific task to claim; omit to claim the next ready task"),
                ("ttl_seconds", "integer", "Resource lease TTL (default 180)"),
                ("supported_types", "string", "Comma-separated list of supported payload types (e.g. 'shell_command', 'agent_prompt')"),
            ],
            &[],
        ),
        tool(
            "task_progress",
            "Report progress on a claimed task. Doubles as the liveness heartbeat: silent tasks are reclaimed by task_reclaim.",
            &[
                ("task_id", "string", "Task being worked on"),
                ("message", "string", "Progress note"),
            ],
            &["task_id", "message"],
        ),
        tool(
            "task_complete",
            "Mark a task completed, release its locks, attach an optional artifact and unblock downstream tasks.",
            &[
                ("task_id", "string", "Task to complete"),
                ("summary", "string", "Summary of delivered work"),
                ("artifact_content", "string", "Artifact payload handed to downstream tasks"),
            ],
            &["task_id"],
        ),
        tool(
            "task_fail",
            "Mark a task as failed and release its resource locks.",
            &[("task_id", "string", "Task that failed"), ("error", "string", "Failure reason")],
            &["task_id", "error"],
        ),
        tool(
            "task_reclaim",
            "Return orphaned tasks (claimed/running, no progress for stale_secs) to pending and drop their locks, so another agent can claim them.",
            &[("stale_secs", "integer", "Silence threshold in seconds (default 600)")],
            &[],
        ),
        tool(
            "a2a_send",
            "Send a message to a peer agent, or broadcast to all agents when 'to' is omitted.",
            &[
                ("to", "string", "Recipient agent ID; omit to broadcast"),
                ("content", "string", "Message body"),
                ("task_id", "string", "Related task ID"),
                ("kind", "string", "direct, broadcast, blocker, question, handoff"),
            ],
            &["content"],
        ),
        tool(
            "a2a_inbox",
            "Read this agent's A2A mailbox (direct + broadcast). Returned messages are marked as read.",
            &[("unread", "boolean", "Only unread messages (default true)")],
            &[],
        ),
        tool(
            "a2a_directory",
            "Query the transactive memory directory. With 'resolve_domain', finds the best specialist agent for a code path or domain via hierarchical prefix matching.",
            &[
                ("resolve_domain", "string", "Code path or domain to resolve (e.g. 'crates/strata-memory/src/jtms.rs')"),
            ],
            &[],
        ),
        tool(
            "a2a_announce",
            "Announce agent capabilities, specialization, and domain authorities to the multi-agent transactive directory.",
            &[
                ("domains", "string", "Comma-separated domains or file paths this agent specializes in"),
                ("confidence", "number", "Confidence score between 0.0 and 1.0 (default 1.0)"),
                ("host", "string", "Host environment (e.g. 'antigravity', 'claude-code', 'cursor')"),
            ],
            &[],
        ),
        tool(
            "a2a_query_transactive",
            "Route a transactive query to the authoritative specialist agent for a domain or code path.",
            &[
                ("domain", "string", "Target domain or file path (e.g. 'crates/strata-core')"),
                ("question", "string", "Question or request for domain expertise"),
            ],
            &["domain", "question"],
        ),
        tool(
            "a2a_delegate",
            "Delegate a sub-task with verification contract to a peer agent or domain specialist. Automatically resolves specialist if 'to' is omitted.",
            &[
                ("objective", "string", "Task objective or prompt to execute"),
                ("domain", "string", "Target code domain (e.g. 'crates/strata-memory')"),
                ("to", "string", "Explicit delegatee agent; resolves via transactive directory if omitted"),
                ("task_id", "string", "Blackboard task ID (auto-generated if omitted)"),
                ("context", "string", "Additional context or instruction for the task"),
                ("artifact", "string", "Expected output artifact path (comma-separated or single)"),
                ("verify", "string", "Verification command (e.g. 'cargo test -p strata-memory')"),
                ("timeout", "integer", "Timeout in seconds (default 60)"),
            ],
            &["objective"],
        ),
        tool(
            "a2a_accept",
            "Accept a pending A2A task delegation.",
            &[
                ("delegation_id", "string", "Delegation ID to accept"),
            ],
            &["delegation_id"],
        ),
        tool(
            "a2a_delegations",
            "List active or historical task delegations.",
            &[
                ("delegator", "string", "Filter by delegator agent"),
                ("delegatee", "string", "Filter by delegatee agent"),
                ("status", "string", "Filter by status: pending, accepted, completed, rejected, timed_out"),
            ],
            &[],
        ),
        tool(
            "a2a_complete_delegation",
            "Mark an A2A task delegation completed with verified summary and artifacts.",
            &[
                ("delegation_id", "string", "Delegation ID to complete"),
                ("summary", "string", "Summary of delivered work"),
                ("artifact", "string", "Produced artifact path (comma-separated or single)"),
                ("success", "boolean", "Whether execution succeeded (default true)"),
            ],
            &["delegation_id", "summary"],
        ),
    ]
}

/// Dispatches a blackboard/A2A tool. Returns `None` if `name` is not one of ours.
pub async fn execute(
    coordinator: &StigmergyCoordinator,
    ipc: Option<&Arc<IpcClient>>,
    name: &str,
    args: &Value,
) -> Option<CallToolResult> {
    if !definitions().iter().any(|d| d.name == name) {
        return None;
    }
    Some(match run(coordinator, ipc, name, args).await {
        Ok(v) => CallToolResult::text(serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => CallToolResult::error(e),
    })
}

fn opt<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn req<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    opt(args, key).ok_or_else(|| format!("Missing required parameter: {key}"))
}

async fn publish(ipc: Option<&Arc<IpcClient>>, event: IpcEvent) {
    if let Some(client) = ipc {
        let _ = client.publish(event).await;
    }
}

fn state_changed(
    task_id: &str,
    plan_id: Option<String>,
    old: TaskStatus,
    new: TaskStatus,
    agent: &str,
) -> IpcEvent {
    IpcEvent::TaskStateChanged {
        task_id: task_id.to_string(),
        plan_id,
        old_status: old,
        new_status: new,
        agent_id: Some(agent.to_string()),
        timestamp_us: chrono::Utc::now().timestamp_micros(),
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::new_v4().simple().to_string()[..12])
}

async fn run(
    coordinator: &StigmergyCoordinator,
    ipc: Option<&Arc<IpcClient>>,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    let err = |e: strata_core::errors::StrataError| e.to_string();
    let agent = resolve_agent_id(opt(args, "agent_id"));

    match name {
        "task_list" => {
            if let Some(id) = opt(args, "task_id") {
                let task = coordinator
                    .get_task(id)
                    .map_err(err)?
                    .ok_or_else(|| format!("Task '{id}' not found on blackboard."))?;
                let artifacts = coordinator.get_task_artifacts(id).map_err(err)?;
                return Ok(json!({ "task": task, "artifacts": artifacts }));
            }
            let filter = TaskFilter {
                plan_id: opt(args, "plan").map(String::from),
                status: opt(args, "status")
                    .map(|s| s.parse::<TaskStatus>())
                    .transpose()?,
                assigned_agent: opt(args, "agent").map(String::from),
                limit: Some(args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize),
            };
            Ok(json!(coordinator.list_tasks(&filter).map_err(err)?))
        }

        "task_claim" => {
            let ttl = args
                .get("ttl_seconds")
                .and_then(Value::as_i64)
                .unwrap_or(180);
            let supported_types: Option<Vec<&str>> = opt(args, "supported_types").map(|s| {
                s.split(',')
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .collect()
            });
            let Some(task) = coordinator
                .claim_task_atomic_typed(
                    opt(args, "task_id"),
                    &agent,
                    ttl,
                    supported_types.as_deref(),
                )
                .map_err(err)?
            else {
                return Ok(json!({
                    "status": "none_available",
                    "message": "No eligible pending tasks are currently available for claiming."
                }));
            };
            publish(
                ipc,
                state_changed(
                    &task.task_id,
                    task.plan_id.clone(),
                    TaskStatus::Pending,
                    TaskStatus::Claimed,
                    &agent,
                ),
            )
            .await;
            let upstream: Vec<TaskArtifact> = task
                .dependencies
                .iter()
                .filter_map(|dep| coordinator.get_task_artifacts(dep).ok())
                .flatten()
                .collect();
            Ok(json!({ "status": "claimed", "task": task, "upstream_artifacts": upstream }))
        }

        "task_progress" => {
            let id = req(args, "task_id")?;
            let gen = args.get("claim_generation").and_then(Value::as_i64);
            if !coordinator
                .update_task_progress_with_generation(id, &agent, req(args, "message")?, gen)
                .map_err(err)?
            {
                return Err(format!("Task '{id}' not found, not active, assigned to another agent, or invalid claim generation."));
            }
            Ok(json!({ "status": "ok", "task_id": id, "agent": agent }))
        }

        "task_complete" => {
            let id = req(args, "task_id")?;
            let gen = args.get("claim_generation").and_then(Value::as_i64);
            let artifact = opt(args, "artifact_content")
                .map(|payload| TaskArtifact::new(new_id("art"), id, &agent, "text", payload));
            if !coordinator
                .complete_task_with_generation(
                    id,
                    &agent,
                    opt(args, "summary"),
                    artifact.as_ref(),
                    gen,
                )
                .map_err(err)?
            {
                return Err(format!("Task '{id}' was not found, not active, assigned to another agent, or invalid claim generation."));
            }
            publish(
                ipc,
                state_changed(id, None, TaskStatus::Running, TaskStatus::Completed, &agent),
            )
            .await;
            Ok(
                json!({ "status": "completed", "task_id": id, "artifact_recorded": artifact.is_some() }),
            )
        }

        "task_fail" => {
            let id = req(args, "task_id")?;
            let gen = args.get("claim_generation").and_then(Value::as_i64);
            let error = req(args, "error")?;
            if !coordinator
                .fail_task_with_generation(id, &agent, error, gen)
                .map_err(err)?
            {
                return Err(format!("Task '{id}' was not found, not active, assigned to another agent, or invalid claim generation."));
            }
            publish(
                ipc,
                state_changed(id, None, TaskStatus::Running, TaskStatus::Failed, &agent),
            )
            .await;
            Ok(json!({ "status": "failed", "task_id": id, "error": error }))
        }

        "task_reclaim" => {
            let stale = args
                .get("stale_secs")
                .and_then(Value::as_i64)
                .unwrap_or(600);
            let reclaimed = coordinator.reclaim_stale_tasks(stale).map_err(err)?;
            for id in &reclaimed {
                publish(
                    ipc,
                    state_changed(id, None, TaskStatus::Running, TaskStatus::Pending, &agent),
                )
                .await;
            }
            Ok(json!({ "reclaimed": reclaimed }))
        }

        "a2a_send" => {
            let content = req(args, "content")?;
            let message_id = new_id("msg");
            let task_id = opt(args, "task_id").map(String::from);
            let mut msg = match opt(args, "to") {
                Some(to) => A2aMessage::direct(&message_id, &agent, to, content),
                None => A2aMessage::broadcast(&message_id, &agent, content),
            };
            if let Some(kind) = opt(args, "kind") {
                msg = msg.with_kind(kind);
            }
            if let Some(t) = &task_id {
                msg = msg.with_task_id(t);
            }
            coordinator.save_a2a_message(&msg).map_err(err)?;

            let timestamp_us = chrono::Utc::now().timestamp_micros();
            let event = match &msg.to_agent {
                Some(to) => IpcEvent::DirectMessage {
                    message_id: message_id.clone(),
                    from_agent: agent.clone(),
                    to_agent: to.clone(),
                    task_id,
                    content: content.to_string(),
                    timestamp_us,
                },
                None => IpcEvent::TaskBroadcast {
                    message_id: message_id.clone(),
                    from_agent: agent.clone(),
                    task_id,
                    kind: msg.kind.clone(),
                    content: content.to_string(),
                    timestamp_us,
                },
            };
            publish(ipc, event).await;
            Ok(json!({ "status": "sent", "message_id": message_id, "to": msg.to_agent }))
        }

        "a2a_inbox" => {
            let unread = args.get("unread").and_then(Value::as_bool).unwrap_or(true);
            let messages = coordinator.get_a2a_inbox(&agent, unread).map_err(err)?;
            let ids: Vec<String> = messages.iter().map(|m| m.message_id.clone()).collect();
            if !ids.is_empty() {
                coordinator.mark_a2a_read(&ids).map_err(err)?;
            }
            Ok(json!(messages))
        }

        "a2a_directory" => {
            if let Some(domain) = opt(args, "resolve_domain") {
                let authority = coordinator
                    .find_best_authority_for_domain(domain)
                    .map_err(err)?;
                Ok(json!({
                    "resolved_domain": domain,
                    "authority": authority
                }))
            } else {
                let agents = coordinator.list_agent_cards().map_err(err)?;
                let authorities = coordinator.list_domain_authorities().map_err(err)?;
                Ok(json!({
                    "agents": agents,
                    "authorities": authorities
                }))
            }
        }

        "a2a_announce" => {
            let host = opt(args, "host").unwrap_or("generic");
            let mut card = AgentCard::new(&agent, host);
            let confidence = args
                .get("confidence")
                .and_then(Value::as_f64)
                .map(|f| f as f32)
                .unwrap_or(1.0);

            let domains_str = opt(args, "domains").unwrap_or("");
            let domains: Vec<String> = domains_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            card.domain_scopes = domains.clone();
            card.confidence_score = confidence;
            coordinator.register_agent_card(&card).map_err(err)?;

            publish(
                ipc,
                IpcEvent::AgentAnnounced {
                    card: card.clone(),
                    timestamp_us: chrono::Utc::now().timestamp_micros(),
                },
            )
            .await;

            let mut registered_authorities = Vec::new();
            for d in &domains {
                let auth = DomainAuthority::new(d, &agent, confidence);
                coordinator.set_domain_authority(&auth).map_err(err)?;
                publish(
                    ipc,
                    IpcEvent::DomainAuthorityUpdated {
                        authority: auth.clone(),
                        timestamp_us: chrono::Utc::now().timestamp_micros(),
                    },
                )
                .await;
                registered_authorities.push(auth);
            }

            Ok(json!({
                "status": "announced",
                "agent_card": card,
                "authorities": registered_authorities
            }))
        }

        "a2a_query_transactive" => {
            let domain = req(args, "domain")?;
            let question = req(args, "question")?;
            let query_id = new_id("query");

            let best_auth = coordinator
                .find_best_authority_for_domain(domain)
                .map_err(err)?;

            let target_agent = best_auth.as_ref().map(|a| a.authoritative_agent.clone());
            let query = TransactiveQuery::new(&query_id, &agent, domain, question);

            // Persist as A2A message in sqlite inbox so recipient receives it
            let msg_id = new_id("msg");
            let a2a_msg =
                match &target_agent {
                    Some(to) => A2aMessage::direct(&msg_id, &agent, to, question)
                        .with_kind("transactive_query"),
                    None => A2aMessage::broadcast(&msg_id, &agent, question)
                        .with_kind("transactive_query"),
                };
            coordinator.save_a2a_message(&a2a_msg).map_err(err)?;

            publish(
                ipc,
                IpcEvent::TransactiveQueryRouted {
                    query: query.clone(),
                    timestamp_us: chrono::Utc::now().timestamp_micros(),
                },
            )
            .await;

            Ok(json!({
                "query_id": query_id,
                "routed_to": target_agent,
                "domain": domain,
                "matched_authority": best_auth,
                "status": if best_auth.is_some() { "routed" } else { "broadcast_unresolved" }
            }))
        }

        "a2a_delegate" => {
            let objective = req(args, "objective")?;
            let domain = opt(args, "domain");
            let to = opt(args, "to");
            let task_id = opt(args, "task_id")
                .map(|s| s.to_string())
                .unwrap_or_else(|| new_id("task"));
            let delegation_id = new_id("del");
            let timeout = args.get("timeout").and_then(Value::as_u64).unwrap_or(60);

            let delegatee_agent = if let Some(target) = to {
                Some(target.to_string())
            } else if let Some(d) = domain {
                coordinator
                    .find_best_authority_for_domain(d)
                    .map_err(err)?
                    .map(|auth| auth.authoritative_agent)
            } else {
                None
            };

            let mut contract = DelegationContract::new(&delegation_id, &agent, &task_id, objective)
                .with_timeout(timeout);

            if let Some(target) = &delegatee_agent {
                contract = contract.with_delegatee(target);
            }
            if let Some(d) = domain {
                contract = contract.with_domain_scope(d);
            }
            if let Some(ctx) = opt(args, "context") {
                contract = contract.with_context(ctx);
            }
            if let Some(art) = opt(args, "artifact") {
                let artifacts: Vec<String> = art
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                contract = contract.with_artifacts(artifacts);
            }
            if let Some(cmd) = opt(args, "verify") {
                contract = contract.with_verification(cmd);
            }

            coordinator.save_delegation(&contract).map_err(err)?;

            // Also create blackboard task if needed
            let mut task_record = strata_core::blackboard::TaskRecord::new(&task_id, objective);
            if let Some(d) = domain {
                task_record = task_record.with_required_resources(vec![d.to_string()]);
            }
            if let Some(ref target) = delegatee_agent {
                task_record.assigned_agent = Some(target.clone());
            }
            let _ = coordinator.create_task(&task_record);

            publish(
                ipc,
                IpcEvent::TaskDelegated {
                    contract: contract.clone(),
                    timestamp_us: chrono::Utc::now().timestamp_micros(),
                },
            )
            .await;

            Ok(json!({
                "status": "delegated",
                "contract": contract
            }))
        }

        "a2a_accept" => {
            let delegation_id = req(args, "delegation_id")?;
            let updated = coordinator
                .accept_delegation(delegation_id, &agent)
                .map_err(err)?;
            if !updated {
                return Err(format!(
                    "Could not accept delegation '{delegation_id}': not found or already accepted"
                ));
            }
            publish(
                ipc,
                IpcEvent::DelegationAccepted {
                    delegation_id: delegation_id.to_string(),
                    delegatee_agent: agent.clone(),
                    timestamp_us: chrono::Utc::now().timestamp_micros(),
                },
            )
            .await;
            Ok(json!({
                "status": "accepted",
                "delegation_id": delegation_id,
                "delegatee": agent
            }))
        }

        "a2a_delegations" => {
            let delegator = opt(args, "delegator");
            let delegatee = opt(args, "delegatee");
            let status = opt(args, "status").and_then(|s| s.parse::<DelegationStatus>().ok());
            let list = coordinator
                .list_delegations(delegator, delegatee, status)
                .map_err(err)?;
            Ok(json!(list))
        }

        "a2a_complete_delegation" => {
            let delegation_id = req(args, "delegation_id")?;
            let summary = req(args, "summary")?;
            let success = args.get("success").and_then(Value::as_bool).unwrap_or(true);
            let artifact_path = opt(args, "artifact");

            let contract_opt = coordinator.get_delegation(delegation_id).map_err(err)?;
            let contract = match contract_opt {
                Some(c) => c,
                None => return Err(format!("Delegation ID '{delegation_id}' not found")),
            };

            let status = if success {
                DelegationStatus::Completed
            } else {
                DelegationStatus::Rejected
            };
            coordinator
                .update_delegation_status(delegation_id, status, Some(&agent), Some(summary), None)
                .map_err(err)?;

            let mut artifacts = Vec::new();
            if let Some(p) = artifact_path {
                for path in p.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    let art_id = new_id("art");
                    let art = TaskArtifact::new(art_id, &contract.task_id, &agent, "file", path);
                    let _ = coordinator.save_task_artifact(&art);
                    artifacts.push(path.to_string());
                }
            }

            if success {
                let _ = coordinator.complete_task(&contract.task_id, &agent, Some(summary), None);
            } else {
                let _ = coordinator.fail_task(&contract.task_id, &agent, summary);
            }

            publish(
                ipc,
                IpcEvent::DelegationCompleted {
                    delegation_id: delegation_id.to_string(),
                    delegatee_agent: agent.clone(),
                    success,
                    summary: summary.to_string(),
                    artifacts: artifacts.clone(),
                    timestamp_us: chrono::Utc::now().timestamp_micros(),
                },
            )
            .await;

            Ok(json!({
                "status": status.as_str(),
                "delegation_id": delegation_id,
                "summary": summary,
                "artifacts": artifacts
            }))
        }

        other => Err(format!("Unknown tool: '{other}'")),
    }
}
