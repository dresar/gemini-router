pub mod repository;

pub use repository::*;

use crate::config::{hash_key, mask_key};
use crate::gemini::types::{KeyRecord, KeyStatus, KeyView};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct KeyManager {
    pool: SqlitePool,
    keys: RwLock<Vec<KeyRecord>>,
}

impl KeyManager {
    pub async fn new(pool: SqlitePool) -> Result<Self, sqlx::Error> {
        let keys = repository::get_all_keys(&pool).await?;
        Ok(Self {
            pool,
            keys: RwLock::new(keys),
        })
    }

    pub fn get_keys(&self) -> Vec<KeyRecord> {
        self.keys.read().clone()
    }

    pub fn get_key_views(&self) -> Vec<KeyView> {
        self.keys.read().iter().map(KeyView::from).collect()
    }

    pub fn get_key_by_id(&self, id: &str) -> Option<KeyRecord> {
        self.keys.read().iter().find(|k| k.id == id).cloned()
    }

    pub fn get_active_keys(&self) -> Vec<KeyRecord> {
        self.keys
            .read()
            .iter()
            .filter(|k| k.status == KeyStatus::Active)
            .cloned()
            .collect()
    }

    pub async fn add_key(&self, api_key: &str, alias: &str) -> Result<KeyRecord, String> {
        let trimmed_key = api_key.trim();
        if trimmed_key.is_empty() {
            return Err("API key cannot be empty".to_string());
        }

        let k_hash = hash_key(trimmed_key);

        {
            let current = self.keys.read();
            if current.iter().any(|k| k.key_hash == k_hash) {
                return Err("API key already exists".to_string());
            }
        }

        let m_key = mask_key(trimmed_key);
        let key_id = Uuid::new_v4().to_string();
        let alias_val = if alias.trim().is_empty() {
            format!("key-{}", &k_hash[..8.min(k_hash.len())])
        } else {
            alias.trim().to_string()
        };

        let record = KeyRecord {
            id: key_id,
            key_hash: k_hash,
            api_key: trimmed_key.to_string(),
            masked_key: m_key,
            alias: alias_val,
            status: KeyStatus::Active,
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            rate_limit_count: 0,
            quota_exhausted_count: 0,
            last_used_at: None,
            last_error: None,
            average_latency_ms: 0.0,
            cooldown_expires_at: None,
            in_flight: 0,
        };

        repository::insert_key(&self.pool, &record)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        self.keys.write().push(record.clone());
        Ok(record)
    }

    pub async fn batch_import_text(&self, raw_input: &str) -> Result<BatchImportResult, String> {
        let mut items = Vec::new();

        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(raw_input) {
            if let Some(arr) = json_val.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        items.push((s.to_string(), String::new()));
                    } else if let Some(obj) = item.as_object() {
                        let k = obj.get("key").and_then(|v| v.as_str()).unwrap_or_default();
                        let a = obj.get("alias").and_then(|v| v.as_str()).unwrap_or_default();
                        if !k.is_empty() {
                            items.push((k.to_string(), a.to_string()));
                        }
                    }
                }
            } else if let Some(obj) = json_val.as_object() {
                if let Some(keys_field) = obj.get("keys").and_then(|v| v.as_str()) {
                    for line in keys_field.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            items.push((trimmed.to_string(), String::new()));
                        }
                    }
                }
            }
        }

        if items.is_empty() {
            for line in raw_input.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some((k, a)) = trimmed.split_once(',') {
                    items.push((k.trim().to_string(), a.trim().to_string()));
                } else if let Some((k, a)) = trimmed.split_once(':') {
                    items.push((k.trim().to_string(), a.trim().to_string()));
                } else {
                    items.push((trimmed.to_string(), String::new()));
                }
            }
        }

        let res = repository::batch_import(&self.pool, items)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        if !res.keys.is_empty() {
            let mut current = self.keys.write();
            for new_key in &res.keys {
                current.push(new_key.clone());
            }
        }

        Ok(res)
    }

    pub async fn update_key_status(
        &self,
        id: &str,
        status: KeyStatus,
        last_error: Option<String>,
        cooldown_expires_at: Option<DateTime<Utc>>,
    ) -> Result<bool, String> {
        let updated = repository::update_status(
            &self.pool,
            id,
            status,
            last_error.as_deref(),
            cooldown_expires_at,
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

        if updated {
            let mut keys = self.keys.write();
            if let Some(k) = keys.iter_mut().find(|k| k.id == id) {
                k.status = status;
                k.last_error = last_error;
                k.cooldown_expires_at = cooldown_expires_at;
            }
        }

        Ok(updated)
    }

    pub async fn record_request_outcome(
        &self,
        id: &str,
        success: bool,
        latency_ms: f64,
        is_rate_limit: bool,
        is_quota_exhausted: bool,
    ) -> Result<(), String> {
        repository::update_stats(
            &self.pool,
            id,
            success,
            latency_ms,
            is_rate_limit,
            is_quota_exhausted,
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

        let mut keys = self.keys.write();
        if let Some(k) = keys.iter_mut().find(|k| k.id == id) {
            k.total_requests += 1;
            if success {
                k.successful_requests += 1;
            } else {
                k.failed_requests += 1;
            }
            if is_rate_limit {
                k.rate_limit_count += 1;
            }
            if is_quota_exhausted {
                k.quota_exhausted_count += 1;
            }
            k.average_latency_ms = if k.total_requests == 1 {
                latency_ms
            } else {
                (k.average_latency_ms * (k.total_requests - 1) as f64 + latency_ms)
                    / k.total_requests as f64
            };
            k.last_used_at = Some(Utc::now());
        }

        Ok(())
    }

    pub async fn update_alias(&self, id: &str, alias: &str) -> Result<bool, String> {
        let updated = repository::update_alias(&self.pool, id, alias)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        if updated {
            let mut keys = self.keys.write();
            if let Some(k) = keys.iter_mut().find(|k| k.id == id) {
                k.alias = alias.to_string();
            }
        }

        Ok(updated)
    }

    pub async fn delete_key(&self, id: &str) -> Result<bool, String> {
        let deleted = repository::delete_key(&self.pool, id)
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        if deleted {
            let mut keys = self.keys.write();
            keys.retain(|k| k.id != id);
        }

        Ok(deleted)
    }

    pub async fn reload(&self) -> Result<(), sqlx::Error> {
        let fresh_keys = repository::get_all_keys(&self.pool).await?;
        *self.keys.write() = fresh_keys;
        Ok(())
    }
}
