use axum::{routing::get, Json, Router};
use gemini_router::config::AppConfig;
use gemini_router::database;
use gemini_router::key_manager::KeyManager;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

async fn health_check() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
    tracing::info!("Shutdown signal received, shutting down gracefully");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,gemini_router=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Arc::new(AppConfig::load());
    tracing::info!(
        host = %config.host,
        port = %config.port,
        db_url = %config.database_url,
        "Starting Gemini Key Router backend core"
    );

    let pool = database::init_pool(&config.database_url).await?;
    tracing::info!("Database connection pool established");

    database::run_migrations(&pool).await?;
    tracing::info!("Database migrations executed successfully");

    let key_mgr = Arc::new(KeyManager::new(pool.clone()).await?);
    let key_count = key_mgr.get_keys().len();
    tracing::info!(loaded_keys = key_count, "Key manager cache initialized");

    let app = Router::new()
        .route("/health", get(health_check));

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Gemini Key Router listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}
