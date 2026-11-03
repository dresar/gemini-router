use axum::Router;
use gemini_router::admin;
use gemini_router::config::AppConfig;
use gemini_router::database;
use gemini_router::gemini::types::StrategyType;
use gemini_router::key_manager::KeyManager;
use gemini_router::proxy;
use gemini_router::rotation::RotationEngine;
use gemini_router::tester::FastTester;
use gemini_router::AppState;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
        "Starting Gemini Key Router backend"
    );

    let pool = database::init_pool(&config.database_url).await?;
    tracing::info!("Database connection pool established");

    database::run_migrations(&pool).await?;
    tracing::info!("Database migrations executed successfully");

    let key_mgr = Arc::new(KeyManager::new(pool.clone()).await?);
    let key_count = key_mgr.get_keys().len();
    tracing::info!(loaded_keys = key_count, "Key manager cache initialized");

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .build()?;

    let strategy = StrategyType::from_str_loose(&config.default_strategy);
    let rotation_engine = Arc::new(RotationEngine::new(
        key_mgr.clone(),
        strategy,
        config.cooldown_rate_limit_secs,
        config.cooldown_quota_hours,
    ));

    let (event_tx, _) = tokio::sync::broadcast::channel(256);

    let tester = Arc::new(FastTester::new(
        http_client.clone(),
        key_mgr.clone(),
        event_tx.clone(),
        config.tester_concurrency,
        config.gemini_base_url.clone(),
    ));

    let state = Arc::new(AppState {
        config: config.clone(),
        db: pool,
        http_client,
        key_manager: key_mgr,
        rotation_engine,
        tester,
        event_tx,
    });

    let app = Router::new()
        .nest("/admin", admin::admin_router(state.clone()))
        .merge(proxy::proxy_router(state.clone()))
        .fallback_service(ServeDir::new("static"))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Gemini Key Router listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}
