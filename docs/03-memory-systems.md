# Memory Systems

## Memory Taxonomies

Strata structures cognitive memory across four distinct functional tiers:

| Tier | Substrate & Content | Write / Update Semantics | Retrieval Mechanics |
|---|---|---|---|
| **Working Memory** | Active task context, execution scratchpad, transient state | Mutated atomically per control cycle | Direct reference to active typestate |
| **Episodic Memory** | Action trajectories, environmental observations, outcome logs | Appended to immutable write-ahead log (WAL) post-execution | Multi-modal similarity (dense embeddings + temporal recency + BM25) |
| **Semantic Memory** | Verified factual propositions, ontological entities, relations | Ingested post-verification via Bi-Temporal JTMS | Entity resolution, subgraph queries, relational traversals |
| **Procedural Memory** | Operational recipes, tool policies, automated heuristics | Consolidated after verified repetitive success | Intent pattern matching + precondition satisfiability |

## Schema & Record Model

Every discrete memory unit conforms to a strongly typed record schema:

- **Identifier**: Globally unique UUIDv7 / Content-Addressable Hash (SHA-256).
- **Structured Payload**: Strongly typed domain data or normalized markdown representation.
- **Dense Vector Embedding**: Optional fixed-dimension vector embedding (FastEmbed ONNX) for semantic similarity.
- **Entity Mentions**: Extracted symbolic entities, AST anchors (Tree-Sitter), and Git Merkle tree references.
- **Epistemic Provenance**: Source origin, justification links, creation timestamp ($t_{\text{created}}$), confidence score $c \in [0.0, 1.0]$, and intrinsic importance metric $I$.
- **Bi-Temporal Bounds**: Transaction time and valid time intervals (`valid_from`, `valid_until`, `replaced_by`).
- **Activation Dynamics**: Access frequency count, timestamp of last recall, and base-level activation decayed via deterministic ACT-R / Ebbinghaus algorithms.
- **Evidential Graph Links**: Directed edges denoting justification, premise derivation, generalization, or contradiction.

## Hybrid Multi-Stage Retrieval Pipeline

To balance semantic generalization with exact lexical precision, Strata employs a hybrid scoring function across candidate memory nodes:

$$S(m, q) = w_{\text{sem}} \cdot \text{Sim}_{\text{dense}}(m, q) + w_{\text{bm25}} \cdot \text{Score}_{\text{BM25}}(m, q) + w_{\text{act}} \cdot A_i(t) + w_{\text{ent}} \cdot \mathbb{I}_{\text{entities}}(m, q) - \lambda \cdot (1.0 - c_m)$$

Where:
- $\text{Sim}_{\text{dense}}(m, q)$ denotes the cosine similarity between query and memory embeddings.
- $\text{Score}_{\text{BM25}}(m, q)$ provides lexical precision over exact identifiers, symbols, and error traces.
- $A_i(t) = \ln \left( \sum_{k=1}^{n} (t - t_k)^{-d} \right)$ represents the ACT-R base-level activation over historical access times $t_k$ with decay parameter $d$.
- $\mathbb{I}_{\text{entities}}(m, q)$ measures symbolic and AST scope overlap.
- $\lambda \cdot (1.0 - c_m)$ penalizes speculative or unverified assertions.
- **Diversity Re-Ranking**: Maximal Marginal Relevance (MMR) is applied to diversify returned candidates, preventing context saturation by redundant episodic variations.

## Knowledge Graph & Spreading Activation (HippoRAG / ACT-R)

To capture non-obvious multi-hop associations without incurring LLM token costs or latency, Strata implements a deterministic in-memory Spreading Activation Knowledge Graph:

### Graph Topology & Edge Weights
The graph models five heterogeneous node variants connected by strongly typed edges:
- **Nodes**: `Memory(Uuid)`, `Fact(Uuid)`, `Symbol(String)`, `File(String)`, `Concept(String)`.
- **Edges & Weights**:
  - `Supports` ($w = 0.95$): Direct epistemic justification links.
  - `ReferencesSymbol` ($w = 0.85$): AST-grounded identifier mentions.
  - `MentionsFile` ($w = 0.80$): Workspace file path references.
  - `Calls` ($w = 0.75$): Native deterministic call graph invocations.
  - `CoOccurred` ($w = 0.60$): Co-occurrence in the same execution episode.
  - `SharedTag` ($w = 0.50$): Overlapping semantic categories.

### Activation Propagation Dynamics
Activation spreads across discrete steps ($T = 3$) following the ACT-R diffusion equation:

$$\Delta A_t(v) = \sum_{u \in \text{In}(v)} A_{t-1}(u) \cdot w(u, v) \cdot \lambda$$

$$A_t(v) = \min\left(1.0, \, \mu \cdot A_{t-1}(v) + \Delta A_t(v)\right)$$

Where:
- $\lambda = 0.8$ represents the per-hop transmission decay.
- $\mu = 0.6$ is the self-activation retention factor.
- Energy saturation is capped at $1.0$, and nodes below $\theta = 0.05$ are pruned.

### Tri-Modal Reciprocal Rank Fusion (RRF)
The retrieval ranker fuses three orthogonal signals:
1. **BM25 Lexical Score** (FTS5): Exact matches on symbols, function names, and error strings.
2. **Dense Vector Cosine Similarity** (FastEmbed ONNX): Semantic concept similarity.
3. **Graph Spreading Activation** (HippoRAG): Multi-hop associative discovery linking memories that share no common keywords with the query but are connected through code anchors.

The final rank score is fused via weighted RRF:

$$\text{RRF}(d) = w_{\text{bm25}} \cdot \frac{1}{k + r_{\text{bm25}}(d)} + w_{\text{vec}} \cdot \frac{1}{k + r_{\text{vec}}(d)} + w_{\text{graph}} \cdot \frac{1}{k + r_{\text{graph}}(d)}$$

---

## Two-Tier Memory Federation & Cross-Project Knowledge Transfer

To enable multi-agent and cross-project collaboration without polluting repository-specific boundaries, Strata employs a two-tier storage hierarchy:

### Storage Segregation
1. **Workspace Store** (`<workspace>/.strata/strata.db`):
   - Scope: Local to the repository.
   - Content: Tree-Sitter AST symbol hashes, Git Merkle commit diffs, raw episodic action logs, and ephemeral stigmergic leases.
2. **Developer-Global Store** (`~/.strata/global.db`):
   - Scope: Developer machine-wide.
   - Content: Universal compiler anti-patterns (e.g., recurring borrow-checker or bundler errors), reusable procedural skills, and human-promoted Core Tier axioms.

### Federated Retrieval & Blending
- Queries automatically search both stores in parallel.
- The ranker applies a **Federation Priority Multiplier**:
  - Local candidates receive weight $1.0$.
  - Global candidates receive weight $0.8$ (ensuring repository-specific context takes precedence while surfacing cross-project insights).
- Each returned memory record is tagged with `store_origin: "local" | "global"`.

### Selective Promotion & Transfer
- **Promotion to Global**:
  ```bash
  strata promote --id <UUID> --to-global --reason "Approved cross-project architectural pattern"
  ```
  Or via the MCP tool `memory_promote_global`.
- **Cross-Project Knowledge Transfer**:
  ```bash
  strata transfer --from /path/to/source-repo --tier core --limit 50
  ```
  Supports filtering by tier, memory type, search query, and selective exclusion (`--no-failures`, `--no-skills`).

---

## Consolidation and Principled Forgetting

- **Immutability of Episodic Reality**: Raw observation traces and trajectory logs are strictly immutable write-ahead records.
- **Hierarchical Consolidation**: Asynchronous consolidation pipelines synthesize episodic logs into macro-summaries, extract generalized semantic rules, and deprecate superseded working hypotheses. Derived nodes explicitly cite underlying episodic records as premises.
- **Principled Forgetting**: In accordance with the Ebbinghaus retention function and JTMS logic, items with decaying activation drop below retrieval thresholds and are relegated to compressed cold storage. Contradicted beliefs are marked invalid (`valid_until = now()`, `replaced_by = new_id`), preserving non-repudiable audit trails.

## Empirical Evaluation & Benchmarks

- **Factual Recall Longevity**: Factual accuracy and precision evaluated across horizons of 10, 100, and 1,000 execution cycles.
- **Cross-Task Interference**: Resistance to proactive and retroactive interference between structurally overlapping domains.
- **Retrieval Ablation**: Comparative benchmarks evaluating pure vector RAG vs. pure graph search vs. hybrid multi-stage retrieval.
- **Stale Memory Pollution Rate**: Quantitative proportion of invalidated, superseded, or speculative premises surfaced into working prompts.
