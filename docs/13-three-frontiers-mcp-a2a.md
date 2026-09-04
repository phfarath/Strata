# Three Frontiers: Shared Memory over MCP and A2A

## Context & Executive Scope

This specification synthesizes foundational research on shared memory architectures for autonomous agent swarms, delineating immediately deployable infrastructure from long-term research frontiers. The Model Context Protocol (MCP) and Agent-to-Agent (A2A) protocols provide transport interoperability; they do not inherently solve knowledge consolidation, confidence calibration, access control boundaries, or memory hygiene.

## Frontier 1 — MCP as an External Hippocampus

### Architectural Proposal

An external memory MCP server ingests high-frequency episodic traces and consolidates them via background, asynchronous pipelines into durable semantic and procedural representations. The conceptual architecture is grounded in Complementary Learning Systems (CLS) theory: rapid episodic acquisition paired with slow, interleaved integration into generalized knowledge structures.

### Core Capabilities

- `write_episode`: Persists event traces, execution state snapshots, outcomes, and causal provenance.
- `consolidate`: Synthesizes candidate facts, invariant rules, and procedural skills from raw episodes.
- `replay`: Re-evaluates prioritized episodic trajectories based on task salience and reward signals.
- `decay`: Attenuates salience or evicts obsolete entries according to cognitive forgetting curves.
- `check_conflict`: Identifies contradictory assertions and belief divergences prior to knowledge promotion.

### Data Schemas

| Entity | Critical Fields |
|---|---|
| Episode | `agent_id`, `session_id`, `event_trace`, `timestamp`, `salience_score`, `state_snapshot` |
| Semantic Fact | `content`, `source_episodes`, `confidence_score`, `stability`, `validity_interval` |
| Procedure | `objective`, `tool_call_sequence`, `pre_conditions`, `post_conditions`, `performance_metrics` |

### MVP & Risk Surface

The initial implementation focuses strictly on `write_episode` and `consolidate` atop a temporal-graph/relational backend. Benchmark multi-session task trajectories with and without out-of-band consolidation. The primary architectural risk is over-engineering a costly biological metaphor that yields negligible empirical improvements; cognitive analogies serve as design heuristics, not strict neuroscience emulation requirements.

## Frontier 2 — Transactive A2A Memory

### Architectural Proposal

Specialized agent nodes advertise memory capabilities and handle distributed read/write operations for peer agents. The system maintains a transactive memory directory: beyond indexing content, it tracks which agent, microservice, or domain expert maintains authority over specific knowledge partitions.

### Experimental Agent Card Extension

```json
{
  "memory_capabilities": ["episodic", "semantic", "procedural"],
  "memory_persistence": "longterm",
  "memory_tenant_scopes": ["per_user", "per_org"],
  "memory_privacy_mode": "isolated"
}
```

> [!NOTE]
> These metadata fields represent an experimental convention for research prototyping rather than a finalized A2A standard.

### Data Contracts

The `MemoryRecord` entity encapsulates unique identifier, owner, subject entity, category, payload, ACL rules, and causal provenance. The `TransactiveIndex` maps topic spaces to authoritative specialist agents alongside calibrated confidence scores. Every inter-agent transaction must strictly enforce tenant boundaries, authorization scopes, read/write permissions, and explicit delegation grants.

### MVP & Risk Surface

Deploy a dedicated A2A memory agent exposing `store_memory` and `query_memory`, integrated with two consumer agents in a sandboxed evaluation domain. Primary failure modes include cross-tenant ACL leakage, protocol lock-in, and tracing overhead in distributed consensus. Shared memory abstractions must never expose private agent-internal scratchpads or unvalidated working state.

## Frontier 3 — Metacognition for Memory Governance

### Architectural Proposal

A decoupled metacognitive layer continuously estimates information utility, empirical confidence, obsolescence trajectories, and systemic contradictions across the memory store. It actively governs retention, promotion, verification scheduling, down-weighting, and eviction.

### Core Capabilities

- `evaluate_memory`: Computes contextual relevance, calibrated confidence, and obsolescence indices.
- `schedule_retrieval_test`: Schedules active verification probes for mission-critical memories and invariant rules.
- `resolve_conflict`: Preserves competing hypotheses while recording formal JTMS belief revision resolutions.
- `estimate_obsolescence`: Derives decay velocity from age, source reliability, domain shift indicators, and downstream error rates.

### Minimal Metadata Schema

Records maintain `memory_id`, `importance_weight`, `confidence_score`, `last_accessed_at`, `last_verified_at`, `error_count`, `provenance_sources`, and `outcome_history`. Confidence is treated as a continuously calibrated probabilistic metric rather than a static binary attribute.

### MVP & Risk Surface

Enrich existing schemas with confidence and `last_success` metadata; implement `evaluate_memory`; down-rank or trigger re-verification for memories correlated with repeated execution regressions. The primary operational risk is an uncalibrated decay heuristic that prematurely evicts vital context or incurs prohibitive LLM verification token costs.

## Cross-Cutting Protocol Foundations

| Architectural Domain | Proposed Convention |
|---|---|
| MCP Discovery | `memory/capabilities` resource schema and categorical memory-type tags |
| Access Control (ACL) | Resource owner, reader/writer grants, tenant isolation, and project scope boundaries |
| Provenance | Originating tool, author agent, execution timestamp, confidence, verification history |
| Reconsolidation | Monotonic versioning with immutable links to historical antecedents |
| Observability | Append-only event logs, distributed task correlation IDs, and access audit trails |

## Recommended Phased Roadmap

1. **Frontier 1 (Core Runtime)**: Implement episodic acquisition, causal provenance tracking, and offline consolidation.
2. **Frontier 3 (Metacognitive Control)**: Integrate quality calibration, contradiction resolution, and Ebbinghaus/ACT-R decay.
3. **Frontier 2 (Transactive A2A)**: Expose inter-agent federated memory only after tenant isolation, ACL boundaries, and audit logging are empirically validated.

This staged progression enforces safety: memory structures should never be shared across autonomous agents before internal validity, hygiene, and governance are formally proven.
