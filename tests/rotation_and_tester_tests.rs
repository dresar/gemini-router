use axum::{
    body::Body,
    extract::{Path, Request},
    http::StatusCode,
    response::Response,
    routing::post,
    Router,
};
use chrono::{Duration, Utc};
use gemini_router::database::{init_pool, run_migrations};
use gemini_router::gemini::types::{KeyRecord, KeyStatus, StrategyType, SystemEvent};
use gemini_router::key_manager::repository::insert_key;
use gemini_router::key_manager::KeyManager;
use gemini_router::rotation::strategy::{
    LeastRecentlyUsedStrategy, LowestFailureRateStrategy, LowestLatencyStrategy,
    RoundRobinStrategy, SelectionStrategy, WeightedStrategy,
};
use gemini_router::rotation::RotationEngine;
use gemini_router::tester::FastTester;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

fn create_sample_key(
    id: &str,
    status: KeyStatus,
    latency: f64,
    total: u64,
    failed: u64,
    last_used: Option<chrono::DateTime<Utc>>,
    cooldown: Option<chrono::DateTime<Utc>>,
) -> KeyRecord {
    KeyRecord {
        id: id.to_string(),
        key_hash: format!("hash-{}", id),
        api_key: format!("AIzaSyMockKey-{}", id),
        masked_key: format!("AIza...{}", id),
        alias: format!("alias-{}", id),
        status,
        total_requests: total,
        successful_requests: total.saturating_sub(failed),
        failed_requests: failed,
        rate_limit_count: 0,
        quota_exhausted_count: 0,
        last_used_at: last_used,
        last_error: None,
        average_latency_ms: latency,
        cooldown_expires_at: cooldown,
        in_flight: 0,
    }
}

#[test]
fn test_round_robin_strategy() {
    let k1 = create_sample_key("k1", KeyStatus::Active, 100.0, 0, 0, None, None);
    let k2 = create_sample_key("k2", KeyStatus::Active, 100.0, 0, 0, None, None);
    let k3 = create_sample_key("k3", KeyStatus::Active, 100.0, 0, 0, None, None);

    let keys = vec![k1, k2, k3];
    let rr = RoundRobinStrategy::new();

    assert_eq!(rr.select(&keys).unwrap().id, "k1");
    assert_eq!(rr.select(&keys).unwrap().id, "k2");
    assert_eq!(rr.select(&keys).unwrap().id, "k3");
    assert_eq!(rr.select(&keys).unwrap().id, "k1");
    assert_eq!(rr.select(&keys).unwrap().id, "k2");

    let empty: Vec<KeyRecord> = Vec::new();
    assert!(rr.select(&empty).is_none());
}

#[test]
fn test_least_recently_used_strategy() {
    let now = Utc::now();
    let k1 = create_sample_key(
        "k1",
        KeyStatus::Active,
        100.0,
        5,
        0,
        Some(now - Duration::minutes(10)),
        None,
    );
    let k2 = create_sample_key("k2", KeyStatus::Active, 100.0, 0, 0, None, None);
    let k3 = create_sample_key(
        "k3",
        KeyStatus::Active,
        100.0,
        5,
        0,
        Some(now - Duration::minutes(30)),
        None,
    );
    let k4 = create_sample_key(
        "k4",
        KeyStatus::Active,
        100.0,
        5,
        0,
        Some(now - Duration::minutes(2)),
        None,
    );

    let keys = vec![k1.clone(), k2.clone(), k3.clone(), k4.clone()];
    let lru = LeastRecentlyUsedStrategy::new();

    assert_eq!(lru.select(&keys).unwrap().id, "k2");

    let keys_without_none = vec![k1, k3, k4];
    assert_eq!(lru.select(&keys_without_none).unwrap().id, "k3");

    let empty: Vec<KeyRecord> = Vec::new();
    assert!(lru.select(&empty).is_none());
}

#[test]
fn test_lowest_failure_rate_strategy() {
    let k1 = create_sample_key("k1", KeyStatus::Active, 100.0, 10, 5, None, None);
    let k2 = create_sample_key("k2", KeyStatus::Active, 100.0, 10, 1, None, None);
    let k3 = create_sample_key("k3", KeyStatus::Active, 100.0, 10, 3, None, None);

    let keys = vec![k1, k2, k3];
    let lfr = LowestFailureRateStrategy::new();

    assert_eq!(lfr.select(&keys).unwrap().id, "k2");

    let k_unused = create_sample_key("k4", KeyStatus::Active, 100.0, 0, 0, None, None);
    let mut keys_with_unused = keys;
    keys_with_unused.push(k_unused);
    assert_eq!(lfr.select(&keys_with_unused).unwrap().id, "k4");
}

#[test]
fn test_lowest_latency_strategy() {
    let k1 = create_sample_key("k1", KeyStatus::Active, 450.0, 10, 0, None, None);
    let k2 = create_sample_key("k2", KeyStatus::Active, 85.0, 10, 0, None, None);
    let k3 = create_sample_key("k3", KeyStatus::Active, 210.0, 10, 0, None, None);

    let keys = vec![k1, k2, k3];
    let ll = LowestLatencyStrategy::new();

    assert_eq!(ll.select(&keys).unwrap().id, "k2");

    let empty: Vec<KeyRecord> = Vec::new();
    assert!(ll.select(&empty).is_none());
}

#[test]
fn test_weighted_strategy() {
    let now = Utc::now();
    let k_bad = create_sample_key(
        "bad",
        KeyStatus::Active,
        1200.0,
        100,
        80,
        Some(now - Duration::seconds(5)),
        None,
    );
    let k_good = create_sample_key(
        "good",
        KeyStatus::Active,
        60.0,
        100,
        1,
        Some(now - Duration::hours(2)),
        None,
    );

    let keys = vec![k_bad, k_good];
    let weighted = WeightedStrategy::new();

    assert_eq!(weighted.select(&keys).unwrap().id, "good");
}

#[tokio::test]
async fn test_cooldown_elimination_and_recovery() {
    let now = Utc::now();
    let k1 = create_sample_key(
        "k1-cooling",
        KeyStatus::Active,
        100.0,
        0,
        0,
        None,
        Some(now + Duration::seconds(120)),
    );
    let k2 = create_sample_key(
        "k2-expired",
        KeyStatus::RateLimited,
        100.0,
        0,
        0,
        None,
        Some(now - Duration::seconds(30)),
    );
    let k3 = create_sample_key("k3-disabled", KeyStatus::Disabled, 100.0, 0, 0, None, None);
    let k4 = create_sample_key("k4-active", KeyStatus::Active, 100.0, 0, 0, None, None);

    let engine = RotationEngine::new_with_keys(vec![k1, k2, k3, k4]);

    let eligible = engine.get_eligible_keys();
    let eligible_ids: Vec<String> = eligible.iter().map(|k| k.id.clone()).collect();

    assert!(!eligible_ids.contains(&"k1-cooling".to_string()));
    assert!(eligible_ids.contains(&"k2-expired".to_string()));
    assert!(!eligible_ids.contains(&"k3-disabled".to_string()));
    assert!(eligible_ids.contains(&"k4-active".to_string()));

    let selected = engine.select_key(StrategyType::RoundRobin);
    assert!(selected.is_some());
    let id = selected.unwrap().id;
    assert!(id == "k2-expired" || id == "k4-active");
}

#[tokio::test]
async fn test_detection_429_rate_limit_vs_quota_exhaustion() {
    let k = create_sample_key("k-test", KeyStatus::Active, 0.0, 0, 0, None, None);
    let engine = RotationEngine::new_with_keys(vec![k]);

    engine
        .record_result(
            "k-test",
            false,
            120.0,
            Some("HTTP 429: ResourceExhausted: rate limit exceeded, please retry"),
        )
        .await;

    let keys = engine.get_all_keys();
    let current_key = &keys[0];
    assert_eq!(current_key.status, KeyStatus::RateLimited);
    assert_eq!(current_key.rate_limit_count, 1);
    assert!(current_key.cooldown_expires_at.is_some());
    let expires = current_key.cooldown_expires_at.unwrap();
    let diff = (expires - Utc::now()).num_seconds();
    assert!(diff >= 50 && diff <= 120);

    engine
        .record_result(
            "k-test",
            false,
            150.0,
            Some("HTTP 429: Quota exceeded for quota metric 'Queries' and limit 'Queries per day'"),
        )
        .await;

    let keys = engine.get_all_keys();
    let current_key = &keys[0];
    assert_eq!(current_key.status, KeyStatus::QuotaExhausted);
    assert_eq!(current_key.quota_exhausted_count, 1);
    let expires = current_key.cooldown_expires_at.unwrap();
    let diff_hours = (expires - Utc::now()).num_hours();
    assert!(diff_hours >= 11 && diff_hours <= 13);

    engine
        .record_result(
            "k-test",
            false,
            80.0,
            Some("HTTP 400: API_KEY_INVALID: API key not valid"),
        )
        .await;

    let keys = engine.get_all_keys();
    let current_key = &keys[0];
    assert_eq!(current_key.status, KeyStatus::Invalid);
    assert!(current_key.cooldown_expires_at.is_none());

    engine.record_result("k-test", true, 90.0, None).await;

    let keys = engine.get_all_keys();
    let current_key = &keys[0];
    assert_eq!(current_key.status, KeyStatus::Active);
    assert_eq!(current_key.successful_requests, 1);
    assert_eq!(current_key.failed_requests, 3);
    assert_eq!(current_key.total_requests, 4);
    assert!(current_key.cooldown_expires_at.is_none());
}

#[tokio::test]
async fn test_execute_with_failover() {
    let k1 = create_sample_key("k1", KeyStatus::Active, 100.0, 0, 0, None, None);
    let k2 = create_sample_key("k2", KeyStatus::Active, 100.0, 0, 0, None, None);
    let k3 = create_sample_key("k3", KeyStatus::Active, 100.0, 0, 0, None, None);

    let engine = RotationEngine::new_with_keys(vec![k1, k2, k3]);

    let outcome = engine
        .execute_with_failover(StrategyType::RoundRobin, 3, |key| async move {
            if key.id == "k1" {
                Err((429, "Rate limit reached".to_string()))
            } else if key.id == "k2" {
                Err((503, "Service unavailable".to_string()))
            } else {
                Ok("success-payload".to_string())
            }
        })
        .await;

    assert!(outcome.is_ok());
    assert_eq!(outcome.unwrap(), "success-payload");

    let keys = engine.get_all_keys();
    let key1 = keys.iter().find(|k| k.id == "k1").unwrap();
    let key2 = keys.iter().find(|k| k.id == "k2").unwrap();
    let key3 = keys.iter().find(|k| k.id == "k3").unwrap();

    assert_eq!(key1.status, KeyStatus::RateLimited);
    assert_eq!(key2.status, KeyStatus::TemporaryError);
    assert_eq!(key3.status, KeyStatus::Active);

    let failed_outcome = engine
        .execute_with_failover(StrategyType::RoundRobin, 3, |_key| async move {
            Err::<(String,), _>((429, "All keys exhausted".to_string()))
        })
        .await;

    assert!(failed_outcome.is_err());
    assert_eq!(failed_outcome.unwrap_err().status_code, 503);
}

async fn mock_gemini_handler(Path(model): Path<String>, req: Request<Body>) -> Response<Body> {
    let header_val = req
        .headers()
        .get("x-goog-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let uri = req.uri().to_string();
    let key_val = if !header_val.is_empty() {
        header_val
    } else {
        uri.split("key=")
            .nth(1)
            .unwrap_or_default()
            .split('&')
            .next()
            .unwrap_or_default()
            .to_string()
    };

    let _ = model;

    if key_val.contains("active") {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"candidates":[{"content":{"parts":[{"text":"pong"}]}}]}]"#))
            .unwrap()
    } else if key_val.contains("invalid") {
        Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"error":{"code":400,"message":"API key not valid","status":"API_KEY_INVALID"}}"#,
            ))
            .unwrap()
    } else if key_val.contains("rate") {
        Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"error":{"code":429,"message":"Resource has been exhausted: rate limit exceeded","status":"RESOURCE_EXHAUSTED"}}"#,
            ))
            .unwrap()
    } else if key_val.contains("quota") {
        Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"error":{"code":429,"message":"Quota exceeded for quota metric 'Queries' and limit 'Queries per day'","status":"RESOURCE_EXHAUSTED"}}"#,
            ))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"error":{"code":503,"message":"Service Unavailable","status":"UNAVAILABLE"}}"#,
            ))
            .unwrap()
    }
}

#[tokio::test]
async fn test_parallel_tester_with_mock_upstream() {
    let mock_app = Router::new().route(
        "/v1beta/models/:model:generateContent",
        post(mock_gemini_handler),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, mock_app).await.unwrap();
    });

    let mock_url = format!("http://{}", mock_addr);

    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();

    let keys_data = vec![
        ("mock-active", KeyStatus::Active),
        ("mock-invalid", KeyStatus::Active),
        ("mock-rate", KeyStatus::Active),
        ("mock-quota", KeyStatus::Active),
        ("mock-server-error", KeyStatus::Active),
    ];

    for (name, st) in &keys_data {
        let mut key = create_sample_key(name, *st, 0.0, 0, 0, None, None);
        key.api_key = format!("AIzaSy-{}", name);
        insert_key(&pool, &key).await.unwrap();
    }

    let key_mgr = Arc::new(KeyManager::new(pool).await.unwrap());
    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel::<SystemEvent>(100);

    let tester = FastTester::new(reqwest::Client::new(), key_mgr.clone(), Some(event_tx))
        .with_base_url(&mock_url)
        .with_concurrency(4)
        .with_model("gemini-1.5-flash");

    let results = tester.test_all_keys().await;
    assert_eq!(results.len(), 5);

    let res_active = results.iter().find(|r| r.key_id == "mock-active").unwrap();
    assert_eq!(res_active.status, KeyStatus::Active);
    assert_eq!(res_active.http_status, Some(200));

    let res_invalid = results.iter().find(|r| r.key_id == "mock-invalid").unwrap();
    assert_eq!(res_invalid.status, KeyStatus::Invalid);
    assert_eq!(res_invalid.http_status, Some(400));

    let res_rate = results.iter().find(|r| r.key_id == "mock-rate").unwrap();
    assert_eq!(res_rate.status, KeyStatus::RateLimited);
    assert_eq!(res_rate.http_status, Some(429));

    let res_quota = results.iter().find(|r| r.key_id == "mock-quota").unwrap();
    assert_eq!(res_quota.status, KeyStatus::QuotaExhausted);
    assert_eq!(res_quota.http_status, Some(429));

    let res_err = results
        .iter()
        .find(|r| r.key_id == "mock-server-error")
        .unwrap();
    assert_eq!(res_err.status, KeyStatus::TemporaryError);
    assert_eq!(res_err.http_status, Some(503));

    let cached_keys = key_mgr.get_keys();
    let cached_active = cached_keys.iter().find(|k| k.id == "mock-active").unwrap();
    assert_eq!(cached_active.status, KeyStatus::Active);
    assert_eq!(cached_active.total_requests, 1);
    assert_eq!(cached_active.successful_requests, 1);

    let cached_invalid = cached_keys.iter().find(|k| k.id == "mock-invalid").unwrap();
    assert_eq!(cached_invalid.status, KeyStatus::Invalid);

    let mut event_count = 0;
    while let Ok(event) = event_rx.try_recv() {
        event_count += 1;
        match event {
            SystemEvent::TestStarted { total_keys } => {
                assert_eq!(total_keys, 5);
            }
            SystemEvent::TestProgress { total, .. } => {
                assert_eq!(total, 5);
            }
            SystemEvent::TestCompleted {
                tested,
                active,
                failed,
            } => {
                assert_eq!(tested, 5);
                assert_eq!(active, 1);
                assert_eq!(failed, 4);
            }
            _ => {}
        }
    }
    assert!(event_count >= 6);
}
