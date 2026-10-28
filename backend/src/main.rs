#![forbid(unsafe_code)]

mod api;
mod config;
mod ingest;
mod model;
mod store;
mod wa;

use anyhow::Result;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::{broadcast, mpsc};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
    .init();

  let config = config::Config::load()?;
  tracing::info!(
    addr = %config.bind_addr(),
    session_db = %config.session_db().display(),
    app_db = %config.app_db().display(),
    front_origin = %config.front_origin,
    send_per_sec = config.send_per_sec,
    "backend iniciado"
  );

  let store = store::Store::new(store::Db::open(&config.app_db())?);

  let (tx, rx) = mpsc::channel(1024);
  let whatsapp = wa::start(
    wa::WaOptions {
      session_db: config.session_db(),
      history_msgs_per_chat: config.history_msgs_per_chat,
    },
    tx,
  )
  .await?;

  let (events, _) = broadcast::channel(1024);
  let ingest = tokio::spawn(ingest::run(store.clone(), rx, events.clone()));

  let app = api::router(
    api::AppState {
      store: store.clone(),
      wa: Arc::clone(&whatsapp),
      events,
      token: config.token.as_str().into(),
      limiter: Arc::new(api::RateLimiter::new(config.send_per_sec)),
    },
    &config.front_origin,
  )?;
  let listener = tokio::net::TcpListener::bind(config.bind_addr())
    .await
    .with_context(|| format!("abrindo {}", config.bind_addr()))?;
  tracing::info!(addr = %config.bind_addr(), "API ouvindo");
  axum::serve(listener, app)
    .with_graceful_shutdown(whatsapp_rust::prelude::shutdown_signal())
    .await?;

  tracing::info!(state = ?whatsapp.state(), "encerrando");
  whatsapp.shutdown().await;
  ingest.abort();
  store.run(|db| db.checkpoint()).await?;
  Ok(())
}
