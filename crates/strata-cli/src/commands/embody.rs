use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use serde_json::json;

use strata_reasoning::embodiment::{
    DeliberativeActionProposal, JointState, ReactiveController, SafetyEnvelopeConfig,
    SafetySupervisor, SimEnvironment, SphereGeofence, Vec3,
};

#[derive(Debug, Clone, Args)]
pub struct EmbodyArgs {
    #[command(subcommand)]
    pub command: EmbodyCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum EmbodyCommand {
    /// Run a 100 Hz reactive control & safety envelope simulation
    #[command(name = "simulate", alias = "sim")]
    Simulate {
        /// Action mode: 'move' (MoveToPose) or 'pick-and-place' (PickAndPlace)
        #[arg(
            long,
            default_value = "move",
            help = "Action mode: 'move' or 'pick-and-place'"
        )]
        mode: String,

        /// Target Cartesian coordinates 'x,y,z' (used as place_pos in pick-and-place)
        #[arg(long, default_value = "1.0,1.0,0.0", help = "Target pose 'x,y,z'")]
        target: String,

        /// Pick Cartesian coordinates 'x,y,z' (only for pick-and-place mode)
        #[arg(long, default_value = "0.3,0.3,0.0", help = "Pick pose 'x,y,z'")]
        pick: String,

        /// Optional spherical keep-out obstacle 'x,y,z,radius,margin'
        #[arg(
            long,
            help = "Optional spherical keep-out geofence 'x,y,z,radius,margin' (e.g. '0.5,0.5,0.0,0.15,0.05')"
        )]
        obstacle: Option<String>,

        /// Maximum allowable joint torque (Nm)
        #[arg(
            long,
            default_value_t = 50.0,
            help = "Maximum joint torque ceiling (Nm)"
        )]
        max_torque: f64,

        /// Maximum prediction error threshold before reflexive e-stop
        #[arg(
            long,
            default_value_t = 0.25,
            help = "Prediction error threshold for reflexive emergency stop"
        )]
        max_prediction_error: f64,

        /// Optional external disturbance force 'fx,fy,fz' applied during steps 20..60
        #[arg(long, help = "External disturbance force 'fx,fy,fz'")]
        disturbance: Option<String>,

        /// Optional injected prediction error anomaly at step 10
        #[arg(
            long,
            help = "Inject prediction error anomaly to test reflexive e-stop"
        )]
        inject_anomaly: Option<f64>,

        /// Number of 100 Hz control steps to simulate
        #[arg(
            long,
            default_value_t = 250,
            help = "Simulation steps at 100 Hz (10ms per step)"
        )]
        steps: usize,

        /// Disable SafetySupervisor (unsupervised baseline)
        #[arg(long, help = "Run without SafetySupervisor for baseline comparison")]
        unsupervised: bool,

        /// Output telemetry report as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },

    /// Supervise and verify a single candidate command against safety envelopes
    #[command(name = "check", alias = "guard")]
    Check {
        /// Current end-effector position 'x,y,z'
        #[arg(long, default_value = "0.0,0.0,0.0", help = "Current position 'x,y,z'")]
        current_pos: String,

        /// Proposed target position 'x,y,z'
        #[arg(
            long,
            default_value = "0.5,0.5,0.0",
            help = "Proposed target position 'x,y,z'"
        )]
        target_pos: String,

        /// Proposed joint torques 't1,t2,t3' (Nm)
        #[arg(
            long,
            default_value = "10.0,10.0,10.0",
            help = "Proposed joint torques 't1,t2,t3'"
        )]
        torques: String,

        /// Optional spherical keep-out obstacle 'x,y,z,radius,margin'
        #[arg(
            long,
            help = "Optional spherical keep-out geofence 'x,y,z,radius,margin'"
        )]
        obstacle: Option<String>,

        /// Maximum allowable joint torque (Nm)
        #[arg(
            long,
            default_value_t = 50.0,
            help = "Maximum joint torque ceiling (Nm)"
        )]
        max_torque: f64,

        /// Observed prediction error
        #[arg(long, default_value_t = 0.0, help = "Observed prediction error")]
        prediction_error: f64,

        /// Maximum prediction error threshold before reflexive e-stop
        #[arg(long, default_value_t = 0.25, help = "Prediction error threshold")]
        max_prediction_error: f64,

        /// Output as raw JSON
        #[arg(long, help = "Output as raw JSON")]
        json: bool,
    },
}

pub fn parse_vec3(s: &str) -> Result<Vec3> {
    let parts: Vec<f64> = s
        .split(',')
        .map(|p| {
            p.trim()
                .parse::<f64>()
                .map_err(|e| anyhow!("Invalid float '{}' in Vec3 '{}': {}", p, s, e))
        })
        .collect::<Result<Vec<f64>>>()?;
    if parts.len() != 3 {
        return Err(anyhow!(
            "Expected 3 comma-separated floats 'x,y,z', got '{}'",
            s
        ));
    }
    Ok(Vec3::new(parts[0], parts[1], parts[2]))
}

pub fn parse_sphere_geofence(s: &str) -> Result<SphereGeofence> {
    let parts: Vec<f64> = s
        .split(',')
        .map(|p| {
            p.trim()
                .parse::<f64>()
                .map_err(|e| anyhow!("Invalid float '{}' in obstacle '{}': {}", p, s, e))
        })
        .collect::<Result<Vec<f64>>>()?;
    if parts.len() != 5 {
        return Err(anyhow!(
            "Expected 5 comma-separated floats 'x,y,z,radius,margin', got '{}'",
            s
        ));
    }
    Ok(SphereGeofence::new(
        "keep_out_obstacle",
        Vec3::new(parts[0], parts[1], parts[2]),
        parts[3],
        parts[4],
    ))
}

pub async fn run_embody(args: EmbodyArgs) -> Result<()> {
    match args.command {
        EmbodyCommand::Simulate {
            mode,
            target,
            pick,
            obstacle,
            max_torque,
            max_prediction_error,
            disturbance,
            inject_anomaly,
            steps,
            unsupervised,
            json,
        } => {
            let target_vec = parse_vec3(&target)?;
            let pick_vec = parse_vec3(&pick)?;
            let dist_vec = match &disturbance {
                Some(d) => Some(parse_vec3(d)?),
                None => None,
            };
            let mut zones = Vec::new();
            if let Some(obs_str) = &obstacle {
                zones.push(parse_sphere_geofence(obs_str)?);
            }

            let config = SafetyEnvelopeConfig {
                max_joint_torque: max_torque,
                max_prediction_error_threshold: max_prediction_error,
                keep_out_zones: zones.clone(),
                ..Default::default()
            };

            let supervisor = if unsupervised {
                None
            } else {
                Some(SafetySupervisor::new(config))
            };

            let mut sim = SimEnvironment::new(Vec3::zero())
                .with_geofences(zones)
                .with_torque_limit(max_torque);
            let mut controller = ReactiveController::new(supervisor).with_gains(160.0, 25.0);

            let proposal = if mode.eq_ignore_ascii_case("pick-and-place")
                || mode.eq_ignore_ascii_case("pick_and_place")
            {
                DeliberativeActionProposal::PickAndPlace {
                    pick_pos: pick_vec,
                    place_pos: target_vec,
                    grip_force: 15.0,
                }
            } else {
                DeliberativeActionProposal::MoveToPose {
                    target: target_vec,
                    max_speed: 1.0,
                }
            };
            controller.set_action_proposal(proposal);

            for step in 0..steps {
                if let Some(d) = dist_vec {
                    if (20..60).contains(&step) {
                        sim.apply_disturbance(d);
                    } else {
                        sim.apply_disturbance(Vec3::zero());
                    }
                }
                if let Some(anom) = inject_anomaly {
                    if step == 10 {
                        sim.inject_anomaly(anom);
                    }
                }
                controller.step(&mut sim);
            }

            let report = controller.generate_telemetry_report(&sim, None);
            let payload = json!({
                "mode": mode,
                "supervised": !unsupervised,
                "steps_executed": sim.step_count,
                "final_position": sim.current_state.end_effector_pos,
                "emergency_stop_latched": sim.current_state.emergency_stop_latched,
                "telemetry": report,
            });

            if json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("\n{}", report.format_ascii_report());
                println!(
                    "  Final End-Effector Pose : {}",
                    sim.current_state.end_effector_pos
                );
                println!(
                    "  Emergency Stop Latched  : {}",
                    sim.current_state.emergency_stop_latched
                );
                println!();
            }
            Ok(())
        }

        EmbodyCommand::Check {
            current_pos,
            target_pos,
            torques,
            obstacle,
            max_torque,
            prediction_error,
            max_prediction_error,
            json,
        } => {
            let cur = parse_vec3(&current_pos)?;
            let tgt = parse_vec3(&target_pos)?;
            let torque_vec = parse_vec3(&torques)?;
            let mut zones = Vec::new();
            if let Some(obs_str) = &obstacle {
                zones.push(parse_sphere_geofence(obs_str)?);
            }

            let config = SafetyEnvelopeConfig {
                max_joint_torque: max_torque,
                max_prediction_error_threshold: max_prediction_error,
                keep_out_zones: zones,
                ..Default::default()
            };
            let mut supervisor = SafetySupervisor::new(config);
            let proposed_joints = JointState::new(
                vec![tgt.x, tgt.y, tgt.z],
                vec![0.0, 0.0, 0.0],
                vec![torque_vec.x, torque_vec.y, torque_vec.z],
            );

            let (safe_target, safe_joints, interventions, e_stop_latched) = supervisor
                .supervise_command(
                    cur,
                    Vec3::zero(),
                    tgt,
                    proposed_joints,
                    prediction_error,
                    false,
                    0.01,
                );

            let payload = json!({
                "current_pos": cur,
                "proposed_target": tgt,
                "safe_target": safe_target,
                "safe_torques": safe_joints.torques,
                "interventions": interventions,
                "emergency_stop_latched": e_stop_latched,
            });

            if json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("\n🛡️  STRATA SAFETY SUPERVISOR COMMAND CHECK");
                println!("════════════════════════════════════════════════════════");
                println!("  Current Position       : {cur}");
                println!("  Proposed Target        : {tgt}");
                println!("  Supervised Safe Target : {safe_target}");
                println!("  Supervised Torques (Nm): {:?}", safe_joints.torques);
                println!("  Interventions Triggered: {:?}", interventions);
                println!("  Emergency Stop Latched : {e_stop_latched}\n");
            }
            Ok(())
        }
    }
}
