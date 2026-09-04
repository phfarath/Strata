# Strata vs. Supermemory: Strategic Thesis & Architectural Differentiation

> **Supermemory** = *"Give your AI perfect recall."*  
> **Strata** = *"Give agents a persistent cognitive state & continual learning."*

---

## 1. Executive Overview & Strategic Positioning

Supermemory positions itself primarily as **context and recall infrastructure** for general-purpose AI applications: automated extraction of user preferences, profile construction, temporal graphs, conversational RAG, and third-party SaaS connectors (Google Drive, Gmail, Notion, GitHub) exposed via MCP and browser extensions.

The strategic thesis of **Strata** is fundamentally distinct: rather than building "Supermemory rewritten in Rust", Strata provides an **embedded persistent cognition and continual learning runtime** where autonomous software engineering agents accumulate, refine, and share empirical problem-solving experience across execution cycles.

```text
                    STRATA COGNITIVE RUNTIME

 Events / Conversations / Actions / Tools / File Changes
                         │
                         ▼
                  ┌──────────────┐
                  │ Observation  │
                  │   Engine     │
                  └──────┬───────┘
                         │
                         ▼
              ┌─────────────────────┐
              │ Memory Classifier   │
              └─────────┬───────────┘
                        │
       ┌────────────────┼──────────────────┐
       ▼                ▼                  ▼
   Episodic         Semantic          Procedural
   Memory           Memory            Memory
   (Events/Logs)    (JTMS Facts)      (Learned Skills)
       │                │                  │
       └────────────────┼──────────────────┘
                        ▼
              ┌──────────────────┐
              │ Consolidation    │
              │ + Contradiction  │
              │ + Forgetting     │
              └────────┬─────────┘
                       ▼
            Truth Maintenance Graph (JTMS)
                       │
                       ▼
            Context Selection Engine
                       │
             token budget / intent / causality
                       │
                       ▼
                CONNECTED AGENTS
      (Cursor, Claude Code, Codex, Gemini)
```

---

## 2. Core Pillars of Differentiation

### 1. Procedural Memory & Continual Agent Learning
A cognitive agent should not merely recall user preferences (e.g., *"the developer prefers Rust"*). It must preserve actionable, verified problem-solving procedures and operational strategies:
> *"During the previous Railway deployment of this microservice, execution failed due to an unassigned dynamic port. The verified mitigation sequence is A → B → C."*

```yaml
procedure:
  task: deploy_backend_railway
  learned_strategy:
    1. build multi-stage Dockerfile
    2. remove static VOLUME directive
    3. track Cargo.lock in git
    4. bind to dynamic PORT env var
  confidence: 0.96
  learned_from:
    - execution_812
    - execution_940
  failure_patterns_mitigated:
    - docker_volume_unsupported
```

### 2. Causal Provenance & Auditable Justifications ("Why Believe X?")
Every belief in Strata maintains an auditable dependency and evidentiary justification graph:

```text
Memory Record:
├── id
├── content
├── type (Episodic | Semantic | Procedural | NegativePattern)
├── source (User | Agent | Tool | Linter)
├── timestamp
├── confidence (0.0 to 1.0)
├── evidence[] (commits, files, execution logs)
├── derived_from[]
├── contradictions[]
├── supersedes[] (JTMS belief revision)
└── usefulness_score
```

When an agent makes an architectural or implementation decision, it can explicitly justify its premise:
> *"I accept proposition X (calibrated confidence: 0.92) derived from conversation thread #182, commit `87ac31`, and execution trace #922."*

### 3. Memory ≠ Context (Context Selection Engine)
A persistent knowledge store may contain 100,000 memories, yet an LLM's prompt window can only ingest a disciplined fraction without triggering reasoning degradation (*context rot*).

Strata's Context Selection Engine filters the optimal ~8 memories dynamically based on:
- Task & Causal Relevance
- Recency & ACT-R / Ebbinghaus Cognitive Decay Functions
- Calibrated Confidence & Contradiction Resolution (JTMS state: IN)
- Strict Token Budget Enforcement (~300–500 tokens per digest)

### 4. Native Rust Memory Runtime (Ultra-Portable & Minimal Overhead)
Rust is not merely an implementation detail; it empowers a **self-contained cognitive runtime**:
- In-process local SQLite WAL delivering sub-millisecond retrieval (< 5ms).
- Extremely lightweight daemon footprint (< 10MB RAM).
- Single, self-contained cross-platform static binary (`strata`).
- Universal stdio/IPC MCP transport enabling instant agent integration without external node/python runtime overhead.

---

## 3. Competitive Capability Matrix

| Architectural Capability | Supermemory | Strata (Strategic Position) |
|---|---|---|
| RAG & Vector Embeddings | ✅ | ✅ |
| Semantic Memory | ✅ | ✅ |
| Episodic Memory | ✅ | ✅ |
| Temporal Knowledge Graph | ✅ | ✅ (with Bi-Temporal JTMS Belief Revision) |
| Mathematical Cognitive Decay | ✅ | ✅ (Deterministic ACT-R + Ebbinghaus) |
| User Profile Scoping | ✅ | ✅ |
| External SaaS Connectors | ✅ (Google Drive, Notion, etc.) | Initial focus on repositories, codebases & developer toolchains |
| Universal MCP Transport | ✅ | ✅ (Multi-version stdio/IPC protocol) |
| Shared Multi-Agent Memory | ✅ | ✅ (Local SQLite WAL + Optional Cloud CDC Sync) |
| **Procedural Learning (Skills)** | Secondary / Roadmap | **Core Architectural Pillar** |
| **Memory Provenance & Causal Justification** | Limited | **Core Architectural Pillar** |
| **Silent Anti-Pattern Auto-Capture** | Unaddressed | **Core Architectural Pillar** |
| **Autonomous Continual Agent Learning** | Unaddressed | **Core Product Focus** |
| **Embedded Rust Native Runtime** | Node.js / Python | **Native Rust Core (< 10MB RAM)** |

---

## 4. The Strategic Wedge: Autonomous Coding Agents

Rather than attempting to support dozens of generic SaaS integrations at inception, Strata focuses with precision on **Autonomous Coding Agents and Developer Workspaces**:

```text
                  STRATA RUNTIME
                        │
       ┌────────────────┼────────────────┐
       ▼                ▼                ▼
  Claude Code         Cursor           Codex / Gemini
       │                │                │
       └────────────────┼────────────────┘
                        ▼
             Unified Shared Cognitive Memory
```

With a single `strata init` or `strata login`, every agent deployed within the codebase instantly inherits the full institutional corpus: past engineering decisions, architectural heuristics, validated execution procedures, and compiler failure mitigations.

---

## 5. Core Engine Primitives

1. `remember(event)`: Ingests and classifies execution events, architectural decisions, and agent interactions.
2. `recall(context, budget)`: Selects optimal task context while strictly enforcing allocated token budgets.
3. `learn(outcome)`: Distills verified execution successes and failures into procedural skills and negative anti-patterns.
4. `forget(threshold)`: Prunes obsolete or low-utility memory chunks via mathematical cognitive decay curves.
5. `explain(memory_id)`: Produces the complete evidentiary chain and JTMS dependency graph supporting a stored belief.
