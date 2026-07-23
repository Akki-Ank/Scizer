//! Operator tool for the "API key per caller, assigned out of band, no
//! signup flow" auth model (see docs/ARCHITECTURE.md's M6 decisions).
//! Prints the raw key exactly once -- only its SHA-256 hash is stored.

use sizer_cloud::{auth, config::Config, db};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let mut args = std::env::args().skip(1);
    let label = args.next().ok_or_else(|| {
        anyhow::anyhow!("usage: sizer-cloud-keygen <label> [quota_bytes_per_day]")
    })?;
    let quota: i64 = match args.next() {
        Some(raw) => raw.parse()?,
        None => 5_368_709_120, // 5 GiB/day default
    };

    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let raw_key = auth::generate_key();
    let hash = auth::hash_key(&raw_key);

    sqlx::query("INSERT INTO api_keys (key_hash, label, quota_bytes_per_day) VALUES ($1, $2, $3)")
        .bind(&hash)
        .bind(&label)
        .bind(quota)
        .execute(&pool)
        .await?;

    println!("API key created for {label:?} (quota {quota} bytes/day).");
    println!("Store it now -- it cannot be retrieved again:");
    println!("{raw_key}");

    Ok(())
}
