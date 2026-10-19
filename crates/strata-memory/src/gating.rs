use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::Instant;

/// High-level classification of an agentic coding task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Purely mechanical or operational tasks (e.g. run test, git status, cargo check, code format, lint).
    Mechanical,
    /// Local code modification (single function edit, local typo fix, local syntax adjustment).
    CodeLocal,
    /// Architectural decisions, multi-component design, system architecture, database schemas.
    Architecture,
    /// Historical inquiry, prior attempts, post-mortems, provenance, rationale ("why").
    Historical,
    /// Complex debugging, race conditions, memory leaks, intermittent failures, anti-patterns.
    Debugging,
    /// General inquiry or unclassified task.
    #[default]
    General,
}

impl fmt::Display for TaskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskType::Mechanical => write!(f, "mechanical"),
            TaskType::CodeLocal => write!(f, "code_local"),
            TaskType::Architecture => write!(f, "architecture"),
            TaskType::Historical => write!(f, "historical"),
            TaskType::Debugging => write!(f, "debugging"),
            TaskType::General => write!(f, "general"),
        }
    }
}

impl FromStr for TaskType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "mechanical" | "terminal" | "command" | "cli" | "op" => Ok(TaskType::Mechanical),
            "codelocal" | "code_local" | "local" | "syntax" | "edit" => Ok(TaskType::CodeLocal),
            "architecture" | "arch" | "design" | "system" => Ok(TaskType::Architecture),
            "historical" | "history" | "past" | "why" | "provenance" => Ok(TaskType::Historical),
            "debugging" | "debug" | "troubleshooting" | "bug" => Ok(TaskType::Debugging),
            _ => Ok(TaskType::General),
        }
    }
}

/// Categorized intention detected by the gating classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    /// Pure terminal command / operational bypass (e.g., shell command, formatting, file listing, git check).
    MechanicalBypass,
    /// Local code syntax edit bypass (e.g., typo, formatting, local variable rename).
    LocalCodeBypass,
    /// Trivial / conversational acknowledgment bypass (e.g. "ok", "yes", "thanks", empty).
    TrivialBypass,
    /// Architectural inquiry or system design requiring institutional knowledge.
    ArchitectureRecall,
    /// Historical decision, rationale, or post-mortem retrieval.
    HistoricalRecall,
    /// Debugging failure patterns, anti-patterns, or known bugs.
    DebuggingRecall,
    /// Global convention, security policy, or domain rule lookup.
    GlobalRuleRecall,
    /// Ambiguous or open-ended query defaulting to memory recall.
    GeneralRecall,
}

impl fmt::Display for IntentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntentKind::MechanicalBypass => write!(f, "mechanical_bypass"),
            IntentKind::LocalCodeBypass => write!(f, "local_code_bypass"),
            IntentKind::TrivialBypass => write!(f, "trivial_bypass"),
            IntentKind::ArchitectureRecall => write!(f, "architecture_recall"),
            IntentKind::HistoricalRecall => write!(f, "historical_recall"),
            IntentKind::DebuggingRecall => write!(f, "debugging_recall"),
            IntentKind::GlobalRuleRecall => write!(f, "global_rule_recall"),
            IntentKind::GeneralRecall => write!(f, "general_recall"),
        }
    }
}

/// Complete decision report produced by the deterministic gating classifier (< 1ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallDecision {
    /// Whether memory retrieval should be triggered.
    pub should_recall: bool,
    /// The task type considered for classification.
    pub task_type: TaskType,
    /// Specific intent category detected.
    pub intent: IntentKind,
    /// Explanation of why the decision was made.
    pub reason: &'static str,
    /// Measured evaluation latency in microseconds.
    pub latency_micros: u64,
}

/// Fast deterministic classifier evaluating whether a user prompt requires institutional memory recall.
pub struct RecallGatingClassifier;

impl RecallGatingClassifier {
    /// Classifies a prompt and task type into a `RecallDecision` in < 1ms deterministically.
    pub fn classify(prompt: &str, task_type: TaskType) -> RecallDecision {
        let start = Instant::now();
        let trimmed = prompt.trim();

        // 1. Trivial or empty prompts bypass immediately
        if trimmed.is_empty() {
            return RecallDecision {
                should_recall: false,
                task_type,
                intent: IntentKind::TrivialBypass,
                reason: "Empty prompt does not require memory retrieval",
                latency_micros: start.elapsed().as_micros() as u64,
            };
        }

        let lower = trimmed.to_lowercase();

        if is_trivial_conversation(&lower) {
            return RecallDecision {
                should_recall: false,
                task_type,
                intent: IntentKind::TrivialBypass,
                reason: "Conversational acknowledgment does not require memory retrieval",
                latency_micros: start.elapsed().as_micros() as u64,
            };
        }

        // 2. High-priority recall indicators (Architecture, Rules, History, Anti-patterns, Security)
        // These signals override task types because even a mechanical command like
        // "cargo check auth token validation rules" needs institutional knowledge.
        if has_historical_inquiry_signals(&lower) {
            return RecallDecision {
                should_recall: true,
                task_type,
                intent: IntentKind::HistoricalRecall,
                reason: "Historical inquiry or decision rationale detected",
                latency_micros: start.elapsed().as_micros() as u64,
            };
        }

        if has_failure_or_debugging_signals(&lower) {
            return RecallDecision {
                should_recall: true,
                task_type,
                intent: IntentKind::DebuggingRecall,
                reason: "Failure anti-patterns or debugging context detected",
                latency_micros: start.elapsed().as_micros() as u64,
            };
        }

        if has_global_rule_or_security_signals(&lower) {
            return RecallDecision {
                should_recall: true,
                task_type,
                intent: IntentKind::GlobalRuleRecall,
                reason: "Global project rules, conventions, or security constraints detected",
                latency_micros: start.elapsed().as_micros() as u64,
            };
        }

        if has_architecture_or_design_signals(&lower) {
            return RecallDecision {
                should_recall: true,
                task_type,
                intent: IntentKind::ArchitectureRecall,
                reason: "System architecture or high-level design detected",
                latency_micros: start.elapsed().as_micros() as u64,
            };
        }

        // 3. TaskType-specific gating rules
        match task_type {
            TaskType::Mechanical => {
                // If no high-priority recall signals fired above, mechanical tasks bypass recall
                RecallDecision {
                    should_recall: false,
                    task_type,
                    intent: IntentKind::MechanicalBypass,
                    reason: "Mechanical task does not require institutional memory",
                    latency_micros: start.elapsed().as_micros() as u64,
                }
            }
            TaskType::CodeLocal => {
                // Local code edits that did not touch architecture, rules, or security bypass recall
                RecallDecision {
                    should_recall: false,
                    task_type,
                    intent: IntentKind::LocalCodeBypass,
                    reason: "Local code modification without architectural scope bypasses recall",
                    latency_micros: start.elapsed().as_micros() as u64,
                }
            }
            TaskType::Architecture => {
                // Architectural tasks default to recall
                RecallDecision {
                    should_recall: true,
                    task_type,
                    intent: IntentKind::ArchitectureRecall,
                    reason: "Architecture task type requires institutional memory context",
                    latency_micros: start.elapsed().as_micros() as u64,
                }
            }
            TaskType::Historical => {
                // Historical inquiries default to recall
                RecallDecision {
                    should_recall: true,
                    task_type,
                    intent: IntentKind::HistoricalRecall,
                    reason: "Historical task type requires institutional memory context",
                    latency_micros: start.elapsed().as_micros() as u64,
                }
            }
            TaskType::Debugging => {
                // Debugging tasks default to recall
                RecallDecision {
                    should_recall: true,
                    task_type,
                    intent: IntentKind::DebuggingRecall,
                    reason: "Debugging task type requires failure pattern context",
                    latency_micros: start.elapsed().as_micros() as u64,
                }
            }
            TaskType::General => {
                // 4. For general / unclassified tasks, check if the prompt is purely mechanical
                if is_pure_mechanical_command(&lower) {
                    RecallDecision {
                        should_recall: false,
                        task_type,
                        intent: IntentKind::MechanicalBypass,
                        reason: "Deterministic mechanical command pattern detected",
                        latency_micros: start.elapsed().as_micros() as u64,
                    }
                } else if is_pure_local_code_edit(&lower) {
                    RecallDecision {
                        should_recall: false,
                        task_type,
                        intent: IntentKind::LocalCodeBypass,
                        reason: "Deterministic local code syntax modification detected",
                        latency_micros: start.elapsed().as_micros() as u64,
                    }
                } else {
                    // General development query defaults to recalling relevant memories
                    RecallDecision {
                        should_recall: true,
                        task_type,
                        intent: IntentKind::GeneralRecall,
                        reason: "General development query benefits from memory context",
                        latency_micros: start.elapsed().as_micros() as u64,
                    }
                }
            }
        }
    }
}

/// Primary entry point: deterministic decision whether to recall memories for a prompt (< 1ms).
///
/// # Arguments
/// * `prompt` - The user input or prompt string.
/// * `task_type` - Categorized task type (e.g. Mechanical, CodeLocal, Architecture, Debugging).
///
/// # Returns
/// `true` if memory recall should be performed; `false` if it can be safely bypassed.
pub fn should_recall(prompt: &str, task_type: TaskType) -> bool {
    RecallGatingClassifier::classify(prompt, task_type).should_recall
}

// =========================================================================
// Deterministic Fast Matchers (Optimized Substring / Token Scans, < 100μs)
// =========================================================================

fn is_trivial_conversation(lower: &str) -> bool {
    const TRIVIAL_WORDS: &[&str] = &[
        "ok", "sim", "yes", "y", "no", "nao", "não", "done", "pronto", "thanks", "obrigado",
        "valeu", "show", "continue", "proceed", "go ahead", "pode ser", "perfeito", "certo",
        "beleza", "legal", "top", "cancel", "stop", "pare",
    ];

    let t = lower.trim().trim_end_matches(['.', '!', '?', ';', ',']);
    TRIVIAL_WORDS.contains(&t)
}

fn has_historical_inquiry_signals(lower: &str) -> bool {
    const HISTORICAL_PATTERNS: &[&str] = &[
        "why did we",
        "why is this",
        "por que",
        "porque",
        "por quê",
        "rationale",
        "post-mortem",
        "postmortem",
        "provenance",
        "last time we",
        "da última vez",
        "da ultima vez",
        "tried before",
        "attempted before",
        "who decided",
        "history of",
        "histórico de",
        "historico de",
        "decision record",
        "adr-",
        "adr ",
        "revert decision",
    ];

    HISTORICAL_PATTERNS.iter().any(|&p| lower.contains(p))
}

fn has_failure_or_debugging_signals(lower: &str) -> bool {
    const FAILURE_PATTERNS: &[&str] = &[
        "anti-pattern",
        "antipattern",
        "known bug",
        "known failure",
        "failure mode",
        "root cause",
        "deadlock",
        "race condition",
        "lock poisoned",
        "memory leak",
        "segfault",
        "flaky test",
        "intermittent",
        "intermitente",
        "regressão",
        "regressao",
        "regression",
        "workaround",
        "panic in thread",
        "stack overflow",
    ];

    FAILURE_PATTERNS.iter().any(|&p| lower.contains(p))
}

fn has_global_rule_or_security_signals(lower: &str) -> bool {
    const RULE_PATTERNS: &[&str] = &[
        "convention",
        "convenção",
        "convencao",
        "guideline",
        "coding standard",
        "security rule",
        "security policy",
        "segurança",
        "seguranca",
        "política de segurança",
        "politica de seguranca",
        "auth token",
        "api key",
        "secret handling",
        "branch rule",
        "never do",
        "global rule",
        "regras globais",
        "do not commit",
        "stigmergy",
        "jtms",
    ];

    RULE_PATTERNS.iter().any(|&p| lower.contains(p))
}

fn has_architecture_or_design_signals(lower: &str) -> bool {
    const ARCH_PATTERNS: &[&str] = &[
        "architecture",
        "arquitetura",
        "how should we structure",
        "como devemos estruturar",
        "system design",
        "design pattern",
        "database schema",
        "schema migration",
        "modularity",
        "crate boundary",
        "microservice",
        "monolith",
        "subsystem",
        "data model",
        "domain model",
        "hexagonal",
        "ports and adapters",
        "ipc protocol",
        "concurrency model",
    ];

    ARCH_PATTERNS.iter().any(|&p| lower.contains(p))
}

fn is_pure_mechanical_command(lower: &str) -> bool {
    // Commands that begin with standard CLI invocation or purely operational verbs
    const CLI_PREFIXES: &[&str] = &[
        "git status",
        "git diff",
        "git log",
        "git branch",
        "git checkout",
        "git pull",
        "git push",
        "git add",
        "git commit",
        "cargo check",
        "cargo build",
        "cargo test",
        "cargo clippy",
        "cargo fmt",
        "cargo run",
        "npm run",
        "npm test",
        "pnpm test",
        "yarn test",
        "python -m",
        "pytest",
        "uv run",
        "cat ",
        "echo ",
        "touch ",
        "mkdir ",
        "rm ",
        "mv ",
        "cp ",
    ];

    for &prefix in CLI_PREFIXES {
        if lower.starts_with(prefix) {
            return true;
        }
    }

    // Bare commands requiring word boundary to avoid false positives like "directory" matching "dir"
    const BARE_COMMANDS: &[&str] = &["ls", "dir", "pwd"];
    for &cmd in BARE_COMMANDS {
        if lower == cmd
            || lower.starts_with(&format!("{cmd} "))
            || lower.starts_with(&format!("{cmd} -"))
            || lower.starts_with(&format!("{cmd} /"))
        {
            return true;
        }
    }

    const MECHANICAL_ACTIONS: &[&str] = &[
        "run the tests",
        "run tests",
        "run unit tests",
        "check compilation",
        "format code",
        "format the code",
        "run cargo fmt",
        "run clippy",
        "check git status",
        "list files",
        "clean build",
        "show directory tree",
    ];

    MECHANICAL_ACTIONS
        .iter()
        .any(|&act| lower == act || lower.starts_with(act))
}

fn is_pure_local_code_edit(lower: &str) -> bool {
    const LOCAL_SYNTAX_PATTERNS: &[&str] = &[
        "fix typo in comment",
        "fix typo in docstring",
        "corrigir typo",
        "corrigir erro de digitação",
        "rename variable",
        "renomear variável",
        "renomear variavel",
        "indent this line",
        "sort imports",
        "organize imports",
        "remove unused import",
        "add semicolon",
        "add type annotation",
        "change visibility to pub",
    ];

    LOCAL_SYNTAX_PATTERNS.iter().any(|&pat| lower.contains(pat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_type_parsing_and_display() {
        assert_eq!(
            "mechanical".parse::<TaskType>().unwrap(),
            TaskType::Mechanical
        );
        assert_eq!(
            "terminal".parse::<TaskType>().unwrap(),
            TaskType::Mechanical
        );
        assert_eq!(
            "code_local".parse::<TaskType>().unwrap(),
            TaskType::CodeLocal
        );
        assert_eq!(
            "architecture".parse::<TaskType>().unwrap(),
            TaskType::Architecture
        );
        assert_eq!(
            "historical".parse::<TaskType>().unwrap(),
            TaskType::Historical
        );
        assert_eq!(
            "debugging".parse::<TaskType>().unwrap(),
            TaskType::Debugging
        );
        assert_eq!(
            "anything_else".parse::<TaskType>().unwrap(),
            TaskType::General
        );

        assert_eq!(TaskType::Mechanical.to_string(), "mechanical");
        assert_eq!(TaskType::Architecture.to_string(), "architecture");
    }

    #[test]
    fn test_mechanical_commands_bypass_recall() {
        let mechanical_prompts = [
            "git status",
            "git diff HEAD~1",
            "git log -n 5",
            "git checkout main",
            "cargo check --workspace",
            "cargo test --package strata-memory",
            "cargo clippy --all-targets",
            "cargo fmt --check",
            "npm run build",
            "npm test",
            "pytest tests/",
            "ls -la src/",
            "pwd",
            "run tests",
            "format code",
            "check compilation",
        ];

        for prompt in mechanical_prompts {
            assert!(
                !should_recall(prompt, TaskType::Mechanical),
                "Mechanical prompt '{prompt}' should bypass recall under TaskType::Mechanical"
            );
            assert!(
                !should_recall(prompt, TaskType::General),
                "Mechanical command '{prompt}' should bypass recall under TaskType::General"
            );
            let dec = RecallGatingClassifier::classify(prompt, TaskType::General);
            assert_eq!(dec.intent, IntentKind::MechanicalBypass);
        }
    }

    #[test]
    fn test_local_code_syntax_edits_bypass_recall() {
        let local_edits = [
            "fix typo in comment for calculate_sum",
            "rename variable count to total",
            "indent this line properly",
            "sort imports alphabetically",
            "remove unused import std::fmt",
            "corrigir typo no comentário da função",
            "add semicolon at end of line 42",
        ];

        for prompt in local_edits {
            assert!(
                !should_recall(prompt, TaskType::CodeLocal),
                "Local syntax edit '{prompt}' should bypass recall under TaskType::CodeLocal"
            );
            assert!(
                !should_recall(prompt, TaskType::General),
                "Local syntax edit '{prompt}' should bypass recall under TaskType::General"
            );
            let dec = RecallGatingClassifier::classify(prompt, TaskType::General);
            assert_eq!(dec.intent, IntentKind::LocalCodeBypass);
        }
    }

    #[test]
    fn test_trivial_conversational_prompts_bypass() {
        let trivial = [
            "ok", "sim", "yes", "done", "thanks", "valeu", "show", "pode ser", "", "   ",
        ];

        for prompt in trivial {
            assert!(
                !should_recall(prompt, TaskType::General),
                "Trivial prompt '{prompt}' should bypass recall"
            );
            let dec = RecallGatingClassifier::classify(prompt, TaskType::General);
            assert_eq!(dec.intent, IntentKind::TrivialBypass);
        }
    }

    #[test]
    fn test_architecture_and_design_intent_recalls() {
        let arch_prompts = [
            "how should we structure the architecture of the new persistence crate?",
            "como devemos estruturar a arquitetura hexagonal deste módulo?",
            "database schema migration plan for episodic pyramid tables",
            "propose a new IPC protocol for daemon and client communication",
            "modularity boundaries and crate dependencies across the workspace",
        ];

        for prompt in arch_prompts {
            assert!(
                should_recall(prompt, TaskType::Architecture),
                "Architecture prompt '{prompt}' should trigger recall under TaskType::Architecture"
            );
            assert!(
                should_recall(prompt, TaskType::General),
                "Architecture prompt '{prompt}' should trigger recall under TaskType::General"
            );
            let dec = RecallGatingClassifier::classify(prompt, TaskType::General);
            assert_eq!(dec.intent, IntentKind::ArchitectureRecall);
        }
    }

    #[test]
    fn test_historical_inquiries_recall() {
        let historical_prompts = [
            "why did we choose SQLite over RocksDB?",
            "por que alteramos a política de retenção da última vez?",
            "what was the rationale behind the JTMS belief revision mechanism?",
            "has this approach been attempted before and failed?",
            "who decided the 1-hour lease expiry in ADR-0012?",
        ];

        for prompt in historical_prompts {
            assert!(
                should_recall(prompt, TaskType::Historical),
                "Historical prompt '{prompt}' should trigger recall"
            );
            assert!(
                should_recall(prompt, TaskType::General),
                "Historical prompt '{prompt}' should trigger recall under General"
            );
            let dec = RecallGatingClassifier::classify(prompt, TaskType::General);
            assert_eq!(dec.intent, IntentKind::HistoricalRecall);
        }
    }

    #[test]
    fn test_failure_antipatterns_and_debugging_recall() {
        let debug_prompts = [
            "investigating race condition and lock poisoned error in sqlite store",
            "anti-pattern with recursive call graph traversal causing stack overflow",
            "known bug with deadlocks during concurrent task claiming",
            "flaky test failing intermittently with timeout",
            "what is the root cause of the regression in serialization?",
        ];

        for prompt in debug_prompts {
            assert!(
                should_recall(prompt, TaskType::Debugging),
                "Debugging prompt '{prompt}' should trigger recall"
            );
            assert!(
                should_recall(prompt, TaskType::General),
                "Debugging prompt '{prompt}' should trigger recall under General"
            );
            let dec = RecallGatingClassifier::classify(prompt, TaskType::General);
            assert_eq!(dec.intent, IntentKind::DebuggingRecall);
        }
    }

    #[test]
    fn test_global_rules_and_security_recall() {
        let rule_prompts = [
            "what are our global rules for git branch management?",
            "regras globais sobre nunca commitar na branch main diretamente",
            "how do we handle API keys and secrets safely?",
            "security policy regarding auth token storage",
            "conventions for JTMS contradiction arbitration",
        ];

        for prompt in rule_prompts {
            assert!(
                should_recall(prompt, TaskType::General),
                "Global rule prompt '{prompt}' should trigger recall"
            );
            let dec = RecallGatingClassifier::classify(prompt, TaskType::General);
            assert_eq!(dec.intent, IntentKind::GlobalRuleRecall);
        }
    }

    #[test]
    fn test_recall_signal_overrides_mechanical_and_local_tasks() {
        // Even if classified as Mechanical or CodeLocal, high-priority signals force recall
        let prompt_auth = "cargo test auth token security policy";
        assert!(
            should_recall(prompt_auth, TaskType::Mechanical),
            "Mechanical task with security rule signal must recall"
        );

        let prompt_arch = "rename variable to match database schema migration convention";
        assert!(
            should_recall(prompt_arch, TaskType::CodeLocal),
            "CodeLocal task with architecture signal must recall"
        );
    }

    #[test]
    fn test_deterministic_latency_sub_millisecond_guarantee() {
        let sample_prompts = [
            "git status",
            "how should we structure the architecture of the new crate?",
            "ok",
            "fix typo in comment",
            "why did we choose SQLite over RocksDB?",
            "cargo test --package strata-memory",
            "investigating race condition and lock poisoned error",
            "what are our rules regarding git branches?",
        ];

        // Warm up instruction cache
        for prompt in sample_prompts {
            let _ = RecallGatingClassifier::classify(prompt, TaskType::General);
        }

        let total_iterations = 10_000;
        let start = Instant::now();

        for i in 0..total_iterations {
            let prompt = sample_prompts[i % sample_prompts.len()];
            let task_type = match i % 6 {
                0 => TaskType::Mechanical,
                1 => TaskType::CodeLocal,
                2 => TaskType::Architecture,
                3 => TaskType::Historical,
                4 => TaskType::Debugging,
                _ => TaskType::General,
            };

            let _ = RecallGatingClassifier::classify(prompt, task_type);
        }

        let total_elapsed = start.elapsed();
        let avg_micros = total_elapsed.as_micros() as f64 / total_iterations as f64;
        println!(
            "Benchmarked {} gating classifications in {:?} (avg: {:.2}μs per prompt)",
            total_iterations, total_elapsed, avg_micros
        );

        assert!(
            avg_micros < 100.0,
            "Average latency was {:.2}μs, expected well under 100μs (< 1000μs)",
            avg_micros
        );
    }

    #[test]
    fn test_bare_command_boundary_avoidance() {
        // "dir" and "dir /s" are mechanical commands
        assert!(!should_recall("dir", TaskType::General));
        assert!(!should_recall("dir /s", TaskType::General));
        assert!(!should_recall("ls -la", TaskType::General));

        // Words starting with "dir" that are not commands should not be falsely bypassed
        let dec = RecallGatingClassifier::classify(
            "directory structure recommendations",
            TaskType::General,
        );
        assert_eq!(dec.intent, IntentKind::GeneralRecall);
        assert!(dec.should_recall);
    }
}
