use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use tracing::info;

#[derive(Debug, Clone)]
pub struct McpInstallOptions {
    pub client: Option<String>,
    pub global: bool,
    pub workspace_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct McpUninstallOptions {
    pub client: Option<String>,
    pub global: bool,
    pub workspace_dir: PathBuf,
}

/// Target configuration file descriptor
#[derive(Debug, Clone)]
pub struct McpTarget {
    pub name: &'static str,
    pub path: PathBuf,
    pub is_global: bool,
}

/// Discovers candidate MCP config paths based on the host system and options
pub fn resolve_mcp_targets(
    client_filter: Option<&str>,
    force_global: bool,
    workspace: &Path,
) -> Vec<McpTarget> {
    let mut targets = Vec::new();
    let filter = client_filter.unwrap_or("all").to_lowercase();
    let home = dirs::home_dir();

    // 1. Cursor
    if filter == "all" || filter == "cursor" {
        // Workspace target
        if !force_global {
            targets.push(McpTarget {
                name: "Cursor (Workspace)",
                path: workspace.join(".cursor").join("mcp.json"),
                is_global: false,
            });
        }
        // Global target
        if let Some(ref h) = home {
            targets.push(McpTarget {
                name: "Cursor (Global)",
                path: h.join(".cursor").join("mcp.json"),
                is_global: true,
            });
        }
    }

    // 2. Claude Desktop
    if filter == "all" || filter == "claude" || filter == "claude-desktop" {
        let claude_config_path = if cfg!(target_os = "macos") {
            home.as_ref().map(|h| {
                h.join("Library")
                    .join("Application Support")
                    .join("Claude")
                    .join("claude_desktop_config.json")
            })
        } else if cfg!(target_os = "windows") {
            dirs::config_dir().map(|c| c.join("Claude").join("claude_desktop_config.json"))
        } else {
            home.as_ref().map(|h| {
                h.join(".config")
                    .join("Claude")
                    .join("claude_desktop_config.json")
            })
        };

        if let Some(path) = claude_config_path {
            targets.push(McpTarget {
                name: "Claude Desktop (Global)",
                path,
                is_global: true,
            });
        }
    }

    // 3. Windsurf (Cascade)
    if filter == "all" || filter == "windsurf" {
        if !force_global {
            targets.push(McpTarget {
                name: "Windsurf (Workspace)",
                path: workspace.join(".windsurf").join("mcp.json"),
                is_global: false,
            });
        }
        if let Some(ref h) = home {
            targets.push(McpTarget {
                name: "Windsurf (Global)",
                path: h.join(".codeium").join("windsurf").join("mcp_config.json"),
                is_global: true,
            });
        }
    }

    // 4. Claude Code CLI
    if filter == "all" || filter == "claude-code" {
        if let Some(ref h) = home {
            targets.push(McpTarget {
                name: "Claude Code CLI (Global)",
                path: h.join(".claude.json"),
                is_global: true,
            });
        }
    }

    targets
}

/// Safely merges an MCP server configuration into existing JSON text without destroying other servers
pub fn merge_mcp_server(
    original_json: &str,
    server_name: &str,
    command: &str,
    args: &[&str],
) -> Result<String> {
    let mut root: Value = if original_json.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(original_json).unwrap_or_else(|_| json!({}))
    };

    if !root.is_object() {
        root = json!({});
    }

    let root_obj = root.as_object_mut().context("Root must be a JSON object")?;

    // Ensure "mcpServers" key exists and is an object
    if !root_obj.contains_key("mcpServers") || !root_obj["mcpServers"].is_object() {
        root_obj.insert("mcpServers".to_string(), json!({}));
    }

    let mcp_servers = root_obj
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
        .context("Failed to get mcpServers as object")?;

    // Inject or update server definition
    let server_entry = json!({
        "command": command,
        "args": args
    });

    mcp_servers.insert(server_name.to_string(), server_entry);

    let formatted = serde_json::to_string_pretty(&root)?;
    Ok(formatted)
}

/// Removes an MCP server configuration from an existing JSON text
pub fn remove_mcp_server(original_json: &str, server_name: &str) -> Result<(String, bool)> {
    if original_json.trim().is_empty() {
        return Ok(("{}".to_string(), false));
    }

    let mut root: Value = match serde_json::from_str(original_json) {
        Ok(v) => v,
        Err(_) => return Ok((original_json.to_string(), false)),
    };

    let mut removed = false;
    if let Some(root_obj) = root.as_object_mut() {
        if let Some(mcp_servers) = root_obj.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
            if mcp_servers.remove(server_name).is_some() {
                removed = true;
            }
        }
    }

    let formatted = serde_json::to_string_pretty(&root)?;
    Ok((formatted, removed))
}

/// Executes the `strata mcp install` command
pub fn run_mcp_install(options: McpInstallOptions) -> Result<()> {
    info!("Running smart MCP auto-installation...");
    let targets = resolve_mcp_targets(
        options.client.as_deref(),
        options.global,
        &options.workspace_dir,
    );

    if targets.is_empty() {
        println!("⚠️ No matching MCP targets found for client filter: {:?}", options.client);
        return Ok(());
    }

    println!("\n🔌 Strata Universal MCP Auto-Installer\n");
    let mut installed_count = 0;

    for target in &targets {
        // Skip global if user explicitly requested only workspace
        if options.workspace_dir != Path::new(".") && !options.global && target.is_global {
            continue;
        }

        let file_exists = target.path.exists();
        let original_content = if file_exists {
            fs::read_to_string(&target.path).unwrap_or_default()
        } else {
            String::new()
        };

        match merge_mcp_server(&original_content, "strata", "strata", &["mcp"]) {
            Ok(updated_json) => {
                if let Some(parent) = target.path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Err(e) = fs::write(&target.path, updated_json) {
                    println!("  ❌ {} ({}): failed to write: {}", target.name, target.path.display(), e);
                } else {
                    println!("  ✓ {} -> {}", target.name, target.path.display());
                    installed_count += 1;
                }
            }
            Err(e) => {
                println!("  ❌ {} ({}): failed to merge config: {}", target.name, target.path.display(), e);
            }
        }
    }

    if installed_count > 0 {
        println!(
            "\n✨ Successfully configured Strata MCP server across {} client(s)!",
            installed_count
        );
        println!("  Restart or reload your editor to activate persistent memory tools.\n");
    } else {
        println!("\n⚠️ No configuration files were updated.\n");
    }

    Ok(())
}

/// Executes the `strata mcp uninstall` command
pub fn run_mcp_uninstall(options: McpUninstallOptions) -> Result<()> {
    info!("Running MCP uninstallation...");
    let targets = resolve_mcp_targets(
        options.client.as_deref(),
        options.global,
        &options.workspace_dir,
    );

    println!("\n🗑️  Strata MCP Uninstaller\n");
    let mut removed_count = 0;

    for target in &targets {
        if !target.path.exists() {
            continue;
        }

        let content = fs::read_to_string(&target.path).unwrap_or_default();
        match remove_mcp_server(&content, "strata") {
            Ok((updated_json, removed)) => {
                if removed {
                    if let Err(e) = fs::write(&target.path, updated_json) {
                        println!("  ❌ {}: failed to write: {}", target.name, e);
                    } else {
                        println!("  ✓ Removed from {}", target.name);
                        removed_count += 1;
                    }
                }
            }
            Err(e) => {
                println!("  ❌ {}: error processing config: {}", target.name, e);
            }
        }
    }

    if removed_count > 0 {
        println!("\n✨ Successfully uninstalled Strata from {} client(s).\n", removed_count);
    } else {
        println!("\n  Strata was not found in any discovered client configuration.\n");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_mcp_server_into_empty_string() {
        let result = merge_mcp_server("", "strata", "strata", &["mcp"]).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(
            parsed["mcpServers"]["strata"]["command"],
            "strata"
        );
        assert_eq!(
            parsed["mcpServers"]["strata"]["args"][0],
            "mcp"
        );
    }

    #[test]
    fn test_merge_mcp_server_preserves_existing_tools() {
        let existing = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"]
                },
                "github": {
                    "command": "docker",
                    "args": ["run", "-i", "mcp/github"]
                }
            },
            "customSetting": 42
        }"#;

        let result = merge_mcp_server(existing, "strata", "strata", &["mcp"]).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        // Check that existing servers are 100% intact
        assert_eq!(parsed["mcpServers"]["filesystem"]["command"], "npx");
        assert_eq!(parsed["mcpServers"]["github"]["command"], "docker");
        assert_eq!(parsed["customSetting"], 42);

        // Check that strata was injected
        assert_eq!(parsed["mcpServers"]["strata"]["command"], "strata");
        assert_eq!(parsed["mcpServers"]["strata"]["args"][0], "mcp");
    }

    #[test]
    fn test_merge_mcp_server_updates_existing_strata_entry() {
        let existing = r#"{
            "mcpServers": {
                "strata": {
                    "command": "old-strata",
                    "args": ["legacy-flag"]
                }
            }
        }"#;

        let result = merge_mcp_server(existing, "strata", "strata", &["mcp", "--verbose"]).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["mcpServers"]["strata"]["command"], "strata");
        assert_eq!(parsed["mcpServers"]["strata"]["args"][0], "mcp");
        assert_eq!(parsed["mcpServers"]["strata"]["args"][1], "--verbose");
    }

    #[test]
    fn test_remove_mcp_server_preserves_others() {
        let existing = r#"{
            "mcpServers": {
                "strata": {
                    "command": "strata",
                    "args": ["mcp"]
                },
                "github": {
                    "command": "docker"
                }
            }
        }"#;

        let (result, removed) = remove_mcp_server(existing, "strata").unwrap();
        assert!(removed);

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["mcpServers"].get("strata").is_none());
        assert!(parsed["mcpServers"].get("github").is_some());
    }

    #[test]
    fn test_remove_mcp_server_when_not_present() {
        let existing = r#"{
            "mcpServers": {
                "github": {
                    "command": "docker"
                }
            }
        }"#;

        let (result, removed) = remove_mcp_server(existing, "strata").unwrap();
        assert!(!removed);

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["mcpServers"].get("github").is_some());
    }
}
