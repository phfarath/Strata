use anyhow::{bail, Result};
use strata_memory::{
    Consolidator, FutilityAccumulator, FutilitySignal, FutilityVerdict, SqliteStore,
};

/// One simulated step of an agent working on a task.
enum Step {
    /// Failed tool call with its raw error text.
    Fail(String),
    Ok,
    /// Forward movement that is not a success (e.g. a file edit between retries).
    Progress,
}

struct Trace {
    name: &'static str,
    /// Ground truth: should a futility signal fire on this trace?
    is_loop: bool,
    steps: Vec<Step>,
}

fn fail(msg: impl Into<String>) -> Step {
    Step::Fail(msg.into())
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
            steps: (0..12)
                .map(|_| fail("error[E0599]: no method named `foo` found"))
                .collect(),
        },
        // Many different failures with an edit between each, then success: real progress.
        Trace {
            name: "legit_iteration",
            is_loop: false,
            steps: words[..8]
                .iter()
                .flat_map(|w| {
                    [
                        fail(format!("error: unresolved import {w}")),
                        Step::Progress,
                    ]
                })
                .chain(std::iter::once(Step::Ok))
                .collect(),
        },
        // Same underlying dead end, but the error text drifts each time (different
        // module names) and nothing changes between attempts.
        Trace {
            name: "drifting_loop",
            is_loop: true,
            steps: words
                .iter()
                .map(|w| fail(format!("cannot find module {w} in scope")))
                .collect(),
        },
        // Flaky tool: three identical timeouts, then it works.
        Trace {
            name: "flaky_then_success",
            is_loop: false,
            steps: vec![
                fail("request timed out after 30s"),
                fail("request timed out after 30s"),
                fail("request timed out after 30s"),
                Step::Ok,
            ],
        },
    ]
}

/// Stop rule the current system can support: stop once the same failure signature has
/// been seen `BASELINE_OCCURRENCE_THRESHOLD` times. Strata has no built-in stop signal
/// today, so this is the best comparator derived from `FailurePattern.occurrences`.
const BASELINE_OCCURRENCE_THRESHOLD: u64 = 3;

/// A candidate may stop a loop trace no later than this attempt.
const MAX_ATTEMPTS_TO_DETECT: usize = 5;

/// Returns the 1-based step at which the baseline would stop, if any.
fn baseline_first_stop(trace: &Trace) -> Result<Option<usize>> {
    let store = SqliteStore::open_in_memory()?;
    let consolidator = Consolidator::new();
    for (i, step) in trace.steps.iter().enumerate() {
        if let Step::Fail(err) = step {
            let pattern = consolidator.record_tool_failure(&store, "safe_shell", err, "", None)?;
            if pattern.occurrences >= BASELINE_OCCURRENCE_THRESHOLD {
                return Ok(Some(i + 1));
            }
        }
    }
    Ok(None)
}

/// Returns the 1-based failed-attempt count at which the accumulator first signals futility.
fn accumulator_first_stop(trace: &Trace) -> Option<usize> {
    let mut acc = FutilityAccumulator::new();
    let mut attempts = 0;
    for step in &trace.steps {
        let signal = match step {
            Step::Fail(err) => {
                attempts += 1;
                // Same signature source as the baseline: tool + normalized error.
                FutilitySignal::Failure(Consolidator::failure_signature("safe_shell", err))
            }
            Step::Ok => FutilitySignal::Success,
            Step::Progress => FutilitySignal::Progress,
        };
        if acc.observe(signal) == FutilityVerdict::Futile {
            return Some(attempts);
        }
    }
    None
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Score {
    detected: usize,
    missed: usize,
    false_stops: usize,
    slowest_detection: usize,
}

fn score(trace_stops: &[(bool, Option<usize>)]) -> Score {
    let mut s = Score::default();
    for (is_loop, stop) in trace_stops {
        match (is_loop, stop) {
            (true, Some(at)) => {
                s.detected += 1;
                s.slowest_detection = s.slowest_detection.max(*at);
            }
            (true, None) => s.missed += 1,
            (false, Some(_)) => s.false_stops += 1,
            (false, None) => {}
        }
    }
    s
}

/// A/B eval for the futility accumulator (glia-inspired, see docs/10-research-roadmap.md).
///
/// Compares a plain occurrence-count rule (baseline) with `FutilityAccumulator` on
/// synthetic agent traces. Success criteria were fixed before implementing the
/// candidate: detect every loop within `MAX_ATTEMPTS_TO_DETECT` attempts and produce
/// zero stops on non-loop traces.
pub async fn run_agent_loop_futility_baseline_scenario() -> Result<()> {
    println!("\n🔁 Running Eval Scenario: Agent Loop Futility (baseline vs accumulator)");

    let mut baseline = Vec::new();
    let mut candidate = Vec::new();
    for trace in traces() {
        let b = baseline_first_stop(&trace)?;
        let c = accumulator_first_stop(&trace);
        println!(
            "  [{}] loop={} baseline={:?} accumulator={:?}",
            trace.name, trace.is_loop, b, c
        );
        baseline.push((trace.is_loop, b));
        candidate.push((trace.is_loop, c));
    }

    let (b, c) = (score(&baseline), score(&candidate));
    println!("  Baseline:    {b:?}");
    println!("  Accumulator: {c:?}");

    // Baseline numbers are pinned so a change in the existing failure path is noticed.
    if (b.detected, b.missed, b.false_stops) != (1, 1, 1) {
        bail!("Unexpected baseline: {b:?} (expected detected=1 missed=1 false_stops=1)");
    }
    if c.missed != 0 || c.false_stops != 0 || c.slowest_detection > MAX_ATTEMPTS_TO_DETECT {
        bail!("Accumulator misses success criteria: {c:?}");
    }
    Ok(())
}
