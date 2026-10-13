use std::collections::VecDeque;

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
#[derive(Debug, Default, Clone)]
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
}
