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
        let target = match (pattern.file_path(), pattern.symbol_name()) {
            (Some(f), Some(s)) if !f.is_empty() && !s.is_empty() => format!("{f}::{s}"),
            (Some(f), _) if !f.is_empty() => f.to_string(),
            (_, Some(s)) if !s.is_empty() => s.to_string(),
            _ => pattern
                .description
                .lines()
                .next()
                .unwrap_or(&pattern.pattern_name)
                .to_string(),
        };

        let err = pattern.error_class().unwrap_or(&pattern.error_type);

        let action = if let Some(corr) = pattern.verified_correction() {
            format!("Verified Correction: {corr}")
        } else {
            format!("Mitigation: {}", pattern.mitigation)
        };

        let raw = format!("[KNOWN ANTI-PATTERN]: {target} ({err}) | {action}");
        truncate_surgical_guardrail(&raw, 45)
    }

    /// Extracts (file, symbol, error_class) from compiler, runtime or test output.
    pub fn extract_symbol_anchor(output: &str) -> (Option<String>, Option<String>, Option<String>) {
        let mut file = None;
        let mut symbol = None;
        let mut error_class = None;

        for line in output.lines() {
            let trimmed = line.trim();

            // 1. Error Class Extraction
            if error_class.is_none() {
                if let Some(pos) = trimmed.find("error[E") {
                    if let Some(end) = trimmed[pos..].find(']') {
                        error_class = Some(trimmed[pos + 6..pos + end].to_string());
                    }
                } else if trimmed.contains("error TS") {
                    if let Some(pos) = trimmed.find("error TS") {
                        let sub = &trimmed[pos + 8..];
                        let code: String = sub.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if !code.is_empty() {
                            error_class = Some(format!("TS{code}"));
                        }
                    }
                } else if trimmed.contains("panicked at")
                    || trimmed.contains("assertion `left == right` failed")
                {
                    error_class = Some("AssertionFailure".to_string());
                } else if let Some(py_err) = extract_python_error_class(trimmed) {
                    error_class = Some(py_err);
                } else if trimmed.contains("MODULE_NOT_FOUND")
                    || trimmed.contains("Cannot find module")
                {
                    error_class = Some("MODULE_NOT_FOUND".to_string());
                } else if trimmed.contains("EADDRINUSE")
                    || trimmed.contains("Address already in use")
                {
                    error_class = Some("EADDRINUSE".to_string());
                }
            }

            // 2. File Extraction
            if file.is_none() {
                if let Some(pos) = trimmed.find("--> ") {
                    let path_part = trimmed[pos + 4..].trim_start();
                    let f = path_part.split(':').next().unwrap_or("").trim();
                    if !f.is_empty() {
                        file = Some(normalize_file_path(f));
                    }
                } else if let Some(after_prefix) = trimmed.strip_prefix("File \"") {
                    if let Some(end_quote) = after_prefix.find('\"') {
                        let f = &after_prefix[..end_quote];
                        file = Some(normalize_file_path(f));
                    }
                } else if trimmed.contains(".py::") {
                    if let Some(idx) = trimmed.find(".py::") {
                        let word_start = trimmed[..idx]
                            .rfind(|c: char| c.is_whitespace())
                            .map(|p| p + 1)
                            .unwrap_or(0);
                        let f = &trimmed[word_start..idx + 3];
                        file = Some(normalize_file_path(f));
                    }
                } else if trimmed.contains("panicked at") {
                    if let Some(comma_idx) = trimmed.rfind(',') {
                        let candidate = trimmed[comma_idx + 1..].trim();
                        if candidate.contains(".rs:") {
                            let f = candidate.split(':').next().unwrap_or("").trim();
                            if !f.is_empty() {
                                file = Some(normalize_file_path(f));
                            }
                        }
                    } else if let Some(at_idx) = trimmed.find("panicked at ") {
                        let candidate = trimmed[at_idx + 12..].trim();
                        if candidate.contains(".rs:") {
                            let f = candidate.split(':').next().unwrap_or("").trim();
                            if !f.is_empty() {
                                file = Some(normalize_file_path(f));
                            }
                        }
                    }
                } else if trimmed.contains(".ts")
                    || trimmed.contains(".js")
                    || trimmed.contains(".tsx")
                    || trimmed.contains(".jsx")
                {
                    if let Some(f) = extract_js_ts_file(trimmed) {
                        file = Some(normalize_file_path(&f));
                    }
                }
            }

            // 3. Symbol Extraction
            if symbol.is_none() {
                if trimmed.contains(", in ") {
                    if let Some(in_idx) = trimmed.find(", in ") {
                        let sym = trimmed[in_idx + 5..].trim();
                        let clean_sym = sym
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect::<String>();
                        if !clean_sym.is_empty() {
                            symbol = Some(clean_sym);
                        }
                    }
                } else if trimmed.contains("::")
                    && (trimmed.contains(".py::")
                        || trimmed.contains("FAILED")
                        || trimmed.contains("test "))
                {
                    if let Some(colons) = trimmed.find("::") {
                        let after = &trimmed[colons + 2..];
                        let sym: String = after
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                            .collect();
                        if !sym.is_empty() {
                            symbol = Some(sym);
                        }
                    }
                } else if trimmed.starts_with("test ") && trimmed.contains(" ... FAILED") {
                    let sym = &trimmed[5..trimmed.find(" ... FAILED").unwrap()].trim();
                    symbol = Some(sym.to_string());
                } else if trimmed.starts_with("thread '") && trimmed.contains("' panicked") {
                    if let Some(end_q) = trimmed[8..].find('\'') {
                        symbol = Some(trimmed[8..8 + end_q].to_string());
                    }
                } else if trimmed.contains("in initializer of `") {
                    if let Some(st) = trimmed.find("in initializer of `") {
                        let sub = &trimmed[st + 19..];
                        if let Some(end) = sub.find('`') {
                            symbol = Some(sub[..end].to_string());
                        }
                    }
                } else if trimmed.contains("no method named `") && trimmed.contains("` found for ")
                {
                    let m_start = trimmed.find("no method named `").unwrap() + 17;
                    if let Some(m_end) = trimmed[m_start..].find('`') {
                        let method = &trimmed[m_start..m_start + m_end];
                        if let Some(f_idx) = trimmed.find("found for ") {
                            let after_for = &trimmed[f_idx + 10..];
                            if let Some(bt_start) = after_for.find('`') {
                                if let Some(bt_end) = after_for[bt_start + 1..].find('`') {
                                    let ty = &after_for[bt_start + 1..bt_start + 1 + bt_end];
                                    symbol = Some(format!("{ty}::{method}"));
                                }
                            }
                        }
                        if symbol.is_none() {
                            symbol = Some(method.to_string());
                        }
                    }
                } else if trimmed.contains("cannot borrow `") && trimmed.contains("` as mutable") {
                    let start_b = trimmed.find("cannot borrow `").unwrap() + 15;
                    if let Some(end_b) = trimmed[start_b..].find('`') {
                        symbol = Some(trimmed[start_b..start_b + end_b].to_string());
                    }
                } else if trimmed.starts_with("at ") {
                    if let Some(open_p) = trimmed.find('(') {
                        let func_part = trimmed[3..open_p].trim();
                        let clean = func_part
                            .trim_start_matches("async ")
                            .trim_start_matches("Object.");
                        if !clean.is_empty() && !clean.contains('/') && !clean.contains('\\') {
                            symbol = Some(clean.to_string());
                        }
                    }
                }
            }
        }

        (file, symbol, error_class)
    }

    /// Suggests verified correction for known error classes (Dual-Polarity).
    pub fn suggest_verified_correction(
        error_class_or_type: &str,
        signature: &str,
    ) -> Option<&'static str> {
        let key = error_class_or_type.to_uppercase();
        if key.contains("E0502") || key.contains("BORROW") {
            Some("Clone reference or narrow mutable borrow scope before await")
        } else if key.contains("E0063") || key.contains("INITIALIZER") {
            Some("Initialize all required struct fields or derive Default")
        } else if key.contains("E0433") || key.contains("E0425") || key.contains("UNDECLARED") {
            Some("Import missing symbol with `use` or verify crate dependencies")
        } else if key.contains("E0308") || key.contains("MISMATCH") {
            Some("Convert value with .into() or align expression type with signature")
        } else if key.contains("E0599") || key.contains("NOMETHOD") {
            Some("Implement required trait or check method name on type")
        } else if key.contains("TS2322") || key.contains("TS2345") {
            Some("Align type assignment or update interface type definition")
        } else if key.contains("MODULENOTFOUND") || key.contains("MODULE_NOT_FOUND") {
            Some("Install missing package in workspace or virtual environment")
        } else if key.contains("EADDRINUSE") || key.contains("PORT") {
            Some("Bind dynamically to PORT environment variable instead of static port")
        } else if key.contains("ASSERTION")
            || signature.contains("test_failure")
            || signature.contains("pytest")
        {
            Some("Align test assertion with actual returned value")
        } else {
            None
        }
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

        let mut parsed_pattern = None;

        if cmd_lower.contains("cargo") || cmd_lower.contains("rustc") {
            parsed_pattern = Self::parse_rust_error(command, &combined, context, scope.clone());
        }

        if parsed_pattern.is_none()
            && (cmd_lower.contains("npm")
                || cmd_lower.contains("npx")
                || cmd_lower.contains("yarn")
                || cmd_lower.contains("pnpm")
                || cmd_lower.contains("jest")
                || cmd_lower.contains("vitest")
                || cmd_lower.contains("tsc"))
        {
            parsed_pattern = Self::parse_npm_error(command, &combined, context, scope.clone());
        }

        if parsed_pattern.is_none()
            && (cmd_lower.contains("pytest")
                || cmd_lower.contains("python")
                || cmd_lower.contains("py"))
        {
            parsed_pattern = Self::parse_python_error(command, &combined, context, scope.clone());
        }

        if parsed_pattern.is_none()
            && (combined.contains("Address already in use") || combined.contains("EADDRINUSE"))
        {
            let mut p = FailurePattern::new(
                "network_port_collision",
                "Port Binding Collision",
                "Address already in use / Static port collision",
                "DO NOT hardcode static PORT. Always bind dynamically to std::env::var(\"PORT\") or random free port.",
            );
            p.error_type = "PortCollisionError".to_string();
            p.trigger_condition = command.to_string();
            p.severity = FailureSeverity::High;
            if let Some(s) = scope.clone() {
                p.scope = s;
            }
            if let Some(ctx) = context {
                p.metadata = serde_json::json!({ "context": ctx });
            }
            parsed_pattern = Some(p);
        }

        let mut pattern = match parsed_pattern {
            Some(p) => p,
            None => {
                Self::parse_generic_error(command, &combined, exit_code, context, scope.clone())?
            }
        };

        // Extract and anchor to symbol/file
        let (file, symbol, error_class) = Self::extract_symbol_anchor(&combined);
        if file.is_some() || symbol.is_some() || error_class.is_some() {
            let f = file.unwrap_or_default();
            pattern = pattern.with_symbol_anchor(f, symbol.as_deref(), error_class.as_deref());
        }

        // Dual-polarity: attach verified correction when available
        if pattern.verified_correction().is_none() {
            let err_class = pattern.error_class().unwrap_or(&pattern.error_type);
            if let Some(corr) = Self::suggest_verified_correction(err_class, &pattern.signature) {
                pattern = pattern.with_verified_correction(corr);
            }
        }

        Some(pattern)
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
            return Err(StrataError::Validation(
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

fn truncate_surgical_guardrail(text: &str, max_tokens: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let max_words = (max_tokens * 8) / 10;
    if words.len() <= max_words {
        text.to_string()
    } else {
        let truncated = words[..max_words].join(" ");
        format!("{truncated}...")
    }
}

fn normalize_file_path(p: &str) -> String {
    let cleaned = p
        .trim()
        .trim_matches(|c| c == '\'' || c == '\"' || c == '(' || c == ')' || c == ':');
    let replaced = cleaned.replace('\\', "/");
    replaced.trim_start_matches("./").to_string()
}

fn extract_python_error_class(line: &str) -> Option<String> {
    let candidates = [
        "AssertionError",
        "ModuleNotFoundError",
        "ImportError",
        "AttributeError",
        "TypeError",
        "KeyError",
        "IndexError",
        "ValueError",
        "ZeroDivisionError",
        "RuntimeError",
        "NameError",
        "FileNotFoundError",
        "PermissionError",
        "TimeoutError",
    ];
    for &c in &candidates {
        if line.contains(c) {
            return Some(c.to_string());
        }
    }
    None
}

fn extract_js_ts_file(line: &str) -> Option<String> {
    let exts = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];
    for ext in exts {
        if let Some(ext_idx) = line.find(ext) {
            let after_ext = ext_idx + ext.len();
            let start = line[..ext_idx]
                .rfind(|c: char| c.is_whitespace() || c == '(' || c == '"' || c == '\'')
                .map(|p| p + 1)
                .unwrap_or(0);
            let path = &line[start..after_ext];
            if !path.is_empty()
                && (path.contains('/') || path.contains('\\') || path.ends_with(ext))
            {
                return Some(path.to_string());
            }
        }
    }
    None
}
