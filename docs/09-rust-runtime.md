# The Strata Rust Runtime

## Architectural Role of Rust

Strata is architected as a high-performance, persistent cognitive runtime implemented in Rust. Foundation LLMs provide probabilistic natural language interpretation and heuristic candidate generation, but they are treated as untrusted, stateless inference services. 

The trusted computing base (TCB) of Strata is written in Rust to guarantee:
- **Memory Safety & Determinism**: Zero-cost abstractions, fearless concurrency, and lifetime guarantees ensure thread-safe state management without garbage collection pauses.
- **Strict Typestate & Capability Boundaries**: Security privileges, tool access, and plan transitions are verified at compile time and enforced via typed capability tokens.
- **Low Operational Footprint**: Minimal latency and memory overhead enable 100% local-first deployment on developer workstations while supporting cloud parity multi-tenant deployments.
- **Immutable Auditability**: Event sourcing and structured memory stores ensure verifiable execution records and bit-for-bit replayability.

## Modular Crate Architecture

The Strata workspace is organized into atomic, strictly decoupled crates:
- `strata-core`: Canonical domain models, state structs, strongly typed event envelopes, capability security primitives, and system invariants.
- `strata-memory`: Tri-tier memory hierarchy (working, episodic, semantic, procedural), local vector indices (FastEmbed ONNX, SQLite-vec), and Justification-Based Truth Maintenance System (JTMS) contradiction engines.
- `strata-world`: Dynamic causal belief graphs, environmental state tracking, and Tree-Sitter AST code anchoring integrated with Git Merkle trees.
- `strata-planning`: Directed Acyclic Graph (DAG) task schedulers, topological task dependency resolvers, cost/risk evaluators, and localized replanning engines.
- `strata-reasoning`: LLM provider client adapters, search algorithms (Tree-of-Thoughts, beam search), calibration estimators, and out-of-band deterministic verifiers.
- `strata-tools`: JSON Schema definitions, capability validation barriers, sandboxed execution runtimes (Wasm/process isolation), and telemetry hooks.
- `strata-evals`: Automated scenario testbeds, deterministic regression harnesses, statistical calibration evaluations, and ablation benchmarks.
- `strata-api`: Stdio Model Context Protocol (MCP) server, CLI binaries (`strata-cli`), Axum-based HTTP/WebSocket services, and OpenTelemetry-compliant tracing.

## Core Trait Interfaces

Strata enforces radical modularity through composable async traits. Concrete implementations can be seamlessly substituted between in-memory mock suites, embedded local engines, and distributed cloud backends:

```rust
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn record(&self, entry: &MemoryEntry) -> Result<MemoryId, MemoryError>;
    async fn retrieve(&self, query: &MemoryQuery) -> Result<Vec<MemoryRecord>, MemoryError>;
    async fn consolidate(&self, policy: &ConsolidationPolicy) -> Result<ConsolidationReport, MemoryError>;
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> Value;
    async fn invoke(&self, ctx: &SecurityContext, params: Value) -> Result<ToolResult, ToolError>;
}

#[async_trait]
pub trait Verifier: Send + Sync {
    async fn verify(&self, claim: &Proposition, evidence: &[Evidence]) -> Result<VerificationOutcome, VerificationError>;
}

#[async_trait]
pub trait Planner: Send + Sync {
    async fn next_step(&self, state: &AgentState, dag: &GoalDag) -> Result<PlanAction, PlanningError>;
    async fn replan_subgraph(&self, failed_node: NodeId, dag: &mut GoalDag) -> Result<(), PlanningError>;
}
```

## Event Sourcing and State Persistence

Strata adheres to an append-only event sourcing architecture:
- **Canonical Event Envelope**: Every discrete transition is recorded as an immutable event (`ObservationReceived`, `PlanCreated`, `ActionAuthorized`, `ToolInvoked`, `OutcomeObserved`, `MemoryConsolidated`, `ContradictionResolved`).
- **Deterministic Materialization**: Working memory state, current belief graphs, and DAG progression are materialized projections derived from the event stream. Any past state can be deterministically reconstructed for regression analysis, debugging, and post-mortem review.
- **Local-First Storage Engine**: Events are durably persisted using SQLite with Write-Ahead Logging (WAL) and synchronous disk commits, paired with local fast vector search.

## Technology Stack and Systems Composition

- **Async Runtime & Concurrency**: `tokio` (multi-threaded work-stealing scheduler), `futures`
- **Serialization & Schema**: `serde`, `serde_json`, `schemars`
- **Persistence & Vector Storage**: `sqlx` (SQLite / PostgreSQL), `rusqlite`, embedded vector similarity search
- **AST Parsing & Code Anchoring**: `tree-sitter`, `tree-sitter-rust`, `git2`
- **Inference & Embeddings**: `fastembed` (local ONNX runtime for sub-millisecond local embeddings), asynchronous HTTP clients (`reqwest`) for foundation LLM inference
- **Networking & Server**: `axum`, `tower`, `tower-http`, Model Context Protocol (MCP) SDK
- **Observability & Diagnostics**: `tracing`, `tracing-subscriber`, `metrics`
