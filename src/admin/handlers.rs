use crate::admin::auth::{authenticate, verify_admin_key};
use crate::gemini::types::{KeyStatus, KeyView, StrategyType};
use crate::AppState;
use axum::extract::{Path, Request, State};
use axum::http::{header::HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Response;
use axum::Json;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let mut authenticated = authenticate(&headers, &state.config.admin_key);

    if !authenticated {
        if let Some(query) = uri.query() {
            for param in query.split('&') {
                if let Some((k, v)) = param.split_once('=') {
                    if k == "token" && verify_admin_key(&state.config.admin_key, v) {
                        authenticated = true;
                        break;
                    }
                }
            }
        }
    }

    if !authenticated {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "Unauthorized: Invalid or missing admin authentication credentials"
            })),
        ));
    }

    Ok(next.run(request).await)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminStats {
    pub total_keys: usize,
    pub active_keys: usize,
    pub rate_limited_keys: usize,
    pub quota_exhausted_keys: usize,
    pub invalid_keys: usize,
    pub disabled_keys: usize,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub average_latency_ms: f64,
}

pub async fn get_stats(State(state): State<Arc<AppState>>) -> Json<AdminStats> {
    let keys = state.key_manager.get_keys();
    let total_keys = keys.len();

    let mut active_keys = 0;
    let mut rate_limited_keys = 0;
    let mut quota_exhausted_keys = 0;
    let mut invalid_keys = 0;
    let mut disabled_keys = 0;

    let mut total_requests = 0;
    let mut successful_requests = 0;
    let mut failed_requests = 0;
    let mut latency_sum = 0.0;
    let mut latency_count = 0;

    for k in &keys {
        match k.status {
            KeyStatus::Active => active_keys += 1,
            KeyStatus::RateLimited => rate_limited_keys += 1,
            KeyStatus::QuotaExhausted => quota_exhausted_keys += 1,
            KeyStatus::Invalid => invalid_keys += 1,
            KeyStatus::Disabled => disabled_keys += 1,
            _ => {}
        }

        total_requests += k.total_requests;
        successful_requests += k.successful_requests;
        failed_requests += k.failed_requests;

        if k.total_requests > 0 {
            latency_sum += k.average_latency_ms;
            latency_count += 1;
        }
    }

    let average_latency_ms = if latency_count > 0 {
        latency_sum / latency_count as f64
    } else {
        0.0
    };

    Json(AdminStats {
        total_keys,
        active_keys,
        rate_limited_keys,
        quota_exhausted_keys,
        invalid_keys,
        disabled_keys,
        total_requests,
        successful_requests,
        failed_requests,
        average_latency_ms,
    })
}

pub async fn get_keys(State(state): State<Arc<AppState>>) -> Json<Vec<KeyView>> {
    Json(state.key_manager.get_key_views())
}

#[derive(Debug, Deserialize)]
pub struct ImportKeysRequest {
    pub keys: Value,
}

pub async fn import_keys(
    State(state): State<Arc<AppState>>,
    body: String,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result = state
        .key_manager
        .batch_import_text(&body)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Import error: {e}")})),
            )
        })?;

    Ok(Json(json!({
        "imported": result.inserted,
        "skipped": result.duplicates,
        "inserted": result.inserted,
        "duplicates": result.duplicates,
        "total": result.inserted + result.duplicates
    })))
}

#[derive(Debug, Deserialize)]
pub struct AddKeyRequest {
    pub key: String,
    pub alias: Option<String>,
}

pub async fn add_key(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AddKeyRequest>,
) -> Result<(StatusCode, Json<KeyView>), (StatusCode, Json<Value>)> {
    let alias = payload.alias.unwrap_or_default();
    let record = state
        .key_manager
        .add_key(&payload.key, &alias)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Failed to add key: {e}")})),
            )
        })?;

    Ok((StatusCode::CREATED, Json(KeyView::from(&record))))
}

pub async fn delete_key(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let deleted = state.key_manager.delete_key(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to delete key: {e}")})),
        )
    })?;

    if deleted {
        Ok(Json(json!({"deleted": true, "id": id})))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Key not found"})),
        ))
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateKeyRequest {
    pub status: Option<String>,
    pub alias: Option<String>,
}

pub async fn patch_key(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateKeyRequest>,
) -> Result<Json<KeyView>, (StatusCode, Json<Value>)> {
    if let Some(ref st_str) = payload.status {
        let status = KeyStatus::from_str_loose(st_str);
        state
            .key_manager
            .update_key_status(&id, status, None, None)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to update status: {e}")})),
                )
            })?;
    }

    if let Some(ref alias) = payload.alias {
        state
            .key_manager
            .update_alias(&id, alias)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to update alias: {e}")})),
                )
            })?;
    }

    let record = state
        .key_manager
        .get_key_by_id(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "Key not found"}))))?;

    Ok(Json(KeyView::from(&record)))
}

#[derive(Debug, Deserialize)]
pub struct TestKeyRequest {
    pub id: String,
}

pub async fn test_single_key(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<TestKeyRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (status, latency, err) = state
        .tester
        .test_single(&payload.id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Test failed: {e}")})),
            )
        })?;

    Ok(Json(json!({
        "id": payload.id,
        "status": status.as_str(),
        "latency_ms": latency,
        "error": err
    })))
}

pub async fn test_all_keys(State(state): State<Arc<AppState>>) -> Json<Value> {
    let queued = state.tester.test_all();
    Json(json!({
        "status": "started",
        "total": queued
    }))
}

pub async fn get_rotation(State(state): State<Arc<AppState>>) -> Json<Value> {
    let info = state.rotation_engine.get_info();
    Json(json!({
        "strategy": info.strategy.as_str(),
        "total_rotations": info.total_rotations,
        "failovers": info.failovers,
        "in_flight": info.in_flight,
        "last_used_key": info.last_used_key
    }))
}

pub async fn get_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.event_tx.subscribe();

    let stream = async_stream::stream! {
        while let Ok(event) = rx.recv().await {
            if let Ok(data) = serde_json::to_string(&event) {
                yield Ok(Event::default().data(data));
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsView {
    pub strategy: String,
    pub concurrency: usize,
    pub request_timeout_secs: u64,
    pub retry_budget: u32,
    pub cooldown_rate_limit_secs: u64,
    pub cooldown_quota_hours: u64,
}

pub async fn get_settings(State(state): State<Arc<AppState>>) -> Json<SettingsView> {
    Json(SettingsView {
        strategy: state.rotation_engine.get_strategy().to_string(),
        concurrency: state.config.tester_concurrency,
        request_timeout_secs: state.config.request_timeout_secs,
        retry_budget: state.config.retry_budget,
        cooldown_rate_limit_secs: state.config.cooldown_rate_limit_secs,
        cooldown_quota_hours: state.config.cooldown_quota_hours,
    })
}

#[derive(Debug, Deserialize)]
pub struct PatchSettingsRequest {
    pub strategy: Option<String>,
    pub concurrency: Option<usize>,
    pub request_timeout_secs: Option<u64>,
    pub retry_budget: Option<u32>,
}

pub async fn patch_settings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PatchSettingsRequest>,
) -> Json<SettingsView> {
    if let Some(ref st_str) = payload.strategy {
        let strategy = StrategyType::from_str_loose(st_str);
        state.rotation_engine.set_strategy(strategy);
    }

    Json(SettingsView {
        strategy: state.rotation_engine.get_strategy().to_string(),
        concurrency: payload.concurrency.unwrap_or(state.config.tester_concurrency),
        request_timeout_secs: payload
            .request_timeout_secs
            .unwrap_or(state.config.request_timeout_secs),
        retry_budget: payload.retry_budget.unwrap_or(state.config.retry_budget),
        cooldown_rate_limit_secs: state.config.cooldown_rate_limit_secs,
        cooldown_quota_hours: state.config.cooldown_quota_hours,
    })
}
