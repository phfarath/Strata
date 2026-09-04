# Reasoning and Metacognition

## Reasoning Architecture

Reasoning is formal, guided search over hypotheses, candidate actions, and justifications—not merely autoregressive token generation or unstructured verbosity. The Strata runtime decouples generative language model proposals from deterministic evaluation by coordinating three primary subsystems:
1. **Generative LLM Engine**: Emits candidate reasoning steps, sub-goals, and exploratory hypotheses.
2. **Computational & Search Tools**: Provide external exact computation, semantic retrieval, symbol navigation, and environment inspection.
3. **Independent Verification Layer**: Evaluates validity, tracks premises, enforces invariant logic, and prevents epistemic drift before actions are committed.

## Reasoning Methodologies

Strata combines symbolic search and neural generation into a hybrid reasoning pipeline:
- **Hierarchical Problem Decomposition**: Breaking complex high-level objectives into directed sub-problems and structured goal graphs.
- **Self-Consistency Sampling**: Generating parallel stochastic rollouts over reasoning paths to isolate semantic consensus, compute sample variance, and flag divergent deductions.
- **Tree and Graph Search**: Executing guided search algorithms (e.g., Tree-of-Thoughts, Monte Carlo Tree Search, A*) over observable state transitions, evaluating node utility at each branch.
- **Deterministic Verifiers**: Grounding reasoning steps in compilers (`cargo check`, `rustc`), Tree-Sitter AST syntax validators, linters, unit tests, and domain-specific invariants.
- **Justification-Based Truth Maintenance (JTMS)**: Formally tracking dependencies between premises, assumptions, and inferred lemmas. When an underlying assertion or environmental state changes, the bi-temporal JTMS non-monotonically retracts downstream invalidated beliefs without corrupting consistent memory.

## Operational Metacognition

Operational metacognition provides continuous real-time estimation of epistemic uncertainty across model responses, planned DAG paths, retrieved memories, and anticipated tool actions. 

When estimated confidence falls below an acceptable threshold at a critical juncture, the runtime triggers defensive cognitive strategies:
- **Targeted Multi-Hop Retrieval**: Querying peripheral and episodic stores for clarifying historical context or relevant prior failures.
- **Active Information Gathering**: Emitting non-destructive exploratory inspection queries to narrow the hypothesis space.
- **Independent Cross-Verification**: Dispatching candidate conclusions to alternative verification models or symbolic checkers.
- **Human Escalation**: Halting execution and requesting human-in-the-loop disambiguation when uncertainty persists in high-stakes environments.

## Calibration and Uncertainty Quantification

Subjective model confidence must correspond directly to empirical success probabilities. Strata benchmarks and calibrates reasoning engines using standard statistical metrics:
- **Brier Score**: Measuring mean squared divergence between probabilistic predictions and actual binary task outcomes.
- **Expected Calibration Error (ECE)**: Assessing the disparity between confidence binning and accuracy across reasoning traces.
- **Selective Classification & Abstention Rate**: Evaluating the agent's ability to withhold ungrounded actions when confidence is below critical operating margins.
- **Semantic Entropy**: Quantifying semantic divergence across invariant paraphrased prompts to isolate genuine ambiguity from stylistic variance.
- **Confidence False-Positive Rate (Overconfidence Penalty)**: Penalizing high-confidence decisions that result in assertion failures or broken invariants.

## Decision Rule and Risk-Aware Execution

The runtime governs action dispatch through explicit expected utility optimization:

$$\mathbb{E}[U] = \mathbb{E}[\text{Progress}] - \text{Cost} - \text{Risk}$$

Where:
- $\mathbb{E}[\text{Progress}]$ denotes estimated advancement toward topological DAG completion.
- $\text{Cost}$ incorporates API token expenditure, computational latency, and physical resource consumption.
- $\text{Risk}$ models the severity and reversibility of failure modes.

### Risk Boundaries
- **Reversible Actions** (e.g., localized scratchpad edits, read-only inspections, sandboxed test execution): Permitted under standard calibrated confidence thresholds.
- **Irreversible Actions** (e.g., destructive disk mutations, production deployments, external irreversible API calls): Require strict high-confidence verifier approval, proof of pre-conditions, and explicit user-level cryptographic or interactive authorization.
