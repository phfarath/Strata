use std::io::IsTerminal;

/// Resolves the effective agent ID according to the priority cascade:
/// 1. Explicit CLI argument (`--agent`)
/// 2. `STRATA_AGENT_ID` environment variable
/// 3. Fallback: `agent-{pid}`
pub fn resolve_agent_id(cli_arg: Option<&str>) -> String {
    if let Some(id) = cli_arg {
        if !id.trim().is_empty() {
            return id.trim().to_string();
        }
    }

    if let Ok(id) = std::env::var("STRATA_AGENT_ID") {
        if !id.trim().is_empty() {
            return id.trim().to_string();
        }
    }

    format!("agent-{}", std::process::id())
}

/// Resolves the effective host environment according to priority:
/// 1. Explicit CLI argument (`--host`)
/// 2. `STRATA_AGENT_HOST` environment variable
/// 3. Fallback: `cli`
pub fn resolve_agent_host(cli_arg: Option<&str>) -> String {
    if let Some(host) = cli_arg {
        if !host.trim().is_empty() {
            return host.trim().to_string();
        }
    }

    if let Ok(host) = std::env::var("STRATA_AGENT_HOST") {
        if !host.trim().is_empty() {
            return host.trim().to_string();
        }
    }

    "cli".to_string()
}

/// Resolves the effective task ID according to priority:
/// 1. Explicit CLI argument (`--task`)
/// 2. `STRATA_TASK_ID` environment variable
/// 3. None
pub fn resolve_task_id(cli_arg: Option<&str>) -> Option<String> {
    if let Some(task) = cli_arg {
        if !task.trim().is_empty() {
            return Some(task.trim().to_string());
        }
    }

    if let Ok(task) = std::env::var("STRATA_TASK_ID") {
        if !task.trim().is_empty() {
            return Some(task.trim().to_string());
        }
    }

    None
}

/// Determines whether the output should be formatted as strict machine-readable JSON:
/// 1. Explicit `--json` CLI flag
/// 2. `STRATA_OUTPUT=json` environment variable
/// 3. Stdout is redirected to a pipe or file (non-terminal)
pub fn is_json_mode(cli_json_flag: bool) -> bool {
    if cli_json_flag {
        return true;
    }

    if let Ok(val) = std::env::var("STRATA_OUTPUT") {
        if val.eq_ignore_ascii_case("json") {
            return true;
        }
    }

    !std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_agent_id_explicit() {
        assert_eq!(resolve_agent_id(Some("agent-cursor")), "agent-cursor");
    }

    #[test]
    fn test_resolve_agent_host_default() {
        assert_eq!(resolve_agent_host(None), "cli");
    }

    #[test]
    fn test_is_json_mode_flag() {
        assert!(is_json_mode(true));
    }
}
