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
async fn spawn_test_server(auth_token: Option<String>) -> (String, tokio::task::JoinHandle<()>) {
    let storage = ServerStorage::in_memory().expect("Failed to create in-memory server storage");
    let (ws_tx, _) = tokio::sync::broadcast::channel::<WsBroadcastMsg>(64);

    let state = Arc::new(AppState {
        storage,
        auth_token,
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
        created_at: Utc::now(),
        last_updated_at: Utc::now(),
        status: FactStatus::Active,
        version: 1,
        replaced_by: None,
        tags: vec!["railway".to_string(), "deployment".to_string()],
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
