use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};

/// Deterministic 3D Euclidean vector for robotics kinematics and dynamics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    /// Constructs a new 3D vector.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Returns the zero vector (0.0, 0.0, 0.0).
    pub const fn zero() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    /// Computes the Euclidean norm (length) of the vector.
    pub fn norm(&self) -> f64 {
        self.norm_squared().sqrt()
    }

    /// Computes the squared Euclidean norm (avoids sqrt for fast distance comparisons).
    pub fn norm_squared(&self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// Computes the Euclidean distance to another point.
    pub fn distance(&self, other: &Self) -> f64 {
        (*self - *other).norm()
    }

    /// Computes the squared Euclidean distance to another point.
    pub fn distance_squared(&self, other: &Self) -> f64 {
        (*self - *other).norm_squared()
    }

    /// Returns the unit vector in the direction of self, or zero if norm is near zero.
    pub fn normalized(&self) -> Self {
        let n = self.norm();
        if n > 1e-9 {
            *self / n
        } else {
            Self::zero()
        }
    }

    /// Computes the dot product with another vector.
    pub fn dot(&self, other: &Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Computes the cross product with another vector.
    pub fn cross(&self, other: &Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    /// Clamps the vector's length to at most `max_norm`.
    pub fn clamp_norm(&self, max_norm: f64) -> Self {
        let n = self.norm();
        if n > max_norm && n > 1e-9 {
            *self * (max_norm / n)
        } else {
            *self
        }
    }

    /// Clamps the vector element-wise within an axis-aligned box [min, max].
    pub fn clamp_box(&self, min: &Self, max: &Self) -> Self {
        Self {
            x: self.x.clamp(min.x, max.x),
            y: self.y.clamp(min.y, max.y),
            z: self.z.clamp(min.z, max.z),
        }
    }

    /// Linearly interpolates between `self` and `other` by parameter `t` in [0, 1].
    pub fn lerp(&self, other: &Self, t: f64) -> Self {
        let factor = t.clamp(0.0, 1.0);
        *self + (*other - *self) * factor
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl SubAssign for Vec3 {
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, scalar: f64) -> Self::Output {
        Self {
            x: self.x * scalar,
            y: self.y * scalar,
            z: self.z * scalar,
        }
    }
}

impl MulAssign<f64> for Vec3 {
    fn mul_assign(&mut self, scalar: f64) {
        self.x *= scalar;
        self.y *= scalar;
        self.z *= scalar;
    }
}

impl Div<f64> for Vec3 {
    type Output = Self;
    fn div(self, scalar: f64) -> Self::Output {
        Self {
            x: self.x / scalar,
            y: self.y / scalar,
            z: self.z / scalar,
        }
    }
}

impl DivAssign<f64> for Vec3 {
    fn div_assign(&mut self, scalar: f64) {
        self.x /= scalar;
        self.y /= scalar;
        self.z /= scalar;
    }
}

impl fmt::Display for Vec3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:.3}, {:.3}, {:.3})", self.x, self.y, self.z)
    }
}

/// Dynamic joint state representation of an articulated manipulator or mobile base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointState {
    pub positions: Vec<f64>,
    pub velocities: Vec<f64>,
    pub torques: Vec<f64>,
}

impl JointState {
    /// Creates a new JointState with explicit positions, velocities, and torques.
    pub fn new(positions: Vec<f64>, velocities: Vec<f64>, torques: Vec<f64>) -> Self {
        Self {
            positions,
            velocities,
            torques,
        }
    }

    /// Initializes a JointState of `num_joints` with all zeros.
    pub fn zeros(num_joints: usize) -> Self {
        Self {
            positions: vec![0.0; num_joints],
            velocities: vec![0.0; num_joints],
            torques: vec![0.0; num_joints],
        }
    }

    /// Zeroes velocities and torques (e.g. for emergency stop or holding).
    pub fn zero_dynamics(&mut self) {
        for v in &mut self.velocities {
            *v = 0.0;
        }
        for t in &mut self.torques {
            *t = 0.0;
        }
    }

    /// Clamps joint torques symmetrically to `[-max_torque, max_torque]`.
    /// Returns true if any torque was clamped.
    pub fn clamp_torques(&mut self, max_torque: f64) -> bool {
        let mut clamped = false;
        for t in &mut self.torques {
            if t.abs() > max_torque {
                *t = t.clamp(-max_torque, max_torque);
                clamped = true;
            }
        }
        clamped
    }

    /// Clamps joint positions symmetrically to `[-max_pos_abs, max_pos_abs]`.
    /// Returns true if any position was clamped.
    pub fn clamp_positions(&mut self, max_pos_abs: f64) -> bool {
        let mut clamped = false;
        for p in &mut self.positions {
            if p.abs() > max_pos_abs {
                *p = p.clamp(-max_pos_abs, max_pos_abs);
                clamped = true;
            }
        }
        clamped
    }

    /// Returns the maximum absolute torque across all joints.
    pub fn max_torque_abs(&self) -> f64 {
        self.torques.iter().map(|t| t.abs()).fold(0.0, f64::max)
    }
}

/// Complete physical and sensorimotor state of the embodied agent at a discrete time step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbodiedState {
    pub step: u64,
    pub timestamp_ms: u64,
    pub end_effector_pos: Vec3,
    pub joints: JointState,
    pub external_disturbance: Vec3,
    pub emergency_stop_latched: bool,
}

impl EmbodiedState {
    /// Constructs a new EmbodiedState.
    pub fn new(step: u64, timestamp_ms: u64, end_effector_pos: Vec3, joints: JointState) -> Self {
        Self {
            step,
            timestamp_ms,
            end_effector_pos,
            joints,
            external_disturbance: Vec3::zero(),
            emergency_stop_latched: false,
        }
    }

    /// Sets the external physical disturbance vector acting on the end effector.
    pub fn with_disturbance(mut self, disturbance: Vec3) -> Self {
        self.external_disturbance = disturbance;
        self
    }
}

/// High-level symbolic / cognitive proposals emitted by the Deliberative Layer (1 Hz – 0.1 Hz).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DeliberativeActionProposal {
    /// Navigate end-effector to a Cartesian target pose with speed ceiling.
    MoveToPose { target: Vec3, max_speed: f64 },
    /// Composite skill: approach pick pose, grasp object, transfer to place pose, release.
    PickAndPlace {
        pick_pos: Vec3,
        place_pos: Vec3,
        grip_force: f64,
    },
    /// Maintain current pose with zero commanded velocity and active stiffness.
    HoldPosition,
}

/// Comprehensive telemetry report benchmarking embodied safety, control, and sim-to-real transfer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbodimentTelemetryReport {
    /// Fraction of target tasks executed successfully to completion (0.0 – 1.0).
    pub task_success_rate: f64,
    /// Invariant breaches that escaped supervisor supervision (must be strictly 0).
    pub safety_boundary_violations: usize,
    /// Interventions performed by SafetySupervisor (torque clamp, geofence deflection, e-stop).
    pub supervisor_interventions: usize,
    /// Root-mean-square tracking error under external disturbances.
    pub disturbance_rejection_rms_error: f64,
    /// Quantitative delta between nominal simulation and domain-shifted physical tracking.
    pub sim_to_real_transfer_gap: f64,
    /// Count of reflexive emergency stops latched due to prediction error threshold breaches.
    pub emergency_stops_triggered: usize,
}

impl EmbodimentTelemetryReport {
    /// Renders a formatted ASCII table of the embodied telemetry report.
    pub fn format_ascii_report(&self) -> String {
        format!(
            "+--------------------------------------------------------+\n\
             |          STRATA EMBODIED TELEMETRY REPORT              |\n\
             +--------------------------------------------------------+\n\
             | Task Success Rate (TSR)            : {:>19.1}% |\n\
             | Safety Boundary Violations         : {:>20} |\n\
             | Supervisor Interventions           : {:>20} |\n\
             | Disturbance Rejection RMS Error    : {:>20.4} |\n\
             | Sim-to-Real Transfer Gap           : {:>20.4} |\n\
             | Emergency Stops Latched/Triggered  : {:>20} |\n\
             +--------------------------------------------------------+",
            self.task_success_rate * 100.0,
            self.safety_boundary_violations,
            self.supervisor_interventions,
            self.disturbance_rejection_rms_error,
            self.sim_to_real_transfer_gap,
            self.emergency_stops_triggered
        )
    }
}
