use crate::gemini::types::{KeyRecord, KeyStatus, StrategyType};
use crate::key_manager::KeyManager;
use chrono::{Duration, Utc};
use parking_lot::RwLock;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationInfo {
    pub strategy: StrategyType,
    pub total_rotations: u64,
    pub failovers: u64,
    pub in_flight: u32,
    pub last_used_key: Option<String>,
}

pub struct RotationEngine {
    key_manager: Arc<KeyManager>,
    strategy: RwLock<StrategyType>,
    rr_index: AtomicUsize,
    total_rotations: AtomicU64,
    total_failovers: AtomicU64,
    last_used_key: RwLock<Option<String>>,
    cooldown_rate_limit_secs: u64,
    cooldown_quota_hours: u64,
}

impl RotationEngine {
    pub fn new(
        key_manager: Arc<KeyManager>,
        strategy: StrategyType,
        cooldown_rate_limit_secs: u64,
        cooldown_quota_hours: u64,
    ) -> Self {
        Self {
            key_manager,
            strategy: RwLock::new(strategy),
            rr_index: AtomicUsize::new(0),
            total_rotations: AtomicU64::new(0),
            total_failovers: AtomicU64::new(0),
            last_used_key: RwLock::new(None),
            cooldown_rate_limit_secs,
            cooldown_quota_hours,
        }
    }

    pub fn get_strategy(&self) -> StrategyType {
        *self.strategy.read()
    }

    pub fn set_strategy(&self, strategy: StrategyType) {
        *self.strategy.write() = strategy;
    }

    pub fn get_info(&self) -> RotationInfo {
        let in_flight = self
            .key_manager
            .get_keys()
            .iter()
            .map(|k| k.in_flight)
            .sum();

        RotationInfo {
            strategy: self.get_strategy(),
            total_rotations: self.total_rotations.load(Ordering::Relaxed),
            failovers: self.total_failovers.load(Ordering::Relaxed),
            in_flight,
            last_used_key: self.last_used_key.read().clone(),
        }
    }

    pub fn select_key(&self) -> Result<KeyRecord, String> {
        let now = Utc::now();
        let all_keys = self.key_manager.get_active_keys();

        let eligible: Vec<KeyRecord> = all_keys
            .into_iter()
            .filter(|k| {
                if let Some(exp) = k.cooldown_expires_at {
                    exp <= now
                } else {
                    true
                }
            })
            .collect();

        if eligible.is_empty() {
            return Err("No active API keys available for rotation".to_string());
        }

        let strategy = self.get_strategy();
        let selected = match strategy {
            StrategyType::RoundRobin => {
                let idx = self.rr_index.fetch_add(1, Ordering::Relaxed) % eligible.len();
                eligible[idx].clone()
            }
            StrategyType::LeastRecentlyUsed => {
                let mut sorted = eligible;
                sorted.sort_by(|a, b| match (a.last_used_at, b.last_used_at) {
                    (None, None) => std::cmp::Ordering::Equal,
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (Some(t1), Some(t2)) => t1.cmp(&t2),
                });
                sorted[0].clone()
            }
            StrategyType::LowestFailureRate => {
                let mut sorted = eligible;
                sorted.sort_by(|a, b| {
                    let rate_a = if a.total_requests == 0 {
                        0.0
                    } else {
                        a.failed_requests as f64 / a.total_requests as f64
                    };
                    let rate_b = if b.total_requests == 0 {
                        0.0
                    } else {
                        b.failed_requests as f64 / b.total_requests as f64
                    };
                    rate_a
                        .partial_cmp(&rate_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                sorted[0].clone()
            }
            StrategyType::LowestLatency => {
                let mut sorted = eligible;
                sorted.sort_by(|a, b| {
                    a.average_latency_ms
                        .partial_cmp(&b.average_latency_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                sorted[0].clone()
            }
            StrategyType::Weighted => {
                let mut rng = rand::thread_rng();
                eligible
                    .choose(&mut rng)
                    .cloned()
                    .unwrap_or_else(|| eligible[0].clone())
            }
        };

        *self.last_used_key.write() = Some(selected.masked_key.clone());
        self.total_rotations.fetch_add(1, Ordering::Relaxed);

        Ok(selected)
    }

    pub async fn record_failover(
        &self,
        key_id: &str,
        status: KeyStatus,
        error_msg: Option<String>,
    ) {
        self.total_failovers.fetch_add(1, Ordering::Relaxed);

        let cooldown = match status {
            KeyStatus::RateLimited => {
                Some(Utc::now() + Duration::seconds(self.cooldown_rate_limit_secs as i64))
            }
            KeyStatus::QuotaExhausted | KeyStatus::BillingError => {
                Some(Utc::now() + Duration::hours(self.cooldown_quota_hours as i64))
            }
            KeyStatus::TemporaryError => {
                Some(Utc::now() + Duration::seconds(self.cooldown_rate_limit_secs as i64))
            }
            _ => None,
        };

        let _ = self
            .key_manager
            .update_key_status(key_id, status, error_msg, cooldown)
            .await;
    }
}
