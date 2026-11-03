use crate::streaming::create_sse_response;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

pub async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "timestamp": Utc::now().to_rfc3339()
    }))
}

pub async fn list_models_v1(
    State(state): State<Arc<AppState>>,
    raw_query: RawQuery,
) -> Response {
    forward_list_models(state, "v1", raw_query.0).await
}

pub async fn list_models_v1beta(
    State(state): State<Arc<AppState>>,
    raw_query: RawQuery,
) -> Response {
    forward_list_models(state, "v1beta", raw_query.0).await
}

pub async fn get_model_v1(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    raw_query: RawQuery,
) -> Response {
    forward_get_model(state, "v1", path, raw_query.0).await
}

pub async fn get_model_v1beta(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    raw_query: RawQuery,
) -> Response {
    forward_get_model(state, "v1beta", path, raw_query.0).await
}

pub async fn model_action_v1(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    raw_query: RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_model_action(state, "v1", path, raw_query.0, headers, body).await
}

pub async fn model_action_v1beta(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    raw_query: RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_model_action(state, "v1beta", path, raw_query.0, headers, body).await
}

async fn forward_list_models(
    state: Arc<AppState>,
    api_version: &str,
    raw_query: Option<String>,
) -> Response {
    let retry_budget = state.config.retry_budget.max(1);
    let mut last_error_response = None;
    let mut tried_keys = std::collections::HashSet::new();

    for _attempt in 0..retry_budget {
        let strategy = state.rotation_engine.get_strategy();
        let key = match state.rotation_engine.select_key_excluding(strategy, &tried_keys) {
            Some(k) => k,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": {
                            "code": 503,
                            "message": "No active API keys available for rotation",
                            "status": "UNAVAILABLE"
                        }
                    })),
                )
                    .into_response();
            }
        };
        tried_keys.insert(key.id.clone());

        let base_url = state.config.gemini_base_url.trim_end_matches('/');
        let mut target_url = format!("{base_url}/{api_version}/models");
        if let Some(ref q) = raw_query {
            if !q.is_empty() {
                target_url.push('?');
                target_url.push_str(q);
            }
        }

        let start = Instant::now();
        let res_res = state
            .http_client
            .get(&target_url)
            .header("x-goog-api-key", &key.api_key)
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as f64;

        match res_res {
            Ok(res) => {
                let status = res.status();
                if status == StatusCode::TOO_MANY_REQUESTS {
                    state
                        .rotation_engine
                        .record_result(&key.id, false, latency_ms, Some("HTTP 429: Rate limited"))
                        .await;
                    last_error_response = Some((
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({"error": {"code": 429, "message": "Rate limited", "status": "RESOURCE_EXHAUSTED"}})),
                    ).into_response());
                    continue;
                }

                if status == StatusCode::SERVICE_UNAVAILABLE {
                    state
                        .rotation_engine
                        .record_result(&key.id, false, latency_ms, Some("HTTP 503: Service Unavailable"))
                        .await;
                    last_error_response = Some((
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error": {"code": 503, "message": "Service unavailable", "status": "UNAVAILABLE"}})),
                    ).into_response());
                    continue;
                }

                state
                    .rotation_engine
                    .record_result(&key.id, status.is_success(), latency_ms, None)
                    .await;

                return build_safe_response(res, &key.api_key, &key.masked_key).await;
            }
            Err(e) => {
                state
                    .rotation_engine
                    .record_result(&key.id, false, latency_ms, Some(&e.to_string()))
                    .await;
                last_error_response = Some((
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": {"code": 502, "message": e.to_string(), "status": "UNAVAILABLE"}})),
                ).into_response());
            }
        }
    }

    last_error_response.unwrap_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"code": 502, "message": "Retry budget exhausted"}})),
        )
            .into_response()
    })
}

async fn forward_get_model(
    state: Arc<AppState>,
    api_version: &str,
    path: String,
    raw_query: Option<String>,
) -> Response {
    let retry_budget = state.config.retry_budget.max(1);
    let mut last_error_response = None;
    let mut tried_keys = std::collections::HashSet::new();

    for _attempt in 0..retry_budget {
        let strategy = state.rotation_engine.get_strategy();
        let key = match state.rotation_engine.select_key_excluding(strategy, &tried_keys) {
            Some(k) => k,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": {
                            "code": 503,
                            "message": "No active API keys available for rotation",
                            "status": "UNAVAILABLE"
                        }
                    })),
                )
                    .into_response();
            }
        };
        tried_keys.insert(key.id.clone());

        let base_url = state.config.gemini_base_url.trim_end_matches('/');
        let mut target_url = format!("{base_url}/{api_version}/models/{path}");
        if let Some(ref q) = raw_query {
            if !q.is_empty() {
                target_url.push('?');
                target_url.push_str(q);
            }
        }

        let start = Instant::now();
        let res_res = state
            .http_client
            .get(&target_url)
            .header("x-goog-api-key", &key.api_key)
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as f64;

        match res_res {
            Ok(res) => {
                let status = res.status();
                if status == StatusCode::TOO_MANY_REQUESTS {
                    state
                        .rotation_engine
                        .record_result(&key.id, false, latency_ms, Some("HTTP 429: Rate limited"))
                        .await;
                    last_error_response = Some((
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({"error": {"code": 429, "message": "Rate limited", "status": "RESOURCE_EXHAUSTED"}})),
                    ).into_response());
                    continue;
                }

                if status == StatusCode::SERVICE_UNAVAILABLE {
                    state
                        .rotation_engine
                        .record_result(&key.id, false, latency_ms, Some("HTTP 503: Service Unavailable"))
                        .await;
                    last_error_response = Some((
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error": {"code": 503, "message": "Service unavailable", "status": "UNAVAILABLE"}})),
                    ).into_response());
                    continue;
                }

                state
                    .rotation_engine
                    .record_result(&key.id, status.is_success(), latency_ms, None)
                    .await;

                return build_safe_response(res, &key.api_key, &key.masked_key).await;
            }
            Err(e) => {
                state
                    .rotation_engine
                    .record_result(&key.id, false, latency_ms, Some(&e.to_string()))
                    .await;
                last_error_response = Some((
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": {"code": 502, "message": e.to_string(), "status": "UNAVAILABLE"}})),
                ).into_response());
            }
        }
    }

    last_error_response.unwrap_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"code": 502, "message": "Retry budget exhausted"}})),
        )
            .into_response()
    })
}

async fn forward_model_action(
    state: Arc<AppState>,
    api_version: &str,
    path: String,
    raw_query: Option<String>,
    _client_headers: HeaderMap,
    body: Bytes,
) -> Response {
    let is_streaming = path.ends_with(":streamGenerateContent")
        || raw_query
            .as_deref()
            .map(|q| q.contains("alt=sse"))
            .unwrap_or(false);

    let retry_budget = state.config.retry_budget.max(1);
    let mut last_error_response = None;
    let mut tried_keys = std::collections::HashSet::new();

    for _attempt in 0..retry_budget {
        let strategy = state.rotation_engine.get_strategy();
        let key = match state.rotation_engine.select_key_excluding(strategy, &tried_keys) {
            Some(k) => k,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": {
                            "code": 503,
                            "message": "No active API keys available for rotation",
                            "status": "UNAVAILABLE"
                        }
                    })),
                )
                    .into_response();
            }
        };
        tried_keys.insert(key.id.clone());

        let base_url = state.config.gemini_base_url.trim_end_matches('/');
        let mut target_url = format!("{base_url}/{api_version}/models/{path}");

        // Build query string ensuring alt=sse is present if streaming
        let mut query_parts = Vec::new();
        if let Some(ref q) = raw_query {
            if !q.is_empty() {
                query_parts.push(q.clone());
            }
        }
        if is_streaming
            && !raw_query
                .as_deref()
                .map(|q| q.contains("alt=sse"))
                .unwrap_or(false)
        {
            query_parts.push("alt=sse".to_string());
        }

        if !query_parts.is_empty() {
            target_url.push('?');
            target_url.push_str(&query_parts.join("&"));
        }

        let start = Instant::now();
        let res_res = state
            .http_client
            .post(&target_url)
            .header("x-goog-api-key", &key.api_key)
            .header(CONTENT_TYPE, "application/json")
            .body(body.clone())
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as f64;

        match res_res {
            Ok(res) => {
                let status = res.status();

                // Check for 429 Too Many Requests -> Failover
                if status == StatusCode::TOO_MANY_REQUESTS {
                    state
                        .rotation_engine
                        .record_result(&key.id, false, latency_ms, Some("HTTP 429: Rate limited"))
                        .await;
                    last_error_response = Some((
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({
                            "error": {
                                "code": 429,
                                "message": "Rate limit exceeded upstream",
                                "status": "RESOURCE_EXHAUSTED"
                            }
                        })),
                    ).into_response());
                    continue;
                }

                // Check for 503 Service Unavailable -> Failover
                if status == StatusCode::SERVICE_UNAVAILABLE {
                    state
                        .rotation_engine
                        .record_result(&key.id, false, latency_ms, Some("HTTP 503: Service Unavailable"))
                        .await;
                    last_error_response = Some((
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({
                            "error": {
                                "code": 503,
                                "message": "Upstream service temporarily unavailable",
                                "status": "UNAVAILABLE"
                            }
                        })),
                    ).into_response());
                    continue;
                }

                state
                    .rotation_engine
                    .record_result(&key.id, status.is_success(), latency_ms, None)
                    .await;

                // If streaming and success, return SSE stream without buffering
                if is_streaming && status.is_success() {
                    return create_sse_response(res);
                }

                // Standard non-streaming or error response
                return build_safe_response(res, &key.api_key, &key.masked_key).await;
            }
            Err(e) => {
                state
                    .rotation_engine
                    .record_result(&key.id, false, latency_ms, Some(&e.to_string()))
                    .await;
                last_error_response = Some((
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "error": {
                            "code": 502,
                            "message": e.to_string(),
                            "status": "UNAVAILABLE"
                        }
                    })),
                ).into_response());
            }
        }
    }

    last_error_response.unwrap_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "code": 502,
                    "message": "All retry attempts exhausted",
                    "status": "UNAVAILABLE"
                }
            })),
        )
            .into_response()
    })
}

async fn build_safe_response(
    res: reqwest::Response,
    plain_key: &str,
    masked_key: &str,
) -> Response {
    let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::OK);
    let content_type = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();

    let bytes = res.bytes().await.unwrap_or_default();

    // Mask any accidental exposure of the plaintext key in response body
    let safe_bytes = if !plain_key.is_empty() && bytes.len() >= plain_key.len() {
        let plain_bytes = plain_key.as_bytes();
        if bytes.windows(plain_bytes.len()).any(|w| w == plain_bytes) {
            let s = String::from_utf8_lossy(&bytes);
            let sanitized = s.replace(plain_key, masked_key);
            Bytes::from(sanitized.into_bytes())
        } else {
            bytes
        }
    } else {
        bytes
    };

    let mut response_builder = Response::builder().status(status);

    if let Ok(val) = HeaderValue::from_str(&content_type) {
        response_builder = response_builder.header(CONTENT_TYPE, val);
    }

    response_builder
        .body(axum::body::Body::from(safe_bytes))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to build response").into_response()
        })
}
