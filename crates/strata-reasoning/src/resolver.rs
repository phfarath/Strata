use std::sync::Arc;
use strata_core::config::StrataConfig;
use strata_core::errors::StrataError;
use strata_core::traits::ReasoningEngine;
use tracing::info;

use crate::adapters::{
    AnthropicAdapter, GeminiAdapter, OllamaAdapter, OpenAiAdapter, OpenRouterAdapter,
    DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL, DEFAULT_OPENROUTER_MODEL,
};
use crate::mock::MockReasoningEngine;

/// The type of provider that was successfully resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedProviderKind {
    Ollama { model: String, url: String },
    Gemini { model: String },
    Anthropic { model: String },
    OpenAi { model: String },
    OpenRouter { model: String },
    Mock,
}

impl std::fmt::Display for ResolvedProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ollama { model, url } => write!(f, "ollama ({model} @ {url})"),
            Self::Gemini { model } => write!(f, "gemini ({model})"),
            Self::Anthropic { model } => write!(f, "anthropic ({model})"),
            Self::OpenAi { model } => write!(f, "openai ({model})"),
            Self::OpenRouter { model } => write!(f, "openrouter ({model})"),
            Self::Mock => write!(f, "mock (deterministic offline)"),
        }
    }
}

/// A resolved reasoning provider containing its metadata and boxed engine.
pub struct ResolvedProvider {
    pub kind: ResolvedProviderKind,
    pub engine: Arc<dyn ReasoningEngine>,
}

impl std::fmt::Debug for ResolvedProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedProvider")
            .field("kind", &self.kind)
            .finish()
    }
}

/// Resolves the reasoning engine following the priority cascade:
/// 1. Explicit CLI arguments (`override_provider`, `override_model`)
/// 2. Loaded persistent config (`StrataConfig`)
/// 3. Auto-detect local Ollama (<1.5s probe)
/// 4. Google Gemini (`GEMINI_API_KEY` / `GOOGLE_API_KEY`)
/// 5. Anthropic (`ANTHROPIC_API_KEY`)
/// 6. OpenAI (`OPENAI_API_KEY`)
/// 7. OpenRouter (`OPENROUTER_API_KEY`)
/// 8. Fallback to MockReasoningEngine
pub async fn resolve_reasoning_engine(
    config: &StrataConfig,
    override_provider: Option<&str>,
    override_model: Option<&str>,
) -> Result<ResolvedProvider, StrataError> {
    // 1. Explicit provider override from CLI arguments (filter out "auto" to trigger cascade)
    let override_provider = override_provider.filter(|p| *p != "auto");
    if let Some(prov) = override_provider {
        return resolve_explicit_provider(prov, override_model, config).await;
    }

    // 2. Explicit provider configured in config file
    if let Some(ref prov) = config.provider {
        if prov != "auto" {
            return resolve_explicit_provider(prov, override_model, config).await;
        }
    }

    // 3. Automatic Cascade: Check local Ollama auto-detect first (0-cost, 100% offline)
    let ollama_url = config
        .ollama_url
        .clone()
        .or_else(|| std::env::var("OLLAMA_HOST").ok())
        .or_else(|| std::env::var("STRATA_OLLAMA_URL").ok())
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string());

    if let Some(models) = OllamaAdapter::probe_models(&ollama_url).await {
        // Only auto-select Ollama if there is at least one installed model or an explicit model was requested
        if !models.is_empty() || override_model.is_some() || config.model.is_some() {
            let chosen_model = override_model
                .map(|s| s.to_string())
                .or_else(|| config.model.clone())
                .or_else(|| {
                    models
                        .iter()
                        .find(|m| m.contains("coder") || m.contains("code"))
                        .cloned()
                })
                .or_else(|| {
                    models
                        .iter()
                        .find(|m| {
                            m.contains("qwen") || m.contains("llama") || m.contains("deepseek")
                        })
                        .cloned()
                })
                .or_else(|| models.first().cloned())
                .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_string());

            info!(
                "Auto-detected local Ollama instance at {} with model {}",
                ollama_url, chosen_model
            );
            let adapter = OllamaAdapter::new(&chosen_model).with_base_url(&ollama_url);
            return Ok(ResolvedProvider {
                kind: ResolvedProviderKind::Ollama {
                    model: chosen_model,
                    url: ollama_url,
                },
                engine: Arc::new(adapter),
            });
        }
    }

    let model_arg = override_model
        .map(|s| s.to_string())
        .or_else(|| config.model.clone());

    // 4. Google Gemini (env vars or config.api_key with AIza prefix or fallback)
    let gemini_key = std::env::var("GEMINI_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_API_KEY"))
        .or_else(|_| std::env::var("STRATA_GEMINI_API_KEY"))
        .ok()
        .or_else(|| {
            config
                .api_key
                .clone()
                .filter(|k| k.starts_with("AIza") || k.len() == 39)
        });
    if let Some(key) = gemini_key {
        let model = model_arg
            .clone()
            .unwrap_or_else(|| "gemini-2.0-flash".to_string());
        let adapter = GeminiAdapter::new(key, &model);
        info!("Resolved Google Gemini provider with model {}", model);
        return Ok(ResolvedProvider {
            kind: ResolvedProviderKind::Gemini { model },
            engine: Arc::new(adapter),
        });
    }

    // 5. Anthropic (env vars or config.api_key with sk-ant prefix)
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("STRATA_ANTHROPIC_API_KEY"))
        .ok()
        .or_else(|| config.api_key.clone().filter(|k| k.starts_with("sk-ant-")));
    if let Some(key) = anthropic_key {
        let model = model_arg
            .clone()
            .unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string());
        let adapter = AnthropicAdapter::new(key, &model);
        info!("Resolved Anthropic provider with model {}", model);
        return Ok(ResolvedProvider {
            kind: ResolvedProviderKind::Anthropic { model },
            engine: Arc::new(adapter),
        });
    }

    // 6. OpenAI (env vars or config.api_key with sk-proj / sk- prefix)
    let openai_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("STRATA_OPENAI_API_KEY"))
        .ok()
        .or_else(|| {
            config.api_key.clone().filter(|k| {
                k.starts_with("sk-") && !k.starts_with("sk-ant-") && !k.starts_with("sk-or-")
            })
        });
    if let Some(key) = openai_key {
        let model = model_arg
            .clone()
            .unwrap_or_else(|| "gpt-4o-mini".to_string());
        let adapter = OpenAiAdapter::new(key, &model);
        info!("Resolved OpenAI provider with model {}", model);
        return Ok(ResolvedProvider {
            kind: ResolvedProviderKind::OpenAi { model },
            engine: Arc::new(adapter),
        });
    }

    // 7. OpenRouter (env vars or config.api_key with sk-or prefix or general key)
    let openrouter_key = std::env::var("OPENROUTER_API_KEY")
        .or_else(|_| std::env::var("STRATA_OPENROUTER_API_KEY"))
        .ok()
        .or_else(|| config.api_key.clone().filter(|k| k.starts_with("sk-or-")));
    if let Some(key) = openrouter_key {
        let model = model_arg.unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.to_string());
        let adapter = OpenRouterAdapter::new(key, &model);
        info!("Resolved OpenRouter provider with model {}", model);
        return Ok(ResolvedProvider {
            kind: ResolvedProviderKind::OpenRouter { model },
            engine: Arc::new(adapter),
        });
    }

    // 8. Deterministic Fallback: MockReasoningEngine
    info!("No active LLM providers or API keys detected; falling back to MockReasoningEngine");
    Ok(ResolvedProvider {
        kind: ResolvedProviderKind::Mock,
        engine: Arc::new(MockReasoningEngine::new()),
    })
}

async fn resolve_explicit_provider(
    provider_name: &str,
    model_override: Option<&str>,
    config: &StrataConfig,
) -> Result<ResolvedProvider, StrataError> {
    let model_choice = model_override
        .map(|s| s.to_string())
        .or_else(|| config.model.clone());

    match provider_name.to_lowercase().as_str() {
        "ollama" | "local" => {
            let url = config
                .ollama_url
                .clone()
                .or_else(|| std::env::var("OLLAMA_HOST").ok())
                .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string());
            let model = model_choice.unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_string());
            let adapter = OllamaAdapter::new(&model).with_base_url(&url);
            Ok(ResolvedProvider {
                kind: ResolvedProviderKind::Ollama { model, url },
                engine: Arc::new(adapter),
            })
        }
        "gemini" | "google" => {
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("GEMINI_API_KEY").ok())
                .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
                .or_else(|| std::env::var("STRATA_GEMINI_API_KEY").ok())
                .ok_or_else(|| {
                    StrataError::Configuration(
                        "Gemini API key not found in config or GEMINI_API_KEY / GOOGLE_API_KEY environment variables".to_string(),
                    )
                })?;
            let model = model_choice.unwrap_or_else(|| "gemini-2.0-flash".to_string());
            let adapter = GeminiAdapter::new(api_key, &model);
            Ok(ResolvedProvider {
                kind: ResolvedProviderKind::Gemini { model },
                engine: Arc::new(adapter),
            })
        }
        "anthropic" | "claude" => {
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .or_else(|| std::env::var("STRATA_ANTHROPIC_API_KEY").ok())
                .ok_or_else(|| {
                    StrataError::Configuration(
                        "Anthropic API key not found in config or ANTHROPIC_API_KEY environment variable".to_string(),
                    )
                })?;
            let model =
                model_choice.unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string());
            let adapter = AnthropicAdapter::new(api_key, &model);
            Ok(ResolvedProvider {
                kind: ResolvedProviderKind::Anthropic { model },
                engine: Arc::new(adapter),
            })
        }
        "openai" | "gpt" => {
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .or_else(|| std::env::var("STRATA_OPENAI_API_KEY").ok())
                .ok_or_else(|| {
                    StrataError::Configuration(
                        "OpenAI API key not found in config or OPENAI_API_KEY environment variable".to_string(),
                    )
                })?;
            let model = model_choice.unwrap_or_else(|| "gpt-4o-mini".to_string());
            let adapter = OpenAiAdapter::new(api_key, &model);
            Ok(ResolvedProvider {
                kind: ResolvedProviderKind::OpenAi { model },
                engine: Arc::new(adapter),
            })
        }
        "openrouter" => {
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                .or_else(|| std::env::var("STRATA_OPENROUTER_API_KEY").ok())
                .ok_or_else(|| {
                    StrataError::Configuration(
                        "OpenRouter API key not found in config or OPENROUTER_API_KEY environment variable".to_string(),
                    )
                })?;
            let model =
                model_choice.unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.to_string());
            let adapter = OpenRouterAdapter::new(api_key, &model);
            Ok(ResolvedProvider {
                kind: ResolvedProviderKind::OpenRouter { model },
                engine: Arc::new(adapter),
            })
        }
        "mock" => Ok(ResolvedProvider {
            kind: ResolvedProviderKind::Mock,
            engine: Arc::new(MockReasoningEngine::new()),
        }),
        unknown => Err(StrataError::Configuration(format!(
            "Unsupported provider '{}'. Supported providers: ollama, gemini, anthropic, openai, openrouter, mock",
            unknown
        ))),
    }
}

/// Probes a reasoning provider to verify connectivity or configuration without incurring substantial token usage.
pub async fn probe_provider(
    provider_name: &str,
    model: Option<&str>,
    config: &StrataConfig,
) -> Result<String, StrataError> {
    match provider_name.to_lowercase().as_str() {
        "auto" => {
            let resolved = resolve_reasoning_engine(config, None, model).await?;
            Ok(format!(
                "Auto resolution successfully resolved: {}",
                resolved.kind
            ))
        }
        "ollama" | "local" => {
            let url = config
                .ollama_url
                .clone()
                .or_else(|| std::env::var("OLLAMA_HOST").ok())
                .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string());
            if !OllamaAdapter::ping_available(&url).await {
                return Err(StrataError::Reasoning(format!(
                    "Cannot reach Ollama at {}. Ensure Ollama is installed and running ('ollama serve').",
                    url
                )));
            }
            let models = OllamaAdapter::list_installed_models(&url).await;
            Ok(format!(
                "Ollama is reachable at {}. Installed models: [{}].",
                url,
                if models.is_empty() {
                    "none detected (run 'ollama pull qwen2.5-coder:7b')".to_string()
                } else {
                    models.join(", ")
                }
            ))
        }
        "gemini" | "google" => {
            let has_key = config.api_key.is_some()
                || std::env::var("GEMINI_API_KEY").is_ok()
                || std::env::var("GOOGLE_API_KEY").is_ok();
            if !has_key {
                return Err(StrataError::Configuration(
                    "No Gemini API key found in config or GEMINI_API_KEY / GOOGLE_API_KEY".to_string(),
                ));
            }
            let m = model.unwrap_or("gemini-2.0-flash");
            Ok(format!("Google Gemini is configured with model '{m}'."))
        }
        "anthropic" | "claude" => {
            let has_key =
                config.api_key.is_some() || std::env::var("ANTHROPIC_API_KEY").is_ok();
            if !has_key {
                return Err(StrataError::Configuration(
                    "No Anthropic API key found in config or ANTHROPIC_API_KEY".to_string(),
                ));
            }
            let m = model.unwrap_or("claude-3-5-sonnet-20241022");
            Ok(format!("Anthropic Claude is configured with model '{m}'."))
        }
        "openai" | "gpt" => {
            let has_key = config.api_key.is_some() || std::env::var("OPENAI_API_KEY").is_ok();
            if !has_key {
                return Err(StrataError::Configuration(
                    "No OpenAI API key found in config or OPENAI_API_KEY".to_string(),
                ));
            }
            let m = model.unwrap_or("gpt-4o-mini");
            Ok(format!("OpenAI is configured with model '{m}'."))
        }
        "openrouter" => {
            let has_key =
                config.api_key.is_some() || std::env::var("OPENROUTER_API_KEY").is_ok();
            if !has_key {
                return Err(StrataError::Configuration(
                    "No OpenRouter API key found in config or OPENROUTER_API_KEY".to_string(),
                ));
            }
            let m = model.unwrap_or(DEFAULT_OPENROUTER_MODEL);
            Ok(format!("OpenRouter is configured with model '{m}'."))
        }
        "mock" => Ok("Mock deterministic reasoning engine is active and ready.".to_string()),
        unknown => Err(StrataError::Configuration(format!(
            "Unsupported provider '{}'. Supported providers: ollama, gemini, anthropic, openai, openrouter, mock",
            unknown
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resolve_explicit_mock_provider() {
        let config = StrataConfig::default();
        let resolved = resolve_reasoning_engine(&config, Some("mock"), None)
            .await
            .unwrap();
        assert_eq!(resolved.kind, ResolvedProviderKind::Mock);
        let res = resolved.engine.prompt(None, "ping", None).await.unwrap();
        assert!(res.contains("Mock response"));
    }

    #[tokio::test]
    async fn test_resolve_explicit_ollama_provider() {
        let config = StrataConfig {
            ollama_url: Some("http://127.0.0.1:11434".to_string()),
            ..Default::default()
        };
        let resolved =
            resolve_reasoning_engine(&config, Some("ollama"), Some("deepseek-coder:6.7b"))
                .await
                .unwrap();

        match resolved.kind {
            ResolvedProviderKind::Ollama { model, url } => {
                assert_eq!(model, "deepseek-coder:6.7b");
                assert_eq!(url, "http://127.0.0.1:11434");
            }
            _ => panic!("Expected Ollama resolved provider"),
        }
    }

    #[tokio::test]
    async fn test_resolve_explicit_provider_missing_key() {
        // Clear any env variables temporarily
        let old_anthropic = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");

        let config = StrataConfig::default();
        let res = resolve_reasoning_engine(&config, Some("anthropic"), None).await;
        assert!(res.is_err());
        match res.unwrap_err() {
            StrataError::Configuration(msg) => {
                assert!(msg.contains("Anthropic API key not found"));
            }
            other => panic!("Expected configuration error, got {:?}", other),
        }

        if let Some(k) = old_anthropic {
            std::env::set_var("ANTHROPIC_API_KEY", k);
        }
    }

    #[tokio::test]
    async fn test_resolve_unsupported_provider() {
        let config = StrataConfig::default();
        let res = resolve_reasoning_engine(&config, Some("unknown-ai"), None).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_probe_provider_mock() {
        let config = StrataConfig::default();
        let res = probe_provider("mock", None, &config).await.unwrap();
        assert!(res.contains("Mock deterministic reasoning engine"));
    }
}
