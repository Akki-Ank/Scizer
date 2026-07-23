use std::sync::Arc;

use axum::routing::{get, post};
use axum::{middleware, Router};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::auth;
use crate::config::Config;
use crate::queue::Queue;
use crate::routes;
use crate::storage::Storage;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub queue: Queue,
    pub storage: Arc<Storage>,
    pub config: Arc<Config>,
}

pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route("/v1/jobs/{id}/submit", post(routes::jobs::submit_job))
        .route(
            "/v1/jobs/{id}",
            get(routes::jobs::get_job).delete(routes::jobs::cancel_job),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ));

    Router::new()
        .route("/healthz", get(routes::health::healthz))
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
