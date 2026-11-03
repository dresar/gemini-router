use crate::gemini::types::KeyRecord;
use chrono::Utc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub trait SelectionStrategy: Send + Sync {
    fn select<'a>(&self, keys: &'a [KeyRecord]) -> Option<&'a KeyRecord>;
}

pub struct RoundRobinStrategy {
    counter: AtomicUsize,
}

impl RoundRobinStrategy {
    pub fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }

    pub fn with_initial_counter(val: usize) -> Self {
        Self {
            counter: AtomicUsize::new(val),
        }
    }

    pub fn current_index(&self) -> usize {
        self.counter.load(Ordering::Relaxed)
    }
}

impl Default for RoundRobinStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionStrategy for RoundRobinStrategy {
    fn select<'a>(&self, keys: &'a [KeyRecord]) -> Option<&'a KeyRecord> {
        if keys.is_empty() {
            return None;
        }
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % keys.len();
        keys.get(idx)
    }
}

pub struct LeastRecentlyUsedStrategy;

impl LeastRecentlyUsedStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LeastRecentlyUsedStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionStrategy for LeastRecentlyUsedStrategy {
    fn select<'a>(&self, keys: &'a [KeyRecord]) -> Option<&'a KeyRecord> {
        if keys.is_empty() {
            return None;
        }
        keys.iter().min_by(|a, b| match (a.last_used_at, b.last_used_at) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(ta), Some(tb)) => ta.cmp(&tb),
        })
    }
}

pub struct LowestFailureRateStrategy;

impl LowestFailureRateStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LowestFailureRateStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionStrategy for LowestFailureRateStrategy {
    fn select<'a>(&self, keys: &'a [KeyRecord]) -> Option<&'a KeyRecord> {
        if keys.is_empty() {
            return None;
        }
        keys.iter().min_by(|a, b| {
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
                .then_with(|| a.failed_requests.cmp(&b.failed_requests))
                .then_with(|| {
                    a.average_latency_ms
                        .partial_cmp(&b.average_latency_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
    }
}

pub struct LowestLatencyStrategy;

impl LowestLatencyStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LowestLatencyStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionStrategy for LowestLatencyStrategy {
    fn select<'a>(&self, keys: &'a [KeyRecord]) -> Option<&'a KeyRecord> {
        if keys.is_empty() {
            return None;
        }
        keys.iter().min_by(|a, b| {
            a.average_latency_ms
                .partial_cmp(&b.average_latency_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.failed_requests.cmp(&b.failed_requests))
        })
    }
}

pub struct WeightedStrategy;

impl WeightedStrategy {
    pub fn new() -> Self {
        Self
    }

    pub fn calculate_score(key: &KeyRecord, now: chrono::DateTime<Utc>) -> f64 {
        let latency_factor = 1.0 / (1.0 + key.average_latency_ms.max(0.0) / 100.0);
        let failure_rate = if key.total_requests == 0 {
            0.0
        } else {
            (key.failed_requests as f64 / key.total_requests as f64).clamp(0.0, 1.0)
        };
        let success_factor = 1.0 - failure_rate;
        let idle_secs = match key.last_used_at {
            None => 3600.0,
            Some(t) => (now - t).num_milliseconds().max(0) as f64 / 1000.0,
        };
        let idle_factor = 1.0 + (idle_secs / 60.0).min(10.0);
        latency_factor * (0.1 + 0.9 * success_factor) * idle_factor
    }
}

impl Default for WeightedStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionStrategy for WeightedStrategy {
    fn select<'a>(&self, keys: &'a [KeyRecord]) -> Option<&'a KeyRecord> {
        if keys.is_empty() {
            return None;
        }
        let now = Utc::now();
        keys.iter().max_by(|a, b| {
            let score_a = Self::calculate_score(a, now);
            let score_b = Self::calculate_score(b, now);
            score_a
                .partial_cmp(&score_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}
