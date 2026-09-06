pub use strata_core::config::{StrataConfig, DEFAULT_STRATA_ENDPOINT};

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
            provider: Some("ollama".to_string()),
            model: Some("qwen2.5-coder:7b".to_string()),
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
        assert_eq!(loaded.provider.as_deref(), Some("ollama"));
        assert_eq!(loaded.model.as_deref(), Some("qwen2.5-coder:7b"));

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
