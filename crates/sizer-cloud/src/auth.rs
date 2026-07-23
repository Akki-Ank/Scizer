//! API-key auth (see docs' M6 decisions: per-caller API key, not full user
//! accounts -- simplest thing that lets per-caller quotas/retention work,
//! keys generated/assigned out of band via `sizer-cloud-keygen`, no
//! signup flow). Only a SHA-256 hash of the key is ever stored or
//! compared; the raw key is shown once at creation time and never again.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::app::AppState;
use crate::error::ApiError;

#[derive(Debug, Clone, FromRow)]
pub struct ApiKeyContext {
    pub id: Uuid,
    pub label: String,
    pub quota_bytes_per_day: i64,
}

pub fn hash_key(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

/// A fresh, random API key: `sk_` prefix (so keys are visually
/// identifiable in logs/config) + 32 random bytes hex-encoded.
pub fn generate_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("sk_{}", hex::encode(bytes))
}

pub async fn require_api_key(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let raw_key = req
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let hash = hash_key(raw_key);

    let key = sqlx::query_as::<_, ApiKeyContext>(
        "SELECT id, label, quota_bytes_per_day FROM api_keys WHERE key_hash = $1 AND disabled_at IS NULL",
    )
    .bind(hash)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    req.extensions_mut().insert(key);
    Ok(next.run(req).await)
}
