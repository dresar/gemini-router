pub mod handler;

pub use handler::*;

use crate::AppState;
use axum::routing::get;
use axum::Router;
use std::sync::Arc;

pub fn proxy_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/models", get(list_models_v1))
        .route("/v1/models/*path", get(get_model_v1).post(model_action_v1))
        .route("/v1beta/models", get(list_models_v1beta))
        .route(
            "/v1beta/models/*path",
            get(get_model_v1beta).post(model_action_v1beta),
        )
        .with_state(state)
}
