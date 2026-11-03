use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

impl KeyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyStatus::Active => "ACTIVE",
            KeyStatus::Invalid => "INVALID",
            KeyStatus::RateLimited => "RATE_LIMITED",
            KeyStatus::QuotaExhausted => "QUOTA_EXHAUSTED",
            KeyStatus::BillingError => "BILLING_ERROR",
            KeyStatus::TemporaryError => "TEMPORARY_ERROR",
            KeyStatus::Disabled => "DISABLED",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_uppercase().as_str() {
            "ACTIVE" => KeyStatus::Active,
            "INVALID" => KeyStatus::Invalid,
            "RATE_LIMITED" | "RATELIMITED" => KeyStatus::RateLimited,
            "QUOTA_EXHAUSTED" | "QUOTAEXHAUSTED" => KeyStatus::QuotaExhausted,
            "BILLING_ERROR" | "BILLINGERROR" => KeyStatus::BillingError,
            "TEMPORARY_ERROR" | "TEMPORARYERROR" => KeyStatus::TemporaryError,
            "DISABLED" => KeyStatus::Disabled,
            _ => KeyStatus::Active,
        }
    }
}

impl std::fmt::Display for KeyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for KeyStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_str_loose(s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyType {
    RoundRobin,
    LeastRecentlyUsed,
    LowestFailureRate,
    LowestLatency,
    Weighted,
}

impl StrategyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            StrategyType::RoundRobin => "round_robin",
            StrategyType::LeastRecentlyUsed => "least_recently_used",
            StrategyType::LowestFailureRate => "lowest_failure_rate",
            StrategyType::LowestLatency => "lowest_latency",
            StrategyType::Weighted => "weighted",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "round_robin" | "roundrobin" => StrategyType::RoundRobin,
            "least_recently_used" | "lru" => StrategyType::LeastRecentlyUsed,
            "lowest_failure_rate" | "lowestfailure" => StrategyType::LowestFailureRate,
            "lowest_latency" | "lowestlatency" => StrategyType::LowestLatency,
            "weighted" => StrategyType::Weighted,
            _ => StrategyType::RoundRobin,
        }
    }
}

impl Default for StrategyType {
    fn default() -> Self {
        StrategyType::RoundRobin
    }
}

impl std::fmt::Display for StrategyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for StrategyType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_str_loose(s))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRecord {
    pub id: String,
    pub key_hash: String,
    pub api_key: String,
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
    pub in_flight: u32,
}

pub type KeyEntry = KeyRecord;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl From<&KeyRecord> for KeyView {
    fn from(r: &KeyRecord) -> Self {
        Self {
            id: r.id.clone(),
            masked_key: r.masked_key.clone(),
            alias: r.alias.clone(),
            status: r.status,
            total_requests: r.total_requests,
            successful_requests: r.successful_requests,
            failed_requests: r.failed_requests,
            rate_limit_count: r.rate_limit_count,
            quota_exhausted_count: r.quota_exhausted_count,
            last_used_at: r.last_used_at,
            last_error: r.last_error.clone(),
            average_latency_ms: r.average_latency_ms,
            cooldown_expires_at: r.cooldown_expires_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SystemEvent {
    KeyTested {
        id: String,
        masked_key: String,
        status: KeyStatus,
        latency_ms: Option<u64>,
        error: Option<String>,
    },
    TestStarted {
        total_keys: usize,
    },
    TestProgress {
        current: usize,
        total: usize,
        key_id: String,
        status: KeyStatus,
        latency_ms: Option<u64>,
    },
    TestCompleted {
        tested: usize,
        active: usize,
        failed: usize,
    },
    KeyStatusChanged {
        id: String,
        masked_key: String,
        old_status: KeyStatus,
        new_status: KeyStatus,
        reason: Option<String>,
    },
    KeyAdded {
        id: String,
        masked_key: String,
        alias: String,
    },
    KeyDeleted {
        id: String,
    },
    StrategyChanged {
        new_strategy: StrategyType,
    },
    StatsUpdated {
        total_requests: u64,
        successful_requests: u64,
        failed_requests: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<Blob>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_response: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blob {
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateContentRequest {
    pub contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateContentResponse {
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_feedback: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<UsageMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_ratings: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_token_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_token_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_token_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiErrorResponse {
    pub error: GeminiErrorDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiErrorDetail {
    pub code: u32,
    pub message: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<serde_json::Value>>,
}
