# World Model

## Functional Role

The world model maintains the agent's internal epistemic representation of external environments: entities, relational topologies, dynamic state spaces, action capabilities, and causal transition dynamics. Strata's initial runtime prioritizes an explicit, probabilistic, and event-driven symbolic model over black-box continuous latent embeddings, ensuring full auditability, formal verification, and explainable decision paths.

## Foundational Representation: Epistemic Belief Graph

The environment state is formalized as an attributed, directed multi-graph $\mathcal{G} = (\mathcal{V}, \mathcal{E})$:

- **Vertices ($\mathcal{V}$)**: Represent discrete entities (code symbols, files, process handles, microservices) and semantic state predicates (e.g., `ServiceListening(port=8080)`, `FileModified(path)`).
- **Edges ($\mathcal{E}$)**: Typed causal, dependency, temporal, and hierarchical relations (e.g., `PreconditionOf`, `CausallyProduces`, `DependsOn`, `Contradicts`).
- **Epistemic Tuple**: Every propositional assertion $p \in \mathcal{V} \cup \mathcal{E}$ is anchored by an epistemic tuple:
  $$\langle c \in [0.0, 1.0], \; \mathcal{J}, \; [t_{\text{valid\_from}}, t_{\text{valid\_until}}], \; \mathcal{H} \rangle$$
  denoting confidence score $c$, evidential justifications $\mathcal{J}$ (grounded in sensor/tool observations), temporal validity bounds, and competing alternative hypotheses $\mathcal{H}$.

## Dynamic State Estimation & Belief Revision

Upon ingesting environmental observations $\mathbf{o}_t$ post-action:

1. **Entity Grounding & Resolution**: Map observed symbols and process outputs to canonical graph nodes via deterministic AST anchors and lexical identifiers.
2. **Prediction Error Assessment**: Compare the observed transition $s_t \xrightarrow{a_t} s_{t+1}$ against the counterfactual forecast synthesized during planning, computing residual surprise $\delta_t = \|s_{t+1} - \hat{s}_{t+1}\|$.
3. **Truth Maintenance & Conflict Resolution**: Invoke the bi-temporal Justification-based Truth Maintenance System (JTMS). When an empirical observation contradicts an existing belief, compute the Minimal Unsatisfiable Core (MUC), retract invalidated downstream conclusions, and spawn an exploratory sub-goal to reconcile the anomaly.
4. **Bayesian Confidence Calibration**: Dynamically update prior confidence scores conditioned on sensor reliability, observation frequency, and temporal decay.

## Counterfactual Planning & Predictive Simulation

Prior to committing a candidate action $a \in \mathcal{A}$, the world model evaluates simulated rollouts across multi-objective criteria:

- **Precondition Satisfiability**: Verify whether required antecedent predicates hold true within calibrated confidence thresholds ($\tau_{\text{conf}}$).
- **Expected Transition Distribution**: Forecast the distribution over prospective successor states $\hat{s}_{t+1} \sim \mathcal{T}(s_t, a)$.
- **Risk & Irreversibility Scoring**: Quantify action blast radius, side-effect irreversibility, and system degradation risks.
- **Information Gain (Epistemic Value)**: Calculate expected entropy reduction over ambiguous or high-uncertainty belief nodes, enabling the planner to balance goal exploitation with active epistemic exploration (Bayesian experimental design).

## Architectural Evolution Path

- **Phase I (Discrete & Symbolic)**: Explicit belief graphs, relational schemas, and deterministic rule engines optimized for software engineering, tool APIs, and shell environments.
- **Phase II (Hybrid & Continuous Latent)**: Integration of Recurrent State Space Models (RSSM / Dreamer architectures) and learned transition dynamics when extending into high-dimensional continuous sensory streams, visual interfaces, and physical robotic embodiment.
