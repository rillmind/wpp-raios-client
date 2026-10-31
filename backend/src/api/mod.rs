//! API local (axum) em 127.0.0.1, com token estático.

mod error;
mod events;
mod handlers;
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use tokio::sync::{broadcast, watch};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

pub use error::ApiError;
pub use events::ApiEvent;

use crate::store::Store;
use crate::wa::WhatsApp;

#[derive(Clone)]
pub struct AppState {
  pub store: Store,
  pub wa: Arc<dyn WhatsApp>,
  pub events: broadcast::Sender<ApiEvent>,
  pub token: Arc<str>,
  pub limiter: Arc<RateLimiter>,
  /// Vira `true` no desligamento; encerra os streams SSE abertos.
  pub shutdown: watch::Receiver<bool>,
}

/// `front_dir`: build do front (`frontend/dist`) servido na mesma origem.
pub fn router(
  state: AppState,
  front_origin: &str,
  front_dir: Option<&std::path::Path>,
) -> anyhow::Result<Router> {
  let cors = CorsLayer::new()
    .allow_origin(HeaderValue::from_str(front_origin)?)
    .allow_methods([Method::GET, Method::POST])
    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

  let api = Router::new()
    .route("/api/status", get(handlers::status))
    .route("/api/chats", get(handlers::list_chats))
    .route("/api/chats/{jid}/messages", get(handlers::messages))
    .route("/api/chats/{jid}/read", post(handlers::mark_read))
    .route("/api/chats/{jid}/send", post(handlers::send))
    .route("/api/chats/{jid}/avatar", get(handlers::avatar))
    .route("/api/chats/{jid}/messages/{id}/media", get(handlers::media))
    .route("/api/events", get(handlers::events))
    .route_layer(middleware::from_fn_with_state(state.clone(), auth))
    // Fora da autenticação: só diz que o processo responde (healthcheck do container).
    .route("/healthz", get(|| async { "ok" }))
    // Fora da autenticação: no primeiro uso a pessoa ainda não tem o token à mão.
    // Só devolve o QR enquanto não há celular pareado.
    .route("/api/pairing", get(handlers::pairing))
    .with_state(state);

  let app = match front_dir {
    Some(dir) => {
      api.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html"))))
    }
    None => api,
  };
  Ok(app.layer(cors))
}

/// `Authorization: Bearer <token>`. O `EventSource` do navegador não envia
/// cabeçalhos, então `/api/events` aceita também `?token=`.
async fn auth(
  State(state): State<AppState>,
  req: Request,
  next: Next,
) -> Result<Response, ApiError> {
  let header_token = req
    .headers()
    .get(header::AUTHORIZATION)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.strip_prefix("Bearer "));
  let query_token = (req.uri().path() == "/api/events")
    .then(|| req.uri().query())
    .flatten()
    .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")));
  match header_token.or(query_token) {
    Some(t) if constant_time_eq(t.as_bytes(), state.token.as_bytes()) => Ok(next.run(req).await),
    _ => Err(ApiError::Unauthorized),
  }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
  a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Janela fixa de 1 s: no máximo `per_sec` envios por janela.
pub struct RateLimiter {
  per_sec: u32,
  window: Mutex<(Instant, u32)>,
}

impl RateLimiter {
  pub fn new(per_sec: u32) -> Self {
    Self {
      per_sec: per_sec.max(1),
      window: Mutex::new((Instant::now(), 0)),
    }
  }

  pub fn try_acquire(&self) -> bool {
    let Ok(mut w) = self.window.lock() else {
      return false;
    };
    let now = Instant::now();
    if now.duration_since(w.0) >= Duration::from_secs(1) {
      *w = (now, 0);
    }
    if w.1 < self.per_sec {
      w.1 += 1;
      true
    } else {
      false
    }
  }
}
