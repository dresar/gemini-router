use crate::config::{hash_key, mask_key};
use crate::gemini::types::{KeyRecord, KeyStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchImportResult {
    pub total: usize,
    pub inserted: usize,
    pub duplicates: usize,
    pub keys: Vec<KeyRecord>,
}

fn parse_sqlite_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(ndt.and_utc());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(ndt.and_utc());
    }
    s.parse::<DateTime<Utc>>().ok()
}

fn map_row_to_key_record(row: &sqlx::sqlite::SqliteRow) -> KeyRecord {
    let status_str: String = row.get("status");
    let last_used_str: Option<String> = row.get("last_used_at");
    let cooldown_str: Option<String> = row.get("cooldown_expires_at");

    KeyRecord {
        id: row.get("id"),
        key_hash: row.get("key_hash"),
        api_key: row.get("api_key"),
        masked_key: row.get("masked_key"),
        alias: row.get("alias"),
        status: KeyStatus::from_str_loose(&status_str),
        total_requests: row.get::<i64, _>("total_requests") as u64,
        successful_requests: row.get::<i64, _>("successful_requests") as u64,
        failed_requests: row.get::<i64, _>("failed_requests") as u64,
        rate_limit_count: row.get::<i64, _>("rate_limit_count") as u64,
        quota_exhausted_count: row.get::<i64, _>("quota_exhausted_count") as u64,
        last_used_at: last_used_str.as_deref().and_then(parse_sqlite_datetime),
        last_error: row.get("last_error"),
        average_latency_ms: row.get("average_latency_ms"),
        cooldown_expires_at: cooldown_str.as_deref().and_then(parse_sqlite_datetime),
        in_flight: 0,
    }
}

pub async fn insert_key(pool: &SqlitePool, key: &KeyRecord) -> Result<(), sqlx::Error> {
    let last_used_rfc = key.last_used_at.map(|t| t.to_rfc3339());
    let cooldown_rfc = key.cooldown_expires_at.map(|t| t.to_rfc3339());

    sqlx::query(
        r#"
        INSERT INTO api_keys (
            id, key_hash, api_key, masked_key, alias, status,
            total_requests, successful_requests, failed_requests,
            rate_limit_count, quota_exhausted_count, last_used_at,
            last_error, average_latency_ms, cooldown_expires_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(&key.id)
    .bind(&key.key_hash)
    .bind(&key.api_key)
    .bind(&key.masked_key)
    .bind(&key.alias)
    .bind(key.status.as_str())
    .bind(key.total_requests as i64)
    .bind(key.successful_requests as i64)
    .bind(key.failed_requests as i64)
    .bind(key.rate_limit_count as i64)
    .bind(key.quota_exhausted_count as i64)
    .bind(last_used_rfc)
    .bind(&key.last_error)
    .bind(key.average_latency_ms)
    .bind(cooldown_rfc)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn batch_import(
    pool: &SqlitePool,
    items: Vec<(String, String)>,
) -> Result<BatchImportResult, sqlx::Error> {
    let total = items.len();
    let mut inserted = 0;
    let mut duplicates = 0;
    let mut inserted_records = Vec::new();

    for (raw_key, alias) in items {
        let trimmed_key = raw_key.trim();
        if trimmed_key.is_empty() {
            continue;
        }

        let k_hash = hash_key(trimmed_key);
        let m_key = mask_key(trimmed_key);
        let key_id = Uuid::new_v4().to_string();
        let alias_val = if alias.trim().is_empty() {
            format!("key-{}", &k_hash[..8.min(k_hash.len())])
        } else {
            alias.trim().to_string()
        };

        let result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO api_keys (
                id, key_hash, api_key, masked_key, alias, status,
                total_requests, successful_requests, failed_requests,
                rate_limit_count, quota_exhausted_count, last_used_at,
                last_error, average_latency_ms, cooldown_expires_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, 'ACTIVE', 0, 0, 0, 0, 0, NULL, NULL, 0.0, NULL, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(&key_id)
        .bind(&k_hash)
        .bind(trimmed_key)
        .bind(&m_key)
        .bind(&alias_val)
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            inserted += 1;
            inserted_records.push(KeyRecord {
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
            });
        } else {
            duplicates += 1;
        }
    }

    Ok(BatchImportResult {
        total,
        inserted,
        duplicates,
        keys: inserted_records,
    })
}

pub async fn get_all_keys(pool: &SqlitePool) -> Result<Vec<KeyRecord>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            id, key_hash, api_key, masked_key, alias, status,
            total_requests, successful_requests, failed_requests,
            rate_limit_count, quota_exhausted_count, last_used_at,
            last_error, average_latency_ms, cooldown_expires_at
        FROM api_keys
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let records = rows.iter().map(map_row_to_key_record).collect();
    Ok(records)
}

pub async fn get_key_by_id(pool: &SqlitePool, id: &str) -> Result<Option<KeyRecord>, sqlx::Error> {
    let row_opt = sqlx::query(
        r#"
        SELECT
            id, key_hash, api_key, masked_key, alias, status,
            total_requests, successful_requests, failed_requests,
            rate_limit_count, quota_exhausted_count, last_used_at,
            last_error, average_latency_ms, cooldown_expires_at
        FROM api_keys
        WHERE id = ?
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row_opt.as_ref().map(map_row_to_key_record))
}

pub async fn update_status(
    pool: &SqlitePool,
    id: &str,
    status: KeyStatus,
    last_error: Option<&str>,
    cooldown_expires_at: Option<DateTime<Utc>>,
) -> Result<bool, sqlx::Error> {
    let cooldown_rfc = cooldown_expires_at.map(|t| t.to_rfc3339());

    let result = sqlx::query(
        r#"
        UPDATE api_keys
        SET status = ?,
            last_error = ?,
            cooldown_expires_at = ?,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(status.as_str())
    .bind(last_error)
    .bind(cooldown_rfc)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn update_stats(
    pool: &SqlitePool,
    id: &str,
    success: bool,
    latency_ms: f64,
    is_rate_limit: bool,
    is_quota_exhausted: bool,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE api_keys
        SET total_requests = total_requests + 1,
            successful_requests = successful_requests + CASE WHEN ? THEN 1 ELSE 0 END,
            failed_requests = failed_requests + CASE WHEN ? THEN 0 ELSE 1 END,
            rate_limit_count = rate_limit_count + CASE WHEN ? THEN 1 ELSE 0 END,
            quota_exhausted_count = quota_exhausted_count + CASE WHEN ? THEN 1 ELSE 0 END,
            average_latency_ms = (average_latency_ms * total_requests + ?) / (total_requests + 1),
            last_used_at = CURRENT_TIMESTAMP,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(success)
    .bind(success)
    .bind(is_rate_limit)
    .bind(is_quota_exhausted)
    .bind(latency_ms)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn update_alias(
    pool: &SqlitePool,
    id: &str,
    alias: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE api_keys
        SET alias = ?,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(alias)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn delete_key(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM api_keys
        WHERE id = ?
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row_opt = sqlx::query(
        r#"
        SELECT value FROM system_settings WHERE key = ?
        "#,
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;

    Ok(row_opt.map(|r| r.get("value")))
}

pub async fn set_setting(
    pool: &SqlitePool,
    key: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO system_settings (key, value, updated_at)
        VALUES (?, ?, CURRENT_TIMESTAMP)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_all_settings(
    pool: &SqlitePool,
) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT key, value FROM system_settings
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut map = std::collections::HashMap::new();
    for row in rows {
        let k: String = row.get("key");
        let v: String = row.get("value");
        map.insert(k, v);
    }
    Ok(map)
}
