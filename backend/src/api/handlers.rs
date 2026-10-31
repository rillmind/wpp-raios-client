use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use super::error::{ApiError, ApiResult};
use super::{ApiEvent, AppState};
use crate::model::{ConnState, Message, MessageKind, MsgStatus};
use crate::store::ChatSummary;
use crate::wa::UnreadRef;

const DEFAULT_CHATS: u32 = 100;
const MAX_CHATS: u32 = 500;
const DEFAULT_MESSAGES: u32 = 50;
const MAX_MESSAGES: u32 = 200;
/// Teto ao expandir a página para incluir todas as não lidas.
const MAX_UNREAD_PAGE: u32 = 1000;
const MAX_TEXT_CHARS: usize = 4096;
/// Imagens e figurinhas maiores que isso não são baixadas.
const MAX_MEDIA_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Serialize)]
pub struct StatusResponse {
  state: ConnState,
  connected: bool,
}

pub async fn status(State(st): State<AppState>) -> Json<StatusResponse> {
  let state = st.wa.state();
  Json(StatusResponse {
    state,
    connected: state == ConnState::Connected,
  })
}

#[derive(Serialize)]
pub struct PairingResponse {
  state: ConnState,
  /// SVG do QR, só enquanto espera o pareamento.
  qr_svg: Option<String>,
}

pub async fn pairing(State(st): State<AppState>) -> Response {
  let state = st.wa.state();
  let qr_svg = st.wa.pairing_qr().as_deref().and_then(crate::wa::qr_svg);
  (
    [(header::CACHE_CONTROL, "no-store")],
    Json(PairingResponse { state, qr_svg }),
  )
    .into_response()
}

#[derive(Deserialize)]
pub struct ChatsQuery {
  limit: Option<u32>,
  before_ts: Option<i64>,
}

pub async fn list_chats(
  State(st): State<AppState>,
  Query(q): Query<ChatsQuery>,
) -> ApiResult<Json<Vec<ChatSummary>>> {
  let limit = clamp_limit(q.limit, DEFAULT_CHATS, MAX_CHATS)?;
  let chats = st
    .store
    .run(move |db| db.list_chats(limit, q.before_ts))
    .await?;
  Ok(Json(chats))
}

#[derive(Deserialize)]
pub struct MessagesQuery {
  limit: Option<u32>,
  before: Option<i64>,
}

#[derive(Serialize)]
pub struct MessagesResponse {
  messages: Vec<Message>,
  first_unread_id: Option<String>,
  /// Há mensagens mais antigas no banco local.
  has_more: bool,
}

pub async fn messages(
  State(st): State<AppState>,
  Path(jid): Path<String>,
  Query(q): Query<MessagesQuery>,
) -> ApiResult<Json<MessagesResponse>> {
  validate_jid(&jid)?;
  let limit = clamp_limit(q.limit, DEFAULT_MESSAGES, MAX_MESSAGES)?;
  let resp = st
    .store
    .run(move |db| {
      if db.get_chat(&jid)?.is_none() {
        return Ok(None);
      }
      let first_unread = db.first_unread(&jid)?;
      // Na primeira página, garante que todas as não lidas venham.
      let mut page = limit;
      if q.before.is_none() {
        if let Some(fu) = &first_unread {
          page = page.max(db.count_since(&jid, fu.ts)?.min(MAX_UNREAD_PAGE));
        }
      }
      let mut messages = db.last_messages(&jid, page + 1, q.before)?;
      let has_more = messages.len() > page as usize;
      if has_more {
        messages.remove(0);
      }
      Ok(Some(MessagesResponse {
        messages,
        first_unread_id: first_unread.map(|m| m.id),
        has_more,
      }))
    })
    .await?
    .ok_or(ApiError::NotFound)?;
  Ok(Json(resp))
}

pub async fn mark_read(
  State(st): State<AppState>,
  Path(jid): Path<String>,
) -> ApiResult<Json<ChatSummary>> {
  validate_jid(&jid)?;
  ensure_connected(&st)?;
  let j = jid.clone();
  let unread = st
    .store
    .run(move |db| {
      db.get_chat(&j)?
        .ok_or_else(|| anyhow::anyhow!("chat sumiu"))?;
      db.unread_messages(&j)
    })
    .await
    .map_err(|_| ApiError::NotFound)?;
  let refs: Vec<UnreadRef> = unread
    .into_iter()
    .map(|u| UnreadRef {
      id: u.id,
      sender_jid: u.sender_jid,
    })
    .collect();

  // Primeiro no WhatsApp: se falhar, o banco local continua igual ao celular.
  st.wa
    .mark_read(&jid, &refs)
    .await
    .map_err(ApiError::Upstream)?;

  let j = jid.clone();
  let chat = st
    .store
    .run(move |db| {
      db.mark_chat_read(&j)?;
      db.get_chat(&j)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
  let _ = st.events.send(ApiEvent::ChatUpdated(chat.clone()));
  Ok(Json(chat))
}

#[derive(Deserialize)]
pub struct SendBody {
  text: String,
}

pub async fn send(
  State(st): State<AppState>,
  Path(jid): Path<String>,
  Json(body): Json<SendBody>,
) -> ApiResult<Json<Message>> {
  validate_jid(&jid)?;
  // Enviar para `status@broadcast` publicaria um status; canais são de leitura.
  if jid == "status@broadcast" || jid.ends_with("@newsletter") {
    return Err(ApiError::BadRequest(
      "status e canais são só de leitura".into(),
    ));
  }
  let text = body.text.trim();
  if text.is_empty() {
    return Err(ApiError::BadRequest("texto vazio".into()));
  }
  if text.chars().count() > MAX_TEXT_CHARS {
    return Err(ApiError::BadRequest(format!(
      "texto acima de {MAX_TEXT_CHARS} caracteres"
    )));
  }
  ensure_connected(&st)?;
  if !st.limiter.try_acquire() {
    return Err(ApiError::RateLimited);
  }

  let sent = st
    .wa
    .send_text(&jid, text)
    .await
    .map_err(ApiError::Upstream)?;
  let msg = Message {
    id: sent.id,
    chat_jid: jid.clone(),
    sender_jid: None,
    sender_name: None,
    from_me: true,
    ts: sent.ts,
    kind: MessageKind::Text,
    body: Some(text.to_string()),
    quoted_id: None,
    status: Some(MsgStatus::Sent),
    is_read: true,
    media: None,
  };
  let m = msg.clone();
  let chat = st
    .store
    .run(move |db| {
      db.insert_message(&m)?;
      db.get_chat(&m.chat_jid)
    })
    .await?;
  let _ = st.events.send(ApiEvent::MessageNew(msg.clone()));
  if let Some(chat) = chat {
    let _ = st.events.send(ApiEvent::ChatUpdated(chat));
  }
  Ok(Json(msg))
}

/// Baixa a imagem/figurinha do CDN na hora e repassa; nada é gravado.
pub async fn media(
  State(st): State<AppState>,
  Path((jid, id)): Path<(String, String)>,
) -> ApiResult<Response> {
  validate_jid(&jid)?;
  validate_msg_id(&id)?;
  let media = st
    .store
    .run(move |db| db.get_media(&jid, &id))
    .await?
    .ok_or(ApiError::NotFound)?;
  if media.file_length > MAX_MEDIA_BYTES {
    return Err(ApiError::BadRequest("mídia grande demais".into()));
  }
  ensure_connected(&st)?;
  let bytes = st
    .wa
    .download_media(&media)
    .await
    .map_err(ApiError::Upstream)?;
  Ok(image_response(
    bytes,
    image_type(media.mimetype.as_deref(), media.kind),
  ))
}

/// Foto de perfil (miniatura), buscada na hora. 204 quando não há (ou é privada).
pub async fn avatar(State(st): State<AppState>, Path(jid): Path<String>) -> ApiResult<Response> {
  validate_jid(&jid)?;
  ensure_connected(&st)?;
  match st.wa.profile_picture(&jid).await {
    Ok(Some(bytes)) => Ok(image_response(bytes, "image/jpeg")),
    Ok(None) => Ok(StatusCode::NO_CONTENT.into_response()),
    // Comum (privacidade, contato sem conta); não vale um aviso por contato.
    Err(e) => {
      tracing::debug!(error = %e, "foto de perfil indisponível");
      Ok(StatusCode::NO_CONTENT.into_response())
    }
  }
}

/// `no-store`: o navegador guarda só na memória da aba (ver front), nunca em disco.
fn image_response(bytes: Vec<u8>, content_type: &'static str) -> Response {
  (
    [
      (header::CONTENT_TYPE, content_type),
      (header::CACHE_CONTROL, "no-store"),
      (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ],
    bytes,
  )
    .into_response()
}

/// O mimetype vem de quem mandou a mensagem: só tipos de imagem conhecidos passam.
fn image_type(mimetype: Option<&str>, kind: MessageKind) -> &'static str {
  match mimetype.and_then(|m| m.split(';').next()).map(str::trim) {
    Some("image/jpeg") => "image/jpeg",
    Some("image/png") => "image/png",
    Some("image/webp") => "image/webp",
    Some("image/gif") => "image/gif",
    _ if kind == MessageKind::Sticker => "image/webp",
    _ => "image/jpeg",
  }
}

pub async fn events(
  State(st): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
  // Estado atual primeiro, para o front não depender de outra chamada.
  let initial = ApiEvent::ConnectionState {
    state: st.wa.state(),
  };
  let live = BroadcastStream::new(st.events.subscribe()).map(|item| match item {
    Ok(ev) => to_sse(&ev),
    // O cliente ficou para trás: pede para recarregar do banco.
    Err(BroadcastStreamRecvError::Lagged(n)) => {
      Event::default().event("resync").data(n.to_string())
    }
  });
  let mut shutdown = st.shutdown.clone();
  let stream = stream::once(async move { to_sse(&initial) })
    .chain(live)
    .map(Ok)
    // Sem isso o desligamento gracioso esperaria o navegador fechar o stream.
    .take_until(async move {
      let _ = shutdown.wait_for(|stop| *stop).await;
    });
  Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn to_sse(ev: &ApiEvent) -> Event {
  match serde_json::to_string(ev) {
    Ok(json) => Event::default().event(ev.name()).data(json),
    Err(_) => Event::default().event("resync").data("0"),
  }
}

fn ensure_connected(st: &AppState) -> ApiResult<()> {
  if st.wa.state() == ConnState::Connected {
    Ok(())
  } else {
    Err(ApiError::Disconnected)
  }
}

fn clamp_limit(limit: Option<u32>, default: u32, max: u32) -> ApiResult<u32> {
  match limit {
    None => Ok(default),
    Some(0) => Err(ApiError::BadRequest("limit deve ser > 0".into())),
    Some(n) => Ok(n.min(max)),
  }
}

fn validate_msg_id(id: &str) -> ApiResult<()> {
  if !id.is_empty()
    && id.len() <= 128
    && id
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
  {
    Ok(())
  } else {
    Err(ApiError::BadRequest("id inválido".into()))
  }
}

fn validate_jid(jid: &str) -> ApiResult<()> {
  let ok = jid.len() <= 128
    && jid
      .split_once('@')
      .is_some_and(|(user, server)| !user.is_empty() && !server.is_empty())
    && jid
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | ':'));
  if ok {
    Ok(())
  } else {
    Err(ApiError::BadRequest("jid inválido".into()))
  }
}
