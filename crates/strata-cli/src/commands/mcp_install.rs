use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
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
        serde_json::from_str(original_json)
            .context("existing config is not valid JSON; refusing to overwrite it")?
    };

    if !root.is_object() {
        bail!("existing config root is not a JSON object; refusing to overwrite it");
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
        if let Some(mcp_servers) = root_obj
            .get_mut("mcpServers")
            .and_then(|v| v.as_object_mut())
        {
            if mcp_servers.remove(server_name).is_some() {
                removed = true;
            }
        }
    }

    let formatted = serde_json::to_string_pretty(&root)?;
    Ok((formatted, removed))
}

fn timestamp_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn backup_file(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mcp-config");
    let backup = path.with_file_name(format!("{file_name}.strata-backup-{}", timestamp_nanos()));

    fs::copy(path, &backup).with_context(|| {
        format!(
            "failed to back up {} to {}",
            path.display(),
            backup.display()
        )
    })?;

    Ok(backup)
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp = parent.join(format!(
        ".strata-tmp-{}-{}",
        std::process::id(),
        timestamp_nanos()
    ));

    fs::write(&temp, contents)
        .with_context(|| format!("failed to write temporary file {}", temp.display()))?;

    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error).with_context(|| format!("failed to replace {}", path.display()));
    }

    Ok(())
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
        println!(
            "⚠️ No matching MCP targets found for client filter: {:?}",
            options.client
        );
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
            match fs::read_to_string(&target.path) {
                Ok(content) => content,
                Err(e) => {
                    println!(
                        "  ❌ {} ({}): failed to read; left untouched: {}",
                        target.name,
                        target.path.display(),
                        e
                    );
                    continue;
                }
            }
        } else {
            String::new()
        };

        match merge_mcp_server(&original_content, "strata", "strata", &["mcp"]) {
            Ok(updated_json) => {
                if let Some(parent) = target.path.parent() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        println!(
                            "  ❌ {} ({}): failed to create parent directory: {}",
                            target.name,
                            target.path.display(),
                            e
                        );
                        continue;
                    }
                }

                let backup = if file_exists {
                    match backup_file(&target.path) {
                        Ok(path) => Some(path),
                        Err(e) => {
                            println!(
                                "  ❌ {} ({}): failed to create backup; left untouched: {}",
                                target.name,
                                target.path.display(),
                                e
                            );
                            continue;
                        }
                    }
                } else {
                    None
                };

                if let Err(e) = write_atomic(&target.path, &updated_json) {
                    println!(
                        "  ❌ {} ({}): failed to write atomically: {}",
                        target.name,
                        target.path.display(),
                        e
                    );
                } else {
                    match backup {
                        Some(backup) => println!(
                            "  ✓ {} -> {} (backup: {})",
                            target.name,
                            target.path.display(),
                            backup.display()
                        ),
                        None => println!("  ✓ {} -> {}", target.name, target.path.display()),
                    }
                    installed_count += 1;
                }
            }
            Err(e) => {
                println!(
                    "  ❌ {} ({}): invalid or unreadable config; left untouched: {:#}",
                    target.name,
                    target.path.display(),
                    e
                );
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
        println!(
            "\n✨ Successfully uninstalled Strata from {} client(s).\n",
            removed_count
        );
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

        assert_eq!(parsed["mcpServers"]["strata"]["command"], "strata");
        assert_eq!(parsed["mcpServers"]["strata"]["args"][0], "mcp");
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

    fn temp_workspace(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "strata-mcp-install-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn install_into(workspace: &std::path::Path) {
        run_mcp_install(McpInstallOptions {
            client: Some("cursor".to_string()),
            global: false,
            workspace_dir: workspace.to_path_buf(),
        })
        .unwrap();
    }

    #[test]
    fn test_merge_mcp_server_rejects_invalid_json() {
        let invalid = r#"{"mcpServers": {"filesystem": {"command": "npx"}"#;
        let err = merge_mcp_server(invalid, "strata", "strata", &["mcp"]).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("json"),
            "error should explain the config is invalid JSON: {err}"
        );
    }

    #[test]
    fn test_merge_mcp_server_rejects_non_object_root() {
        let err = merge_mcp_server("[]", "strata", "strata", &["mcp"]).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("object"),
            "error should explain the root must be an object: {err}"
        );
    }

    #[test]
    fn test_install_preserves_invalid_json_config() {
        let workspace = temp_workspace("invalid-json");
        let config_path = workspace.join(".cursor").join("mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let original = r#"{"mcpServers": {"filesystem": {"command": "npx"}"#;
        std::fs::write(&config_path, original).unwrap();

        install_into(&workspace);

        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn test_install_preserves_non_utf8_config() {
        let workspace = temp_workspace("non-utf8");
        let config_path = workspace.join(".cursor").join("mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let original = vec![0x7b, 0xff, 0xfe, 0x7d];
        std::fs::write(&config_path, &original).unwrap();

        install_into(&workspace);

        assert_eq!(std::fs::read(&config_path).unwrap(), original);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn test_install_creates_backup_and_preserves_existing_servers() {
        let workspace = temp_workspace("backup");
        let config_path = workspace.join(".cursor").join("mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let original = r#"{"mcpServers":{"github":{"command":"docker"}},"customSetting":42}"#;
        std::fs::write(&config_path, original).unwrap();

        install_into(&workspace);

        let updated: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(updated["mcpServers"]["github"]["command"], "docker");
        assert_eq!(updated["customSetting"], 42);
        assert_eq!(updated["mcpServers"]["strata"]["command"], "strata");

        let backup = std::fs::read_dir(config_path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("mcp.json.strata-backup-")
            })
            .expect("a backup of the original config must be created");
        assert_eq!(std::fs::read_to_string(backup.path()).unwrap(), original);

        let _ = std::fs::remove_dir_all(&workspace);
    }
}
