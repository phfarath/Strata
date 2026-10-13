use crate::store::SqliteStore;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use strata_core::errors::StrataError;

/// Evidence units are integers so the accumulator is exactly reproducible (no float drift).
const NOVEL_FAILURE_EVIDENCE: u32 = 7;
const REPEAT_FAILURE_EVIDENCE: u32 = 10;
const FUTILITY_THRESHOLD: u32 = 30;
/// A progress signal keeps this fraction (num/den) of the accumulated evidence.
const PROGRESS_KEEP_NUM: u32 = 1;
const PROGRESS_KEEP_DEN: u32 = 4;
/// How many recent failure signatures count as "the same failure again".
const RECENT_SIGNATURES: usize = 8;

/// One observation of an agent working on a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// A failed action, identified by a normalized failure signature.
    Failure(String),
    /// A successful action.
    Success,
    /// Evidence of forward movement that is not itself a success (new artifact, edited file, ...).
    Progress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    /// Evidence just crossed the threshold: switch strategy or escalate.
    Futile,
    /// Already signalled; stays latched until progress or success resets it.
    StillFutile,
}

/// Leaky evidence integrator that decides when failures without progress mean the
/// current approach is a dead end. Track one per (agent, task).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FutilityAccumulator {
    evidence: u32,
    latched: bool,
    recent: VecDeque<String>,
}

impl FutilityAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn evidence(&self) -> u32 {
        self.evidence
    }

    pub fn observe(&mut self, signal: Signal) -> Verdict {
        match signal {
            Signal::Failure(sig) => {
                let repeat = self.recent.contains(&sig);
                self.recent.push_back(sig);
                if self.recent.len() > RECENT_SIGNATURES {
                    self.recent.pop_front();
                }
                let add = if repeat {
                    REPEAT_FAILURE_EVIDENCE
                } else {
                    NOVEL_FAILURE_EVIDENCE
                };
                // Cap so a long loop cannot bury the agent in evidence that progress can't clear.
                self.evidence = (self.evidence + add).min(2 * FUTILITY_THRESHOLD);
            }
            Signal::Progress => {
                self.evidence = self.evidence * PROGRESS_KEEP_NUM / PROGRESS_KEEP_DEN;
            }
            Signal::Success => {
                self.evidence = 0;
                self.recent.clear();
            }
        }

        if self.evidence >= FUTILITY_THRESHOLD {
            if self.latched {
                Verdict::StillFutile
            } else {
                self.latched = true;
                Verdict::Futile
            }
        } else {
            self.latched = false;
            Verdict::Continue
        }
    }
}

/// Idle time after which an agent's accumulated evidence is discarded.
pub const IDLE_TTL_SECS: i64 = 1800;

/// One recorded observation, kept so verdicts can be audited and thresholds calibrated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FutilityTraceRow {
    /// "failure", "success" or "neutral".
    pub signal: String,
    pub tool: String,
    pub signature: String,
    pub verdict: String,
    pub evidence: u32,
    pub ts: i64,
}

fn verdict_name(v: Verdict) -> &'static str {
    match v {
        Verdict::Continue => "continue",
        Verdict::Futile => "futile",
        Verdict::StillFutile => "still_futile",
    }
}

/// Shadow mode: feeds a failure into the persisted accumulator and records the verdict.
/// The verdict is only stored, never surfaced to the agent.
pub fn record_shadow_failure(
    store: &SqliteStore,
    agent_id: &str,
    task_key: &str,
    tool: &str,
    signature: &str,
    now: i64,
) -> Result<Verdict, StrataError> {
    let mut acc = store.load_futility_state(agent_id, task_key, now, IDLE_TTL_SECS)?;
    let verdict = acc.observe(Signal::Failure(signature.to_string()));
    store.save_futility_state(agent_id, task_key, &acc, now)?;
    store.append_futility_trace(
        agent_id,
        task_key,
        &FutilityTraceRow {
            signal: "failure".into(),
            tool: tool.into(),
            signature: signature.into(),
            verdict: verdict_name(verdict).into(),
            evidence: acc.evidence(),
            ts: now,
        },
    )?;
    Ok(verdict)
}

/// Shadow mode for a command that exited 0. It only counts as a recovery (and resets the
/// evidence) when the same `tool` was the last thing to fail for this (agent, task);
/// any other success is logged as neutral so unrelated successful calls cannot hide a loop.
pub fn record_shadow_success(
    store: &SqliteStore,
    agent_id: &str,
    task_key: &str,
    tool: &str,
    now: i64,
) -> Result<Verdict, StrataError> {
    let recent = store.list_futility_trace(agent_id, task_key, 50)?;
    let recovered = recent
        .iter()
        .rev()
        .find(|r| r.signal == "failure")
        .is_some_and(|r| r.tool == tool);

    let mut acc = store.load_futility_state(agent_id, task_key, now, IDLE_TTL_SECS)?;
    let (signal, verdict) = if recovered {
        ("success", acc.observe(Signal::Success))
    } else if acc.latched {
        ("neutral", Verdict::StillFutile)
    } else {
        ("neutral", Verdict::Continue)
    };
    if recovered {
        store.save_futility_state(agent_id, task_key, &acc, now)?;
    }
    store.append_futility_trace(
        agent_id,
        task_key,
        &FutilityTraceRow {
            signal: signal.into(),
            tool: tool.into(),
            signature: String::new(),
            verdict: verdict_name(verdict).into(),
            evidence: acc.evidence(),
            ts: now,
        },
    )?;
    Ok(verdict)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn fail(s: &str) -> Signal {
        Signal::Failure(s.to_string())
    }

    fn first_futile(acc: &mut FutilityAccumulator, signals: Vec<Signal>) -> Option<usize> {
        signals
            .into_iter()
            .enumerate()
            .find_map(|(i, s)| (acc.observe(s) == Verdict::Futile).then_some(i + 1))
    }

    #[test]
    fn identical_failures_trip_within_five() {
        let mut acc = FutilityAccumulator::new();
        let signals = (0..12).map(|_| fail("a")).collect();
        let at = first_futile(&mut acc, signals).expect("loop must be detected");
        assert!(at <= 5, "tripped too late: {at}");
    }

    #[test]
    fn drifting_failures_without_progress_trip_within_five() {
        let mut acc = FutilityAccumulator::new();
        let signals = (0..12).map(|i| fail(&format!("sig{i}"))).collect();
        let at = first_futile(&mut acc, signals).expect("drifting loop must be detected");
        assert!(at <= 5, "tripped too late: {at}");
    }

    #[test]
    fn progress_between_failures_never_trips() {
        let mut acc = FutilityAccumulator::new();
        let mut signals = Vec::new();
        for i in 0..20 {
            signals.push(fail(&format!("sig{i}")));
            signals.push(Signal::Progress);
        }
        assert_eq!(first_futile(&mut acc, signals), None);
    }

    #[test]
    fn flaky_tool_then_success_does_not_trip() {
        let mut acc = FutilityAccumulator::new();
        let signals = vec![fail("t"), fail("t"), fail("t"), Signal::Success];
        assert_eq!(first_futile(&mut acc, signals), None);
        assert_eq!(acc.evidence(), 0);
    }

    #[test]
    fn verdict_latches_then_resets_on_progress() {
        let mut acc = FutilityAccumulator::new();
        let mut verdicts = Vec::new();
        for _ in 0..6 {
            verdicts.push(acc.observe(fail("a")));
        }
        assert_eq!(
            verdicts.iter().filter(|v| **v == Verdict::Futile).count(),
            1
        );
        assert_eq!(*verdicts.last().unwrap(), Verdict::StillFutile);

        assert_eq!(acc.observe(Signal::Progress), Verdict::Continue);
        // After reset a single failure must not immediately re-trip.
        assert_eq!(acc.observe(fail("a")), Verdict::Continue);
    }

    fn fail_sig(s: &str) -> Signal {
        Signal::Failure(s.to_string())
    }

    #[test]
    fn state_roundtrips_through_store_and_keeps_behavior() {
        let store = crate::SqliteStore::open_in_memory().unwrap();
        let mut acc = FutilityAccumulator::new();
        for _ in 0..3 {
            acc.observe(fail_sig("a"));
        }
        store
            .save_futility_state("agent", "task", &acc, 1_000)
            .unwrap();

        let mut loaded = store
            .load_futility_state("agent", "task", 1_010, 1_800)
            .unwrap();
        assert_eq!(loaded.evidence(), acc.evidence());
        // Same next observation must give the same verdict as the original.
        assert_eq!(loaded.observe(fail_sig("a")), acc.observe(fail_sig("a")));
    }

    #[test]
    fn state_is_isolated_per_agent_and_task() {
        let store = crate::SqliteStore::open_in_memory().unwrap();
        let mut acc = FutilityAccumulator::new();
        acc.observe(fail_sig("a"));
        store
            .save_futility_state("agent-1", "task-1", &acc, 1_000)
            .unwrap();

        for (agent, task) in [("agent-2", "task-1"), ("agent-1", "task-2")] {
            let other = store
                .load_futility_state(agent, task, 1_000, 1_800)
                .unwrap();
            assert_eq!(other.evidence(), 0);
        }
    }

    #[test]
    fn idle_state_expires_and_missing_state_is_fresh() {
        let store = crate::SqliteStore::open_in_memory().unwrap();
        assert_eq!(
            store
                .load_futility_state("a", "t", 0, 1_800)
                .unwrap()
                .evidence(),
            0
        );

        let mut acc = FutilityAccumulator::new();
        acc.observe(fail_sig("a"));
        store.save_futility_state("a", "t", &acc, 1_000).unwrap();
        assert!(
            store
                .load_futility_state("a", "t", 2_800, 1_800)
                .unwrap()
                .evidence()
                > 0
        );
        assert_eq!(
            store
                .load_futility_state("a", "t", 2_801, 1_800)
                .unwrap()
                .evidence(),
            0
        );
    }

    #[test]
    fn save_overwrites_previous_state() {
        let store = crate::SqliteStore::open_in_memory().unwrap();
        let mut acc = FutilityAccumulator::new();
        acc.observe(fail_sig("a"));
        store.save_futility_state("a", "t", &acc, 1_000).unwrap();
        acc.observe(Signal::Success);
        store.save_futility_state("a", "t", &acc, 1_001).unwrap();
        assert_eq!(
            store
                .load_futility_state("a", "t", 1_002, 1_800)
                .unwrap()
                .evidence(),
            0
        );
    }

    #[test]
    fn shadow_failures_persist_across_calls_and_trip_once() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut verdicts = Vec::new();
        for i in 0..6 {
            verdicts
                .push(record_shadow_failure(&store, "a", "t", "cargo", "sig", 100 + i).unwrap());
        }
        assert_eq!(
            verdicts.iter().filter(|v| **v == Verdict::Futile).count(),
            1
        );
        assert_eq!(
            verdicts[3],
            Verdict::Futile,
            "identical failures trip on the 4th"
        );

        let trace = store.list_futility_trace("a", "t", 50).unwrap();
        assert_eq!(trace.len(), 6);
        assert_eq!(trace[3].verdict, "futile");
        assert_eq!(trace[0].verdict, "continue");
        assert_eq!(trace[0].signal, "failure");
    }

    #[test]
    fn shadow_success_of_failed_tool_resets_but_other_success_is_neutral() {
        let store = SqliteStore::open_in_memory().unwrap();
        for i in 0..3 {
            record_shadow_failure(&store, "a", "t", "cargo test", "sig", i).unwrap();
        }
        let before = store
            .load_futility_state("a", "t", 10, IDLE_TTL_SECS)
            .unwrap()
            .evidence();
        assert!(before > 0);

        record_shadow_success(&store, "a", "t", "ls", 11).unwrap();
        let after_ls = store
            .load_futility_state("a", "t", 12, IDLE_TTL_SECS)
            .unwrap()
            .evidence();
        assert_eq!(
            after_ls, before,
            "unrelated success must not reset evidence"
        );

        record_shadow_success(&store, "a", "t", "cargo test", 13).unwrap();
        let after_fix = store
            .load_futility_state("a", "t", 14, IDLE_TTL_SECS)
            .unwrap()
            .evidence();
        assert_eq!(after_fix, 0, "same command passing is a recovery");

        let signals: Vec<String> = store
            .list_futility_trace("a", "t", 50)
            .unwrap()
            .into_iter()
            .map(|r| r.signal)
            .collect();
        assert_eq!(
            signals,
            ["failure", "failure", "failure", "neutral", "success"]
        );
    }

    #[test]
    fn trace_is_isolated_per_agent_and_task_and_limited() {
        let store = SqliteStore::open_in_memory().unwrap();
        for i in 0..5 {
            record_shadow_failure(&store, "a", "t", "x", "s", i).unwrap();
        }
        record_shadow_failure(&store, "b", "t", "x", "s", 9).unwrap();
        assert_eq!(store.list_futility_trace("b", "t", 50).unwrap().len(), 1);
        let last_two = store.list_futility_trace("a", "t", 2).unwrap();
        assert_eq!(last_two.len(), 2);
        assert!(last_two[0].ts < last_two[1].ts, "oldest first");
        assert_eq!(last_two[1].ts, 4);
    }
}
