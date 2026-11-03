use gemini_router::admin;
use gemini_router::config::AppConfig;
use gemini_router::database;
use gemini_router::gemini::types::StrategyType;
use gemini_router::key_manager::KeyManager;
use gemini_router::proxy;
use gemini_router::rotation::RotationEngine;
use gemini_router::tester::FastTester;
use gemini_router::AppState;
use reqwest::header::HeaderMap;
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup_test_server(mock_server: &MockServer) -> (String, Arc<AppState>) {
    let mut config = AppConfig::default();
    config.gemini_base_url = mock_server.uri();
    config.admin_key = "test-secret-key-123".to_string();
    config.retry_budget = 3;
    let config = Arc::new(config);

    let pool = database::init_pool("sqlite::memory:").await.unwrap();
    database::run_migrations(&pool).await.unwrap();

    let key_mgr = Arc::new(KeyManager::new(pool.clone()).await.unwrap());
    let http_client = reqwest::Client::new();
    let (event_tx, _) = tokio::sync::broadcast::channel(100);

    let rotation_engine = Arc::new(RotationEngine::new(
        key_mgr.clone(),
        StrategyType::RoundRobin,
        1,
        1,
    ));

    let tester = Arc::new(FastTester::new(
        http_client.clone(),
        key_mgr.clone(),
        event_tx.clone(),
        5,
        mock_server.uri(),
    ));

    let state = Arc::new(AppState {
        config: config.clone(),
        db: pool,
        http_client: http_client.clone(),
        key_manager: key_mgr,
        rotation_engine,
        tester,
        event_tx,
    });

    let app = axum::Router::new()
        .nest("/admin", admin::admin_router(state.clone()))
        .merge(proxy::proxy_router(state.clone()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{}", addr), state)
}

#[tokio::test]
async fn test_generate_content_success() {
    let mock_server = MockServer::start().await;
    let (server_url, state) = setup_test_server(&mock_server).await;

    state
        .key_manager
        .add_key("AIzaSyValidKey12345", "primary-key")
        .await
        .unwrap();

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
        .and(header("x-goog-api-key", "AIzaSyValidKey12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello world from Gemini API!"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = reqwest::Client::new();
    let payload = json!({
        "contents": [{
            "parts": [{"text": "Hello Gemini"}]
        }]
    });

    let res = client
        .post(format!("{}/v1beta/models/gemini-2.5-flash:generateContent", server_url))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);

    let res_headers = res.headers().clone();
    assert!(!res_headers.contains_key("x-goog-api-key"));

    let body_text = res.text().await.unwrap();
    assert!(body_text.contains("Hello world from Gemini API!"));
    assert!(!body_text.contains("AIzaSyValidKey12345"));
}

#[tokio::test]
async fn test_stream_generate_content_sse() {
    let mock_server = MockServer::start().await;
    let (server_url, state) = setup_test_server(&mock_server).await;

    state
        .key_manager
        .add_key("AIzaSyStreamKey123", "stream-key")
        .await
        .unwrap();

    let sse_body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world streamed!\"}]}}]}\n\n";

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.5-flash:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", "AIzaSyStreamKey123"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream; charset=utf-8")
                .set_body_raw(sse_body, "text/event-stream; charset=utf-8"),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = reqwest::Client::new();
    let payload = json!({
        "contents": [{
            "parts": [{"text": "Stream test"}]
        }]
    });

    let res = client
        .post(format!(
            "{}/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
            server_url
        ))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let ct = res.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("text/event-stream"));

    let body = res.text().await.unwrap();
    assert!(body.contains("Hello"));
    assert!(body.contains("world streamed!"));
}

#[tokio::test]
async fn test_failover_on_429_to_second_key() {
    let mock_server = MockServer::start().await;
    let (server_url, state) = setup_test_server(&mock_server).await;

    state
        .key_manager
        .add_key("AIzaSyFirstKey429", "key-fail-1")
        .await
        .unwrap();
    state
        .key_manager
        .add_key("AIzaSySecondKey200", "key-success-2")
        .await
        .unwrap();

    // Mock 429 for the first key
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
        .and(header("x-goog-api-key", "AIzaSyFirstKey429"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {
                "code": 429,
                "message": "Resource exhausted upstream",
                "status": "RESOURCE_EXHAUSTED"
            }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Mock 200 OK for the second key after failover
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
        .and(header("x-goog-api-key", "AIzaSySecondKey200"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Success after failover!"}],
                    "role": "model"
                }
            }]
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = reqwest::Client::new();
    let payload = json!({
        "contents": [{
            "parts": [{"text": "Trigger failover"}]
        }]
    });

    let res = client
        .post(format!("{}/v1beta/models/gemini-2.5-flash:generateContent", server_url))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("Success after failover!"));
}

#[tokio::test]
async fn test_preserving_tools_and_function_declarations() {
    let mock_server = MockServer::start().await;
    let (server_url, state) = setup_test_server(&mock_server).await;

    state
        .key_manager
        .add_key("AIzaSyToolTestKey", "tool-key")
        .await
        .unwrap();

    let complex_payload = json!({
        "contents": [{
            "role": "user",
            "parts": [{"text": "What is the weather in Tokyo?"}]
        }],
        "tools": [{
            "functionDeclarations": [{
                "name": "get_current_weather",
                "description": "Get current weather in location",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "location": {"type": "STRING"}
                    },
                    "required": ["location"]
                }
            }]
        }],
        "toolConfig": {
            "functionCallingConfig": {"mode": "ANY"}
        },
        "systemInstruction": {
            "parts": [{"text": "You are a weather specialist."}]
        },
        "generationConfig": {
            "temperature": 0.1,
            "maxOutputTokens": 1024
        }
    });

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
        .and(header("x-goog-api-key", "AIzaSyToolTestKey"))
        .and(body_json(&complex_payload))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "get_current_weather",
                            "args": {
                                "location": "Tokyo"
                            }
                        }
                    }]
                }
            }]
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/v1beta/models/gemini-2.5-flash:generateContent", server_url))
        .json(&complex_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let fn_call = &body["candidates"][0]["content"]["parts"][0]["functionCall"];
    assert_eq!(fn_call["name"], "get_current_weather");
    assert_eq!(fn_call["args"]["location"], "Tokyo");
}

#[tokio::test]
async fn test_admin_auth_protection() {
    let mock_server = MockServer::start().await;
    let (server_url, state) = setup_test_server(&mock_server).await;

    state
        .key_manager
        .add_key("AIzaSyAuthTestKey", "auth-test")
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // 1. Unauthenticated request without headers
    let unauth_res = client
        .get(format!("{}/admin/stats", server_url))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_res.status(), 401);

    // 2. Request with invalid Bearer token
    let bad_auth_res = client
        .get(format!("{}/admin/stats", server_url))
        .header("Authorization", "Bearer invalid-secret")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_auth_res.status(), 401);

    // 3. Request with valid Bearer token
    let ok_bearer_res = client
        .get(format!("{}/admin/stats", server_url))
        .header("Authorization", "Bearer test-secret-key-123")
        .send()
        .await
        .unwrap();
    assert_eq!(ok_bearer_res.status(), 200);
    let stats: serde_json::Value = ok_bearer_res.json().await.unwrap();
    assert_eq!(stats["total_keys"], 1);

    // 4. Request with valid x-admin-key header
    let mut custom_headers = HeaderMap::new();
    custom_headers.insert("x-admin-key", "test-secret-key-123".parse().unwrap());
    let ok_custom_res = client
        .get(format!("{}/admin/stats", server_url))
        .headers(custom_headers)
        .send()
        .await
        .unwrap();
    assert_eq!(ok_custom_res.status(), 200);

    // 5. Query param token auth for EventSource
    let ok_query_res = client
        .get(format!("{}/admin/stats?token=test-secret-key-123", server_url))
        .send()
        .await
        .unwrap();
    assert_eq!(ok_query_res.status(), 200);
}
