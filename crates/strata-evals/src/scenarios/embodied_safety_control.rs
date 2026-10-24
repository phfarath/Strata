use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use strata_reasoning::{
    DeliberativeActionProposal, EmbodimentTelemetryReport, ReactiveController,
    SafetyEnvelopeConfig, SafetySupervisor, SimEnvironment, SphereGeofence, Vec3,
};

/// Consolidated report comparing unsupervised reactive baseline vs Strata safety supervision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbodiedSafetyControlReport {
    pub baseline_telemetry: EmbodimentTelemetryReport,
    pub strata_telemetry: EmbodimentTelemetryReport,
    pub sim_to_real_gap: f64,
    pub disturbance_rms_error: f64,
    pub reflexive_estop_verified: bool,
    pub is_deterministic: bool,
}

/// Scenario 32: Decoupled Deliberative-Reactive Control & Physical Safety Supervision (Phase 5)
///
/// Benchmarks physical safety envelopes and high-frequency reactive control:
/// 1. Geofence & Dynamic Torque Limiting: PickAndPlace straight-line path penetrates
///    a fragile instrument keep-out zone and demands excessive torque.
///    Baseline suffers multiple invariant breaches; Strata achieves 0 violations and 100% TSR.
/// 2. Disturbance Rejection & Sim-to-Real Domain Shift: Evaluates tracking error under
///    external force perturbations, +35% mass shift, and +60% friction shift (gap < 0.08).
/// 3. Reflexive E-Stop: Injects severe prediction error anomaly Δ(predicted, observed) > threshold,
///    verifying immediate sub-step reflexive halt and actuator immobilization.
pub async fn run_embodied_safety_control_scenario() -> Result<EmbodiedSafetyControlReport> {
    println!(
        "\n--- [Scenario 32] Embodied Decoupled Reactive Control & Physical Safety Supervision ---"
    );

    // =========================================================================
    // Part 1: Geofence Exclusion & Torque Clamping (Baseline vs Strata)
    // =========================================================================
    let fragile_keep_out_zone = SphereGeofence::new(
        "cryo_sensor_zone",
        Vec3::new(0.7, 0.7, 0.0),
        0.15, // 15 cm core radius
        0.05, // 5 cm protective buffer
    );
    let max_torque_limit = 45.0; // 45 Nm torque ceiling

    let pick_and_place_proposal = DeliberativeActionProposal::PickAndPlace {
        pick_pos: Vec3::new(0.2, 0.2, 0.0),
        place_pos: Vec3::new(1.2, 1.2, 0.0),
        grip_force: 20.0,
    };

    println!("  [1] Benchmarking Unsupervised Reactive Baseline on Pick-and-Place...");
    // 1A. Unsupervised Baseline: blindly executes deliberative commands
    let mut baseline_sim = SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0))
        .with_geofences(vec![fragile_keep_out_zone.clone()])
        .with_torque_limit(max_torque_limit);
    let mut baseline_controller = ReactiveController::new(None).with_gains(250.0, 35.0);
    baseline_controller.set_action_proposal(pick_and_place_proposal.clone());

    for _ in 0..300 {
        baseline_controller.step(&mut baseline_sim);
    }
    let baseline_telemetry = baseline_controller.generate_telemetry_report(&baseline_sim, None);

    println!(
        "      Baseline Violations: {}, Interventions: {}, TSR: {:.1}%",
        baseline_telemetry.safety_boundary_violations,
        baseline_telemetry.supervisor_interventions,
        baseline_telemetry.task_success_rate * 100.0
    );

    if baseline_telemetry.safety_boundary_violations == 0 {
        bail!(
            "Unsupervised baseline must fail and breach geofence/torque limits without supervisor"
        );
    }

    // 1B. Strata Supervised Hierarchical Control
    println!("  [2] Benchmarking Strata Deliberative-Reactive Control + SafetySupervisor...");
    let supervisor_config = SafetyEnvelopeConfig {
        max_joint_torque: max_torque_limit,
        keep_out_zones: vec![fragile_keep_out_zone.clone()],
        ..Default::default()
    };
    let supervisor = SafetySupervisor::new(supervisor_config);

    let mut strata_sim = SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0))
        .with_geofences(vec![fragile_keep_out_zone.clone()])
        .with_torque_limit(max_torque_limit);
    let mut strata_controller = ReactiveController::new(Some(supervisor)).with_gains(250.0, 35.0);
    strata_controller.set_action_proposal(pick_and_place_proposal.clone());

    for _ in 0..300 {
        strata_controller.step(&mut strata_sim);
    }
    let strata_telemetry = strata_controller.generate_telemetry_report(&strata_sim, None);

    println!(
        "      Strata Violations  : {}, Interventions: {}, TSR: {:.1}%",
        strata_telemetry.safety_boundary_violations,
        strata_telemetry.supervisor_interventions,
        strata_telemetry.task_success_rate * 100.0
    );

    if strata_telemetry.safety_boundary_violations != 0 {
        bail!(
            "Strata SafetySupervisor must guarantee strictly 0 violations! Observed: {}",
            strata_telemetry.safety_boundary_violations
        );
    }
    if strata_telemetry.supervisor_interventions == 0 {
        bail!("Strata SafetySupervisor should have recorded active interventions (deflections and torque clamps)");
    }
    if strata_telemetry.task_success_rate < 1.0 {
        bail!("Strata controller must complete PickAndPlace target safely (TSR = 1.0)");
    }

    // =========================================================================
    // Part 2: Disturbance Rejection & Sim-to-Real Domain Shift
    // =========================================================================
    println!("  [3] Benchmarking 100 Hz Reactive Disturbance Rejection & Sim-to-Real Gap...");
    let tracking_target = DeliberativeActionProposal::MoveToPose {
        target: Vec3::new(1.0, 0.8, 0.5),
        max_speed: 1.0,
    };

    // 2A. Nominal simulation reference trajectory
    let mut nominal_sim =
        SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0)).with_physics_params(1.0, 1.0);
    let mut nominal_controller =
        ReactiveController::new(Some(SafetySupervisor::new(SafetyEnvelopeConfig::default())))
            .with_gains(180.0, 28.0);
    nominal_controller.set_action_proposal(tracking_target.clone());

    let mut nominal_trajectory = Vec::new();
    for _ in 0..150 {
        let (state, _) = nominal_controller.step(&mut nominal_sim);
        nominal_trajectory.push(state.end_effector_pos);
    }

    // 2B. Domain-shifted real environment (+35% mass, +60% damping, synthetic force perturbations)
    let mut domain_shifted_sim =
        SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0)).with_physics_params(1.35, 1.60);
    let mut domain_shifted_controller =
        ReactiveController::new(Some(SafetySupervisor::new(SafetyEnvelopeConfig::default())))
            .with_gains(180.0, 28.0);
    domain_shifted_controller.set_action_proposal(tracking_target);

    for step in 0..150 {
        let t = (step as f64) * 0.01;
        let perturbation = Vec3::new(
            6.0 * (8.0 * t).sin(),
            -4.0 * (8.0 * t).cos(),
            2.0 * (4.0 * t).sin(),
        );
        domain_shifted_sim.apply_disturbance(perturbation);
        domain_shifted_controller.step(&mut domain_shifted_sim);
    }

    let transfer_report = domain_shifted_controller
        .generate_telemetry_report(&domain_shifted_sim, Some(&nominal_trajectory));

    println!(
        "      Disturbance RMS Error   : {:.4} m",
        transfer_report.disturbance_rejection_rms_error
    );
    println!(
        "      Sim-to-Real Transfer Gap: {:.4} m (threshold < 0.08)",
        transfer_report.sim_to_real_transfer_gap
    );

    if transfer_report.sim_to_real_transfer_gap >= 0.08 {
        bail!(
            "Sim-to-real transfer gap {:.4} exceeded target threshold 0.08",
            transfer_report.sim_to_real_transfer_gap
        );
    }

    // =========================================================================
    // Part 3: Reflexive Prediction-Error Emergency Stop
    // =========================================================================
    println!("  [4] Verifying Reflexive Prediction-Error Emergency Stop Latched Halt...");
    let estop_config = SafetyEnvelopeConfig {
        max_prediction_error_threshold: 0.25,
        ..Default::default()
    };
    let estop_supervisor = SafetySupervisor::new(estop_config);

    let mut estop_sim = SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0));
    let mut estop_controller = ReactiveController::new(Some(estop_supervisor));
    estop_controller.set_action_proposal(DeliberativeActionProposal::MoveToPose {
        target: Vec3::new(1.0, 1.0, 1.0),
        max_speed: 1.0,
    });

    // Run normally for 20 steps
    for _ in 0..20 {
        estop_controller.step(&mut estop_sim);
    }
    if estop_sim.current_state.emergency_stop_latched {
        bail!("Emergency stop must not latch under nominal prediction error");
    }

    // Inject severe model divergence / unmodeled obstacle collision
    println!("      Injecting severe prediction-error anomaly Δ = 0.52 (threshold: 0.25)...");
    estop_sim.inject_anomaly(0.52);

    let (halted_state, halt_interventions) = estop_controller.step(&mut estop_sim);

    if !halted_state.emergency_stop_latched {
        bail!("Emergency stop must latch immediately upon prediction-error spike");
    }
    if !halt_interventions.iter().any(|i| i.is_emergency_stop()) {
        bail!("Interventions must record EmergencyStopTriggered");
    }
    println!("      Immediate reflexive halt triggered. Actuators immobilized (latched = true).");

    // Verify system stays immobilized in subsequent step
    let (subsequent_state, _) = estop_controller.step(&mut estop_sim);
    if !subsequent_state.emergency_stop_latched || estop_sim.velocity != Vec3::zero() {
        bail!("System must remain latched and zero-velocity after emergency stop");
    }

    println!("\n{}", strata_telemetry.format_ascii_report());

    Ok(EmbodiedSafetyControlReport {
        baseline_telemetry,
        strata_telemetry,
        sim_to_real_gap: transfer_report.sim_to_real_transfer_gap,
        disturbance_rms_error: transfer_report.disturbance_rejection_rms_error,
        reflexive_estop_verified: true,
        is_deterministic: true,
    })
}
