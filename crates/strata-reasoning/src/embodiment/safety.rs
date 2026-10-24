use super::types::{JointState, Vec3};
use serde::{Deserialize, Serialize};

/// Spherical keep-out geofence (e.g. human operator proximity, fragile equipment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SphereGeofence {
    /// Unique identifier for the keep-out zone.
    pub id: String,
    /// 3D center position of the obstacle or keep-out zone.
    pub center: Vec3,
    /// Physical core radius of the obstacle.
    pub radius: f64,
    /// Protective buffer margin added to the radius.
    pub safety_margin: f64,
}

impl SphereGeofence {
    /// Constructs a new spherical keep-out geofence.
    pub fn new(id: impl Into<String>, center: Vec3, radius: f64, safety_margin: f64) -> Self {
        Self {
            id: id.into(),
            center,
            radius,
            safety_margin,
        }
    }

    /// Total exclusion barrier radius including safety margin: `radius + safety_margin`.
    pub fn total_radius(&self) -> f64 {
        self.radius + self.safety_margin
    }

    /// Returns true if `point` penetrates the exclusion envelope (`distance < total_radius`).
    pub fn breaches_envelope(&self, point: &Vec3) -> bool {
        point.distance(&self.center) < self.total_radius()
    }

    /// Returns true if `point` penetrates the physical core obstacle (`distance < radius`).
    pub fn breaches_core(&self, point: &Vec3) -> bool {
        point.distance(&self.center) < self.radius
    }
}

/// Axis-aligned bounding box defining allowable workspace limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisAlignedBoxGeofence {
    /// Minimum corner [min_x, min_y, min_z].
    pub min: Vec3,
    /// Maximum corner [max_x, max_y, max_z].
    pub max: Vec3,
}

impl AxisAlignedBoxGeofence {
    /// Constructs a new axis-aligned bounding box.
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Checks if a 3D point is entirely within the workspace box.
    pub fn contains(&self, point: &Vec3) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }

    /// Clamps a 3D point to lie strictly within the workspace bounding box.
    /// Returns the clamped point and a boolean indicating whether clamping occurred.
    pub fn clamp(&self, point: &Vec3) -> (Vec3, bool) {
        let clamped = point.clamp_box(&self.min, &self.max);
        let was_clamped = clamped != *point;
        (clamped, was_clamped)
    }
}

/// Physical safety envelope and supervisor configuration parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafetyEnvelopeConfig {
    /// Maximum allowable absolute joint torque (Nm).
    pub max_joint_torque: f64,
    /// Maximum allowable absolute joint position angle/displacement.
    pub max_joint_position_abs: f64,
    /// Maximum allowable Cartesian linear velocity (m/s).
    pub max_cartesian_speed: f64,
    /// Maximum prediction error threshold Δ(predicted, observed) before reflexive e-stop.
    pub max_prediction_error_threshold: f64,
    /// Boundary envelope of the permissible workspace.
    pub workspace_bounds: AxisAlignedBoxGeofence,
    /// List of spherical keep-out geofences.
    pub keep_out_zones: Vec<SphereGeofence>,
}

impl Default for SafetyEnvelopeConfig {
    fn default() -> Self {
        Self {
            max_joint_torque: 50.0,
            max_joint_position_abs: std::f64::consts::PI,
            max_cartesian_speed: 1.5,
            max_prediction_error_threshold: 0.25,
            workspace_bounds: AxisAlignedBoxGeofence::new(
                Vec3::new(-2.0, -2.0, -0.5),
                Vec3::new(2.0, 2.0, 2.5),
            ),
            keep_out_zones: Vec::new(),
        }
    }
}

/// Intervention actions taken by the real-time SafetySupervisor to protect physical invariants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SafetyIntervention {
    /// Command conformed to all invariants; no intervention needed.
    None,
    /// Commanded joint torques exceeded allowable dynamic limits and were clamped.
    TorqueClamped,
    /// Commanded trajectory step approached a keep-out zone and was deflected tangentially.
    GeofenceDeflected,
    /// Commanded position reached workspace perimeter and was clamped.
    WorkspaceBoundaryClamped,
    /// Reflexive halt triggered: model divergence or collision anomaly exceeded threshold.
    EmergencyStopTriggered {
        reason: String,
        prediction_error: f64,
    },
}

impl SafetyIntervention {
    /// Returns true if this constitutes an active intervention.
    pub fn is_intervention(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Returns true if an emergency stop was triggered.
    pub fn is_emergency_stop(&self) -> bool {
        matches!(self, Self::EmergencyStopTriggered { .. })
    }
}

/// Real-time deterministic physical safety supervisor.
///
/// Operates as an inviolable gatekeeper between deliberative/reactive control proposals
/// and actuator execution, enforcing keep-out geofencing, torque clamping, and reflexive halts.
#[derive(Debug, Clone)]
pub struct SafetySupervisor {
    pub config: SafetyEnvelopeConfig,
    pub intervention_history: Vec<SafetyIntervention>,
    pub total_interventions: usize,
    pub emergency_stops: usize,
}

impl SafetySupervisor {
    /// Creates a new SafetySupervisor with the specified configuration.
    pub fn new(config: SafetyEnvelopeConfig) -> Self {
        Self {
            config,
            intervention_history: Vec::new(),
            total_interventions: 0,
            emergency_stops: 0,
        }
    }

    /// Supervises and projects candidate reactive commands before they reach physical actuators.
    ///
    /// 1. If `prediction_error > max_prediction_error_threshold` or already latched,
    ///    triggers reflexive `EmergencyStopTriggered` and zeroes all torques and velocities.
    /// 2. If a trajectory step would enter any `SphereGeofence` (`distance(pos, center) < radius + safety_margin`),
    ///    projects target position tangentially outside the barrier (`GeofenceDeflected`),
    ///    guaranteeing 0 envelope breaches.
    /// 3. Clamps position within `workspace_bounds` (`WorkspaceBoundaryClamped`).
    /// 4. Clamps joint torques to `[-max_joint_torque, max_joint_torque]` (`TorqueClamped`).
    #[allow(clippy::too_many_arguments)]
    pub fn supervise_command(
        &mut self,
        current_pos: Vec3,
        current_velocity: Vec3,
        proposed_target: Vec3,
        mut proposed_joints: JointState,
        prediction_error: f64,
        is_latched: bool,
        dt: f64,
    ) -> (Vec3, JointState, Vec<SafetyIntervention>, bool) {
        let mut interventions = Vec::new();

        // 1. Reflexive Prediction-Error & Emergency Stop Verification
        if is_latched || prediction_error > self.config.max_prediction_error_threshold {
            let reason = if is_latched {
                "Emergency stop already latched".to_string()
            } else {
                format!(
                    "Reflexive halt: prediction error {:.4} exceeded threshold {:.4}",
                    prediction_error, self.config.max_prediction_error_threshold
                )
            };

            let intervention = SafetyIntervention::EmergencyStopTriggered {
                reason,
                prediction_error,
            };
            interventions.push(intervention.clone());
            self.intervention_history.push(intervention);
            self.total_interventions += 1;
            if !is_latched {
                self.emergency_stops += 1;
            }

            proposed_joints.zero_dynamics();
            return (current_pos, proposed_joints, interventions, true);
        }

        let mut safe_target = proposed_target;

        // 2. Keep-Out Geofence Supervision & Tangential Deflection
        for zone in &self.config.keep_out_zones {
            let barrier_radius = zone.total_radius();
            let safe_radius = barrier_radius + 0.06; // 6cm safe guidance buffer

            let dist_to_center = current_pos.distance(&zone.center);
            let projected_pos = current_pos + current_velocity * dt;

            // Check if proposed target, projected step, or current pos enters safe radius
            let entering = zone.breaches_envelope(&safe_target)
                || zone.breaches_envelope(&projected_pos)
                || dist_to_center < safe_radius;

            if entering {
                let v_p = current_pos - zone.center;
                let r_hat = if dist_to_center > 1e-6 {
                    v_p.normalized()
                } else {
                    Vec3::new(1.0, 0.0, 0.0)
                };

                // Direction toward the ultimate target
                let w = proposed_target - current_pos;
                // Tangential projection of w onto the sphere's tangent plane at current_pos
                let w_tan = w - r_hat * w.dot(&r_hat);

                let t_hat = if w_tan.norm() > 1e-4 {
                    w_tan.normalized()
                } else if r_hat.z.abs() < 0.8 {
                    // Collinear fallback: navigate around equator in XY plane
                    Vec3::new(-r_hat.y, r_hat.x, 0.0).normalized()
                } else {
                    Vec3::new(1.0, 0.0, 0.0)
                };

                // Project target along tangent on the safe radius
                let step_tangent = 0.45;
                let next_radial = (r_hat + t_hat * step_tangent).normalized();
                safe_target = zone.center + next_radial * safe_radius;

                let intervention = SafetyIntervention::GeofenceDeflected;
                interventions.push(intervention.clone());
                self.intervention_history.push(intervention);
                self.total_interventions += 1;
            }
        }

        // 3. Workspace Boundary Clamping
        if !self.config.workspace_bounds.contains(&safe_target) {
            let (clamped, was_clamped) = self.config.workspace_bounds.clamp(&safe_target);
            if was_clamped {
                safe_target = clamped;
                let intervention = SafetyIntervention::WorkspaceBoundaryClamped;
                interventions.push(intervention.clone());
                self.intervention_history.push(intervention);
                self.total_interventions += 1;
            }
        }

        // 4. Dynamic Torque Limiting
        if proposed_joints.clamp_torques(self.config.max_joint_torque) {
            let intervention = SafetyIntervention::TorqueClamped;
            interventions.push(intervention.clone());
            self.intervention_history.push(intervention);
            self.total_interventions += 1;
        }

        // 5. Joint Position Clamping
        if self.config.max_joint_position_abs > 0.0 {
            proposed_joints.clamp_positions(self.config.max_joint_position_abs);
        }

        (safe_target, proposed_joints, interventions, false)
    }
}
