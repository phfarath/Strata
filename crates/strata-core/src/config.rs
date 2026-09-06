use crate::errors::StrataError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_STRATA_ENDPOINT: &str = "https://strata.pedrofarath.me";

/// Strata runtime, reasoning, and cloud configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StrataConfig {
    /// Active reasoning provider (e.g. "ollama", "gemini", "anthropic", "openai", "openrouter", "mock")
    pub provider: Option<String>,

    /// Model slug or identifier (e.g. "qwen2.5-coder:7b", "gemini-2.0-flash", "claude-3-5-sonnet-20241022", "gpt-4o-mini")
    pub model: Option<String>,

    /// API key for the cloud provider (optional if set via environment variables)
    pub api_key: Option<String>,

    /// Base URL for Ollama or OpenAI-compatible local endpoints (default: http://localhost:11434)
    pub ollama_url: Option<String>,

    /// Default sampling temperature for reasoning engines
    pub temperature: Option<f32>,

    /// Cloud sync and authentication settings
    pub endpoint: Option<String>,
    pub token: Option<String>,
    pub workspace_id: Option<String>,
    pub workspace_slug: Option<String>,
    pub user_email: Option<String>,
    pub jwt: Option<String>,
}

impl StrataConfig {
    /// Resolves the default global configuration path: `~/.strata/config.toml`.
    pub fn global_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".strata").join("config.toml"))
    }

    /// Resolves the workspace configuration path: `<workspace>/.strata/config.toml`.
    pub fn workspace_config_path(workspace: Option<&Path>) -> PathBuf {
        let base = workspace
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        base.join(".strata").join("config.toml")
    }

    /// Loads the configuration using standard cascade:
    /// Global config (`~/.strata/config.toml`) layered with Workspace config (`.strata/config.toml`).
    pub fn load() -> Self {
        let mut config = Self::default();

        // 1. Layer global config
        if let Some(global_path) = Self::global_config_path() {
            if global_path.exists() {
                match Self::load_from_file(&global_path) {
                    Ok(loaded) => config.merge(loaded),
                    Err(e) => tracing::warn!(
                        "Failed to parse global config file {}: {}",
                        global_path.display(),
                        e
                    ),
                }
            }
        }

        // 2. Layer workspace config
        let workspace_path = Self::workspace_config_path(None);
        if workspace_path.exists() {
            match Self::load_from_file(&workspace_path) {
                Ok(loaded) => config.merge(loaded),
                Err(e) => tracing::warn!(
                    "Failed to parse workspace config file {}: {}",
                    workspace_path.display(),
                    e
                ),
            }
        }

        config
    }

    /// Loads configuration from an explicit file path.
    pub fn load_from_file(path: &Path) -> Result<Self, StrataError> {
        let content = fs::read_to_string(path).map_err(|e| {
            StrataError::Configuration(format!(
                "Failed to read config file {}: {}",
                path.display(),
                e
            ))
        })?;
        toml::from_str(&content).map_err(|e| {
            StrataError::Configuration(format!("Failed to parse TOML in {}: {}", path.display(), e))
        })
    }

    /// Saves configuration to the specified file path.
    pub fn save_to_file(&self, path: &Path) -> Result<(), StrataError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                StrataError::Configuration(format!(
                    "Failed to create directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| {
            StrataError::Configuration(format!("Failed to serialize config to TOML: {}", e))
        })?;
        fs::write(path, content).map_err(|e| {
            StrataError::Configuration(format!(
                "Failed to write config file {}: {}",
                path.display(),
                e
            ))
        })?;
        Ok(())
    }

    /// Merges another config over this one (non-None fields in `other` take precedence).
    pub fn merge(&mut self, other: Self) {
        if other.provider.is_some() {
            self.provider = other.provider;
        }
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
        if other.ollama_url.is_some() {
            self.ollama_url = other.ollama_url;
        }
        if other.temperature.is_some() {
            self.temperature = other.temperature;
        }
        if other.endpoint.is_some() {
            self.endpoint = other.endpoint;
        }
        if other.token.is_some() {
            self.token = other.token;
        }
        if other.workspace_id.is_some() {
            self.workspace_id = other.workspace_id;
        }
        if other.workspace_slug.is_some() {
            self.workspace_slug = other.workspace_slug;
        }
        if other.user_email.is_some() {
            self.user_email = other.user_email;
        }
        if other.jwt.is_some() {
            self.jwt = other.jwt;
        }
    }

    /// Sets a configuration key by string name.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), StrataError> {
        match key.to_lowercase().as_str() {
            "provider" => self.provider = Some(value.trim().to_lowercase()),
            "model" => self.model = Some(value.trim().to_string()),
            "api_key" | "key" => self.api_key = Some(value.trim().to_string()),
            "ollama_url" | "url" => self.ollama_url = Some(value.trim().to_string()),
            "temperature" | "temp" => {
                let parsed: f32 = value.trim().parse().map_err(|_| {
                    StrataError::Configuration(format!(
                        "Invalid temperature float value: '{}'",
                        value
                    ))
                })?;
                if parsed.is_nan() || !(0.0..=2.0).contains(&parsed) {
                    return Err(StrataError::Configuration(format!(
                        "Temperature must be between 0.0 and 2.0 (got {})",
                        parsed
                    )));
                }
                self.temperature = Some(parsed);
            }
            "endpoint" => self.endpoint = Some(value.trim().to_string()),
            "token" => self.token = Some(value.trim().to_string()),
            "workspace_id" => self.workspace_id = Some(value.trim().to_string()),
            "workspace_slug" => self.workspace_slug = Some(value.trim().to_string()),
            unknown => {
                return Err(StrataError::Configuration(format!(
                    "Unknown configuration key '{}'. Valid keys: provider, model, api_key, ollama_url, temperature, endpoint, token, workspace_id, workspace_slug",
                    unknown
                )));
            }
        }
        Ok(())
    }

    /// Gets a configuration value by key name.
    pub fn get_key(&self, key: &str) -> Option<String> {
        match key.to_lowercase().as_str() {
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "api_key" | "key" => self.api_key.as_ref().map(|k| {
                if k.len() > 8 {
                    format!("{}...{}", &k[..4], &k[k.len() - 4..])
                } else {
                    "***".to_string()
                }
            }),
            "ollama_url" | "url" => self.ollama_url.clone(),
            "temperature" | "temp" => self.temperature.map(|t| t.to_string()),
            _ => None,
        }
    }

    /// Returns a list of all key-value pairs formatted for display.
    pub fn list_keys(&self) -> Vec<(String, String)> {
        let mut list = Vec::new();
        list.push((
            "provider".to_string(),
            self.provider.clone().unwrap_or_else(|| "auto".to_string()),
        ));
        list.push((
            "model".to_string(),
            self.model.clone().unwrap_or_else(|| "default".to_string()),
        ));
        list.push((
            "api_key".to_string(),
            self.api_key
                .as_ref()
                .map(|k| {
                    if k.len() > 8 {
                        format!("{}...{}", &k[..4], &k[k.len() - 4..])
                    } else {
                        "***".to_string()
                    }
                })
                .unwrap_or_else(|| "(not set)".to_string()),
        ));
        list.push((
            "ollama_url".to_string(),
            self.ollama_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
        ));
        list.push((
            "temperature".to_string(),
            self.temperature
                .map(|t| t.to_string())
                .unwrap_or_else(|| "default (0.3)".to_string()),
        ));
        list
    }

    /// Return the standard global config file path `~/.strata/config.toml`
    pub fn config_path() -> Result<PathBuf, StrataError> {
        Self::global_config_path().ok_or_else(|| {
            StrataError::Configuration("Could not determine home directory".to_string())
        })
    }

    /// Save current config to disk at `~/.strata/config.toml`
    pub fn save(&self) -> Result<(), StrataError> {
        let path = Self::config_path()?;
        self.save_to_file(&path)
    }

    /// Alias for load_from_file for backward compatibility.
    pub fn load_from_path(path: &Path) -> Result<Self, StrataError> {
        Self::load_from_file(path)
    }

    /// Alias for save_to_file for backward compatibility.
    pub fn save_to_path(&self, path: &Path) -> Result<(), StrataError> {
        self.save_to_file(path)
    }

    /// Clear saved authentication credentials and persist
    pub fn clear() -> Result<(), StrataError> {
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
        "https://strata.pedrofarath.me".to_string()
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
    fn test_config_set_and_get() {
        let mut config = StrataConfig::default();
        config.set_key("provider", "ollama").unwrap();
        config.set_key("model", "qwen2.5-coder:7b").unwrap();
        config.set_key("temperature", "0.5").unwrap();

        assert_eq!(config.get_key("provider"), Some("ollama".to_string()));
        assert_eq!(
            config.get_key("model"),
            Some("qwen2.5-coder:7b".to_string())
        );
        assert_eq!(config.get_key("temperature"), Some("0.5".to_string()));
    }

    #[test]
    fn test_config_merge() {
        let mut base = StrataConfig {
            provider: Some("openrouter".to_string()),
            model: Some("openrouter/free".to_string()),
            ..Default::default()
        };
        let override_cfg = StrataConfig {
            provider: Some("ollama".to_string()),
            ..Default::default()
        };
        base.merge(override_cfg);

        assert_eq!(base.provider, Some("ollama".to_string()));
        assert_eq!(base.model, Some("openrouter/free".to_string()));
    }
}
