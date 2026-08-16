pub mod scenarios;

pub use scenarios::*;

use anyhow::Result;

/// Runs the complete evaluation suite deterministically
pub async fn run_all_scenarios() -> Result<()> {
    println!("\n========================================================");
    println!("🧪 STRATA COGNITIVE RUNTIME — DETERMINISTIC EVAL SUITE");
    println!("========================================================");

    run_cross_host_transfer_scenario().await?;
    run_silent_failure_avoidance_scenario().await?;

    println!("\n========================================================");
    println!("🎉 ALL EVAL SCENARIOS PASSED (2/2)");
    println!("========================================================\n");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_cross_host_transfer() {
        run_cross_host_transfer_scenario()
            .await
            .expect("Cross-host transfer scenario failed");
    }

    #[tokio::test]
    async fn test_eval_silent_failure_avoidance() {
        run_silent_failure_avoidance_scenario()
            .await
            .expect("Silent failure avoidance scenario failed");
    }
}
