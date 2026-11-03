use crate::gemini::types::{KeyRecord, KeyStatus, SystemEvent};
use crate::key_manager::KeyManager;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Semaphore};

pub struct FastTester {
    http_client: reqwest::Client,
    key_manager: Arc<KeyManager>,
    event_tx: broadcast::Sender<SystemEvent>,
    concurrency: usize,
    gemini_base_url: String,
}

impl FastTester {
    pub fn new(
        http_client: reqwest::Client,
        key_manager: Arc<KeyManager>,
        event_tx: broadcast::Sender<SystemEvent>,
        concurrency: usize,
        gemini_base_url: String,
    ) -> Self {
        Self {
            http_client,
            key_manager,
            event_tx,
            concurrency,
            gemini_base_url,
        }
    }

    pub async fn test_key(
        &self,
        record: &KeyRecord,
    ) -> Result<(KeyStatus, u64, Option<String>), String> {
        let start = Instant::now();
        let url = format!(
            "{}/v1beta/models?key={}",
            self.gemini_base_url.trim_end_matches('/'),
            record.api_key
        );

        let res_res = self
            .http_client
            .get(&url)
            .header("x-goog-api-key", &record.api_key)
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        let (status, err_msg) = match res_res {
            Ok(res) => {
                let code = res.status();
                if code.is_success() {
                    (KeyStatus::Active, None)
                } else {
                    let body_text = res.text().await.unwrap_or_default();
                    let upper = body_text.to_uppercase();
                    let s = match code.as_u16() {
                        429 => KeyStatus::RateLimited,
                        403 => {
                            if upper.contains("RESOURCE_EXHAUSTED") || upper.contains("QUOTA") {
                                KeyStatus::QuotaExhausted
                            } else if upper.contains("BILLING") {
                                KeyStatus::BillingError
                            } else {
                                KeyStatus::Invalid
                            }
                        }
                        400 => {
                            if upper.contains("API_KEY_INVALID") || upper.contains("KEY NOT VALID") {
                                KeyStatus::Invalid
                            } else {
                                KeyStatus::TemporaryError
                            }
                        }
                        500..=599 => KeyStatus::TemporaryError,
                        _ => KeyStatus::Invalid,
                    };
                    (s, Some(body_text))
                }
            }
            Err(e) => (KeyStatus::TemporaryError, Some(e.to_string())),
        };

        let _ = self
            .key_manager
            .update_key_status(&record.id, status, err_msg.clone(), None)
            .await;

        let is_success = status == KeyStatus::Active;
        let is_rl = status == KeyStatus::RateLimited;
        let is_qe = status == KeyStatus::QuotaExhausted;
        let _ = self
            .key_manager
            .record_request_outcome(&record.id, is_success, latency_ms as f64, is_rl, is_qe)
            .await;

        let _ = self.event_tx.send(SystemEvent::KeyTested {
            id: record.id.clone(),
            masked_key: record.masked_key.clone(),
            status,
            latency_ms: Some(latency_ms),
            error: err_msg.clone(),
        });

        Ok((status, latency_ms, err_msg))
    }

    pub async fn test_single(
        &self,
        key_id: &str,
    ) -> Result<(KeyStatus, u64, Option<String>), String> {
        let record = self
            .key_manager
            .get_key_by_id(key_id)
            .ok_or_else(|| "Key not found".to_string())?;

        self.test_key(&record).await
    }

    pub fn test_all(self: &Arc<Self>) -> usize {
        let keys = self.key_manager.get_keys();
        let total = keys.len();
        if total == 0 {
            return 0;
        }

        let _ = self.event_tx.send(SystemEvent::TestStarted { total_keys: total });

        let tester = Arc::clone(self);
        tokio::spawn(async move {
            let semaphore = Arc::new(Semaphore::new(tester.concurrency));
            let mut handles = Vec::with_capacity(total);

            for key in keys {
                let sem = Arc::clone(&semaphore);
                let t = Arc::clone(&tester);
                let handle = tokio::spawn(async move {
                    let _permit = sem.acquire().await;
                    t.test_key(&key).await
                });
                handles.push(handle);
            }

            let mut tested = 0;
            let mut active = 0;
            let mut failed = 0;

            for h in handles {
                if let Ok(Ok((status, _, _))) = h.await {
                    tested += 1;
                    if status == KeyStatus::Active {
                        active += 1;
                    } else {
                        failed += 1;
                    }
                }
            }

            let _ = tester.event_tx.send(SystemEvent::TestCompleted {
                tested,
                active,
                failed,
            });
        });

        total
    }
}
