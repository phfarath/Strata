# Long-Horizon Planning and Autonomy

## Goal and Task Representation (DAG Schedulers)

Persistent autonomy over extended horizons requires robust, non-linear goal representations. Strata structures complex agent directives as Directed Acyclic Graphs (DAGs) of interdependent subgoals and actions:
- **Node Specification**: Each DAG node represents a discrete, verifiable subgoal defined with strict entry preconditions, termination criteria, explicit resource budgets (token, time, monetary cost), risk classification, and required evidence.
- **Topological Dependencies**: Edges encode causal and logical prerequisites. Subgoals can execute concurrently across worker threads or subagents when topological constraints permit.
- **Evidentiary Anchors**: A node transitions to `Completed` only upon presenting verifiable evidence (e.g., successful AST compilation, test suite passing, or verified sensor observation) recorded in the persistent event log.
- **Resumption Policies**: Nodes declare deterministic resumption strategies (idempotent retry, fallback branch, or re-anchored parent goal) in the event of partial execution failure.

## Resilient Execution and Fault Tolerance

Long-running autonomous agents frequently encounter environment shifts, transient tool failures, and context degradation. Strata implements enterprise-grade resilience mechanisms:
- **Atomic State Checkpointing**: Persisting a full snapshot of the execution context, variable bindings, and memory state after every material state transition using local-first SQLite WAL and Git Merkle tree hashing.
- **Tool Idempotency**: Requiring external side-effecting tools to accept idempotency keys and state tokens, enabling safe replays without duplicate side effects.
- **Loop and Deadlock Detection**: Tracking action signatures and state hash histories to detect livelocks, repetitive reasoning loops, and circular tool invocations.
- **Semantic Drift Alarms**: Continuously measuring cosine similarity and goal relevance between active tool calls and the root DAG directive to abort scope creep.
- **Localized Graph Replanning**: When a node fails, the planner isolates the affected subgraph and executes localized replanning rather than discarding the entire mission. Upstream completed justifications recorded in the JTMS remain intact.
- **Hard Resource Envelopes**: Enforcing wall-clock execution deadlines, maximum tool invocation counts, retry backoff with exponential jitter, and capability-scoped security sandboxes.

## Autonomy Governance and Safety Tiers

Strata enforces a graduated five-tier operational hierarchy to balance autonomous speed with guaranteed safety:
1. **Tier 1 — Passive Observation**: Read-only operations, AST querying, repository indexing, log inspection. No side effects; executed autonomously without friction.
2. **Tier 2 — Sandboxed Simulation**: Virtual execution inside ephemeral containers, mock environments, or dry-run mutations. Permitted autonomously under standard resource monitoring.
3. **Tier 3 — Reversible Mutation**: Local workspace modifications anchored to Git worktrees or temporary staging branches with automatic rollback guarantees.
4. **Tier 4 — Bounded External Interaction**: External network communications, rate-limited API queries, or non-destructive external updates guarded by idempotency tokens.
5. **Tier 5 — Irreversible Execution**: Dropping database tables, pushing code to production/main branches, provisioning infrastructure, or sending external transmissions. Strictly requires human-in-the-loop authorization, cryptographic signature, and immediate fail-safe abort hooks.

## Quantitative Autonomy Metrics

Strata evaluates long-horizon agent stability using standard quantitative benchmarks:
- **Task Completion Rate (TCR)**: Percentage of end-to-end multi-step tasks resolved without unhandled failures.
- **Steps per Successful Resolution (SSR)**: Operational efficiency measured as the ratio of necessary actions to total actions dispatched.
- **Mean Time to Recovery (MTTR)**: Latency and step count required for the agent to isolate a tool failure, replan, and resume forward progress.
- **Budget Compliance**: Deviation between estimated token/cost expenditures and actual consumption.
- **Human Intervention Frequency (HIF)**: Number of unscheduled human interventions required per thousand autonomous actions.
- **Goal Drift Index**: Vector distance between final trajectory outputs and original root specifications.
