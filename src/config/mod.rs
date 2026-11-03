use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub admin_key: String,
    pub default_model: String,
    pub default_strategy: String,
    pub tester_concurrency: usize,
    pub request_timeout_secs: u64,
    pub retry_budget: u32,
    pub cooldown_rate_limit_secs: u64,
    pub cooldown_quota_hours: u64,
    pub gemini_base_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            database_url: "sqlite:data/gemini_router.db".to_string(),
            admin_key: "gemini-router-admin-secret-2026".to_string(),
            default_model: "gemini-2.5-flash".to_string(),
            default_strategy: "round_robin".to_string(),
            tester_concurrency: 10,
            request_timeout_secs: 30,
            retry_budget: 3,
            cooldown_rate_limit_secs: 60,
            cooldown_quota_hours: 12,
            gemini_base_url: "https://generativelanguage.googleapis.com".to_string(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        dotenvy::dotenv().ok();

        let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(8080);
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite:data/gemini_router.db".to_string());
        let admin_key = std::env::var("ADMIN_KEY")
            .unwrap_or_else(|_| "gemini-router-admin-secret-2026".to_string());
        let default_model = std::env::var("DEFAULT_MODEL")
            .unwrap_or_else(|_| "gemini-2.5-flash".to_string());
        let default_strategy = std::env::var("DEFAULT_STRATEGY")
            .unwrap_or_else(|_| "round_robin".to_string());
        let tester_concurrency = std::env::var("TESTER_CONCURRENCY")
            .ok()
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(10);
        let request_timeout_secs = std::env::var("REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(30);
        let retry_budget = std::env::var("RETRY_BUDGET")
            .ok()
            .and_then(|r| r.parse::<u32>().ok())
            .unwrap_or(3);
        let cooldown_rate_limit_secs = std::env::var("COOLDOWN_RATE_LIMIT_SECS")
            .ok()
            .and_then(|c| c.parse::<u64>().ok())
            .unwrap_or(60);
        let cooldown_quota_hours = std::env::var("COOLDOWN_QUOTA_HOURS")
            .ok()
            .and_then(|c| c.parse::<u64>().ok())
            .unwrap_or(12);
        let gemini_base_url = std::env::var("GEMINI_BASE_URL")
            .unwrap_or_else(|_| "https://generativelanguage.googleapis.com".to_string());

        Self {
            host,
            port,
            database_url,
            admin_key,
            default_model,
            default_strategy,
            tester_concurrency,
            request_timeout_secs,
            retry_budget,
            cooldown_rate_limit_secs,
            cooldown_quota_hours,
            gemini_base_url,
        }
    }
}

pub fn mask_key(key: &str) -> String {
    let char_count = key.chars().count();
    if char_count <= 8 {
        return "***".to_string();
    }
    let prefix: String = key.chars().take(6).collect();
    let suffix: String = key.chars().skip(char_count.saturating_sub(4)).collect();
    format!("{prefix}...{suffix}")
}

pub fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}
