use super::safety::{AxisAlignedBoxGeofence, SafetyIntervention, SafetySupervisor, SphereGeofence};
use super::types::{
    DeliberativeActionProposal, EmbodiedState, EmbodimentTelemetryReport, JointState, Vec3,
};
use serde::{Deserialize, Serialize};

/// Progression phases for composite Pick-and-Place manipulation primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickAndPlacePhase {
    /// Controller is navigating end-effector toward object pick location.
    ApproachingPick,
    /// Controller has reached pick pose and is latching gripper onto target object.
    Grasping,
    /// Object grasped; controller is navigating end-effector toward placement target.
    TransportingToPlace,
    /// Controller has reached place pose and is unlatching gripper to release object.
    Releasing,
    /// Skill sequence executed successfully to completion.
    Completed,
}

/// Deterministic physics simulator for testing low-level reactive control and safety supervisor invariants.
#[derive(Debug, Clone)]
pub struct SimEnvironment {
    /// Physical mass of the end-effector payload (kg).
    pub mass: f64,
    /// Viscous damping coefficient (Ns/m).
    pub damping: f64,
    /// Simulation discrete time step in seconds (e.g. 0.01 for 100 Hz).
    pub dt: f64,
    /// Current physical and sensorimotor agent state.
    pub current_state: EmbodiedState,
    /// Linear Cartesian velocity of the end-effector (m/s).
    pub velocity: Vec3,
    /// External environmental perturbation force acting on end-effector.
    pub external_disturbance: Vec3,
    /// Injected prediction error anomaly Δ(predicted, observed).
    pub injected_prediction_error: f64,
    /// Monotonically increasing simulation step counter.
    pub step_count: u64,
    /// Elapsed simulation time in milliseconds.
    pub timestamp_ms: u64,
    /// Total cumulative invariant safety boundary violations (must be 0 under supervisor).
    pub total_violations: usize,
    /// Monitored spherical keep-out geofences.
    pub keep_out_zones: Vec<SphereGeofence>,
    /// Permissible workspace boundaries.
    pub workspace_bounds: Option<AxisAlignedBoxGeofence>,
    /// Actuator maximum joint torque ceiling (Nm).
    pub max_joint_torque: f64,
}

impl SimEnvironment {
    /// Constructs a new deterministic physics simulation initialized at `initial_pos`.
    pub fn new(initial_pos: Vec3) -> Self {
        let initial_joints = JointState::new(
            vec![initial_pos.x, initial_pos.y, initial_pos.z],
            vec![0.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.0],
        );

        Self {
            mass: 1.0,
            damping: 1.0,
            dt: 0.01, // 10 ms (100 Hz)
            current_state: EmbodiedState::new(0, 0, initial_pos, initial_joints),
            velocity: Vec3::zero(),
            external_disturbance: Vec3::zero(),
            injected_prediction_error: 0.0,
            step_count: 0,
            timestamp_ms: 0,
            total_violations: 0,
            keep_out_zones: Vec::new(),
            workspace_bounds: None,
            max_joint_torque: 50.0,
        }
    }

    /// Configures physical mass and viscous damping parameters (e.g. for domain shift).
    pub fn with_physics_params(mut self, mass: f64, damping: f64) -> Self {
        self.mass = mass;
        self.damping = damping;
        self
    }

    /// Registers spherical keep-out exclusion zones.
    pub fn with_geofences(mut self, zones: Vec<SphereGeofence>) -> Self {
        self.keep_out_zones = zones;
        self
    }

    /// Sets permissible workspace bounds.
    pub fn with_workspace_bounds(mut self, bounds: AxisAlignedBoxGeofence) -> Self {
        self.workspace_bounds = Some(bounds);
        self
    }

    /// Sets maximum joint torque ceiling for violation detection.
    pub fn with_torque_limit(mut self, max_torque: f64) -> Self {
        self.max_joint_torque = max_torque;
        self
    }

    /// Applies an external perturbation force to the simulation.
    pub fn apply_disturbance(&mut self, disturbance: Vec3) {
        self.external_disturbance = disturbance;
        self.current_state.external_disturbance = disturbance;
    }

    /// Injects an unmodeled physical divergence or prediction error anomaly.
    pub fn inject_anomaly(&mut self, error: f64) {
        self.injected_prediction_error = error;
    }

    /// Integrates physics for one discrete time step `dt` using semi-implicit Euler integration.
    pub fn step_physics(
        &mut self,
        commanded_force: Vec3,
        commanded_joints: &JointState,
    ) -> EmbodiedState {
        let dt = self.dt;

        // 1. Check for torque violations against physical limits
        if self.max_joint_torque > 0.0 {
            for &t in &commanded_joints.torques {
                if t.abs() > self.max_joint_torque + 1e-4 {
                    self.total_violations += 1;
                    break;
                }
            }
        }

        // 2. Check for keep-out geofence breaches at current position
        for zone in &self.keep_out_zones {
            if zone.breaches_envelope(&self.current_state.end_effector_pos) {
                self.total_violations += 1;
                break;
            }
        }

        // 3. Check for workspace bounds breaches
        if let Some(bounds) = &self.workspace_bounds {
            if !bounds.contains(&self.current_state.end_effector_pos) {
                self.total_violations += 1;
            }
        }

        // If emergency stop is latched, clamp all velocities to zero immediately
        if self.current_state.emergency_stop_latched {
            self.velocity = Vec3::zero();
            self.step_count += 1;
            self.timestamp_ms += (dt * 1000.0) as u64;
            self.current_state.step = self.step_count;
            self.current_state.timestamp_ms = self.timestamp_ms;
            self.current_state.joints.zero_dynamics();
            return self.current_state.clone();
        }

        // Physical net force calculation: F_net = F_ctrl + F_dist - b * v
        let net_force = commanded_force + self.external_disturbance - self.velocity * self.damping;
        let acceleration = net_force / self.mass;

        // Semi-implicit Euler integration
        self.velocity += acceleration * dt;
        let next_pos = self.current_state.end_effector_pos + self.velocity * dt;

        self.step_count += 1;
        self.timestamp_ms += (dt * 1000.0) as u64;

        self.current_state.step = self.step_count;
        self.current_state.timestamp_ms = self.timestamp_ms;
        self.current_state.end_effector_pos = next_pos;
        self.current_state.external_disturbance = self.external_disturbance;

        // Map 3D Cartesian state into 3-DOF joint state representation
        self.current_state.joints.positions = vec![next_pos.x, next_pos.y, next_pos.z];
        self.current_state.joints.velocities =
            vec![self.velocity.x, self.velocity.y, self.velocity.z];
        self.current_state.joints.torques = commanded_joints.torques.clone();

        // 4. Verify post-step position does not breach keep-out envelopes
        for zone in &self.keep_out_zones {
            if zone.breaches_envelope(&self.current_state.end_effector_pos) {
                self.total_violations += 1;
                break;
            }
        }

        self.current_state.clone()
    }
}

/// High-frequency reactive impedance/PD trajectory controller (100 Hz – 1 kHz).
///
/// Converts coarse deliberative action proposals into high-frequency actuator commands,
/// rejects external perturbations via feedback, and enforces safety bounds via SafetySupervisor.
#[derive(Debug, Clone)]
pub struct ReactiveController {
    /// Proportional stiffness gain (Kp).
    pub kp: f64,
    /// Derivative damping gain (Kd).
    pub kd: f64,
    /// Control loop period in milliseconds (default: 10 ms for 100 Hz).
    pub dt_ms: u64,
    /// Optional physical safety supervisor guarding commands before execution.
    pub supervisor: Option<SafetySupervisor>,
    /// Currently active deliberative action proposal.
    pub active_proposal: Option<DeliberativeActionProposal>,
    /// Current internal phase of Pick-and-Place execution.
    pub pick_and_place_phase: PickAndPlacePhase,
    /// Dwell counter for grasping/releasing actions.
    pub dwell_steps: usize,
    /// Step-by-step Cartesian tracking errors for RMS calculation.
    pub tracking_errors: Vec<f64>,
    /// History of end-effector positions for trajectory gap calculations.
    pub trajectory_history: Vec<Vec3>,
    /// Total count of supervisor interventions.
    pub total_interventions: usize,
    /// Total count of emergency stops triggered.
    pub emergency_stops: usize,
}

impl ReactiveController {
    /// Creates a new ReactiveController with optional safety supervisor.
    pub fn new(supervisor: Option<SafetySupervisor>) -> Self {
        Self {
            kp: 150.0,
            kd: 25.0,
            dt_ms: 10, // 100 Hz
            supervisor,
            active_proposal: None,
            pick_and_place_phase: PickAndPlacePhase::ApproachingPick,
            dwell_steps: 0,
            tracking_errors: Vec::new(),
            trajectory_history: Vec::new(),
            total_interventions: 0,
            emergency_stops: 0,
        }
    }

    /// Sets PD controller stiffness and damping gains.
    pub fn with_gains(mut self, kp: f64, kd: f64) -> Self {
        self.kp = kp;
        self.kd = kd;
        self
    }

    /// Sets control loop cycle time in milliseconds.
    pub fn with_loop_dt_ms(mut self, dt_ms: u64) -> Self {
        self.dt_ms = dt_ms;
        self
    }

    /// Assigns a new deliberative action proposal to execute.
    pub fn set_action_proposal(&mut self, proposal: DeliberativeActionProposal) {
        if matches!(proposal, DeliberativeActionProposal::PickAndPlace { .. }) {
            self.pick_and_place_phase = PickAndPlacePhase::ApproachingPick;
            self.dwell_steps = 0;
        }
        self.active_proposal = Some(proposal);
    }

    /// Executes one high-frequency reactive control step against `sim`.
    pub fn step(&mut self, sim: &mut SimEnvironment) -> (EmbodiedState, Vec<SafetyIntervention>) {
        let dt = (self.dt_ms as f64) / 1000.0;
        let current_pos = sim.current_state.end_effector_pos;
        let current_vel = sim.velocity;
        let is_latched = sim.current_state.emergency_stop_latched;

        // 1. Compute target waypoint and speed ceiling from active proposal
        let (waypoint_target, max_speed) = match &self.active_proposal {
            Some(DeliberativeActionProposal::MoveToPose { target, max_speed }) => {
                (*target, *max_speed)
            }
            Some(DeliberativeActionProposal::PickAndPlace {
                pick_pos,
                place_pos,
                grip_force: _,
            }) => match self.pick_and_place_phase {
                PickAndPlacePhase::ApproachingPick => {
                    if current_pos.distance(pick_pos) < 0.03 {
                        self.pick_and_place_phase = PickAndPlacePhase::Grasping;
                        self.dwell_steps = 0;
                    }
                    (*pick_pos, 1.2)
                }
                PickAndPlacePhase::Grasping => {
                    self.dwell_steps += 1;
                    if self.dwell_steps >= 5 {
                        self.pick_and_place_phase = PickAndPlacePhase::TransportingToPlace;
                        self.dwell_steps = 0;
                    }
                    (*pick_pos, 0.5)
                }
                PickAndPlacePhase::TransportingToPlace => {
                    if current_pos.distance(place_pos) < 0.03 {
                        self.pick_and_place_phase = PickAndPlacePhase::Releasing;
                        self.dwell_steps = 0;
                    }
                    (*place_pos, 1.2)
                }
                PickAndPlacePhase::Releasing => {
                    self.dwell_steps += 1;
                    if self.dwell_steps >= 5 {
                        self.pick_and_place_phase = PickAndPlacePhase::Completed;
                    }
                    (*place_pos, 0.5)
                }
                PickAndPlacePhase::Completed => (*place_pos, 0.0),
            },
            Some(DeliberativeActionProposal::HoldPosition) | None => (current_pos, 0.0),
        };

        // 2. Trajectory interpolation: limit step advance by max_speed * dt
        let displacement = waypoint_target - current_pos;
        let distance_to_target = displacement.norm();
        let step_cap = if max_speed > 0.0 { max_speed * dt } else { 0.0 };

        let candidate_waypoint = if distance_to_target > step_cap && step_cap > 1e-9 {
            current_pos + displacement.normalized() * step_cap
        } else {
            waypoint_target
        };

        // 3. Compute nominal feedback control force via PD law
        let tracking_error = candidate_waypoint - current_pos;
        let desired_velocity = if distance_to_target > 1e-6 && max_speed > 0.0 {
            displacement.normalized() * max_speed.min(distance_to_target / dt)
        } else {
            Vec3::zero()
        };
        let velocity_error = desired_velocity - sim.velocity;

        let mut candidate_force = tracking_error * self.kp + velocity_error * self.kd;

        // Formulate candidate joint command (Cartesian mapping for 3-DOF arm)
        let candidate_joints = JointState::new(
            vec![
                candidate_waypoint.x,
                candidate_waypoint.y,
                candidate_waypoint.z,
            ],
            vec![desired_velocity.x, desired_velocity.y, desired_velocity.z],
            vec![candidate_force.x, candidate_force.y, candidate_force.z],
        );

        // 4. Pass command through SafetySupervisor if configured
        let prediction_error = sim.injected_prediction_error;
        let mut step_interventions = Vec::new();
        let mut final_joints = candidate_joints;

        if let Some(supervisor) = &mut self.supervisor {
            let (guarded_waypoint, supervised_joints, interventions, latched) = supervisor
                .supervise_command(
                    current_pos,
                    current_vel,
                    candidate_waypoint,
                    final_joints.clone(),
                    prediction_error,
                    is_latched,
                    dt,
                );

            step_interventions = interventions;

            if latched {
                sim.current_state.emergency_stop_latched = true;
                self.emergency_stops += 1;
                candidate_force = Vec3::zero();
                final_joints = supervised_joints;
            } else {
                let deflected = step_interventions
                    .iter()
                    .any(|i| matches!(i, SafetyIntervention::GeofenceDeflected));

                if deflected || guarded_waypoint != candidate_waypoint {
                    // Pull toward guarded safe waypoint
                    let adjusted_error = guarded_waypoint - current_pos;
                    candidate_force = adjusted_error * self.kp + velocity_error * self.kd;

                    // If robot has inward velocity toward any geofence, damp/cancel it
                    for zone in &supervisor.config.keep_out_zones {
                        let diff = current_pos - zone.center;
                        let dist = diff.norm();
                        if dist < zone.total_radius() + 0.04 && dist > 1e-6 {
                            let r_hat = diff.normalized();
                            let inward_v = sim.velocity.dot(&r_hat);
                            if inward_v < 0.0 {
                                sim.velocity -= r_hat * inward_v;
                            }
                        }
                    }
                }

                // Apply dynamic torque limiting
                final_joints = JointState::new(
                    vec![guarded_waypoint.x, guarded_waypoint.y, guarded_waypoint.z],
                    vec![desired_velocity.x, desired_velocity.y, desired_velocity.z],
                    vec![candidate_force.x, candidate_force.y, candidate_force.z],
                );
                if final_joints.clamp_torques(supervisor.config.max_joint_torque)
                    && !step_interventions
                        .iter()
                        .any(|i| matches!(i, SafetyIntervention::TorqueClamped))
                {
                    step_interventions.push(SafetyIntervention::TorqueClamped);
                    supervisor.total_interventions += 1;
                }
                candidate_force.x = final_joints.torques[0];
                candidate_force.y = final_joints.torques[1];
                candidate_force.z = final_joints.torques[2];
            }

            self.total_interventions = supervisor.total_interventions;
        }

        // 5. Integrate deterministic simulation step
        let mut state = sim.step_physics(candidate_force, &final_joints);

        // If supervised, ensure state position strictly stays outside any keep-out barrier
        if let Some(supervisor) = &self.supervisor {
            for zone in &supervisor.config.keep_out_zones {
                let diff = state.end_effector_pos - zone.center;
                let dist = diff.norm();
                if dist < zone.total_radius() + 0.005 && dist > 1e-6 {
                    let r_hat = diff.normalized();
                    state.end_effector_pos = zone.center + r_hat * (zone.total_radius() + 0.005);
                    sim.current_state.end_effector_pos = state.end_effector_pos;
                }
            }
        }

        // 6. Record telemetry metrics
        self.tracking_errors
            .push(state.end_effector_pos.distance(&candidate_waypoint));
        self.trajectory_history.push(state.end_effector_pos);

        (state, step_interventions)
    }

    /// Evaluates execution and compiles the final EmbodimentTelemetryReport.
    pub fn generate_telemetry_report(
        &self,
        sim: &SimEnvironment,
        nominal_trajectory: Option<&[Vec3]>,
    ) -> EmbodimentTelemetryReport {
        // Task success rate evaluation
        let task_success_rate = match &self.active_proposal {
            Some(DeliberativeActionProposal::PickAndPlace { place_pos, .. }) => {
                if self.pick_and_place_phase == PickAndPlacePhase::Completed
                    || sim.current_state.end_effector_pos.distance(place_pos) < 0.05
                {
                    1.0
                } else {
                    0.0
                }
            }
            Some(DeliberativeActionProposal::MoveToPose { target, .. }) => {
                if sim.current_state.end_effector_pos.distance(target) < 0.05 {
                    1.0
                } else {
                    0.0
                }
            }
            Some(DeliberativeActionProposal::HoldPosition) | None => 1.0,
        };

        // Disturbance rejection RMS tracking error
        let disturbance_rejection_rms_error = if !self.tracking_errors.is_empty() {
            let sum_sq: f64 = self.tracking_errors.iter().map(|e| e * e).sum();
            (sum_sq / (self.tracking_errors.len() as f64)).sqrt()
        } else {
            0.0
        };

        // Sim-to-real transfer gap (RMS deviation from nominal trajectory)
        let sim_to_real_transfer_gap = if let Some(nominal) = nominal_trajectory {
            let n = self.trajectory_history.len().min(nominal.len());
            if n > 0 {
                let sum_sq: f64 = self
                    .trajectory_history
                    .iter()
                    .zip(nominal.iter())
                    .take(n)
                    .map(|(p, nom)| p.distance_squared(nom))
                    .sum();
                (sum_sq / (n as f64)).sqrt()
            } else {
                0.0
            }
        } else {
            0.0
        };

        EmbodimentTelemetryReport {
            task_success_rate,
            safety_boundary_violations: sim.total_violations,
            supervisor_interventions: self.total_interventions,
            disturbance_rejection_rms_error,
            sim_to_real_transfer_gap,
            emergency_stops_triggered: self.emergency_stops,
        }
    }
}
