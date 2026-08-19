use anyhow::Result;
use clap::Args;
use strata_reasoning::{DagScheduler, GoalDecomposer};

#[derive(Debug, Args, Clone)]
pub struct PlanArgs {
    /// High-level engineering objective or long-horizon task (e.g. "Refactor memory engine and add Redis sync")
    #[arg(value_name = "GOAL")]
    pub goal: Option<String>,

    /// Automatically execute the decomposed Goal DAG wave-by-wave
    #[arg(short = 'e', long)]
    pub execute: bool,

    /// Maximum parallel concurrency during execution (default: 4)
    #[arg(short = 'c', long, default_value_t = 4)]
    pub concurrency: usize,

    /// Disable automatic failure recovery and dynamic DAG patching
    #[arg(long)]
    pub no_recover: bool,

    /// Output full plan and execution report in JSON format
    #[arg(long)]
    pub json: bool,
}

pub async fn run_plan(args: PlanArgs) -> Result<()> {
    let goal = args
        .goal
        .unwrap_or_else(|| "Refactor memory engine and add Redis sync".to_string());

    let decomposer = GoalDecomposer::new();
    let dag = decomposer.decompose(&goal)?;

    if !args.execute {
        if args.json {
            let waves = dag.compute_waves()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "goal": goal,
                    "total_nodes": dag.node_count(),
                    "total_waves": waves.len(),
                    "waves": waves,
                    "dag": dag.export(),
                }))?
            );
        } else {
            render_plan_preview(&goal, &dag);
        }
        return Ok(());
    }

    // Execution mode
    if !args.json {
        println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
        println!("║            🚀 STRATA HIERARCHICAL GOAL DAG SCHEDULER & RUNTIME               ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════╝\n");
        println!("🎯 TARGET OBJECTIVE: \x1b[1;36m{}\x1b[0m", goal);
        println!("⚡ CONCURRENCY CAP:  {} parallel workers", args.concurrency);
        println!("🛡️  AUTO-RECOVERY:    {}", if !args.no_recover { "ENABLED (Dynamic DAG patching)" } else { "DISABLED" });
        println!("\n⏳ Executing waves asynchronously...\n");
    }

    let scheduler = DagScheduler::new()
        .with_concurrency(args.concurrency)
        .with_auto_recover(!args.no_recover);

    let (finished_dag, report) = scheduler.execute(dag).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_execution_report(&report, &finished_dag);
    }

    Ok(())
}

fn render_plan_preview(goal: &str, dag: &strata_reasoning::GoalDag) {
    println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║              📋 STRATA HIERARCHICAL GOAL DECOMPOSITION PLAN                  ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝\n");
    println!("🎯 TARGET OBJECTIVE: \x1b[1;36m{}\x1b[0m", goal);
    println!("📊 DECOMPOSITION:    {} goals across topological waves", dag.node_count());
    println!("{}", dag.to_ascii_tree());
    println!("💡 TIP: Run with \x1b[1;32m--execute\x1b[0m (or \x1b[1;32m-e\x1b[0m) to execute this Goal DAG wave-by-wave.\n");
}

fn render_execution_report(
    report: &strata_reasoning::DagExecutionReport,
    finished_dag: &strata_reasoning::GoalDag,
) {
    println!("{}", finished_dag.to_ascii_tree());

    println!("────────────────────────────────────────────────────────────────────────────────");
    println!("📊 DAG EXECUTION METRICS & SUMMARY");
    println!("────────────────────────────────────────────────────────────────────────────────");
    println!("🆔 PLAN ID:           {}", report.plan_id);
    println!("🌊 EXECUTED WAVES:    {}", report.total_waves);
    println!("✅ COMPLETED GOALS:   \x1b[1;32m{}\x1b[0m / {}", report.completed_nodes, report.total_nodes);
    println!("❌ FAILED GOALS:      {}", if report.failed_nodes > 0 { format!("\x1b[1;31m{}\x1b[0m", report.failed_nodes) } else { "0".to_string() });
    println!("⏭️  SKIPPED GOALS:     {}", report.skipped_nodes);
    println!("🔄 RECOVERY ATTEMPTS: {}", report.recovery_attempts);
    println!("⏱️  TOTAL DURATION:    {} ms", report.duration_ms);

    if report.success {
        println!("\n🎉 VERDICT: \x1b[1;32mALL GOALS AND VERIFICATION GATES PASSED (100% SUCCESS)\x1b[0m\n");
    } else {
        println!("\n⚠️  VERDICT: \x1b[1;31mEXECUTION FAILED OR HALTED WITH UNRECOVERED GOALS\x1b[0m\n");
    }
}
