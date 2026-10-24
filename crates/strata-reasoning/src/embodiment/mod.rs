mod controller;
mod safety;
mod types;

pub use controller::*;
pub use safety::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vec3_kinematic_operations() {
        let v1 = Vec3::new(1.0, 2.0, 3.0);
        let v2 = Vec3::new(4.0, 5.0, 6.0);

        assert_eq!(v1 + v2, Vec3::new(5.0, 7.0, 9.0));
        assert_eq!(v2 - v1, Vec3::new(3.0, 3.0, 3.0));
        assert_eq!(v1 * 2.0, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(v2 / 2.0, Vec3::new(2.0, 2.5, 3.0));

        let norm_sq = v1.norm_squared();
        assert_eq!(norm_sq, 1.0 + 4.0 + 9.0);
        assert!((v1.norm() - 14.0_f64.sqrt()).abs() < 1e-9);

        let dist = v1.distance(&v2);
        assert!((dist - 27.0_f64.sqrt()).abs() < 1e-9);

        let lerped = v1.lerp(&v2, 0.5);
        assert_eq!(lerped, Vec3::new(2.5, 3.5, 4.5));

        let clamped = Vec3::new(10.0, -10.0, 5.0)
            .clamp_box(&Vec3::new(-1.0, -1.0, 0.0), &Vec3::new(1.0, 1.0, 2.0));
        assert_eq!(clamped, Vec3::new(1.0, -1.0, 2.0));

        let clamped_norm = Vec3::new(3.0, 4.0, 0.0).clamp_norm(2.5);
        assert!((clamped_norm.norm() - 2.5).abs() < 1e-6);
    }

    #[test]
    fn test_keep_out_geofence_deflection_zero_breaches() {
        // Obstacle directly on the straight-line path between (0,0,0) and (1,1,0)
        let obstacle = SphereGeofence::new("obstacle_1", Vec3::new(0.5, 0.5, 0.0), 0.15, 0.05);
        let total_barrier = obstacle.total_radius(); // 0.20

        let config = SafetyEnvelopeConfig {
            keep_out_zones: vec![obstacle.clone()],
            ..Default::default()
        };
        let supervisor = SafetySupervisor::new(config);

        let mut sim =
            SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0)).with_geofences(vec![obstacle.clone()]);
        let mut controller = ReactiveController::new(Some(supervisor)).with_gains(160.0, 25.0);

        controller.set_action_proposal(DeliberativeActionProposal::MoveToPose {
            target: Vec3::new(1.0, 1.0, 0.0),
            max_speed: 1.0,
        });

        // Run 250 steps (2.5 seconds) to allow curved deflection path navigation
        for _ in 0..250 {
            let (state, _) = controller.step(&mut sim);
            // Invariant: At EVERY single step, the end-effector MUST NOT breach the keep-out barrier
            assert!(
                !obstacle.breaches_envelope(&state.end_effector_pos),
                "Keep-out envelope breached! Pos: {}, Dist to center: {:.4}, Barrier: {:.4}",
                state.end_effector_pos,
                state.end_effector_pos.distance(&obstacle.center),
                total_barrier
            );
        }

        assert_eq!(
            sim.total_violations, 0,
            "Supervised control must incur 0 safety violations"
        );
        assert!(
            controller.total_interventions > 0,
            "Supervisor should have actively deflected around geofence"
        );

        // Verify task completion
        assert!(
            sim.current_state
                .end_effector_pos
                .distance(&Vec3::new(1.0, 1.0, 0.0))
                < 0.05,
            "Target should be reached safely"
        );
    }

    #[test]
    fn test_unsupervised_baseline_incurs_safety_violations() {
        // Same obstacle on straight-line path, but running WITHOUT supervisor
        let obstacle = SphereGeofence::new("obstacle_1", Vec3::new(0.5, 0.5, 0.0), 0.15, 0.05);

        let mut sim =
            SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0)).with_geofences(vec![obstacle.clone()]);
        let mut controller = ReactiveController::new(None) // Unsupervised!
            .with_gains(160.0, 25.0);

        controller.set_action_proposal(DeliberativeActionProposal::MoveToPose {
            target: Vec3::new(1.0, 1.0, 0.0),
            max_speed: 1.0,
        });

        for _ in 0..150 {
            controller.step(&mut sim);
        }

        // The unsupervised baseline blindly penetrates the obstacle
        assert!(
            sim.total_violations > 0,
            "Unsupervised baseline must suffer geofence safety violations"
        );
    }

    #[test]
    fn test_dynamic_torque_limiting() {
        let max_torque = 30.0;
        let config = SafetyEnvelopeConfig {
            max_joint_torque: max_torque,
            ..Default::default()
        };
        let supervisor = SafetySupervisor::new(config);

        let mut sim = SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0)).with_torque_limit(max_torque);
        let mut controller = ReactiveController::new(Some(supervisor)).with_gains(300.0, 40.0); // Very high gains demanding high torques

        controller.set_action_proposal(DeliberativeActionProposal::MoveToPose {
            target: Vec3::new(2.0, 2.0, 2.0),
            max_speed: 5.0,
        });

        for _ in 0..50 {
            let (state, _) = controller.step(&mut sim);
            for torque in &state.joints.torques {
                assert!(
                    torque.abs() <= max_torque + 1e-4,
                    "Joint torque {:.2} exceeded limit {:.2}",
                    torque,
                    max_torque
                );
            }
        }

        assert_eq!(sim.total_violations, 0);
        assert!(
            controller.total_interventions > 0,
            "Torque clamping interventions must be recorded"
        );
    }

    #[test]
    fn test_disturbance_rejection_under_external_force() {
        let supervisor = SafetySupervisor::new(SafetyEnvelopeConfig::default());
        let mut sim = SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0));
        let mut controller = ReactiveController::new(Some(supervisor)).with_gains(180.0, 28.0);

        controller.set_action_proposal(DeliberativeActionProposal::MoveToPose {
            target: Vec3::new(1.0, 0.5, 0.0),
            max_speed: 1.0,
        });

        // Apply external disturbance impulse from step 20 to 60
        for step in 0..120 {
            if (20..60).contains(&step) {
                sim.apply_disturbance(Vec3::new(8.0, -5.0, 2.0));
            } else {
                sim.apply_disturbance(Vec3::zero());
            }
            controller.step(&mut sim);
        }

        let report = controller.generate_telemetry_report(&sim, None);
        assert!(
            report.disturbance_rejection_rms_error < 0.15,
            "RMS tracking error under disturbance must remain bounded: {:.4}",
            report.disturbance_rejection_rms_error
        );
        assert_eq!(report.task_success_rate, 1.0);
    }

    #[test]
    fn test_reflexive_emergency_stop_upon_high_prediction_error() {
        let config = SafetyEnvelopeConfig {
            max_prediction_error_threshold: 0.20,
            ..Default::default()
        };
        let supervisor = SafetySupervisor::new(config);

        let mut sim = SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0));
        let mut controller = ReactiveController::new(Some(supervisor));

        controller.set_action_proposal(DeliberativeActionProposal::MoveToPose {
            target: Vec3::new(1.0, 1.0, 1.0),
            max_speed: 1.0,
        });

        // Step normally for 10 steps
        for _ in 0..10 {
            controller.step(&mut sim);
        }
        assert!(!sim.current_state.emergency_stop_latched);

        // Inject severe prediction error anomaly
        sim.inject_anomaly(0.42);

        let (state, interventions) = controller.step(&mut sim);

        assert!(
            state.emergency_stop_latched,
            "Emergency stop must be latched upon prediction error spike"
        );
        assert!(
            interventions.iter().any(|i| i.is_emergency_stop()),
            "Interventions must include EmergencyStopTriggered"
        );
        assert_eq!(controller.emergency_stops, 1);

        // Verify robot stays halted in subsequent steps
        let (next_state, _) = controller.step(&mut sim);
        assert!(next_state.emergency_stop_latched);
        assert_eq!(sim.velocity, Vec3::zero());
    }

    #[test]
    fn test_pick_and_place_lifecycle_with_telemetry() {
        let supervisor = SafetySupervisor::new(SafetyEnvelopeConfig::default());
        let mut sim = SimEnvironment::new(Vec3::new(0.0, 0.0, 0.0));
        let mut controller = ReactiveController::new(Some(supervisor));

        controller.set_action_proposal(DeliberativeActionProposal::PickAndPlace {
            pick_pos: Vec3::new(0.3, 0.3, 0.0),
            place_pos: Vec3::new(0.8, 0.8, 0.0),
            grip_force: 15.0,
        });

        // Run until completion
        for _ in 0..250 {
            controller.step(&mut sim);
            if controller.pick_and_place_phase == PickAndPlacePhase::Completed {
                break;
            }
        }

        assert_eq!(
            controller.pick_and_place_phase,
            PickAndPlacePhase::Completed
        );
        let report = controller.generate_telemetry_report(&sim, None);
        assert_eq!(report.task_success_rate, 1.0);
        assert_eq!(report.safety_boundary_violations, 0);

        let formatted = report.format_ascii_report();
        assert!(formatted.contains("STRATA EMBODIED TELEMETRY REPORT"));
        assert!(formatted.contains("100.0%"));
    }
}
