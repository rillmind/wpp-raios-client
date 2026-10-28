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
use tokio::sync::{broadcast, mpsc, oneshot, watch};
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
  let (drain_tx, drain_rx) = oneshot::channel();
  let ingest = tokio::spawn(ingest::run(store.clone(), rx, events.clone(), drain_rx));
  let (stop_tx, stop_rx) = watch::channel(false);

  let app = api::router(
    api::AppState {
      store: store.clone(),
      wa: Arc::clone(&whatsapp),
      events,
      token: config.token.as_str().into(),
      limiter: Arc::new(api::RateLimiter::new(config.send_per_sec)),
      shutdown: stop_rx,
    },
    &config.front_origin,
    config.front_dir.as_deref(),
  )?;
  let listener = tokio::net::TcpListener::bind(config.bind_addr())
    .await
    .with_context(|| format!("abrindo {}", config.bind_addr()))?;
  tracing::info!(addr = %config.bind_addr(), "API ouvindo");
  axum::serve(listener, app)
    .with_graceful_shutdown(async move {
      whatsapp_rust::prelude::shutdown_signal().await;
      let _ = stop_tx.send(true);
    })
    .await?;

  // Ordem: API já fechada → WhatsApp grava a sessão e para de emitir →
  // ingest grava o que restou na fila → checkpoint do WAL.
  tracing::info!(state = ?whatsapp.state(), "encerrando");
  whatsapp.shutdown().await;
  let _ = drain_tx.send(());
  if tokio::time::timeout(std::time::Duration::from_secs(10), ingest)
    .await
    .is_err()
  {
    tracing::warn!("ingest não terminou em 10 s");
  }
  store.run(|db| db.checkpoint()).await?;
  tracing::info!("encerrado");
  Ok(())
}
