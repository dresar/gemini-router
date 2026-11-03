use axum::http::HeaderMap;
use gemini_router::admin::auth::{authenticate, extract_token, verify_admin_key};
use gemini_router::config::{hash_key, mask_key};
use gemini_router::database::{init_pool, run_migrations};
use gemini_router::gemini::types::{KeyRecord, KeyStatus, StrategyType};
use gemini_router::key_manager::repository::*;
use gemini_router::key_manager::KeyManager;

#[test]
fn test_mask_and_hash_key() {
    let raw = "AIzaSyDummyKeyForTestingPurposes1234567";
    let masked = mask_key(raw);
    assert_eq!(masked, "AIzaSy...4567");

    let short = "1234";
    assert_eq!(mask_key(short), "***");

    let hashed = hash_key(raw);
    assert_eq!(hashed.len(), 64);
    assert_eq!(hashed, hash_key(raw));
}

#[test]
fn test_admin_auth() {
    let secret = "gemini-router-admin-secret-2026";
    assert!(verify_admin_key(secret, secret));
    assert!(!verify_admin_key(secret, "wrong-secret"));
    assert!(!verify_admin_key(secret, "short"));

    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer gemini-router-admin-secret-2026".parse().unwrap());
    assert_eq!(extract_token(&headers), Some("gemini-router-admin-secret-2026"));
    assert!(authenticate(&headers, secret));

    let mut custom_headers = HeaderMap::new();
    custom_headers.insert("x-admin-key", "gemini-router-admin-secret-2026".parse().unwrap());
    assert!(authenticate(&custom_headers, secret));
}

#[test]
fn test_enum_parsing() {
    assert_eq!(KeyStatus::from_str_loose("ACTIVE"), KeyStatus::Active);
    assert_eq!(KeyStatus::from_str_loose("rate_limited"), KeyStatus::RateLimited);
    assert_eq!(KeyStatus::from_str_loose("QUOTA_EXHAUSTED"), KeyStatus::QuotaExhausted);
    assert_eq!(KeyStatus::from_str_loose("BILLING_ERROR"), KeyStatus::BillingError);
    assert_eq!(KeyStatus::from_str_loose("DISABLED"), KeyStatus::Disabled);

    assert_eq!(StrategyType::from_str_loose("round_robin"), StrategyType::RoundRobin);
    assert_eq!(StrategyType::from_str_loose("lru"), StrategyType::LeastRecentlyUsed);
    assert_eq!(StrategyType::from_str_loose("weighted"), StrategyType::Weighted);
}

#[tokio::test]
async fn test_sqlite_repository_and_key_manager() {
    let db_url = "sqlite::memory:";
    let pool = init_pool(db_url).await.expect("Failed to init pool");
    run_migrations(&pool).await.expect("Failed to run migrations");

    let sample_key = KeyRecord {
        id: "key-1".to_string(),
        key_hash: hash_key("AIzaSyTest1"),
        api_key: "AIzaSyTest1".to_string(),
        masked_key: mask_key("AIzaSyTest1"),
        alias: "primary-key".to_string(),
        status: KeyStatus::Active,
        total_requests: 0,
        successful_requests: 0,
        failed_requests: 0,
        rate_limit_count: 0,
        quota_exhausted_count: 0,
        last_used_at: None,
        last_error: None,
        average_latency_ms: 0.0,
        cooldown_expires_at: None,
        in_flight: 0,
    };

    insert_key(&pool, &sample_key).await.expect("Insert key failed");

    let fetched = get_key_by_id(&pool, "key-1").await.expect("Fetch failed");
    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.alias, "primary-key");
    assert_eq!(fetched.status, KeyStatus::Active);

    let batch = vec![
        ("AIzaSyTest2".to_string(), "alias-2".to_string()),
        ("AIzaSyTest1".to_string(), "duplicate".to_string()),
        ("AIzaSyTest3".to_string(), "alias-3".to_string()),
    ];
    let import_res = batch_import(&pool, batch).await.expect("Batch import failed");
    assert_eq!(import_res.inserted, 2);
    assert_eq!(import_res.duplicates, 1);

    let all_keys = get_all_keys(&pool).await.expect("Get all failed");
    assert_eq!(all_keys.len(), 3);

    let status_updated = update_status(
        &pool,
        "key-1",
        KeyStatus::RateLimited,
        Some("HTTP 429 Too Many Requests"),
        None,
    )
    .await
    .expect("Update status failed");
    assert!(status_updated);

    let stats_updated = update_stats(&pool, "key-1", true, 120.5, false, false)
        .await
        .expect("Update stats failed");
    assert!(stats_updated);

    let key_mgr = KeyManager::new(pool.clone()).await.expect("KeyManager new failed");
    assert_eq!(key_mgr.get_keys().len(), 3);

    let add_res = key_mgr.add_key("AIzaSyTest4", "key-4").await;
    assert!(add_res.is_ok());
    assert_eq!(key_mgr.get_keys().len(), 4);

    let del_res = key_mgr.delete_key("key-1").await;
    assert!(del_res.unwrap());
    assert_eq!(key_mgr.get_keys().len(), 3);

    set_setting(&pool, "default_strategy", "lowest_latency")
        .await
        .expect("Set setting failed");
    let setting_val = get_setting(&pool, "default_strategy").await.expect("Get setting failed");
    assert_eq!(setting_val, Some("lowest_latency".to_string()));
}
