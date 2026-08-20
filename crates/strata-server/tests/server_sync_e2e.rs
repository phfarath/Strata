use std::sync::Arc;
use std::time::Duration;
use chrono::Utc;
use futures::StreamExt;
use reqwest::Client;
use strata_core::schemas::{EpisodicMemory, EvidenceRef, FactStatus, SemanticFact, SyncConfig, SyncDelta};
use strata_core::state::Scope;
use strata_memory::{SqliteStore, SyncEngine};
use strata_server::{create_app, AppState, ServerStorage, WsBroadcastMsg};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use uuid::Uuid;

/// Helper function to spawn an ephemeral in-memory Strata sync server on an ephemeral port.
async fn spawn_test_server(legacy_secret: Option<String>) -> (String, tokio::task::JoinHandle<()>) {
    let storage = ServerStorage::in_memory().expect("Failed to create in-memory server storage");
    let (ws_tx, _) = tokio::sync::broadcast::channel::<WsBroadcastMsg>(64);

    let state = Arc::new(AppState {
        storage,
        jwt_secret: "test-jwt-secret-key-1234567890".to_string(),
        legacy_secret,
        custom_domain: Some("strata.pedrofarath.me".to_string()),
        ws_broadcast: ws_tx,
        start_time: std::time::Instant::now(),
    });

    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind random TCP port");
    let addr = listener.local_addr().expect("Failed to get local address");
    let base_url = format!("http://{}", addr);

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (base_url, handle)
}

#[tokio::test]
async fn test_health_check_endpoint() {
    let (base_url, _handle) = spawn_test_server(None).await;
    let client = Client::new();

    let resp = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .expect("Failed to call /health");

    assert!(resp.status().is_success());
    let json: serde_json::Value = resp.json().await.expect("Failed to parse health response");
    assert_eq!(json["status"], "ok");
    assert!(json["uptime_secs"].is_number());
}

#[tokio::test]
async fn test_saas_signup_login_workspace_and_api_keys_flow() {
    let (base_url, _handle) = spawn_test_server(Some("legacy-secret".to_string())).await;
    let client = Client::new();

    // 1. Signup new user
    let signup_payload = serde_json::json!({
        "email": "dev@strata.ai",
        "password": "strong-password-1234",
        "full_name": "Pedro Dev",
        "workspace_name": "Pedro's Team Space"
    });

    let signup_resp = client
        .post(format!("{base_url}/api/v1/auth/signup"))
        .json(&signup_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(signup_resp.status(), reqwest::StatusCode::OK);
    let signup_data: serde_json::Value = signup_resp.json().await.unwrap();
    assert_eq!(signup_data["user"]["email"], "dev@strata.ai");
    let jwt_token = signup_data["token"].as_str().unwrap();
    let workspace_id = signup_data["workspaces"][0]["id"].as_str().unwrap();

    // 2. Login with credentials
    let login_payload = serde_json::json!({
        "email": "dev@strata.ai",
        "password": "strong-password-1234"
    });
    let login_resp = client
        .post(format!("{base_url}/api/v1/auth/login"))
        .json(&login_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), reqwest::StatusCode::OK);

    // 3. Query /me endpoint with JWT
    let me_resp = client
        .get(format!("{base_url}/api/v1/auth/me"))
        .bearer_auth(jwt_token)
        .send()
        .await
        .unwrap();
    assert_eq!(me_resp.status(), reqwest::StatusCode::OK);

    // 4. Create new API key for workspace
    let key_payload = serde_json::json!({
        "workspace_id": workspace_id,
        "name": "Cursor MacBook Pro"
    });
    let create_key_resp = client
        .post(format!("{base_url}/api/v1/keys"))
        .bearer_auth(jwt_token)
        .json(&key_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(create_key_resp.status(), reqwest::StatusCode::OK);
    let key_data: serde_json::Value = create_key_resp.json().await.unwrap();
    let api_key_secret = key_data["key"].as_str().unwrap();
    let key_id = key_data["id"].as_str().unwrap();
    assert!(api_key_secret.starts_with("strata_live_"));

    // 5. List API keys
    let list_keys_resp = client
        .get(format!("{base_url}/api/v1/keys?workspace_id={workspace_id}"))
        .bearer_auth(jwt_token)
        .send()
        .await
        .unwrap();
    assert_eq!(list_keys_resp.status(), reqwest::StatusCode::OK);
    let keys_list: Vec<serde_json::Value> = list_keys_resp.json().await.unwrap();
    assert_eq!(keys_list.len(), 1);

    // 6. Test sync using the newly generated API key!
    let status_resp = client
        .get(format!("{base_url}/sync/status?workspace_id={workspace_id}"))
        .bearer_auth(api_key_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(status_resp.status(), reqwest::StatusCode::OK);

    // 7. Revoke API key
    let revoke_resp = client
        .delete(format!("{base_url}/api/v1/keys/{key_id}"))
        .bearer_auth(jwt_token)
        .send()
        .await
        .unwrap();
    assert_eq!(revoke_resp.status(), reqwest::StatusCode::OK);

    // 8. Subsequent sync with revoked API key should fail with 401
    let revoked_sync_resp = client
        .get(format!("{base_url}/sync/status?workspace_id={workspace_id}"))
        .bearer_auth(api_key_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked_sync_resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_authenticated_push_pull_and_status() {
    let secret = "strata-test-secret-key-12345".to_string();
    let (base_url, _handle) = spawn_test_server(Some(secret.clone())).await;
    let client = Client::new();

    // 1. Unauthenticated request should return 401
    let unauth_resp = client
        .get(format!("{base_url}/sync/pull?workspace_id=ws-auth"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Request with wrong token should return 401
    let wrong_auth_resp = client
        .get(format!("{base_url}/sync/pull?workspace_id=ws-auth"))
        .bearer_auth("wrong-secret")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_auth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 3. Request with valid Bearer token should return 200
    let valid_resp = client
        .get(format!("{base_url}/sync/pull?workspace_id=ws-auth"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(valid_resp.status(), reqwest::StatusCode::OK);
    let deltas: Vec<SyncDelta> = valid_resp.json().await.unwrap();
    assert!(deltas.is_empty());
}

#[tokio::test]
async fn test_bidirectional_multi_device_sync_e2e() {
    let secret = "dev-sync-token-abc".to_string();
    let (base_url, _handle) = spawn_test_server(Some(secret.clone())).await;

    let workspace_id = "test-multi-device-workspace";

    // Setup Client A (Machine 1 - e.g. MacBook)
    let store_a = Arc::new(SqliteStore::open_in_memory().expect("Failed to create store A"));
    let mut config_a = SyncConfig::new(workspace_id);
    config_a.endpoint = Some(base_url.clone());
    config_a.token = Some(secret.clone());
    let engine_a = SyncEngine::new(store_a.clone(), config_a);

    // Setup Client B (Machine 2 - e.g. Desktop Linux)
    let store_b = Arc::new(SqliteStore::open_in_memory().expect("Failed to create store B"));
    let mut config_b = SyncConfig::new(workspace_id);
    config_b.endpoint = Some(base_url.clone());
    config_b.token = Some(secret.clone());
    let engine_b = SyncEngine::new(store_b.clone(), config_b);

    // 1. Client A creates a Semantic Fact and an Episodic Memory locally
    let fact_id = Uuid::new_v4();
    let fact = SemanticFact {
        id: fact_id,
        project: Some("Strata".to_string()),
        scope: Scope::Project("Strata".to_string()),
        statement: "Railway deployments require dynamic binding to PORT environment variable".to_string(),
        category: "infrastructure".to_string(),
        evidence: vec![EvidenceRef::new("docker", "railway-test", 1.0)],
        importance: 0.95,
        confidence: 0.98,
        tier: strata_core::state::MemoryTier::Peripheral,
        created_at: Utc::now(),
        last_updated_at: Utc::now(),
        status: FactStatus::Active,
        version: 1,
        replaced_by: None,
        tags: vec!["railway".to_string(), "deployment".to_string()],
        code_anchor: None,
    };
    store_a.insert_or_update_semantic_fact(&fact).expect("Store A fact insert failed");

    // Enqueue CDC delta in store A's outbox
    let fact_delta = SyncDelta::new(
        workspace_id,
        1,
        "semantic_fact",
        serde_json::to_value(&fact).unwrap(),
        strata_memory::sync::compute_version_hash(&serde_json::to_value(&fact).unwrap()),
    );
    store_a.enqueue_delta(&fact_delta).expect("Store A delta enqueue failed");

    let mem = EpisodicMemory::new(
        "session-1",
        "agent",
        "Decided to implement Axum server for Cloud Sync backend",
        Utc::now(),
        Utc::now(),
    )
    .with_project("Strata")
    .with_tags(vec!["sync".to_string(), "cloud".to_string()]);
    let mem_id = mem.id;

    store_a.insert_episodic_memory(&mem).expect("Store A memory insert failed");

    let mem_delta = SyncDelta::new(
        workspace_id,
        2,
        "episodic_memory",
        serde_json::to_value(&mem).unwrap(),
        strata_memory::sync::compute_version_hash(&serde_json::to_value(&mem).unwrap()),
    );
    store_a.enqueue_delta(&mem_delta).expect("Store A delta enqueue failed");

    // 2. Client A pushes deltas to Axum cloud sync server
    let pushed = engine_a.push_deltas().await.expect("Client A push failed");
    assert_eq!(pushed, 2);

    // Verify outbox in store A is now marked synced
    let (pending_a, _) = store_a.get_sync_status(workspace_id).unwrap();
    assert_eq!(pending_a, 0);

    // 3. Client B pulls deltas from Axum cloud sync server
    let pulled_deltas = engine_b.pull_remote().await.expect("Client B pull remote failed");
    assert_eq!(pulled_deltas.len(), 2);

    let applied = engine_b.pull_deltas(pulled_deltas).await.expect("Client B apply deltas failed");
    assert_eq!(applied, 2);

    // 4. Verify Client B now contains the synchronized fact and memory record!
    let fact_on_b = store_b.get_semantic_fact(&fact_id).unwrap();
    assert!(fact_on_b.is_some(), "Semantic fact should exist on Client B");
    let retrieved_fact = fact_on_b.unwrap();
    assert_eq!(retrieved_fact.statement, fact.statement);
    assert_eq!(retrieved_fact.importance, fact.importance);

    let mem_on_b = store_b.get_episodic_memory(&mem_id).unwrap();
    assert!(mem_on_b.is_some(), "Episodic memory should exist on Client B");
    let retrieved_mem = mem_on_b.unwrap();
    assert_eq!(retrieved_mem.summary, mem.summary);

    // 5. Test complete sync_cycle on Client B
    let report_b = engine_b.sync_cycle().await.expect("Client B sync cycle failed");
    assert_eq!(report_b.pushed_count, 0);
    assert_eq!(report_b.errors.len(), 0);
}

#[tokio::test]
async fn test_realtime_websocket_delta_notifications() {
    let secret = "ws-secret-key-xyz".to_string();
    let (base_url, _handle) = spawn_test_server(Some(secret.clone())).await;

    // Convert http:// to ws://
    let ws_url = format!("{}/ws?token={}", base_url.replace("http://", "ws://"), secret);

    // Connect WebSocket client
    let (mut ws_stream, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect to WebSocket endpoint");

    // Read initial welcome message
    let welcome_msg = ws_stream.next().await.unwrap().unwrap();
    let welcome_json: serde_json::Value =
        serde_json::from_str(&welcome_msg.to_text().unwrap()).unwrap();
    assert_eq!(welcome_json["event"], "connected");

    // Client pushes a delta via HTTP POST
    let http_client = Client::new();
    let sample_delta = SyncDelta::new(
        "ws-workspace",
        1,
        "event",
        serde_json::json!({ "type": "test_event" }),
        "hash-12345",
    );

    let push_resp = http_client
        .post(format!("{base_url}/sync/push"))
        .bearer_auth(&secret)
        .json(&serde_json::json!({
            "workspace_id": "ws-workspace",
            "deltas": [sample_delta]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(push_resp.status(), reqwest::StatusCode::OK);

    // WebSocket client should receive a "new_deltas" broadcast event within 3 seconds
    let ws_notification = tokio::time::timeout(Duration::from_secs(3), ws_stream.next())
        .await
        .expect("WebSocket notification timed out")
        .unwrap()
        .unwrap();

    let notification_json: serde_json::Value =
        serde_json::from_str(&ws_notification.to_text().unwrap()).unwrap();
    assert_eq!(notification_json["event"], "new_deltas");
    assert_eq!(notification_json["workspace_id"], "ws-workspace");
    assert_eq!(notification_json["delta_count"], 1);
    assert_eq!(notification_json["max_seq"], 1);
}

#[tokio::test]
async fn test_cli_auth_browser_page_and_authorize_flow() {
    let (base_url, _handle) = spawn_test_server(None).await;
    let client = Client::new();

    // 1. Verify GET /auth/cli renders modern HTML
    let html_resp = client
        .get(format!("{base_url}/auth/cli?port=54321&state=test_state_123"))
        .send()
        .await
        .unwrap();
    assert_eq!(html_resp.status(), reqwest::StatusCode::OK);
    let html_text = html_resp.text().await.unwrap();
    assert!(html_text.contains("Authorize Strata CLI"));
    assert!(html_text.contains("127.0.0.1:54321"));

    // 2. Authorize with signup
    let auth_payload = serde_json::json!({
        "email": "browser-dev@strata.ai",
        "password": "super-secure-password",
        "port": 54321,
        "state": "test_state_123",
        "machine_name": "MacBook Pro M3 Max",
        "is_signup": true,
        "full_name": "Browser Dev"
    });

    let auth_resp = client
        .post(format!("{base_url}/api/v1/auth/cli/authorize"))
        .json(&auth_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(auth_resp.status(), reqwest::StatusCode::OK);
    let auth_data: serde_json::Value = auth_resp.json().await.unwrap();
    let redirect_url = auth_data["redirect_url"].as_str().unwrap();
    let token = auth_data["token"].as_str().unwrap();

    assert!(token.starts_with("strata_live_"));
    assert!(redirect_url.contains("http://127.0.0.1:54321/callback"));
    assert!(redirect_url.contains("test_state_123"));
    assert!(redirect_url.contains(token));
}

#[tokio::test]
async fn test_vector_embeddings_endpoints_and_health() {
    let (base_url, _handle) = spawn_test_server(None).await;
    let client = Client::new();

    // 1. Check health response includes engine flags
    let health_resp = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .unwrap();
    let health_data: serde_json::Value = health_resp.json().await.unwrap();
    assert_eq!(health_data["status"], "ok");
    assert_eq!(health_data["is_postgres"], false);
    assert_eq!(health_data["has_pgvector"], false);

    // 2. Test upsert embedding endpoint
    let upsert_payload = serde_json::json!({
        "workspace_id": "test-ws",
        "memory_id": Uuid::new_v4(),
        "embedding": vec![0.1f32; 384],
        "metadata": { "statement": "Persistent Memory Layer in Rust" }
    });

    let upsert_resp = client
        .post(format!("{base_url}/api/v1/embeddings/upsert"))
        .json(&upsert_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(upsert_resp.status(), reqwest::StatusCode::OK);

    // 3. Test search embedding endpoint
    let search_payload = serde_json::json!({
        "workspace_id": "test-ws",
        "query_embedding": vec![0.1f32; 384],
        "limit": 5
    });

    let search_resp = client
        .post(format!("{base_url}/api/v1/embeddings/search"))
        .json(&search_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(search_resp.status(), reqwest::StatusCode::OK);
    let search_data: serde_json::Value = search_resp.json().await.unwrap();
    assert_eq!(search_data["workspace_id"], "test-ws");
}

#[tokio::test]
async fn test_ping_endpoint_and_security_headers() {
    let (base_url, _handle) = spawn_test_server(None).await;
    let client = Client::new();

    // 1. Query /api/v1/ping and verify ping payload
    let ping_resp = client
        .get(format!("{base_url}/api/v1/ping"))
        .send()
        .await
        .unwrap();
    assert_eq!(ping_resp.status(), reqwest::StatusCode::OK);

    let headers = ping_resp.headers();
    // Verify Security Headers presence
    assert_eq!(
        headers.get("strict-transport-security").unwrap().to_str().unwrap(),
        "max-age=63072000; includeSubDomains; preload"
    );
    assert_eq!(
        headers.get("x-content-type-options").unwrap().to_str().unwrap(),
        "nosniff"
    );
    assert_eq!(
        headers.get("x-frame-options").unwrap().to_str().unwrap(),
        "DENY"
    );
    assert_eq!(
        headers.get("referrer-policy").unwrap().to_str().unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert!(headers.get("content-security-policy").is_some());

    let ping_data: serde_json::Value = ping_resp.json().await.unwrap();
    assert_eq!(ping_data["status"], "pong");
    assert_eq!(ping_data["protocol"], "strata-cloud/v1");
    assert!(ping_data["epoch_ms"].is_number());
    assert!(ping_data["timestamp"].is_string());
}

