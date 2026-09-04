# Cognitive Architecture

## Subsystems & Architecture Breakdown

The Strata cognitive architecture decouples runtime execution into discrete, typed, and auditable subsystems:

| Subsystem | Primary Responsibility | Persistent State Representation |
|---|---|---|
| **Orchestrator** | Coordinates the closed-loop Observe–Decide–Act cycle | Execution graphs, execution checkpoints, and runtime lifecycles |
| **Memory Engine** | Manages multi-tier retrieval, decay, and consolidation | Episodic WAL, semantic knowledge graph, procedural skill registry |
| **World Model** | Maintains epistemic beliefs, transitions, and causal models | Attributed belief graph with bi-temporal truth maintenance (JTMS) |
| **Planner** | Decomposes high-level objectives into actionable subgoals | Directed Acyclic Graph (DAG) of verified subgoals |
| **Reasoner / Verifier** | Generates hypotheses and formally checks invariant constraints | Provenance traces, validation proofs, and invariant evaluations |
| **Tool Gateway** | Mediates tool execution, capability gating, and sandboxing | Access control policies, capability tokens, and audit logs |
| **Learner** | Trajectory distillation, anti-pattern extraction, and policy refinement | Offline trajectory datasets (DPO/KTO/SFT), versioned policy artifacts |

## Perception–Cognition–Action Control Loop

The core execution cycle operates as a deterministic state machine:

1. **Observation Normalization**: Ingest multimodal telemetry, environmental state, and active objectives into strongly typed Rust domain primitives.
2. **Multi-Modal Retrieval**: Query working, episodic, and semantic memory using hybrid indexing (dense ONNX vector embeddings, sparse BM25 lexicons, AST anchors, and entity graph traversal).
3. **Epistemic State Estimation**: Update the belief graph via bi-temporal truth maintenance (JTMS) and compute localized epistemic uncertainty bounds.
4. **Hierarchical Planning**: Formulate or adapt a plan DAG decomposing the active objective into discrete subgoals with formal pre- and post-conditions.
5. **Pre-Flight Verification**: Rigorously validate tool invocations against security policies, precondition invariants, blast-radius risk thresholds, and execution budgets.
6. **Surgical Execution**: Dispatch operations prioritizing reversible actions and transactional idempotency; capture both explicit outputs and out-of-band failure signals.
7. **Consolidation & Feedback**: Log execution trajectories, compute reward/progress differentials, update activation histories (Ebbinghaus/ACT-R decay), and queue candidates for background consolidation.

## Architectural Invariants

- **Epistemic Provenance**: Every atomic fact must maintain strict provenance metadata: source origin, cryptographic hash, temporal validity interval (`valid_from`, `valid_until`), confidence score $c \in [0.0, 1.0]$, and justification links.
- **Auditable Side Effects**: Every external side effect must possess an immutable transaction ID, verified capability token, declarative preconditions, and deterministic execution logs.
- **Deterministic Checkpointing**: Plan DAGs and runtime states are monotonically versioned, enabling deterministic replay and crash recovery across context resets.
- **Bounded Working Context**: Language models receive minimal, tightly budgeted projections of state (<500 tokens) rather than unconstrained memory dumps, preventing attention degradation.
