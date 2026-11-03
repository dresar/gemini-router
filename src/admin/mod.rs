pub mod auth;
pub mod handlers;

pub use auth::*;
pub use handlers::*;

use crate::AppState;
use axum::middleware;
use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;

pub fn admin_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/stats", get(get_stats))
        .route("/keys", get(get_keys).post(add_key))
        .route("/keys/import", post(import_keys))
        .route("/keys/:id", delete(delete_key).patch(patch_key))
        .route("/keys/test", post(test_single_key))
        .route("/keys/test-all", post(test_all_keys))
        .route("/rotation", get(get_rotation))
        .route("/events", get(get_events))
        .route("/settings", get(get_settings).patch(patch_settings))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}
