pub mod admin;
pub mod config;
pub mod database;
pub mod gemini;
pub mod key_manager;
pub mod proxy;
pub mod rotation;
pub mod streaming;
pub mod tester;

use std::sync::Arc;

pub struct AppState {
    pub config: Arc<config::AppConfig>,
    pub db: sqlx::SqlitePool,
    pub http_client: reqwest::Client,
    pub key_manager: Arc<key_manager::KeyManager>,
    pub rotation_engine: Arc<rotation::RotationEngine>,
    pub tester: Arc<tester::FastTester>,
    pub event_tx: tokio::sync::broadcast::Sender<gemini::types::SystemEvent>,
}
