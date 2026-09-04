# Strata Cognitive Agent Runtime & Research Documentation

Research repository, architectural specifications, and engineering blueprints for **Strata**: an embedded, local-first persistent cognitive memory runtime implemented in Rust.

---

## Documentation Structure

### Core Architectural Foundations
- [01-overview.md](01-overview.md) — Problem statement, operational scope, core hypotheses, and evaluation criteria.
- [02-cognitive-architecture.md](02-cognitive-architecture.md) — Cognitive architecture, functional components, and the orchestrator control loop.
- [03-memory-systems.md](03-memory-systems.md) — Multi-tier memory architecture: episodic traces, semantic facts, and procedural skills.
- [04-continual-learning.md](04-continual-learning.md) — Continual learning, out-of-band consolidation, experience replay, and safe belief revision.
- [05-world-model.md](05-world-model.md) — World modeling: causal networks, probabilistic beliefs, and predictive state transitions.
- [06-reasoning-and-metacognition.md](06-reasoning-and-metacognition.md) — Reasoning topologies, automated verifiers, heuristic search, and uncertainty calibration.
- [07-long-horizon-planning.md](07-long-horizon-planning.md) — Long-horizon planning, hierarchical subgoals, Merkle checkpoints, and crash recovery.
- [08-embodiment-and-simulation.md](08-embodiment-and-simulation.md) — Embodiment interfaces: simulation environments, sensory perception, and physical action spaces.
- [09-rust-runtime.md](09-rust-runtime.md) — Native Rust runtime architecture, memory safety guarantees, concurrency model, and trait interfaces.
- [10-research-roadmap.md](10-research-roadmap.md) — Research and development roadmap, execution phases, and empirical validation metrics.
- [11-references.md](11-references.md) — Annotated academic bibliography, foundational literature, and theoretical references.

### Ecosystem, Blueprints & Competitive Intelligence
- [12-experience-cloud-integrations.md](12-experience-cloud-integrations.md) — Experience Cloud product architecture, client adapters, and canonical telemetry event schemas.
- [13-three-frontiers-mcp-a2a.md](13-three-frontiers-mcp-a2a.md) — The three frontiers of agent memory: external hippocampal MCP, transactive A2A memory, and metacognitive governance.
- [14-custom-domain-cloud-security.md](14-custom-domain-cloud-security.md) — Infrastructure guide: custom domain binding, automated Let's Encrypt TLS, and hardened HTTP security headers on Railway.
- [15-blueprint-cortex-mcp-full-spec.md](15-blueprint-cortex-mcp-full-spec.md) — Strata Cortex complete architectural specification: Tri-Tier memory model, Tree-Sitter AST & Git Merkle code anchoring, bi-temporal JTMS, and universal MCP tool contracts.
- [16-competitor-benchmark-deep-research.md](16-competitor-benchmark-deep-research.md) — Comprehensive state of the art and competitive benchmark: deep analysis across Layer 1 (Mem0, Supermemory, Zep, Letta, Cognee, LangMem), Layer 2 (Cursor, Claude Code, Windsurf, Copilot), and Layer 3 research.
- [STRATEGY_SUPERMEMORY_COMPARISON.md](STRATEGY_SUPERMEMORY_COMPARISON.md) — Strategic thesis and architectural differentiation analysis comparing Strata against Supermemory.
- [core-flows.mmd](core-flows.mmd) — Editable Mermaid diagram specifying the full cognitive agent control and memory consolidation loop.

---

## Core Design Principle

The Large Language Model functions as an interpretation and generative reasoning engine, **not** the singular repository for memory, planning, learning, or control. These cognitive capabilities are decoupled into persistent, deterministic, strictly typed, and mathematically auditable subsystems.
