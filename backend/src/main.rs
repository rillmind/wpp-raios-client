#![forbid(unsafe_code)]

mod config;
mod ingest;
mod model;
// Consultas de leitura ganham uso na API (Fase 3).
#[allow(dead_code)]
mod store;
// Envio e leitura ganham uso na API (Fase 3).
#[allow(dead_code)]
mod wa;

use anyhow::Result;
use tokio::sync::mpsc;
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

  let ingest = tokio::spawn(ingest::run(store.clone(), rx));

  whatsapp_rust::prelude::shutdown_signal().await;
  tracing::info!(state = ?whatsapp.state(), "encerrando");
  whatsapp.shutdown().await;
  ingest.abort();
  store.run(|db| db.checkpoint()).await?;
  Ok(())
}
