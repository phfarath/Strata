# Research Roadmap

## Phase 0 — Foundations and Formal Specifications

Establish rigorous mathematical and architectural foundations, domain boundaries, event schemas, and evaluation protocols:
- Define formal type definitions for agent state, goal DAGs, memory entries, and capability tokens.
- Establish the canonical taxonomy of system events and state transitions.
- Formulate risk categorization policies, safety tiers, and confidence thresholds.
- Construct deterministic test suites and gold-standard scenario benchmarks.
- **Deliverable**: State model specification, typed schema contracts, and deterministic test harnesses.

## Phase 1 — Minimal Deterministic Runtime

Implement the foundational single-agent execution engine in Rust:
- Build the core event loop supporting the observe–orient–decide–act cycle.
- Implement the append-only event log with SQLite WAL persistence and replay capabilities.
- Develop the capability-scoped tool execution gateway with schema validation and sandboxing.
- Integrate atomic state checkpointing and rollback mechanisms.
- **Deliverable**: A functional local-first Rust runtime executing short-horizon digital tasks with 100% auditability and deterministic reproducibility.

## Phase 2 — Multi-Tier Cognitive Memory Engine

Incorporate human-inspired, multi-tier memory subsystems with active consolidation:
- Implement working memory, episodic event logging, and procedural skill registries.
- Build hybrid retrieval combining BM25 lexical search, local ONNX vector embeddings, and graph traversal.
- Deploy the Justification-Based Truth Maintenance System (JTMS) to resolve factual contradictions and invalidate retracted beliefs non-monotonically.
- Implement background memory consolidation and deliberate forgetting mechanisms.
- Benchmark retention, precision, and latency against naive in-context stuffing and raw vector RAG baselines.
- **Deliverable**: Standardized memory benchmark suite demonstrating superior long-term factual recall and zero context-window pollution.

## Phase 3 — Long-Horizon Planning and Metacognition

Enable multi-hour autonomous execution over complex problem graphs:
- Implement topological Goal DAG schedulers with concurrent subgoal dispatch.
- Deploy out-of-band deterministic verifiers (compilers, AST validators, test suites) to gate state transitions.
- Implement statistical confidence calibration (Brier score, ECE) and semantic uncertainty quantification.
- Integrate automated localized replanning and livelock detection.
- **Deliverable**: Statistically validated reduction in cascading multi-step failures on long-horizon software engineering benchmarks.

## Phase 4 — Dynamic World Modeling and Continual Learning

Transition from static execution to continuous learning from experience:
- Implement dynamic causal belief graphs updated via prediction-error signals.
- Mine high-quality Direct Preference Optimization (DPO), Kahneman-Tversky Optimization (KTO), and Supervised Fine-Tuning (SFT) datasets autonomously from agent execution trajectories.
- Extract recurring successful multi-step action patterns into reusable procedural skills.
- Deploy experience replay buffers to prevent catastrophic forgetting.
- **Deliverable**: Empirically measured cross-task skill transfer with zero regression on previously mastered domains.

## Phase 5 — Simulation and Embodied Physical Control

Bridge cognitive planning with simulated and physical robotics environments:
- Integrate high-fidelity physics simulation environments (e.g., MuJoCo, Isaac Sim).
- Decouple high-level deliberative cognitive reasoning from high-frequency reactive controllers.
- Deploy real-time safety supervisors, dynamic torque limiters, and physical keep-out geofencing.
- Benchmark sim-to-real transfer and disturbance rejection.
- **Deliverable**: Verified safety-bounded deliberative agent successfully completing manipulation and navigation tasks in simulated environments.

## Experimental Discipline and Empirical Rigor

Every research phase must adhere strictly to formal empirical standards:
- **Mandatory Baselines**: Every new mechanism (e.g., JTMS memory, DAG planner) must be compared against established standard baselines (e.g., vanilla context window, naive RAG, zero-shot ReAct).
- **Ablation Studies**: Quantify the individual contribution of each subsystem by systematic component removal.
- **Reproducible Traces**: All experimental runs must generate bit-for-bit reproducible event logs, including random seed tracking, prompt versions, and tool outputs.
- **Blind Evaluation**: Benchmark outputs must be evaluated by independent deterministic scripts or blind human reviewers wherever subjective quality assessment is required.
- **Explicit Stop Criteria**: Predefine quantitative success thresholds before experimental execution; anecdotal demonstrations do not constitute valid evidence of general autonomous capability.
