use crate::gemini::types::{KeyRecord, KeyStatus, StrategyType};
use crate::key_manager::KeyManager;
use crate::rotation::strategy::{
    LeastRecentlyUsedStrategy, LowestFailureRateStrategy, LowestLatencyStrategy,
    RoundRobinStrategy, SelectionStrategy, WeightedStrategy,
};
use chrono::Utc;
use parking_lot::RwLock;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailoverError {
    pub status_code: u16,
    pub message: String,
}

impl std::fmt::Display for FailoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status_code, self.message)
    }
}

impl std::error::Error for FailoverError {}

#[derive(Debug, Clone)]
pub enum FailoverAttempt<T> {
    Success(T),
    Retryable { status: u16, message: String },
    Fatal { status: u16, message: String },
}

pub trait IntoFailoverAttempt<T> {
    fn into_attempt(self) -> FailoverAttempt<T>;
}

impl<T> IntoFailoverAttempt<T> for FailoverAttempt<T> {
    fn into_attempt(self) -> FailoverAttempt<T> {
        self
    }
}

impl<T> IntoFailoverAttempt<T> for Result<T, (u16, String)> {
    fn into_attempt(self) -> FailoverAttempt<T> {
        match self {
            Ok(v) => FailoverAttempt::Success(v),
            Err((status, message)) => {
                if status == 429 || status == 503 || status == 500 {
                    FailoverAttempt::Retryable { status, message }
                } else {
                    FailoverAttempt::Fatal { status, message }
                }
            }
        }
    }
}

impl<T> IntoFailoverAttempt<T> for Result<T, (u16, &'static str)> {
    fn into_attempt(self) -> FailoverAttempt<T> {
        match self {
            Ok(v) => FailoverAttempt::Success(v),
            Err((status, message)) => {
                if status == 429 || status == 503 || status == 500 {
                    FailoverAttempt::Retryable {
                        status,
                        message: message.to_string(),
                    }
                } else {
                    FailoverAttempt::Fatal {
                        status,
                        message: message.to_string(),
                    }
                }
            }
        }
    }
}

impl<T> IntoFailoverAttempt<T> for Result<T, FailoverError> {
    fn into_attempt(self) -> FailoverAttempt<T> {
        match self {
            Ok(v) => FailoverAttempt::Success(v),
            Err(e) => {
                if e.status_code == 429 || e.status_code == 503 || e.status_code == 500 {
                    FailoverAttempt::Retryable {
                        status: e.status_code,
                        message: e.message,
                    }
                } else {
                    FailoverAttempt::Fatal {
                        status: e.status_code,
                        message: e.message,
                    }
                }
            }
        }
    }
}

fn extract_retry_after(msg: &str) -> Option<u64> {
    let lower = msg.to_lowercase();
    if let Some(idx) = lower.find("retry-after") {
        let remainder = &msg[idx + 11..];
        let digits: String = remainder
            .chars()
            .skip_while(|c| *c == ':' || *c == ' ' || *c == '=')
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(val) = digits.parse::<u64>() {
            return Some(val);
        }
    }
    if let Some(idx) = lower.find("retry in ") {
        let remainder = &msg[idx + 9..];
        let digits: String = remainder
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(val) = digits.parse::<u64>() {
            return Some(val);
        }
    }
    None
}

pub struct RotationEngine {
    keys: Arc<RwLock<Vec<KeyRecord>>>,
    round_robin: RoundRobinStrategy,
    lru: LeastRecentlyUsedStrategy,
    lowest_failure: LowestFailureRateStrategy,
    lowest_latency: LowestLatencyStrategy,
    weighted: WeightedStrategy,
    key_manager: Option<Arc<KeyManager>>,
}

impl RotationEngine {
    pub fn new(keys: Arc<RwLock<Vec<KeyRecord>>>) -> Self {
        Self {
            keys,
            round_robin: RoundRobinStrategy::new(),
            lru: LeastRecentlyUsedStrategy::new(),
            lowest_failure: LowestFailureRateStrategy::new(),
            lowest_latency: LowestLatencyStrategy::new(),
            weighted: WeightedStrategy::new(),
            key_manager: None,
        }
    }

    pub fn new_with_keys(keys: Vec<KeyRecord>) -> Self {
        Self::new(Arc::new(RwLock::new(keys)))
    }

    pub fn with_key_manager(mut self, key_manager: Arc<KeyManager>) -> Self {
        self.key_manager = Some(key_manager);
        self
    }

    pub fn keys(&self) -> Arc<RwLock<Vec<KeyRecord>>> {
        Arc::clone(&self.keys)
    }

    pub fn get_all_keys(&self) -> Vec<KeyRecord> {
        self.keys.read().clone()
    }

    pub fn get_eligible_keys(&self) -> Vec<KeyRecord> {
        let now = Utc::now();
        let mut keys_guard = self.keys.write();
        for k in keys_guard.iter_mut() {
            if let Some(expires) = k.cooldown_expires_at {
                if expires <= now {
                    k.cooldown_expires_at = None;
                    if k.status == KeyStatus::RateLimited
                        || k.status == KeyStatus::TemporaryError
                        || k.status == KeyStatus::QuotaExhausted
                    {
                        k.status = KeyStatus::Active;
                    }
                }
            }
        }

        keys_guard
            .iter()
            .filter(|k| {
                k.status == KeyStatus::Active
                    && (k.cooldown_expires_at.is_none() || k.cooldown_expires_at.unwrap() <= now)
            })
            .cloned()
            .collect()
    }

    pub fn select_key(&self, strategy: StrategyType) -> Option<KeyRecord> {
        self.select_key_excluding(strategy, &HashSet::new())
    }

    pub fn select_key_excluding(
        &self,
        strategy: StrategyType,
        excluded_ids: &HashSet<String>,
    ) -> Option<KeyRecord> {
        let eligible = self.get_eligible_keys();
        let candidates: Vec<KeyRecord> = eligible
            .into_iter()
            .filter(|k| !excluded_ids.contains(&k.id))
            .collect();

        if candidates.is_empty() {
            return None;
        }

        let selected = match strategy {
            StrategyType::RoundRobin => self.round_robin.select(&candidates),
            StrategyType::LeastRecentlyUsed => self.lru.select(&candidates),
            StrategyType::LowestFailureRate => self.lowest_failure.select(&candidates),
            StrategyType::LowestLatency => self.lowest_latency.select(&candidates),
            StrategyType::Weighted => self.weighted.select(&candidates),
        };

        selected.cloned()
    }

    pub async fn record_result(
        &self,
        key_id: &str,
        success: bool,
        latency_ms: f64,
        error_opt: Option<&str>,
    ) {
        let now = Utc::now();
        let outcome = {
            let mut keys_guard = self.keys.write();
            let key = match keys_guard.iter_mut().find(|k| k.id == key_id) {
                Some(k) => k,
                None => return,
            };

            key.total_requests += 1;
            key.last_used_at = Some(now);

            if key.total_requests == 1 {
                key.average_latency_ms = latency_ms;
            } else {
                key.average_latency_ms = (key.average_latency_ms
                    * (key.total_requests - 1) as f64
                    + latency_ms)
                    / key.total_requests as f64;
            }

            if success {
                key.successful_requests += 1;
                key.last_error = None;
                key.status = KeyStatus::Active;
                key.cooldown_expires_at = None;
                (KeyStatus::Active, None, None, false, false)
            } else {
                key.failed_requests += 1;
                let err_msg = error_opt.unwrap_or("Unknown error");
                key.last_error = Some(err_msg.to_string());
                let last_error = key.last_error.clone();
                let err_lower = err_msg.to_lowercase();

                if err_lower.contains("api_key_invalid")
                    || err_lower.contains("api key not valid")
                    || err_lower.contains("service_blocked")
                    || (err_lower.contains("400") && err_lower.contains("key"))
                {
                    key.status = KeyStatus::Invalid;
                    key.cooldown_expires_at = None;
                    (KeyStatus::Invalid, last_error, None, false, false)
                } else if err_lower.contains("403")
                    || err_lower.contains("billing")
                    || err_lower.contains("permission_denied")
                {
                    key.status = KeyStatus::BillingError;
                    key.cooldown_expires_at = None;
                    (KeyStatus::BillingError, last_error, None, false, false)
                } else if err_lower.contains("429")
                    || err_lower.contains("resource_exhausted")
                    || err_lower.contains("rate limit")
                    || err_lower.contains("quota")
                {
                    if err_lower.contains("per day")
                        || err_lower.contains("daily")
                        || err_lower.contains("per_day")
                        || err_lower.contains("free tier limit")
                        || err_lower.contains("quota_exhausted")
                        || err_lower.contains("quota exceeded")
                    {
                        key.quota_exhausted_count += 1;
                        key.status = KeyStatus::QuotaExhausted;
                        let backoff_pow =
                            key.quota_exhausted_count.saturating_sub(1).min(3) as i32;
                        let backoff_mult = 2f64.powi(backoff_pow);
                        let jitter: f64 = rand::thread_rng().gen_range(0.95..1.05);
                        let cooldown_secs = (43200.0 * backoff_mult * jitter) as i64;
                        let expires = Some(now + chrono::Duration::seconds(cooldown_secs));
                        key.cooldown_expires_at = expires;
                        (KeyStatus::QuotaExhausted, last_error, expires, false, true)
                    } else {
                        key.rate_limit_count += 1;
                        key.status = KeyStatus::RateLimited;
                        let base_secs = extract_retry_after(err_msg).unwrap_or(60) as f64;
                        let backoff_pow =
                            key.rate_limit_count.saturating_sub(1).min(5) as i32;
                        let backoff_mult = 2f64.powi(backoff_pow);
                        let jitter: f64 = rand::thread_rng().gen_range(0.9..1.2);
                        let cooldown_secs =
                            (base_secs * backoff_mult * jitter).min(3600.0) as i64;
                        let expires = Some(now + chrono::Duration::seconds(cooldown_secs));
                        key.cooldown_expires_at = expires;
                        (KeyStatus::RateLimited, last_error, expires, true, false)
                    }
                } else if err_lower.contains("503")
                    || err_lower.contains("500")
                    || err_lower.contains("temporary")
                    || err_lower.contains("unavailable")
                {
                    key.status = KeyStatus::TemporaryError;
                    let jitter: f64 = rand::thread_rng().gen_range(0.9..1.1);
                    let cooldown_secs = (15.0 * jitter) as i64;
                    let expires = Some(now + chrono::Duration::seconds(cooldown_secs));
                    key.cooldown_expires_at = expires;
                    (KeyStatus::TemporaryError, last_error, expires, false, false)
                } else {
                    (key.status, last_error, key.cooldown_expires_at, false, false)
                }
            }
        };

        if let Some(ref km) = self.key_manager {
            let (new_status, last_error, cooldown_expires_at, is_rate_limit, is_quota) = outcome;
            let _ = km
                .update_key_status(key_id, new_status, last_error, cooldown_expires_at)
                .await;
            let _ = km
                .record_request_outcome(
                    key_id,
                    success,
                    latency_ms,
                    is_rate_limit,
                    is_quota,
                )
                .await;
        }
    }

    pub async fn execute_with_failover<F, Fut, T, R>(
        &self,
        strategy: StrategyType,
        retry_budget: usize,
        mut req_fn: F,
    ) -> Result<T, FailoverError>
    where
        F: FnMut(KeyRecord) -> Fut,
        Fut: std::future::Future<Output = R>,
        R: IntoFailoverAttempt<T>,
    {
        let initial_eligible = self.get_eligible_keys();
        if initial_eligible.is_empty() {
            return Err(FailoverError {
                status_code: 503,
                message: "No eligible API keys available (all keys are exhausted, cooling down, or invalid)".to_string(),
            });
        }

        let max_attempts = initial_eligible.len().min(retry_budget).max(1);
        let mut tried_keys = HashSet::new();
        let mut last_error_msg = "Unknown error during failover execution".to_string();

        for _ in 0..max_attempts {
            let key = match self.select_key_excluding(strategy, &tried_keys) {
                Some(k) => k,
                None => break,
            };
            tried_keys.insert(key.id.clone());

            let start = std::time::Instant::now();
            let attempt_result = req_fn(key.clone()).await.into_attempt();
            let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

            match attempt_result {
                FailoverAttempt::Success(val) => {
                    self.record_result(&key.id, true, latency_ms, None).await;
                    return Ok(val);
                }
                FailoverAttempt::Retryable { status, message } => {
                    last_error_msg = format!("HTTP {}: {}", status, message);
                    let err_payload = format!("{}: {}", status, message);
                    self.record_result(&key.id, false, latency_ms, Some(&err_payload)).await;
                    continue;
                }
                FailoverAttempt::Fatal { status, message } => {
                    let err_payload = format!("{}: {}", status, message);
                    self.record_result(&key.id, false, latency_ms, Some(&err_payload)).await;
                    return Err(FailoverError {
                        status_code: status,
                        message,
                    });
                }
            }
        }

        Err(FailoverError {
            status_code: 503,
            message: format!(
                "All eligible Gemini keys failed or retry budget exhausted. Last error: {}",
                last_error_msg
            ),
        })
    }
}
