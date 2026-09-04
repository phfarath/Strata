# Embodiment and Simulation

## Foundational Principles

Embodied intelligence requires a tightly closed loop linking perception, internal state estimation, action generation, and environmental feedback. While natural language provides an expressive interface for high-level intent specification, it cannot replace continuous state estimation, low-latency control, kinematic calculations, or deterministic physical safety envelopes. 

Strata treats embodiment as a hierarchical control problem where symbolic cognitive reasoning interfaces with deterministic reactive controllers.

## Technical Progression

The pathway from software-based cognitive autonomy to physical embodiment proceeds through five structured stages:
1. **Digital Software Tooling**: Operating within structured digital environments (file systems, terminal execution, AST manipulation) with immediate, observable state diffs.
2. **Simulated Physical Environments**: Interacting with physics engines (e.g., MuJoCo, Isaac Sim, PyBullet) with synthetic sensor streams (RGB-D, LiDAR, proprioception) and deterministic seed control.
3. **High-Level Skill Parameterization**: Training and deploying deliberative policies that decompose natural language directives into parameterized motor primitives and affordances.
4. **Specialized Low-Level Controllers**: Implementing real-time reactive controllers (e.g., Model Predictive Control, Operational Space Control, diffusion policies) governing trajectory generation, joint torque limits, and dynamic obstacle avoidance.
5. **Physical Hardware Deployment**: Operating on physical hardware platforms equipped with external supervisor daemons, physical emergency-stop (e-stop) relays, and hardware-enforced spatial keep-out zones.

## Decoupled Cognitive and Reactive Architecture

To maintain both intelligent long-term reasoning and sub-millisecond physical safety, Strata decouples the architecture across temporal and computational tiers:

### 1. Deliberative Layer (Cognitive Agent)
- Operates at coarse temporal resolution (1 Hz – 0.1 Hz).
- Manages long-horizon DAG planning, episodic memory retrieval, scene graph interpretation, and semantic skill selection.
- Emits high-level parameterized action proposals (e.g., `PickAndPlace(target: ObjectHandle, pose: Transform)`).

### 2. Reactive Layer (Local Controller)
- Operates at high temporal resolution (100 Hz – 1 kHz) in a real-time thread.
- Handles trajectory interpolation, impedance control, balance stabilization, and sensor latency mitigation.
- Enforces hard safety barriers: instantly overrides or rejects cognitive commands that violate joint limits, exceed torque thresholds, or breach collision boundaries.

### 3. World Model & Dynamic State Estimator
- Fuses multi-modal observations into a unified temporal belief graph.
- Predicts forward physical consequences and counterfactual rollouts.
- Computes prediction errors ($\Delta(\text{predicted}, \text{observed})$) to trigger reflexive halts when environmental reality diverges from internal model assumptions.

## Evaluation and Validation Metrics

Embodied agent policies are benchmarked across five core axes:
- **Task Success Rate (TSR)**: Fraction of physical manipulation or navigation tasks executed to specification.
- **Safety Boundary Invariant Violations**: Frequency of joint over-extension, excessive collision force, or geofence breaches (target: zero).
- **Disturbance Rejection Robustness**: Ability of the system to maintain equilibrium and recover task flow in the presence of unexpected external physical perturbations.
- **Out-of-Distribution Generalization**: Performance transfer to novel object geometries, varying lighting conditions, and altered physical friction parameters.
- **Sim-to-Real Transfer Gap**: Quantitative performance delta observed when transitioning policies from synthetic simulation environments to physical robotic platforms.
