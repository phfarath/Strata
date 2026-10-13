use anyhow::{bail, Result};
use strata_memory::{Consolidator, SqliteStore};

/// One simulated agent attempt: `Some(error)` is a failed tool call, `None` is a success.
struct Attempt {
    error: Option<String>,
}

struct Trace {
    name: &'static str,
    /// Ground truth: should a futility signal fire on this trace?
    is_loop: bool,
    attempts: Vec<Attempt>,
}

fn fail(msg: impl Into<String>) -> Attempt {
    Attempt {
        error: Some(msg.into()),
    }
}

fn ok() -> Attempt {
    Attempt { error: None }
}

fn traces() -> Vec<Trace> {
    let words = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu",
    ];
    vec![
        // Same failure over and over. A futility signal is expected.
        Trace {
            name: "identical_loop",
            is_loop: true,
            attempts: (0..12)
                .map(|_| fail("error[E0599]: no method named `foo` found"))
                .collect(),
        },
        // Many different failures, then success: real progress, no signal expected.
        Trace {
            name: "legit_iteration",
            is_loop: false,
            attempts: words[..8]
                .iter()
                .map(|w| fail(format!("error: unresolved import {w}")))
                .chain(std::iter::once(ok()))
                .collect(),
        },
        // Same underlying dead end, but the error text drifts each time (different
        // module names), so each failure gets a distinct signature.
        Trace {
            name: "drifting_loop",
            is_loop: true,
            attempts: words
                .iter()
                .map(|w| fail(format!("cannot find module {w} in scope")))
                .collect(),
        },
        // Flaky tool: three identical timeouts, then it works.
        Trace {
            name: "flaky_then_success",
            is_loop: false,
            attempts: vec![
                fail("request timed out after 30s"),
                fail("request timed out after 30s"),
                fail("request timed out after 30s"),
                ok(),
            ],
        },
    ]
}

/// Stop rule the current system can support: stop once the same failure signature has
/// been seen `BASELINE_OCCURRENCE_THRESHOLD` times. Strata has no built-in stop signal
/// today, so this is the best comparator derived from `FailurePattern.occurrences`.
const BASELINE_OCCURRENCE_THRESHOLD: u64 = 3;

/// Returns the 1-based attempt at which the baseline would stop, if any.
fn baseline_first_stop(trace: &Trace) -> Result<Option<usize>> {
    let store = SqliteStore::open_in_memory()?;
    let consolidator = Consolidator::new();
    for (i, attempt) in trace.attempts.iter().enumerate() {
        if let Some(err) = &attempt.error {
            let pattern = consolidator.record_tool_failure(&store, "safe_shell", err, "", None)?;
            if pattern.occurrences >= BASELINE_OCCURRENCE_THRESHOLD {
                return Ok(Some(i + 1));
            }
        }
    }
    Ok(None)
}

/// Baseline for the futility accumulator experiment (glia-inspired, see docs/10-research-roadmap.md).
///
/// Measures how well a plain occurrence-count rule detects agent loops, so that a
/// candidate mechanism has a number to beat. Predefined success criteria for the
/// candidate (not enforced here): detect every loop trace within 5 attempts and
/// produce zero stops on the non-loop traces.
pub async fn run_agent_loop_futility_baseline_scenario() -> Result<()> {
    println!("\n🔁 Running Eval Scenario: Agent Loop Futility Baseline");

    let mut detected = 0;
    let mut missed = 0;
    let mut false_stops = 0;

    for trace in traces() {
        let stop = baseline_first_stop(&trace)?;
        println!(
            "  [{}] loop={} first_stop={:?}",
            trace.name, trace.is_loop, stop
        );
        match (trace.is_loop, stop) {
            (true, Some(_)) => detected += 1,
            (true, None) => missed += 1,
            (false, Some(_)) => false_stops += 1,
            (false, None) => {}
        }
    }

    println!("  Baseline: detected={detected} missed={missed} false_stops={false_stops}");

    // The baseline catches identical loops, misses drifting ones, and falsely stops
    // a flaky tool that would have recovered. These numbers are what a candidate must improve on.
    if (detected, missed, false_stops) != (1, 1, 1) {
        bail!("Unexpected baseline: detected={detected} missed={missed} false_stops={false_stops} (expected 1/1/1)");
    }
    Ok(())
}
