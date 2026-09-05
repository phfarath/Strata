use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

pub const STRATA_MARKER_START: &str = "<!-- STRATA_MEMORY_START -->";
pub const STRATA_MARKER_END: &str = "<!-- STRATA_MEMORY_END -->";

#[derive(Debug, Clone)]
pub struct InitOptions {
    pub workspace_dir: PathBuf,
    pub target_host: Option<String>,
    pub force: bool,
}

pub fn run_init(options: InitOptions) -> Result<()> {
    let root = &options.workspace_dir;
    info!("Initializing Strata in workspace: {}", root.display());

    let target = options.target_host.as_deref().unwrap_or("all");

    let mut configured = Vec::new();

    if (target == "all" || target == "cursor") && configure_cursor(root, options.force)? {
        configured.push("Cursor (.cursor/rules/strata.mdc, .cursor/mcp.json)");
    }

    if (target == "all" || target == "claude" || target == "claude-code")
        && configure_claude_code(root, options.force)?
    {
        configured.push("Claude Code (.claude/settings.json, CLAUDE.md)");
    }

    if (target == "all" || target == "codex") && configure_codex(root, options.force)? {
        configured.push("Codex (AGENTS.md, .codex/config.toml)");
    }

    if (target == "all" || target == "gemini" || target == "antigravity")
        && configure_gemini(root, options.force)?
    {
        configured.push("Gemini/Antigravity (.gemini/GEMINI.md)");
    }

    println!("\n✨ Strata initialized successfully!");
    if configured.is_empty() {
        println!("  No new host configurations were needed (already up to date).");
    } else {
        println!("  Configured hosts:");
        for host in configured {
            println!("    ✓ {host}");
        }
    }
    println!("\n  Universal memory transport is active via MCP and lifecycle hooks.\n");

    Ok(())
}

fn configure_cursor(root: &Path, _force: bool) -> Result<bool> {
    let cursor_dir = root.join(".cursor");
    let rules_dir = cursor_dir.join("rules");
    fs::create_dir_all(&rules_dir).context("Failed to create .cursor/rules directory")?;

    // 1. .cursor/rules/strata.mdc
    let rule_file = rules_dir.join("strata.mdc");
    let rule_content = r#"---
description: Strata Persistent Cognitive Memory Integration
globs: *
alwaysApply: true
---
# Strata Memory Rules
- Before starting complex tasks or refactorings, run `memory_search` to check prior architectural decisions and known failure anti-patterns.
- When an approach encounters unexpected obstacles or fails, record the anti-pattern via `memory_write` (type: negative_pattern) or execute via `strata hook wrap -- <cmd>`.
- After validating key architectural solutions or decisions, persist durable insights using `memory_write`.
- Dereference memory handles only when deep context is required.
"#;
    fs::write(&rule_file, rule_content)?;

    // 2. .cursor/mcp.json
    let mcp_file = cursor_dir.join("mcp.json");
    let mcp_config = serde_json::json!({
        "mcpServers": {
            "strata": {
                "command": "strata",
                "args": ["mcp"]
            }
        }
    });
    let formatted = serde_json::to_string_pretty(&mcp_config)?;
    fs::write(&mcp_file, formatted)?;

    Ok(true)
}

fn configure_claude_code(root: &Path, _force: bool) -> Result<bool> {
    let claude_dir = root.join(".claude");
    let hooks_dir = claude_dir.join("hooks");
    fs::create_dir_all(&hooks_dir).context("Failed to create .claude/hooks directory")?;

    // 1. .claude/settings.json with lifecycle hooks
    let settings_file = claude_dir.join("settings.json");
    let hooks_config = serde_json::json!({
        "hooks": {
            "SessionStart": {
                "command": "strata hook session-start"
            },
            "UserPromptSubmit": {
                "command": "strata hook user-prompt --query \"$PROMPT\""
            },
            "Compact": {
                "command": "strata hook compact"
            },
            "SessionEnd": {
                "command": "strata hook session-end"
            },
            "PostToolExecution": {
                "command": "strata hook post-tool --tool \"$TOOL_NAME\" --error \"$ERROR\""
            }
        }
    });
    let formatted = serde_json::to_string_pretty(&hooks_config)?;
    fs::write(&settings_file, formatted)?;

    // 2. CLAUDE.md instruction block
    let claude_md = root.join("CLAUDE.md");
    let instruction_block = format!(
        "{STRATA_MARKER_START}\n## Strata Persistent Memory\n- Contextual memory and known anti-patterns are automatically injected via hooks on session start and prompt submit.\n- Use the MCP tools (`memory_search`, `memory_get`, `memory_write`, `memory_digest`) when exploring context or persisting verified architectural decisions.\n- Execute build/test commands via `strata hook wrap -- <cmd>` to automatically synthesize failure anti-patterns out-of-band.\n- Record negative patterns immediately upon encountering dead-ends or tool errors.\n{STRATA_MARKER_END}\n"
    );
    inject_instruction_block(&claude_md, &instruction_block)?;

    Ok(true)
}

fn configure_codex(root: &Path, _force: bool) -> Result<bool> {
    let codex_dir = root.join(".codex");
    fs::create_dir_all(&codex_dir).context("Failed to create .codex directory")?;

    // 1. .codex/config.toml
    let config_file = codex_dir.join("config.toml");
    let config_content = r#"[mcp.servers.strata]
command = "strata"
args = ["mcp"]
"#;
    fs::write(&config_file, config_content)?;

    // 2. AGENTS.md instruction block
    let agents_md = root.join("AGENTS.md");
    let instruction_block = format!(
        "{STRATA_MARKER_START}\n## Strata Memory Protocol\n- Consult Strata memory tools (`memory_search`, `memory_get`) before planning non-trivial tasks.\n- Check known failure anti-patterns before running destructive or complex operations.\n- Wrap test/build commands with `strata hook wrap -- <cmd>` to capture compiler failures out-of-band.\n- Record durable takeaways via `memory_write`.\n{STRATA_MARKER_END}\n"
    );
    inject_instruction_block(&agents_md, &instruction_block)?;

    Ok(true)
}

fn configure_gemini(root: &Path, _force: bool) -> Result<bool> {
    let gemini_dir = root.join(".gemini");
    fs::create_dir_all(&gemini_dir).context("Failed to create .gemini directory")?;

    let gemini_md = gemini_dir.join("GEMINI.md");
    let instruction_block = format!(
        "{STRATA_MARKER_START}\n## Strata Memory Protocol\n- Check Strata memory pointers and known anti-patterns before complex tasks.\n- Wrap build/test runs with `strata hook wrap -- <cmd>` to automatically learn failure guardrails.\n- Persist verified solutions and architectural guidelines with `memory_write`.\n{STRATA_MARKER_END}\n"
    );
    inject_instruction_block(&gemini_md, &instruction_block)?;

    Ok(true)
}

fn inject_instruction_block(path: &Path, block: &str) -> Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    if let (Some(start_idx), Some(end_idx)) = (
        existing.find(STRATA_MARKER_START),
        existing.find(STRATA_MARKER_END),
    ) {
        let before = &existing[..start_idx];
        let after = &existing[end_idx + STRATA_MARKER_END.len()..];
        let new_content = format!("{before}{block}{after}");
        fs::write(path, new_content.trim_start())?;
    } else {
        let new_content = if existing.trim().is_empty() {
            block.to_string()
        } else {
            format!("{}\n\n{}", existing.trim_end(), block)
        };
        fs::write(path, new_content)?;
    }

    Ok(())
}
