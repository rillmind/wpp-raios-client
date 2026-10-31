use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
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
