use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::{debug, info, warn};

use strata_core::{
    errors::StrataError,
    state::{FailurePattern, FailureSeverity, Scope},
    traits::MemoryEngine,
};

/// Result of executing and intercepting a shell command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InterceptionResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub anti_pattern: Option<FailurePattern>,
    pub surgical_guardrail: Option<String>,
}

/// Intelligent parser for compiler and test-runner failure outputs.
pub struct AntiPatternParser;

impl AntiPatternParser {
    /// Formats an actionable constraint into a surgical guardrail (< 50 tokens).
    pub fn format_surgical_guardrail(pattern: &FailurePattern) -> String {
        format!(
            "[KNOWN ANTI-PATTERN]: {} | Mitigation: {} (Target: {})",
            pattern
                .description
                .lines()
                .next()
                .unwrap_or(&pattern.pattern_name),
            pattern.mitigation,
            pattern.trigger_condition
        )
    }

    /// Analyzes command execution error and synthesizes a structured FailurePattern.
    pub fn parse(
        command: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
        context: Option<&str>,
        scope: Option<Scope>,
    ) -> Option<FailurePattern> {
        if exit_code == 0 {
            return None;
        }

        let combined = format!("{stderr}\n{stdout}");
        let cmd_lower = command.to_lowercase();

        // 1. Rust / Cargo test & build errors
        if cmd_lower.contains("cargo") || cmd_lower.contains("rustc") {
            if let Some(fp) = Self::parse_rust_error(command, &combined, context, scope.clone()) {
                return Some(fp);
            }
        }

        // 2. Node / NPM / Jest / Vitest / TSC errors
        if cmd_lower.contains("npm")
            || cmd_lower.contains("npx")
            || cmd_lower.contains("yarn")
            || cmd_lower.contains("pnpm")
            || cmd_lower.contains("jest")
            || cmd_lower.contains("vitest")
            || cmd_lower.contains("tsc")
        {
            if let Some(fp) = Self::parse_npm_error(command, &combined, context, scope.clone()) {
                return Some(fp);
            }
        }

        // 3. Python / Pytest errors
        if cmd_lower.contains("pytest") || cmd_lower.contains("python") || cmd_lower.contains("py")
        {
            if let Some(fp) = Self::parse_python_error(command, &combined, context, scope.clone()) {
                return Some(fp);
            }
        }

        // 4. Port / Network / Docker conflicts
        if combined.contains("Address already in use") || combined.contains("EADDRINUSE") {
            let mut pattern = FailurePattern::new(
                "network_port_collision",
                "Port Binding Collision",
                "Address already in use / Static port collision",
                "DO NOT hardcode static PORT. Always bind dynamically to std::env::var(\"PORT\") or random free port.",
            );
            pattern.error_type = "PortCollisionError".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // 5. Generic Fallback Parser
        Self::parse_generic_error(command, &combined, exit_code, context, scope)
    }

    fn parse_rust_error(
        command: &str,
        output: &str,
        context: Option<&str>,
        scope: Option<Scope>,
    ) -> Option<FailurePattern> {
        // Cargo package ID mismatch
        if output.contains("did not match any packages") {
            let pkg = output
                .lines()
                .find(|l| l.contains("did not match any packages"))
                .unwrap_or("")
                .trim();
            let mut pattern = FailurePattern::new(
                "cargo_package_not_found",
                "Invalid Cargo Package Specification",
                pkg.to_string(),
                "Use exact package name defined in workspace Cargo.toml.",
            );
            pattern.error_type = "CargoPackageMismatch".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // Borrow checker conflict
        if output.contains("error[E0502]") || output.contains("cannot borrow") {
            let mut pattern = FailurePattern::new(
                "rust_borrow_checker_conflict",
                "Rust Borrow Checker Conflict",
                "Simultaneous mutable and immutable borrow or aliasing conflict in scope",
                "Separate borrow scopes, clone reference, or use interior mutability (Arc/Mutex/RwLock).",
            );
            pattern.error_type = "BorrowCheckerError".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // Missing struct field (E0063)
        if output.contains("error[E0063]") || output.contains("missing field") {
            let field_err = output
                .lines()
                .find(|l| l.contains("missing field") || l.contains("missing `"))
                .unwrap_or("missing struct field");
            let mut pattern = FailurePattern::new(
                "rust_missing_struct_field",
                "Rust Missing Struct Field Initializer",
                field_err.trim().to_string(),
                "Initialize all required struct fields or provide Default::default().",
            );
            pattern.error_type = "StructInitializerError".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::Medium;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // Undeclared type / module / symbol (E0433 / E0425)
        if output.contains("error[E0433]")
            || output.contains("error[E0425]")
            || output.contains("use of undeclared")
        {
            let sym_err = output
                .lines()
                .find(|l| l.contains("use of undeclared") || l.contains("not found in this scope"))
                .unwrap_or("undeclared type or symbol");
            let mut pattern = FailurePattern::new(
                "rust_undeclared_symbol",
                "Rust Undeclared Type or Symbol",
                sym_err.trim().to_string(),
                "Import the missing symbol with `use` or check crate visibility/feature flags.",
            );
            pattern.error_type = "UndeclaredSymbolError".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::Medium;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // Cargo test assertion failure
        if output.contains("test result: FAILED") || output.contains("panicked at") {
            let panic_line = output
                .lines()
                .find(|l| l.contains("panicked at") || l.contains("FAILED"))
                .unwrap_or("cargo test assertion failed");
            let mut pattern = FailurePattern::new(
                "cargo_test_failure",
                "Cargo Unit Test Assertion Failure",
                panic_line.trim().to_string(),
                "Check test assertions and expected return values before re-running.",
            );
            pattern.error_type = "CargoTestFailure".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        None
    }

    fn parse_npm_error(
        command: &str,
        output: &str,
        context: Option<&str>,
        scope: Option<Scope>,
    ) -> Option<FailurePattern> {
        // Module not found
        if output.contains("Cannot find module") || output.contains("MODULE_NOT_FOUND") {
            let mod_line = output
                .lines()
                .find(|l| l.contains("Cannot find module") || l.contains("MODULE_NOT_FOUND"))
                .unwrap_or("Module not found");
            let mut pattern = FailurePattern::new(
                "npm_module_not_found",
                "Node Missing Module Dependency",
                mod_line.trim().to_string(),
                "Install missing dependency in package.json with npm/pnpm/yarn install.",
            );
            pattern.error_type = "NpmModuleNotFoundError".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // TypeScript type error
        if output.contains("error TS") {
            let ts_err = output
                .lines()
                .find(|l| l.contains("error TS"))
                .unwrap_or("TypeScript compiler error");
            let mut pattern = FailurePattern::new(
                "tsc_type_error",
                "TypeScript Type Check Failure",
                ts_err.trim().to_string(),
                "Fix TypeScript type mismatch, missing property or type annotation.",
            );
            pattern.error_type = "TypeScriptError".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::Medium;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // Jest / Vitest test failure
        if output.contains("FAIL ") || output.contains("Tests:       failed") {
            let fail_line = output
                .lines()
                .find(|l| l.contains("FAIL ") || l.contains("✕"))
                .unwrap_or("JavaScript test suite failed");
            let mut pattern = FailurePattern::new(
                "jest_test_failure",
                "JavaScript/TypeScript Test Suite Failure",
                fail_line.trim().to_string(),
                "Verify mock implementations and test assertions before re-executing.",
            );
            pattern.error_type = "JestTestFailure".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        None
    }

    fn parse_python_error(
        command: &str,
        output: &str,
        context: Option<&str>,
        scope: Option<Scope>,
    ) -> Option<FailurePattern> {
        // ModuleNotFoundError
        if output.contains("ModuleNotFoundError") || output.contains("ImportError") {
            let imp_line = output
                .lines()
                .find(|l| l.contains("ModuleNotFoundError") || l.contains("ImportError"))
                .unwrap_or("Python ModuleNotFoundError");
            let mut pattern = FailurePattern::new(
                "python_module_not_found",
                "Python Import or Module Not Found",
                imp_line.trim().to_string(),
                "Install missing Python package in active virtualenv with pip install.",
            );
            pattern.error_type = "PythonModuleNotFoundError".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        // Pytest failure
        if output.contains("FAILED ") || output.contains("AssertionError") {
            let fail_line = output
                .lines()
                .find(|l| l.contains("FAILED ") || l.contains("AssertionError"))
                .unwrap_or("Pytest assertion failure");
            let mut pattern = FailurePattern::new(
                "pytest_failure",
                "Pytest Test Assertion Failure",
                fail_line.trim().to_string(),
                "Fix Python test assertion or update test fixture/parameter.",
            );
            pattern.error_type = "PytestFailure".to_string();
            pattern.trigger_condition = command.to_string();
            pattern.severity = FailureSeverity::High;
            if let Some(s) = scope {
                pattern.scope = s;
            }
            if let Some(ctx) = context {
                pattern.metadata = serde_json::json!({ "context": ctx });
            }
            return Some(pattern);
        }

        None
    }

    fn parse_generic_error(
        command: &str,
        output: &str,
        exit_code: i32,
        context: Option<&str>,
        scope: Option<Scope>,
    ) -> Option<FailurePattern> {
        let first_err_line = output
            .lines()
            .find(|l| {
                let l_low = l.to_lowercase();
                l_low.contains("error") || l_low.contains("fatal") || l_low.contains("failed")
            })
            .unwrap_or_else(|| output.lines().next().unwrap_or("Command execution failed"))
            .trim();

        let mut pattern = FailurePattern::new(
            format!("cmd_exit_code_{exit_code}"),
            format!("Command Failed with Exit Code {exit_code}"),
            if first_err_line.is_empty() {
                format!("Exit code {exit_code} on: {command}")
            } else {
                first_err_line.to_string()
            },
            "Inspect command arguments and execution environment before retrying.",
        );
        pattern.error_type = "CommandExecutionFailure".to_string();
        pattern.trigger_condition = command.to_string();
        pattern.severity = FailureSeverity::Medium;
        if let Some(s) = scope {
            pattern.scope = s;
        }
        if let Some(ctx) = context {
            pattern.metadata = serde_json::json!({ "context": ctx });
        }
        Some(pattern)
    }
}

/// Middleware that wraps command execution, captures failures out-of-band, and persists AntiPatterns.
pub struct CommandInterceptor {
    engine: Option<Arc<dyn MemoryEngine>>,
}

impl Default for CommandInterceptor {
    fn default() -> Self {
        Self::new(None)
    }
}

impl CommandInterceptor {
    pub fn new(engine: Option<Arc<dyn MemoryEngine>>) -> Self {
        Self { engine }
    }

    pub fn with_engine(engine: Arc<dyn MemoryEngine>) -> Self {
        Self {
            engine: Some(engine),
        }
    }

    /// Runs a command through the interceptor middleware, capturing stderr, exit code, and synthesizing AntiPatterns.
    pub async fn execute_and_intercept(
        &self,
        command_args: &[String],
        cwd: Option<&str>,
        timeout: Option<Duration>,
        context: Option<&str>,
        scope: Option<Scope>,
    ) -> Result<InterceptionResult, StrataError> {
        if command_args.is_empty() {
            return Err(StrataError::ValidationError(
                "Command args must not be empty".to_string(),
            ));
        }

        let full_command_str = command_args.join(" ");
        debug!("CommandInterceptor executing: '{full_command_str}'");

        let program = &command_args[0];
        let args = &command_args[1..];

        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let timeout_duration = timeout.unwrap_or(Duration::from_secs(120));
        let start = Instant::now();

        let child_res = cmd.spawn();
        let child = match child_res {
            Ok(c) => c,
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                let err_msg = format!("Failed to spawn process '{program}': {e}");
                let anti_pattern =
                    AntiPatternParser::parse(&full_command_str, "", &err_msg, 127, context, scope);
                let guardrail = anti_pattern
                    .as_ref()
                    .map(AntiPatternParser::format_surgical_guardrail);

                if let (Some(ref eng), Some(ref ap)) = (&self.engine, &anti_pattern) {
                    let _ = eng.record_failure(ap).await;
                }

                return Ok(InterceptionResult {
                    command: full_command_str,
                    exit_code: Some(127),
                    stdout: String::new(),
                    stderr: err_msg,
                    duration_ms,
                    anti_pattern,
                    surgical_guardrail: guardrail,
                });
            }
        };

        let output_res = tokio::time::timeout(timeout_duration, child.wait_with_output()).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match output_res {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(1);

                let anti_pattern = if exit_code != 0 {
                    AntiPatternParser::parse(
                        &full_command_str,
                        &stdout,
                        &stderr,
                        exit_code,
                        context,
                        scope,
                    )
                } else {
                    None
                };

                let guardrail = anti_pattern
                    .as_ref()
                    .map(AntiPatternParser::format_surgical_guardrail);

                if let (Some(ref eng), Some(ref ap)) = (&self.engine, &anti_pattern) {
                    let _ = eng.record_failure(ap).await;
                    info!("Recorded AntiPattern out-of-band: {}", ap.pattern_name);
                }

                Ok(InterceptionResult {
                    command: full_command_str,
                    exit_code: Some(exit_code),
                    stdout,
                    stderr,
                    duration_ms,
                    anti_pattern,
                    surgical_guardrail: guardrail,
                })
            }
            Ok(Err(e)) => {
                let err_msg = format!("Process I/O error: {e}");
                let anti_pattern =
                    AntiPatternParser::parse(&full_command_str, "", &err_msg, 1, context, scope);
                let guardrail = anti_pattern
                    .as_ref()
                    .map(AntiPatternParser::format_surgical_guardrail);

                if let (Some(ref eng), Some(ref ap)) = (&self.engine, &anti_pattern) {
                    let _ = eng.record_failure(ap).await;
                }

                Ok(InterceptionResult {
                    command: full_command_str,
                    exit_code: Some(1),
                    stdout: String::new(),
                    stderr: err_msg,
                    duration_ms,
                    anti_pattern,
                    surgical_guardrail: guardrail,
                })
            }
            Err(_) => {
                let err_msg = format!("Command timed out after {}s", timeout_duration.as_secs());
                warn!("Command timed out: {full_command_str}");
                let anti_pattern =
                    AntiPatternParser::parse(&full_command_str, "", &err_msg, 124, context, scope);
                let guardrail = anti_pattern
                    .as_ref()
                    .map(AntiPatternParser::format_surgical_guardrail);

                if let (Some(ref eng), Some(ref ap)) = (&self.engine, &anti_pattern) {
                    let _ = eng.record_failure(ap).await;
                }

                Ok(InterceptionResult {
                    command: full_command_str,
                    exit_code: Some(124),
                    stdout: String::new(),
                    stderr: err_msg,
                    duration_ms,
                    anti_pattern,
                    surgical_guardrail: guardrail,
                })
            }
        }
    }
}
