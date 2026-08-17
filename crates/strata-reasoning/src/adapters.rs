use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;
use strata_core::errors::StrataError;
use uuid::Uuid;

use crate::engine::{
    ChatMessage, PromptContext, ReasoningEngine, ReasoningOutput, Role, TokenUsage, ToolCall,
};

// ============================================================================
// OpenAI Adapter (also compatible with DeepSeek, Ollama, vLLM, OpenRouter)
// ============================================================================

pub struct OpenAiAdapter {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn new_openrouter(api_key: impl Into<String>, model_slug: impl Into<String>) -> Self {
        Self::new(api_key, model_slug).with_base_url("https://openrouter.ai/api/v1")
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[async_trait]
impl ReasoningEngine for OpenAiAdapter {
    async fn complete(&self, context: &PromptContext) -> Result<ReasoningOutput, StrataError> {
        let mut messages = Vec::new();

        if let Some(ref sys) = context.system_prompt {
            messages.push(json!({
                "role": "system",
                "content": sys,
            }));
        }

        for msg in &context.messages {
            match msg.role {
                Role::System => {
                    messages.push(json!({
                        "role": "system",
                        "content": msg.content,
                    }));
                }
                Role::User => {
                    messages.push(json!({
                        "role": "user",
                        "content": msg.content,
                    }));
                }
                Role::Assistant => {
                    let mut obj = json!({
                        "role": "assistant",
                        "content": if msg.content.is_empty() { serde_json::Value::Null } else { json!(msg.content) }
                    });
                    if let Some(ref tools) = msg.tool_calls {
                        let calls: Vec<_> = tools
                            .iter()
                            .map(|t| {
                                json!({
                                    "id": t.id,
                                    "type": "function",
                                    "function": {
                                        "name": t.name,
                                        "arguments": serde_json::to_string(&t.arguments).unwrap_or_default()
                                    }
                                })
                            })
                            .collect();
                        obj["tool_calls"] = json!(calls);
                    }
                    messages.push(obj);
                }
                Role::Tool => {
                    if let Some(ref results) = msg.tool_results {
                        for res in results {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": res.call_id,
                                "content": serde_json::to_string(&res.result).unwrap_or_default(),
                            }));
                        }
                    }
                }
            }
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
        });

        if let Some(temp) = context.temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(max_t) = context.max_tokens {
            body["max_tokens"] = json!(max_t);
        }

        if !context.tools.is_empty() {
            let tools: Vec<_> = context
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(tools);
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| StrataError::Reasoning(format!("OpenAI HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(StrataError::Reasoning(format!(
                "OpenAI API returned error status {}: {}",
                status, err_body
            )));
        }

        let res_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| StrataError::Reasoning(format!("Failed to parse OpenAI JSON response: {}", e)))?;

        let choice = res_json["choices"]
            .get(0)
            .ok_or_else(|| StrataError::Reasoning("OpenAI response had no choices".to_string()))?;

        let msg = &choice["message"];
        let content = msg["content"].as_str().map(|s| s.to_string());
        let finish_reason = choice["finish_reason"]
            .as_str()
            .unwrap_or("stop")
            .to_string();

        let mut tool_calls = Vec::new();
        if let Some(calls_val) = msg["tool_calls"].as_array() {
            for call in calls_val {
                let id = call["id"].as_str().unwrap_or_default().to_string();
                let name = call["function"]["name"].as_str().unwrap_or_default().to_string();
                let args_raw = call["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: serde_json::Value = serde_json::from_str(args_raw).unwrap_or(json!({}));
                tool_calls.push(ToolCall::new(id, name, arguments));
            }
        }

        let usage = res_json.get("usage").map(|u| TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(ReasoningOutput {
            content,
            tool_calls,
            finish_reason,
            usage,
        })
    }
}

#[async_trait]
impl strata_core::traits::ReasoningEngine for OpenAiAdapter {
    async fn prompt(
        &self,
        system: Option<&str>,
        user: &str,
        context: Option<serde_json::Value>,
    ) -> Result<String, StrataError> {
        let mut ctx = PromptContext::new().with_message(ChatMessage::user(user));
        if let Some(sys) = system {
            ctx = ctx.with_system(sys);
        }
        if let Some(meta) = context {
            ctx.metadata = meta;
        }
        let output = self.complete(&ctx).await?;
        Ok(output.content.unwrap_or_default())
    }
}

// ============================================================================
// OpenRouter Adapter
// ============================================================================

pub const DEFAULT_OPENROUTER_MODEL: &str = "meta-llama/llama-3.3-70b-instruct:free";

pub struct OpenRouterAdapter {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenRouterAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env(model_slug: Option<String>) -> Result<Self, StrataError> {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .or_else(|_| std::env::var("STRATA_OPENROUTER_API_KEY"))
            .map_err(|_| {
                StrataError::Configuration(
                    "OpenRouter API key not found in OPENROUTER_API_KEY or STRATA_OPENROUTER_API_KEY environment variables".to_string(),
                )
            })?;

        let model = model_slug.unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.to_string());
        Ok(Self::new(api_key, model))
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[async_trait]
impl ReasoningEngine for OpenRouterAdapter {
    async fn complete(&self, context: &PromptContext) -> Result<ReasoningOutput, StrataError> {
        let mut messages = Vec::new();

        if let Some(ref sys) = context.system_prompt {
            messages.push(json!({
                "role": "system",
                "content": sys,
            }));
        }

        for msg in &context.messages {
            match msg.role {
                Role::System => {
                    messages.push(json!({
                        "role": "system",
                        "content": msg.content,
                    }));
                }
                Role::User => {
                    messages.push(json!({
                        "role": "user",
                        "content": msg.content,
                    }));
                }
                Role::Assistant => {
                    let mut obj = json!({
                        "role": "assistant",
                        "content": if msg.content.is_empty() { serde_json::Value::Null } else { json!(msg.content) }
                    });
                    if let Some(ref tools) = msg.tool_calls {
                        let calls: Vec<_> = tools
                            .iter()
                            .map(|t| {
                                json!({
                                    "id": t.id,
                                    "type": "function",
                                    "function": {
                                        "name": t.name,
                                        "arguments": serde_json::to_string(&t.arguments).unwrap_or_default()
                                    }
                                })
                            })
                            .collect();
                        obj["tool_calls"] = json!(calls);
                    }
                    messages.push(obj);
                }
                Role::Tool => {
                    if let Some(ref results) = msg.tool_results {
                        for res in results {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": res.call_id,
                                "content": serde_json::to_string(&res.result).unwrap_or_default(),
                            }));
                        }
                    }
                }
            }
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
        });

        if let Some(temp) = context.temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(max_t) = context.max_tokens {
            body["max_tokens"] = json!(max_t);
        }

        if !context.tools.is_empty() {
            let tools: Vec<_> = context
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(tools);
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/strata-cognitive/strata")
            .header("X-Title", "Strata Cognitive Runtime")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| StrataError::Reasoning(format!("OpenRouter HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(StrataError::Reasoning(format!(
                "OpenRouter API returned error status {}: {}",
                status, err_body
            )));
        }

        let res_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| StrataError::Reasoning(format!("Failed to parse OpenRouter JSON response: {}", e)))?;

        let choice = res_json["choices"]
            .get(0)
            .ok_or_else(|| StrataError::Reasoning("OpenRouter response had no choices".to_string()))?;

        let msg = &choice["message"];
        let content = msg["content"].as_str().map(|s| s.to_string());
        let finish_reason = choice["finish_reason"]
            .as_str()
            .unwrap_or("stop")
            .to_string();

        let mut tool_calls = Vec::new();
        if let Some(calls_val) = msg["tool_calls"].as_array() {
            for call in calls_val {
                let id = call["id"].as_str().unwrap_or_default().to_string();
                let name = call["function"]["name"].as_str().unwrap_or_default().to_string();
                let args_raw = call["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: serde_json::Value = serde_json::from_str(args_raw).unwrap_or(json!({}));
                tool_calls.push(ToolCall::new(id, name, arguments));
            }
        }

        let usage = res_json.get("usage").map(|u| TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(ReasoningOutput {
            content,
            tool_calls,
            finish_reason,
            usage,
        })
    }
}

#[async_trait]
impl strata_core::traits::ReasoningEngine for OpenRouterAdapter {
    async fn prompt(
        &self,
        system: Option<&str>,
        user: &str,
        context: Option<serde_json::Value>,
    ) -> Result<String, StrataError> {
        let mut ctx = PromptContext::new().with_message(ChatMessage::user(user));
        if let Some(sys) = system {
            ctx = ctx.with_system(sys);
        }
        if let Some(meta) = context {
            ctx.metadata = meta;
        }
        let output = self.complete(&ctx).await?;
        Ok(output.content.unwrap_or_default())
    }
}

// ============================================================================
// Anthropic Adapter (Claude 3.5 Sonnet / Haiku / Opus)
// ============================================================================

pub struct AnthropicAdapter {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl AnthropicAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl ReasoningEngine for AnthropicAdapter {
    async fn complete(&self, context: &PromptContext) -> Result<ReasoningOutput, StrataError> {
        let mut messages = Vec::new();

        for msg in &context.messages {
            match msg.role {
                Role::User => {
                    messages.push(json!({
                        "role": "user",
                        "content": msg.content,
                    }));
                }
                Role::Assistant => {
                    let mut content_blocks = Vec::new();
                    if !msg.content.is_empty() {
                        content_blocks.push(json!({
                            "type": "text",
                            "text": msg.content,
                        }));
                    }
                    if let Some(ref calls) = msg.tool_calls {
                        for c in calls {
                            content_blocks.push(json!({
                                "type": "tool_use",
                                "id": c.id,
                                "name": c.name,
                                "input": c.arguments,
                            }));
                        }
                    }
                    messages.push(json!({
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                }
                Role::Tool => {
                    if let Some(ref results) = msg.tool_results {
                        let mut blocks = Vec::new();
                        for res in results {
                            blocks.push(json!({
                                "type": "tool_result",
                                "tool_use_id": res.call_id,
                                "content": serde_json::to_string(&res.result).unwrap_or_default(),
                                "is_error": res.is_error,
                            }));
                        }
                        messages.push(json!({
                            "role": "user",
                            "content": blocks,
                        }));
                    }
                }
                Role::System => {} // Anthropic system prompt is a top-level field
            }
        }

        let max_tokens = context.max_tokens.unwrap_or(4096);
        let mut body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": messages,
        });

        if let Some(ref sys) = context.system_prompt {
            body["system"] = json!(sys);
        }

        if let Some(temp) = context.temperature {
            body["temperature"] = json!(temp);
        }

        if !context.tools.is_empty() {
            let tools: Vec<_> = context
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters,
                    })
                })
                .collect();
            body["tools"] = json!(tools);
        }

        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| StrataError::Reasoning(format!("Anthropic HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(StrataError::Reasoning(format!(
                "Anthropic API returned error status {}: {}",
                status, err_body
            )));
        }

        let res_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| StrataError::Reasoning(format!("Failed to parse Anthropic JSON response: {}", e)))?;

        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        if let Some(content_arr) = res_json["content"].as_array() {
            for block in content_arr {
                match block["type"].as_str() {
                    Some("text") => {
                        if let Some(t) = block["text"].as_str() {
                            text_parts.push(t);
                        }
                    }
                    Some("tool_use") => {
                        let id = block["id"].as_str().unwrap_or_default().to_string();
                        let name = block["name"].as_str().unwrap_or_default().to_string();
                        let input = block["input"].clone();
                        tool_calls.push(ToolCall::new(id, name, input));
                    }
                    _ => {}
                }
            }
        }

        let stop_reason = res_json["stop_reason"]
            .as_str()
            .unwrap_or("stop")
            .to_string();

        let usage = res_json.get("usage").map(|u| TokenUsage {
            prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: (u["input_tokens"].as_u64().unwrap_or(0)
                + u["output_tokens"].as_u64().unwrap_or(0)) as u32,
        });

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join("\n"))
        };

        Ok(ReasoningOutput {
            content,
            tool_calls,
            finish_reason: stop_reason,
            usage,
        })
    }
}

#[async_trait]
impl strata_core::traits::ReasoningEngine for AnthropicAdapter {
    async fn prompt(
        &self,
        system: Option<&str>,
        user: &str,
        context: Option<serde_json::Value>,
    ) -> Result<String, StrataError> {
        let mut ctx = PromptContext::new().with_message(ChatMessage::user(user));
        if let Some(sys) = system {
            ctx = ctx.with_system(sys);
        }
        if let Some(meta) = context {
            ctx.metadata = meta;
        }
        let output = self.complete(&ctx).await?;
        Ok(output.content.unwrap_or_default())
    }
}

// ============================================================================
// Gemini Adapter (Google Gemini 1.5 Pro / Flash / 2.0)
// ============================================================================

pub struct GeminiAdapter {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl GeminiAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl ReasoningEngine for GeminiAdapter {
    async fn complete(&self, context: &PromptContext) -> Result<ReasoningOutput, StrataError> {
        let mut contents = Vec::new();

        for msg in &context.messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "model",
                Role::System => "user",
                Role::Tool => "user",
            };

            let mut parts = Vec::new();
            if !msg.content.is_empty() {
                parts.push(json!({ "text": msg.content }));
            }

            if let Some(ref calls) = msg.tool_calls {
                for c in calls {
                    parts.push(json!({
                        "functionCall": {
                            "name": c.name,
                            "args": c.arguments,
                        }
                    }));
                }
            }

            if let Some(ref results) = msg.tool_results {
                for res in results {
                    parts.push(json!({
                        "functionResponse": {
                            "name": res.name,
                            "response": res.result,
                        }
                    }));
                }
            }

            if !parts.is_empty() {
                contents.push(json!({
                    "role": role,
                    "parts": parts,
                }));
            }
        }

        let mut body = json!({
            "contents": contents,
        });

        if let Some(ref sys) = context.system_prompt {
            body["systemInstruction"] = json!({
                "parts": [{ "text": sys }]
            });
        }

        let mut gen_config = json!({});
        if let Some(temp) = context.temperature {
            gen_config["temperature"] = json!(temp);
        }
        if let Some(max_t) = context.max_tokens {
            gen_config["maxOutputTokens"] = json!(max_t);
        }
        if gen_config.as_object().map_or(false, |o| !o.is_empty()) {
            body["generationConfig"] = gen_config;
        }

        if !context.tools.is_empty() {
            let decls: Vec<_> = context
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    })
                })
                .collect();
            body["tools"] = json!([{
                "functionDeclarations": decls
            }]);
        }

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key
        );

        let response = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| StrataError::Reasoning(format!("Gemini HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(StrataError::Reasoning(format!(
                "Gemini API returned error status {}: {}",
                status, err_body
            )));
        }

        let res_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| StrataError::Reasoning(format!("Failed to parse Gemini JSON response: {}", e)))?;

        let candidate = res_json["candidates"]
            .get(0)
            .ok_or_else(|| StrataError::Reasoning("Gemini response had no candidates".to_string()))?;

        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        if let Some(parts_arr) = candidate["content"]["parts"].as_array() {
            for part in parts_arr {
                if let Some(t) = part["text"].as_str() {
                    text_parts.push(t);
                }
                if let Some(fc) = part.get("functionCall") {
                    let name = fc["name"].as_str().unwrap_or_default().to_string();
                    let args = fc["args"].clone();
                    let id = format!("call_{}", Uuid::new_v4());
                    tool_calls.push(ToolCall::new(id, name, args));
                }
            }
        }

        let finish_reason = candidate["finishReason"]
            .as_str()
            .unwrap_or("STOP")
            .to_string();

        let usage = res_json.get("usageMetadata").map(|u| TokenUsage {
            prompt_tokens: u["promptTokenCount"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["candidatesTokenCount"].as_u64().unwrap_or(0) as u32,
            total_tokens: u["totalTokenCount"].as_u64().unwrap_or(0) as u32,
        });

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join("\n"))
        };

        Ok(ReasoningOutput {
            content,
            tool_calls,
            finish_reason,
            usage,
        })
    }
}

#[async_trait]
impl strata_core::traits::ReasoningEngine for GeminiAdapter {
    async fn prompt(
        &self,
        system: Option<&str>,
        user: &str,
        context: Option<serde_json::Value>,
    ) -> Result<String, StrataError> {
        let mut ctx = PromptContext::new().with_message(ChatMessage::user(user));
        if let Some(sys) = system {
            ctx = ctx.with_system(sys);
        }
        if let Some(meta) = context {
            ctx.metadata = meta;
        }
        let output = self.complete(&ctx).await?;
        Ok(output.content.unwrap_or_default())
    }
}
