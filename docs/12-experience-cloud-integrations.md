# Experience Cloud — Integrations & Product Architecture

## Product Thesis

The product is not an isolated memory API. It is an organizational shared experience layer: connect once and allow authorized agents across a team to learn from outcomes produced by other agents.

> Connect once. Every agent your team uses learns from every other agent.

End users should not need to master MCP, hooks, SDKs, or API keys. The external onboarding experience should be streamlined: select a client, authenticate, and authorize.

```text
Select client → Authorize Experience → Connected
```

## Integration Architecture

The architecture defines two complementary interfaces independent of specific host clients.

| Interface | Direction | Function |
|---|---|---|
| Read side: MCP | agent → Experience | Retrieve memories, skills, anti-patterns, and alerts |
| Write side: Event Ingestion | agent → Experience | Record tasks, actions, outcomes, and failure trajectories |

Every external client operates through an adapter layer. The core runtime does not couple to Claude, Cursor, Codex, ChatGPT, Gemini, or Windsurf.

```text
Clients and agents
  ├─ MCP ────────────────► Experience API: Read path
  └─ hooks / OTEL / SDK ─► Event API: Write path
                                  │
                                  ▼
                         Experience Engine
                         ├─ Memory Engine
                         ├─ Experience Engine
                         └─ Skills Engine
```

## Canonical Event Schema

Client adapters normalize vendor-specific telemetry into a unified canonical schema. Storage, indexing, and continual learning subsystems remain strictly agnostic to the originating client.

| Event | Minimal Required Fields |
|---|---|
| `SessionStarted` | organization, agent, session_id, environment |
| `GoalCreated` | objective, scope, risk_level, token_budget |
| `PlanCreated` | plan_graph, dependencies, schema_version |
| `ToolStarted` / `ToolCompleted` | tool_name, summarized_input, execution_result, duration_ms |
| `FileChanged` | repository, file_path, unified_diff or cryptographic content hash |
| `CommandExecuted` / `TestExecuted` | command_line, exit_code, stdout/stderr artifacts, duration_ms |
| `ErrorObserved` | error_classification, stack_trace/evidence, blast_radius_impact |
| `TaskCompleted` | verification_criteria, outcome_status, qualitative_evaluation |
| `SessionEnded` | terminal_state, Merkle checkpoint, pending_consolidation_flag |

All events must include an immutable `event_id`, high-precision timestamp, tenant/organization identifier, session and agent IDs, causal provenance, data classification level, and retention policy. Secrets, tokens, and sensitive payloads are stripped prior to ingestion.

## Initial MCP Surface

- `search_experience`: Queries relevant episodes, solutions, and procedures along with source attribution and calibrated confidence scores.
- `record_outcome`: Records the verified execution outcome of an agentic task.
- `get_known_failures`: Returns recurring failure anti-patterns, root-cause evidence, and actionable guardrails.
- `get_memory_capabilities`: Advertises active memory tiers, retention rules, and access control boundaries.

The API gateway enforces strict multi-tenant isolation across organization, repository, project, and user boundaries. Unauthenticated or ACL-violating retrievals are denied by default.

## Client Integration Priority Matrix

| Priority | Client | Role & Transport |
|---|---|---|
| P1 | Claude Code | MCP for read queries; native plugin/hooks for execution loop telemetry |
| P1 | Cursor | Stdio/IPC MCP integration with zero-friction authentication |
| P1 | Codex | MCP for context retrieval; event telemetry via officially supported interfaces |
| P2 | ChatGPT | Authorized remote actions/tools once native MCP endpoints are supported |
| P2 | Gemini CLI | CLI extension / MCP transport and client-supported lifecycle hooks |
| P3 | Windsurf & others | Adapters conforming to the canonical event and retrieval contract |

Every client-specific integration capability must be validated against official vendor specifications during implementation. The runtime must never assume undocumented hooks, telemetry channels, or elevated permissions.

## Minimum Viable Product (MVP)

The MVP scope targets Claude Code, Cursor, and Codex: remote/stdio MCP transport, OAuth/API-key authentication, canonical event ingestion, episodic storage, and three core MCP tools. The core falsifiable hypothesis: **a validated problem-solving trajectory recorded from one client demonstrably reduces the occurrence of an equivalent failure in a different client**.

## Key Evaluation Metrics

- **Cross-client failure reduction rate**: Percentage decrease in identical error patterns across distinct agent environments.
- **Retrieval precision and utility**: Relevance and acceptance rate of injected historical experiences into agent prompts.
- **Latency profile**: p95 and p99 latency for context retrieval and out-of-band event ingestion.
- **Privacy filter drop rate**: Volume and ratio of telemetry sanitizations preserving secrets and PII.
- **Provenance & ACL compliance**: 100% auditability of all retrieved memory records with verified tenant boundaries.
