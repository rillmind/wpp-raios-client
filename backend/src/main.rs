#![forbid(unsafe_code)]

mod config;

use anyhow::Result;
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

  tokio::signal::ctrl_c().await?;
  tracing::info!("encerrando");
  Ok(())
}
