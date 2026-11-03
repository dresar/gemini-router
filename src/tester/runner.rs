use crate::gemini::types::{KeyRecord, KeyStatus, SystemEvent};
use crate::key_manager::KeyManager;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Semaphore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub key_id: String,
    pub masked_key: String,
    pub status: KeyStatus,
    pub latency_ms: u64,
    pub http_status: Option<u16>,
    pub error_message: Option<String>,
}

pub struct FastTester {
    http_client: reqwest::Client,
    key_manager: Arc<KeyManager>,
    event_tx: Option<broadcast::Sender<SystemEvent>>,
    concurrency: usize,
    base_url: String,
    model: String,
}

impl FastTester {
    pub fn new(
        http_client: reqwest::Client,
        key_manager: Arc<KeyManager>,
        event_tx: Option<broadcast::Sender<SystemEvent>>,
    ) -> Self {
        Self {
            http_client,
            key_manager,
            event_tx,
            concurrency: 10,
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            model: "gemini-1.5-flash".to_string(),
        }
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn classify_response(
        status_code: u16,
        body: &str,
    ) -> (KeyStatus, Option<chrono::DateTime<Utc>>) {
        let now = Utc::now();
        let body_lower = body.to_lowercase();

        match status_code {
            200 => (KeyStatus::Active, None),
            400 => (KeyStatus::Invalid, None),
            401 => (KeyStatus::Invalid, None),
            403 => (KeyStatus::BillingError, None),
            429 => {
                if body_lower.contains("per day")
                    || body_lower.contains("daily")
                    || body_lower.contains("per_day")
                    || body_lower.contains("free tier limit")
                    || body_lower.contains("quota_exhausted")
                    || body_lower.contains("quota exceeded")
                {
                    (
                        KeyStatus::QuotaExhausted,
                        Some(now + chrono::Duration::hours(12)),
                    )
                } else {
                    (
                        KeyStatus::RateLimited,
                        Some(now + chrono::Duration::seconds(60)),
                    )
                }
            }
            500 | 503 => (
                KeyStatus::TemporaryError,
                Some(now + chrono::Duration::seconds(15)),
            ),
            _ => (
                KeyStatus::TemporaryError,
                Some(now + chrono::Duration::seconds(15)),
            ),
        }
    }

    pub async fn test_single_key(&self, key: &KeyRecord) -> TestResult {
        let url = format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            key.api_key
        );

        let payload = json!({
            "contents": [
                {
                    "parts": [
                        {"text": "ping"}
                    ]
                }
            ],
            "generationConfig": {
                "maxOutputTokens": 1
            }
        });

        let start = std::time::Instant::now();
        let response_res = self
            .http_client
            .post(&url)
            .header("x-goog-api-key", &key.api_key)
            .json(&payload)
            .send()
            .await;
        let latency_ms = start.elapsed().as_millis() as u64;

        let (status, http_status, error_message, cooldown) = match response_res {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                let body_text = resp.text().await.unwrap_or_default();
                let (st, cd) = Self::classify_response(status_code, &body_text);
                let err_msg = if st == KeyStatus::Active {
                    None
                } else {
                    Some(format!("HTTP {status_code}: {body_text}"))
                };
                (st, Some(status_code), err_msg, cd)
            }
            Err(err) => {
                let err_str = err.to_string();
                (
                    KeyStatus::TemporaryError,
                    None,
                    Some(err_str),
                    Some(Utc::now() + chrono::Duration::seconds(15)),
                )
            }
        };

        let _ = self
            .key_manager
            .update_key_status(&key.id, status, error_message.clone(), cooldown)
            .await;

        let is_success = status == KeyStatus::Active;
        let is_rate_limit = status == KeyStatus::RateLimited;
        let is_quota = status == KeyStatus::QuotaExhausted;
        let _ = self
            .key_manager
            .record_request_outcome(
                &key.id,
                is_success,
                latency_ms as f64,
                is_rate_limit,
                is_quota,
            )
            .await;

        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(SystemEvent::KeyTested {
                id: key.id.clone(),
                masked_key: key.masked_key.clone(),
                status,
                latency_ms: Some(latency_ms),
                error: error_message.clone(),
            });
        }

        TestResult {
            key_id: key.id.clone(),
            masked_key: key.masked_key.clone(),
            status,
            latency_ms,
            http_status,
            error_message,
        }
    }

    pub async fn test_keys(&self, keys: &[KeyRecord]) -> Vec<TestResult> {
        if keys.is_empty() {
            return Vec::new();
        }

        let total = keys.len();
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(SystemEvent::TestStarted { total_keys: total });
        }

        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let counter = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(total);

        for key in keys {
            let sem = Arc::clone(&semaphore);
            let cnt = Arc::clone(&counter);
            let tx = self.event_tx.clone();
            let key_clone = key.clone();
            let http_client = self.http_client.clone();
            let key_manager = Arc::clone(&self.key_manager);
            let base_url = self.base_url.clone();
            let model = self.model.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.ok();
                let tester = FastTester {
                    http_client,
                    key_manager,
                    event_tx: tx.clone(),
                    concurrency: 1,
                    base_url,
                    model,
                };
                let result = tester.test_single_key(&key_clone).await;
                let current = cnt.fetch_add(1, Ordering::Relaxed) + 1;

                if let Some(ref tx_channel) = tx {
                    let _ = tx_channel.send(SystemEvent::TestProgress {
                        current,
                        total,
                        key_id: result.key_id.clone(),
                        status: result.status,
                        latency_ms: Some(result.latency_ms),
                    });
                }

                result
            }));
        }

        let mut results = Vec::with_capacity(total);
        for handle in handles {
            if let Ok(res) = handle.await {
                results.push(res);
            }
        }

        let active_count = results
            .iter()
            .filter(|r| r.status == KeyStatus::Active)
            .count();
        let failed_count = results.len() - active_count;

        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(SystemEvent::TestCompleted {
                tested: results.len(),
                active: active_count,
                failed: failed_count,
            });
        }

        results
    }

    pub async fn test_all_keys(&self) -> Vec<TestResult> {
        let keys = self.key_manager.get_keys();
        self.test_keys(&keys).await
    }

    pub async fn test_key_by_id(&self, key_id: &str) -> Option<TestResult> {
        let key = self.key_manager.get_key_by_id(key_id)?;
        Some(self.test_single_key(&key).await)
    }
}
