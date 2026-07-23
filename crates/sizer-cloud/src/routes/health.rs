use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::app::AppState;

/// Unauthenticated -- deliberately outside the API-key middleware layer
/// (see `app.rs`), since load balancers/uptime checks shouldn't need a key.
pub async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();
    let redis_ok = state.queue.ping().await.is_ok();

    let status = if db_ok && redis_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(serde_json::json!({ "db": db_ok, "redis": redis_ok })),
    )
}
