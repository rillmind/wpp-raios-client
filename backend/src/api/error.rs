use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
  #[error("não autorizado")]
  Unauthorized,
  #[error("{0}")]
  BadRequest(String),
  #[error("chat não encontrado")]
  NotFound,
  #[error("muitos envios; aguarde um instante")]
  RateLimited,
  #[error("WhatsApp desconectado")]
  Disconnected,
  #[error("falha no WhatsApp: {0}")]
  Upstream(anyhow::Error),
  #[error("erro interno")]
  Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
  fn into_response(self) -> Response {
    let status = match &self {
      Self::Unauthorized => StatusCode::UNAUTHORIZED,
      Self::BadRequest(_) => StatusCode::BAD_REQUEST,
      Self::NotFound => StatusCode::NOT_FOUND,
      Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
      Self::Disconnected => StatusCode::SERVICE_UNAVAILABLE,
      Self::Upstream(_) => StatusCode::BAD_GATEWAY,
      Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    match &self {
      Self::Internal(e) => tracing::error!(error = %e, "erro interno na API"),
      Self::Upstream(e) => tracing::warn!(error = %e, "falha no WhatsApp"),
      _ => {}
    }
    (status, Json(json!({ "error": self.to_string() }))).into_response()
  }
}

pub type ApiResult<T> = Result<T, ApiError>;
