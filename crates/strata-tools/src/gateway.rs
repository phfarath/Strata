use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

use strata_core::{
    errors::StrataError,
    events::{Event, EventPayload, ToolInvoked, ToolResultReceived},
    state::FailurePattern,
    traits::{EventStore, MemoryEngine, Tool, ToolGateway},
};

/// Permission policy governing tool invocations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPolicy {
    pub allowed_tools: Option<HashSet<String>>,
    pub blocked_tools: HashSet<String>,
    pub require_approval: HashSet<String>,
    pub max_calls_per_minute: u32,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self {
            allowed_tools: None, // None means all tools allowed unless blocked
            blocked_tools: HashSet::new(),
            require_approval: HashSet::new(),
            max_calls_per_minute: 120,
        }
    }
}

impl PermissionPolicy {
    pub fn allow_all() -> Self {
        Self::default()
    }

    pub fn with_blocked_tool(mut self, tool: impl Into<String>) -> Self {
        self.blocked_tools.insert(tool.into());
        self
    }

    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = Some(tools.into_iter().collect());
        self
    }

    pub fn with_rate_limit(mut self, max_per_minute: u32) -> Self {
        self.max_calls_per_minute = max_per_minute;
        self
    }
}

/// In-memory sliding window rate limiter.
struct RateLimiter {
    window_duration: Duration,
    max_calls: u32,
    call_history: VecDeque<Instant>,
}

impl RateLimiter {
    fn new(max_calls: u32, window_duration: Duration) -> Self {
        Self {
            window_duration,
            max_calls,
            call_history: VecDeque::new(),
        }
    }

    fn check_and_record(&mut self) -> bool {
        if self.max_calls == 0 {
            return true;
        }

        let now = Instant::now();
        let cutoff = now.checked_sub(self.window_duration).unwrap_or(now);

        while let Some(&t) = self.call_history.front() {
            if t < cutoff {
                self.call_history.pop_front();
            } else {
                break;
            }
        }

        if self.call_history.len() >= self.max_calls as usize {
            false
        } else {
            self.call_history.push_back(now);
            true
        }
    }
}

/// Production implementation of ToolGateway with permission checks, rate limiting,
/// audit logging via EventStore, and out-of-band automatic failure capture via MemoryEngine.
pub struct DefaultToolGateway {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    policy: Arc<RwLock<PermissionPolicy>>,
    rate_limiter: Arc<RwLock<RateLimiter>>,
    event_store: Option<Arc<dyn EventStore>>,
    memory_engine: Option<Arc<dyn MemoryEngine>>,
    client_name: String,
}

impl DefaultToolGateway {
    pub fn new() -> Self {
        let policy = PermissionPolicy::default();
        let max_calls = policy.max_calls_per_minute;
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            policy: Arc::new(RwLock::new(policy)),
            rate_limiter: Arc::new(RwLock::new(RateLimiter::new(
                max_calls,
                Duration::from_secs(60),
            ))),
            event_store: None,
            memory_engine: None,
            client_name: "strata".to_string(),
        }
    }

    pub fn with_policy(mut self, policy: PermissionPolicy) -> Self {
        let max_calls = policy.max_calls_per_minute;
        self.policy = Arc::new(RwLock::new(policy));
        self.rate_limiter = Arc::new(RwLock::new(RateLimiter::new(
            max_calls,
            Duration::from_secs(60),
        )));
        self
    }

    pub fn with_event_store(mut self, store: Arc<dyn EventStore>) -> Self {
        self.event_store = Some(store);
        self
    }

    pub fn with_memory_engine(mut self, engine: Arc<dyn MemoryEngine>) -> Self {
        self.memory_engine = Some(engine);
        self
    }

    pub fn with_client_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = name.into();
        self
    }

    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let mut map = self.tools.write().await;
        map.insert(tool.name().to_string(), tool);
    }

    pub async fn list_tools_schema(&self) -> Vec<serde_json::Value> {
        let map = self.tools.read().await;
        map.values()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters_schema(),
                })
            })
            .collect()
    }

    async fn check_permissions(&self, tool_name: &str) -> Result<(), StrataError> {
        let policy = self.policy.read().await;

        if policy.blocked_tools.contains(tool_name) {
            return Err(StrataError::PermissionDenied(format!(
                "Tool '{}' is blocked by security policy",
                tool_name
            )));
        }

        if let Some(ref allowed) = policy.allowed_tools {
            if !allowed.contains(tool_name) {
                return Err(StrataError::PermissionDenied(format!(
                    "Tool '{}' is not in the allowed tools whitelist",
                    tool_name
                )));
            }
        }

        if policy.require_approval.contains(tool_name) {
            return Err(StrataError::PermissionDenied(format!(
                "Tool '{}' requires explicit human approval before execution",
                tool_name
            )));
        }

        let mut limiter = self.rate_limiter.write().await;
        if !limiter.check_and_record() {
            return Err(StrataError::RateLimitExceeded(format!(
                "Rate limit exceeded (max {} calls/min)",
                policy.max_calls_per_minute
            )));
        }

        Ok(())
    }

    async fn log_tool_invoked(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        session_id: &str,
    ) -> Uuid {
        let invocation_id = Uuid::new_v4();
        if let Some(ref store) = self.event_store {
            let event = Event::new(
                session_id,
                &self.client_name,
                EventPayload::ToolInvoked(ToolInvoked {
                    invocation_id,
                    tool_name: tool_name.to_string(),
                    input: input.clone(),
                    session_id: session_id.to_string(),
                    timestamp: chrono::Utc::now(),
                }),
            );

            if let Err(e) = store.append(&event).await {
                warn!("Failed to append ToolInvoked audit event: {}", e);
            }
        }
        invocation_id
    }

    async fn log_tool_result(
        &self,
        invocation_id: Uuid,
        tool_name: &str,
        result: &serde_json::Value,
        is_error: bool,
        duration_ms: u64,
        session_id: &str,
    ) {
        if let Some(ref store) = self.event_store {
            let event = Event::new(
                session_id,
                &self.client_name,
                EventPayload::ToolResultReceived(ToolResultReceived {
                    invocation_id,
                    tool_name: tool_name.to_string(),
                    result: result.clone(),
                    is_error,
                    duration_ms: Some(duration_ms),
                    timestamp: chrono::Utc::now(),
                }),
            );

            if let Err(e) = store.append(&event).await {
                warn!("Failed to append ToolResultReceived audit event: {}", e);
            }
        }
    }

    async fn record_silent_failure(&self, tool_name: &str, error: &StrataError, input: &serde_json::Value) {
        if let Some(ref engine) = self.memory_engine {
            let (err_type, err_msg) = match error {
                StrataError::Timeout(msg) => ("TimeoutError".to_string(), msg.clone()),
                StrataError::PermissionDenied(msg) => ("PermissionDenied".to_string(), msg.clone()),
                StrataError::ExecutionFailed { code, stderr } => (
                    format!("ExecutionFailed(code={:?})", code),
                    stderr.clone(),
                ),
                StrataError::ValidationError(msg) => ("ValidationError".to_string(), msg.clone()),
                StrataError::NotFound(msg) => ("NotFoundError".to_string(), msg.clone()),
                StrataError::ToolError(msg) => ("ToolExecutionError".to_string(), msg.clone()),
                StrataError::RateLimitExceeded(msg) => ("RateLimitExceeded".to_string(), msg.clone()),
                _ => ("InternalError".to_string(), error.to_string()),
            };

            let signature = format!("sig-{}-{}", tool_name, err_type.to_lowercase());
            let pattern_name = format!("{} Failure", tool_name);
            let description = format!("Tool '{}' failed with {}: {}", tool_name, err_type, err_msg);
            let mitigation = format!(
                "Verify input schema and avoid failed pattern with input: {}",
                serde_json::to_string(input).unwrap_or_default()
            );

            let mut failure = FailurePattern::new(signature, pattern_name, description, mitigation);
            failure.error_type = err_type;
            failure.trigger_condition = format!("tool_name == '{}'", tool_name);

            info!(
                tool = %tool_name,
                "Out-of-band silent failure pattern recorded into memory engine"
            );

            if let Err(e) = engine.record_failure(&failure).await {
                error!("Failed to record silent failure pattern in memory: {}", e);
            }
        }
    }

    pub async fn invoke_with_session(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value, StrataError> {
        let session = session_id.unwrap_or("default");

        // 1. Permission checks & rate limiting
        self.check_permissions(tool_name).await?;

        // 2. Audit logging: ToolInvoked
        let invocation_id = self.log_tool_invoked(tool_name, &input, session).await;

        // 3. Find tool
        let tool = {
            let map = self.tools.read().await;
            map.get(tool_name).cloned().ok_or_else(|| {
                StrataError::NotFound(format!("Tool '{}' is not registered in gateway", tool_name))
            })?
        };

        // 4. Execution with timing
        let start = Instant::now();
        let exec_res = tool.execute(input.clone()).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match exec_res {
            Ok(result) => {
                // Audit logging: ToolResultReceived (success)
                self.log_tool_result(invocation_id, tool_name, &result, false, duration_ms, session)
                    .await;
                Ok(result)
            }
            Err(err) => {
                let err_val = json!({ "error": err.to_string() });

                // Audit logging: ToolResultReceived (error)
                self.log_tool_result(invocation_id, tool_name, &err_val, true, duration_ms, session)
                    .await;

                // Out-of-band automatic failure capture
                self.record_silent_failure(tool_name, &err, &input).await;

                Err(err)
            }
        }
    }
}

impl Default for DefaultToolGateway {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolGateway for DefaultToolGateway {
    async fn register_tool(&self, tool: Arc<dyn Tool>) -> Result<(), StrataError> {
        self.register(tool).await;
        Ok(())
    }

    async fn get_tool(&self, name: &str) -> Result<Option<Arc<dyn Tool>>, StrataError> {
        let map = self.tools.read().await;
        Ok(map.get(name).cloned())
    }

    async fn list_tools(&self) -> Result<Vec<String>, StrataError> {
        let map = self.tools.read().await;
        Ok(map.keys().cloned().collect())
    }

    async fn invoke(
        &self,
        tool_name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, StrataError> {
        self.invoke_with_session(tool_name, input, None).await
    }
}
