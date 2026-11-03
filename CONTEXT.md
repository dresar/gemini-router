# CONTEXT.md - System Contracts & Interface Boundaries

## 1. Data Models

### Key Status Enum
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KeyStatus {
    Active,
    Invalid,
    RateLimited,
    QuotaExhausted,
    BillingError,
    TemporaryError,
    Disabled,
}
```

### Rotation Strategy Enum
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyType {
    RoundRobin,
    LeastRecentlyUsed,
    LowestFailureRate,
    LowestLatency,
    Weighted,
}
```

### Key Record (Database & In-Memory)
```rust
pub struct KeyRecord {
    pub id: String,
    pub key_hash: String,
    pub api_key: String,       // Plaintext in memory, never exposed via REST or logs
    pub masked_key: String,    // e.g. "AIzaSy...7x9Q"
    pub alias: String,
    pub status: KeyStatus,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub rate_limit_count: u64,
    pub quota_exhausted_count: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub average_latency_ms: f64,
    pub cooldown_expires_at: Option<DateTime<Utc>>,
    pub in_flight: u32,
}
```

### Rest Key View (For Frontend & Admin API)
```rust
#[derive(Serialize)]
pub struct KeyView {
    pub id: String,
    pub masked_key: String,
    pub alias: String,
    pub status: KeyStatus,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub rate_limit_count: u64,
    pub quota_exhausted_count: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub average_latency_ms: f64,
    pub cooldown_expires_at: Option<DateTime<Utc>>,
}
```

## 2. Shared AppState
```rust
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: sqlx::SqlitePool,
    pub http_client: reqwest::Client,
    pub key_manager: Arc<KeyManager>,
    pub rotation_engine: Arc<RotationEngine>,
    pub tester: Arc<FastTester>,
    pub event_tx: tokio::sync::broadcast::Sender<SystemEvent>,
}
```

## 3. Endpoints Contract

### Proxy Endpoints (Port 8080)
- `POST /v1beta/models/{model}:generateContent` -> Forwarded to upstream Gemini with rotated key.
- `POST /v1beta/models/{model}:streamGenerateContent` -> Forwarded as native chunked SSE stream.
- `GET /v1/models` & `GET /v1beta/models` -> Forwarded to Gemini models list.
- `GET /health` -> Returns `{"status":"ok","timestamp":"..."}`.

### Admin REST API (Protected by Bearer Token / X-Admin-Key)
- `GET /admin/stats` -> Overall stats.
- `GET /admin/keys` -> List keys (with `KeyView`).
- `POST /admin/keys/import` -> Body: `{"keys": "KEY1\nKEY2..."}` or raw string.
- `POST /admin/keys` -> Body: `{"key": "...", "alias": "..."}`.
- `DELETE /admin/keys/{id}` -> Deletes key.
- `PATCH /admin/keys/{id}` -> Body: `{"status": "DISABLED"}` or `{"alias": "..."}`.
- `POST /admin/keys/test` -> Body: `{"id": "..."}`.
- `POST /admin/keys/test-all` -> Triggers parallel tester.
- `GET /admin/rotation` -> Active strategy, metrics.
- `GET /admin/events` -> SSE stream broadcasting live test progress and telemetry events.
- `GET /admin/settings` -> Current settings.
- `PATCH /admin/settings` -> Update settings (e.g., concurrency, strategy).

### Dashboard
- Hosted at `/` (`GET /` serves `static/index.html`).
- Static assets served from `/static` or embedded.
