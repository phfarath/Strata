use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_STRATA_ENDPOINT: &str = "https://strata.pedrofarath.me";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StrataConfig {
    pub endpoint: Option<String>,
    pub token: Option<String>,
    pub workspace_id: Option<String>,
    pub workspace_slug: Option<String>,
    pub user_email: Option<String>,
    pub jwt: Option<String>,
}

impl StrataConfig {
    /// Return the standard config file path `~/.strata/config.toml`
    pub fn config_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".strata").join("config.toml"))
    }

    /// Load config from disk, returning default if not found
    pub fn load() -> Self {
        match Self::config_path() {
            Ok(path) => Self::load_from_path(&path).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file at: {}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse TOML config at: {}", path.display()))?;
        Ok(config)
    }

    /// Save current config to disk at `~/.strata/config.toml`
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;
        fs::write(path, content)
            .with_context(|| format!("Failed to write config file to: {}", path.display()))?;
        Ok(())
    }

    /// Clear saved authentication credentials
    pub fn clear() -> Result<()> {
        let mut config = Self::load();
        config.token = None;
        config.jwt = None;
        config.user_email = None;
        config.save()
    }

    /// Resolve the sync endpoint: CLI arg > ENV `STRATA_SYNC_ENDPOINT` > Config file > Default
    pub fn resolve_endpoint(arg: Option<&str>) -> String {
        if let Some(a) = arg {
            if !a.trim().is_empty() {
                return a.trim().to_string();
            }
        }
        if let Ok(env_val) = std::env::var("STRATA_SYNC_ENDPOINT") {
            if !env_val.trim().is_empty() {
                return env_val.trim().to_string();
            }
        }
        let cfg = Self::load();
        if let Some(ep) = cfg.endpoint {
            if !ep.trim().is_empty() {
                return ep;
            }
        }
        DEFAULT_STRATA_ENDPOINT.to_string()
    }

    /// Resolve the sync token: CLI arg > ENV `STRATA_SYNC_TOKEN` / `STRATA_AUTH_TOKEN` > Config file
    pub fn resolve_token(arg: Option<&str>) -> Option<String> {
        if let Some(a) = arg {
            if !a.trim().is_empty() {
                return Some(a.trim().to_string());
            }
        }
        if let Ok(t) =
            std::env::var("STRATA_SYNC_TOKEN").or_else(|_| std::env::var("STRATA_AUTH_TOKEN"))
        {
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
        let cfg = Self::load();
        cfg.token.filter(|t| !t.trim().is_empty())
    }

    /// Resolve the workspace ID: CLI arg > ENV `STRATA_WORKSPACE_ID` > Config file > "default"
    pub fn resolve_workspace(arg: Option<&str>) -> String {
        if let Some(a) = arg {
            if !a.trim().is_empty() && a.trim() != "default" {
                return a.trim().to_string();
            }
        }
        if let Ok(ws) = std::env::var("STRATA_WORKSPACE_ID") {
            if !ws.trim().is_empty() {
                return ws.trim().to_string();
            }
        }
        let cfg = Self::load();
        cfg.workspace_slug
            .or(cfg.workspace_id)
            .unwrap_or_else(|| "default".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_save_load_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("strata-test-{}", uuid::Uuid::new_v4()));
        let config_file = temp_dir.join("config.toml");

        let config = StrataConfig {
            endpoint: Some("https://custom.strata.dev".to_string()),
            token: Some("strata_live_test_12345".to_string()),
            workspace_slug: Some("team-alpha".to_string()),
            user_email: Some("test@strata.dev".to_string()),
            ..Default::default()
        };

        config
            .save_to_path(&config_file)
            .expect("Failed to save config");

        let loaded = StrataConfig::load_from_path(&config_file).expect("Failed to load config");
        assert_eq!(
            loaded.endpoint.as_deref(),
            Some("https://custom.strata.dev")
        );
        assert_eq!(loaded.token.as_deref(), Some("strata_live_test_12345"));
        assert_eq!(loaded.workspace_slug.as_deref(), Some("team-alpha"));
        assert_eq!(loaded.user_email.as_deref(), Some("test@strata.dev"));

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
