use std::sync::Arc;

use sizer_cloud::app::{self, AppState};
use sizer_cloud::{config::Config, db, queue::Queue, storage::Storage};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env()?;
    let db_pool = db::connect(&config.database_url).await?;
    sqlx::migrate!("./migrations").run(&db_pool).await?;

    let queue = Queue::connect(&config.redis_url).await?;
    let storage = Storage::new(&config);
    let listen_addr = config.listen_addr;

    let state = AppState {
        db: db_pool,
        queue,
        storage: Arc::new(storage),
        config: Arc::new(config),
    };

    let router = app::build_router(state);
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    tracing::info!("sizer-cloud-api listening on {listen_addr}");
    axum::serve(listener, router).await?;

    Ok(())
}
