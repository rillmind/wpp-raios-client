use std::convert::Infallible;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use super::error::{ApiError, ApiResult};
use super::{ApiEvent, AppState};
use crate::model::{ChatAction, ConnState, Message, MessageAction, MessageKind, MsgStatus, Quote};
use crate::store::ChatSummary;
use crate::wa::{OutgoingMedia, UnreadRef};

const DEFAULT_CHATS: u32 = 100;
const MAX_CHATS: u32 = 500;
const DEFAULT_MESSAGES: u32 = 50;
const MAX_MESSAGES: u32 = 200;
/// Teto ao expandir a página para incluir todas as não lidas.
const MAX_UNREAD_PAGE: u32 = 1000;
const MAX_TEXT_CHARS: usize = 4096;
/// Mídias maiores que isso não são baixadas (o arquivo passa inteiro pela memória).
const MAX_MEDIA_BYTES: u64 = 32 * 1024 * 1024;
const MAX_VIDEO_BYTES: u64 = 100 * 1024 * 1024;
/// Maior arquivo aceito para envio (o mesmo limite do WhatsApp para mídia).
pub const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
const MAX_FILE_NAME_CHARS: usize = 200;

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
      let more_here = messages.len() > page as usize;
      if more_here {
        messages.remove(0);
      }
      // Sem mais aqui, o "carregar mensagens" ainda pode pedir ao celular, desde que haja
      // alguma mensagem guardada (mesmo escondida) para servir de ponto de partida.
      let has_more = more_here || db.oldest_message(&jid)?.is_some();
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

#[derive(Deserialize)]
pub struct LoadOlderBody {
  /// Mensagens antes deste instante (a mais antiga na tela); sem ele, as mais novas.
  before: Option<i64>,
  count: Option<u32>,
}

#[derive(Serialize)]
pub struct LoadOlderResponse {
  /// O que já estava guardado.
  messages: Vec<Message>,
  /// Faltou: o resto foi pedido ao celular e chega pelo evento `history.loaded`.
  requested: bool,
}

const DEFAULT_LOAD_OLDER: u32 = 20;
const MAX_LOAD_OLDER: u32 = 50;

/// "Carregar mensagens": devolve as guardadas e, se faltar, pede as anteriores ao celular.
pub async fn load_older(
  State(st): State<AppState>,
  Path(jid): Path<String>,
  Json(body): Json<LoadOlderBody>,
) -> ApiResult<Json<LoadOlderResponse>> {
  validate_jid(&jid)?;
  let count = clamp_limit(body.count, DEFAULT_LOAD_OLDER, MAX_LOAD_OLDER)?;
  let j = jid.clone();
  let (messages, anchor) = st
    .store
    .run(move |db| {
      if db.get_chat(&j)?.is_none() {
        return Ok(None);
      }
      // O primeiro "carregar" mostra (de vez) a âncora guardada da sincronização.
      db.reveal_hidden(&j)?;
      Ok(Some((
        db.last_messages(&j, count, body.before)?,
        db.oldest_message(&j)?,
      )))
    })
    .await?
    .ok_or(ApiError::NotFound)?;
  let missing = count.saturating_sub(messages.len() as u32);
  // Sem nenhuma mensagem guardada não há de onde partir o pedido ao celular.
  let requested = match (&anchor, missing) {
    (Some(anchor), n) if n > 0 && !is_read_only(&jid) => {
      ensure_connected(&st)?;
      st.wa
        .request_history(anchor, n)
        .await
        .map_err(ApiError::Upstream)?;
      true
    }
    _ => false,
  };
  Ok(Json(LoadOlderResponse {
    messages,
    requested,
  }))
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

/// Conversa individual com uma pessoa (remetente ou marcada num grupo): a que já existe
/// ou uma vazia, que passa a existir na primeira mensagem.
pub async fn person_chat(
  State(st): State<AppState>,
  Path(jid): Path<String>,
) -> ApiResult<Json<ChatSummary>> {
  validate_jid(&jid)?;
  if !(jid.ends_with("@lid") || jid.ends_with("@s.whatsapp.net")) {
    return Err(ApiError::BadRequest("não é uma pessoa".into()));
  }
  Ok(Json(
    st.store.run(move |db| db.chat_for_person(&jid)).await?,
  ))
}

/// Conversa com um número de telefone escrito numa mensagem (só dígitos, com DDI): confere
/// no WhatsApp se ele tem conta e devolve a conversa como em [`person_chat`].
pub async fn phone_chat(
  State(st): State<AppState>,
  Path(digits): Path<String>,
) -> ApiResult<Json<ChatSummary>> {
  if !(8..=15).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
    return Err(ApiError::BadRequest("número inválido".into()));
  }
  ensure_connected(&st)?;
  let (pn, lid) = st
    .wa
    .lookup_phone(&digits)
    .await
    .map_err(ApiError::Upstream)?
    .ok_or_else(|| ApiError::BadRequest("esse número não tem WhatsApp".into()))?;
  let chat = st
    .store
    .run(move |db| {
      if let Some(lid) = &lid {
        db.set_lid_map(lid, &pn)?;
      }
      db.chat_for_person(&pn)
    })
    .await?;
  Ok(Json(chat))
}

#[derive(Deserialize)]
pub struct ActionBody {
  /// `archive`, `unarchive`, `pin`, `unpin`, `mute`, `unmute` ou `unread`.
  action: String,
  /// Só para `mute`: por quantas horas (sem isso, para sempre).
  hours: Option<u32>,
}

/// Arquiva, fixa, silencia ou marca como não lida, no WhatsApp e depois no banco.
pub async fn chat_action(
  State(st): State<AppState>,
  Path(jid): Path<String>,
  Json(body): Json<ActionBody>,
) -> ApiResult<Json<ChatSummary>> {
  validate_jid(&jid)?;
  let action = match (body.action.as_str(), body.hours) {
    ("archive", _) => ChatAction::Archive(true),
    ("unarchive", _) => ChatAction::Archive(false),
    ("pin", _) => ChatAction::Pin(true),
    ("unpin", _) => ChatAction::Pin(false),
    ("mute", None) => ChatAction::Mute(None),
    ("mute", Some(h @ 1..=8760)) => ChatAction::Mute(Some(now_ms() + i64::from(h) * 3_600_000)),
    ("mute", Some(_)) => return Err(ApiError::BadRequest("hours entre 1 e 8760".into())),
    ("unmute", _) => ChatAction::Unmute,
    ("unread", _) => ChatAction::MarkUnread,
    _ => return Err(ApiError::BadRequest("ação desconhecida".into())),
  };
  let j = jid.clone();
  if st.store.run(move |db| db.get_chat(&j)).await?.is_none() {
    return Err(ApiError::NotFound);
  }
  ensure_connected(&st)?;
  // Primeiro no WhatsApp: se falhar, o banco local continua igual ao celular.
  st.wa
    .chat_action(&jid, action)
    .await
    .map_err(ApiError::Upstream)?;
  let chat = st
    .store
    .run(move |db| {
      db.apply_action(&jid, action)?;
      db.get_chat(&jid)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
  let _ = st.events.send(ApiEvent::ChatUpdated(chat.clone()));
  Ok(Json(chat))
}

fn now_ms() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis() as i64)
    .unwrap_or(0)
}

#[derive(Deserialize)]
pub struct SendBody {
  text: String,
  /// Id da mensagem desta conversa que está sendo respondida (citada).
  reply_to: Option<String>,
}

pub async fn send(
  State(st): State<AppState>,
  Path(jid): Path<String>,
  Json(body): Json<SendBody>,
) -> ApiResult<Json<Message>> {
  validate_jid(&jid)?;
  // Enviar para `status@broadcast` publicaria um status; canais são de leitura.
  if is_read_only(&jid) {
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
  let quote = match body.reply_to.clone() {
    None => None,
    Some(id) => {
      validate_msg_id(&id)?;
      let j = jid.clone();
      let quoted = st
        .store
        .run(move |db| db.get_message(&j, &id))
        .await?
        .ok_or(ApiError::NotFound)?;
      Some(Quote {
        text: crate::store::preview(&quoted),
        id: quoted.id,
        from_me: quoted.from_me,
        sender_jid: quoted.sender_jid,
      })
    }
  };
  ensure_connected(&st)?;
  if !st.limiter.try_acquire() {
    return Err(ApiError::RateLimited);
  }

  let sent = st
    .wa
    .send_text(&jid, text, quote.as_ref())
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
    quoted_id: quote.map(|q| q.id),
    status: Some(MsgStatus::Sent),
    is_read: true,
    media: None,
    starred: false,
    pinned: false,
    mentions: vec![],
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

/// Baixa a imagem, figurinha, vídeo ou áudio do CDN na hora e repassa; nada é gravado.
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
  let max = if media.kind == MessageKind::Video {
    MAX_VIDEO_BYTES
  } else {
    MAX_MEDIA_BYTES
  };
  if media.file_length > max {
    return Err(ApiError::BadRequest("mídia grande demais".into()));
  }
  ensure_connected(&st)?;
  let bytes = st
    .wa
    .download_media(&media)
    .await
    .map_err(ApiError::Upstream)?;
  if media.kind == MessageKind::Doc {
    return Ok(attachment_response(bytes, media.file_name.as_deref()));
  }
  Ok(media_response(
    bytes,
    media_type(media.mimetype.as_deref(), media.kind),
  ))
}

/// Foto de perfil (miniatura), buscada na hora. 204 quando não há (ou é privada).
pub async fn avatar(State(st): State<AppState>, Path(jid): Path<String>) -> ApiResult<Response> {
  validate_jid(&jid)?;
  ensure_connected(&st)?;
  match st.wa.profile_picture(&jid).await {
    Ok(Some(bytes)) => Ok(media_response(bytes, "image/jpeg")),
    Ok(None) => Ok(StatusCode::NO_CONTENT.into_response()),
    // Comum (privacidade, contato sem conta); não vale um aviso por contato.
    Err(e) => {
      tracing::debug!(error = %e, "foto de perfil indisponível");
      Ok(StatusCode::NO_CONTENT.into_response())
    }
  }
}

/// `no-store`: o navegador guarda só na memória da aba (ver front), nunca em disco.
fn media_response(bytes: Vec<u8>, content_type: &'static str) -> Response {
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

/// Documento: sempre como download (`attachment`, octet-stream), nunca aberto no
/// navegador, que é como um HTML ou SVG mandado por alguém conseguiria rodar código.
fn attachment_response(bytes: Vec<u8>, file_name: Option<&str>) -> Response {
  let name = file_name.unwrap_or("documento");
  let disposition = format!("attachment; filename*=UTF-8''{}", percent_encode(name));
  (
    [
      (header::CONTENT_TYPE, "application/octet-stream".to_string()),
      (header::CONTENT_DISPOSITION, disposition),
      (header::CACHE_CONTROL, "no-store".to_string()),
      (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
    ],
    bytes,
  )
    .into_response()
}

/// RFC 5987: tudo fora de `A-Za-z0-9-._~` vira `%XX` (bytes UTF-8).
fn percent_encode(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

/// O mimetype vem de quem mandou a mensagem: só tipos conhecidos do mesmo tipo de
/// mídia passam (nada de `text/html`); o resto cai no padrão daquele tipo.
fn media_type(mimetype: Option<&str>, kind: MessageKind) -> &'static str {
  let declared = mimetype.and_then(|m| m.split(';').next()).map(str::trim);
  match kind {
    MessageKind::Video => match declared {
      Some("video/webm") => "video/webm",
      Some("video/3gpp") => "video/3gpp",
      _ => "video/mp4",
    },
    MessageKind::Audio => match declared {
      Some("audio/mpeg") => "audio/mpeg",
      Some("audio/mp4") => "audio/mp4",
      Some("audio/aac") => "audio/aac",
      Some("audio/wav") => "audio/wav",
      _ => "audio/ogg",
    },
    _ => match declared {
      Some("image/jpeg") => "image/jpeg",
      Some("image/png") => "image/png",
      Some("image/webp") => "image/webp",
      Some("image/gif") => "image/gif",
      _ if kind == MessageKind::Sticker => "image/webp",
      _ => "image/jpeg",
    },
  }
}

#[derive(Deserialize)]
pub struct MessageActionBody {
  /// `star`, `unstar`, `pin`, `unpin`, `delete_me`, `delete_all` ou `report`.
  action: String,
  /// Só para `pin`: 1, 7 ou 30 dias.
  days: Option<u32>,
}

/// Até quando dá para apagar para todos (o WhatsApp aceita ~2,5 dias).
const REVOKE_WINDOW_SECS: i64 = 60 * 3600;

/// Favorita, fixa, apaga ou denuncia uma mensagem: no WhatsApp, depois no banco.
pub async fn message_action(
  State(st): State<AppState>,
  Path((jid, id)): Path<(String, String)>,
  Json(body): Json<MessageActionBody>,
) -> ApiResult<Json<Option<Message>>> {
  validate_jid(&jid)?;
  validate_msg_id(&id)?;
  let action = match (body.action.as_str(), body.days) {
    ("star", _) => MessageAction::Star(true),
    ("unstar", _) => MessageAction::Star(false),
    ("pin", None) => MessageAction::Pin(7),
    ("pin", Some(d @ (1 | 7 | 30))) => MessageAction::Pin(d),
    ("pin", Some(_)) => return Err(ApiError::BadRequest("days deve ser 1, 7 ou 30".into())),
    ("unpin", _) => MessageAction::Unpin,
    ("delete_me", _) => MessageAction::DeleteForMe,
    ("delete_all", _) => MessageAction::DeleteForAll,
    ("report", _) => MessageAction::Report,
    _ => return Err(ApiError::BadRequest("ação desconhecida".into())),
  };
  let (j, i) = (jid.clone(), id.clone());
  let msg = st
    .store
    .run(move |db| db.get_message(&j, &i))
    .await?
    .ok_or(ApiError::NotFound)?;
  match action {
    MessageAction::DeleteForAll if !msg.from_me => {
      return Err(ApiError::BadRequest(
        "só dá para apagar para todos as suas mensagens".into(),
      ));
    }
    MessageAction::DeleteForAll if now_ms() / 1000 - msg.ts > REVOKE_WINDOW_SECS => {
      return Err(ApiError::BadRequest(
        "passou do prazo para apagar para todos".into(),
      ));
    }
    MessageAction::Report if msg.from_me => {
      return Err(ApiError::BadRequest(
        "não dá para denunciar a própria mensagem".into(),
      ));
    }
    _ if msg.kind == MessageKind::Revoked && action != MessageAction::DeleteForMe => {
      return Err(ApiError::BadRequest("mensagem apagada".into()));
    }
    _ => {}
  }
  ensure_connected(&st)?;
  // Primeiro no WhatsApp: se falhar, o banco local continua igual ao celular.
  st.wa
    .message_action(&msg, action)
    .await
    .map_err(ApiError::Upstream)?;
  let out = st
    .store
    .run(move |db| {
      let events = match action {
        MessageAction::Star(starred) => {
          db.set_starred(&jid, &id, starred)?;
          crate::ingest::message_changed(db, &jid, &id)?
        }
        MessageAction::Pin(days) => {
          db.set_pinned_until(&jid, &id, Some(now_ms() + i64::from(days) * 86_400_000))?;
          crate::ingest::message_changed(db, &jid, &id)?
        }
        MessageAction::Unpin => {
          db.set_pinned_until(&jid, &id, None)?;
          crate::ingest::message_changed(db, &jid, &id)?
        }
        MessageAction::DeleteForAll => {
          db.revoke_message(&jid, &id)?;
          crate::ingest::message_changed(db, &jid, &id)?
        }
        MessageAction::DeleteForMe => {
          db.delete_message(&jid, &id)?;
          let mut out = vec![ApiEvent::MessageDeleted {
            chat_jid: jid.clone(),
            id: id.clone(),
          }];
          out.extend(db.get_chat(&jid)?.map(ApiEvent::ChatUpdated));
          out
        }
        MessageAction::Report => vec![],
      };
      Ok((db.get_message(&jid, &id)?, events))
    })
    .await?;
  let (updated, events) = out;
  for ev in events {
    let _ = st.events.send(ev);
  }
  Ok(Json(updated))
}

#[derive(Deserialize)]
pub struct ForwardBody {
  /// Conversas de destino (até 5, como no WhatsApp).
  to: Vec<String>,
}

const MAX_FORWARD_TARGETS: usize = 5;

/// Encaminha a mensagem para outras conversas, sem subir a mídia de novo.
pub async fn forward(
  State(st): State<AppState>,
  Path((jid, id)): Path<(String, String)>,
  Json(body): Json<ForwardBody>,
) -> ApiResult<Json<Vec<Message>>> {
  validate_jid(&jid)?;
  validate_msg_id(&id)?;
  if body.to.is_empty() || body.to.len() > MAX_FORWARD_TARGETS {
    return Err(ApiError::BadRequest(format!(
      "escolha de 1 a {MAX_FORWARD_TARGETS} conversas"
    )));
  }
  for to in &body.to {
    validate_jid(to)?;
    if is_read_only(to) {
      return Err(ApiError::BadRequest(
        "status e canais são só de leitura".into(),
      ));
    }
  }
  let (j, i) = (jid.clone(), id.clone());
  let (msg, media) = st
    .store
    .run(move |db| Ok((db.get_message(&j, &i)?, db.get_media(&j, &i)?)))
    .await?;
  let msg = msg.ok_or(ApiError::NotFound)?;
  if matches!(msg.kind, MessageKind::Revoked | MessageKind::ViewOnce)
    || (media.is_none() && msg.kind != MessageKind::Text)
  {
    return Err(ApiError::BadRequest(
      "essa mensagem não pode ser encaminhada".into(),
    ));
  }
  ensure_connected(&st)?;

  let mut sent_msgs = Vec::new();
  for to in body.to {
    if !st.limiter.try_acquire() {
      return Err(ApiError::RateLimited);
    }
    let sent = st
      .wa
      .forward(&to, &msg, media.as_ref())
      .await
      .map_err(ApiError::Upstream)?;
    let copy = Message {
      id: sent.id,
      chat_jid: to.clone(),
      sender_jid: None,
      sender_name: None,
      from_me: true,
      ts: sent.ts,
      kind: msg.kind,
      body: msg.body.clone(),
      quoted_id: None,
      status: Some(MsgStatus::Sent),
      is_read: true,
      media: msg.media.clone(),
      starred: false,
      pinned: false,
      mentions: vec![],
    };
    let (c, m) = (copy.clone(), media.clone());
    let chat = st
      .store
      .run(move |db| {
        if let Some(m) = &m {
          db.insert_media(&c.chat_jid, &c.id, m)?;
        }
        db.insert_message(&c)?;
        db.get_chat(&c.chat_jid)
      })
      .await?;
    let _ = st.events.send(ApiEvent::MessageNew(copy.clone()));
    if let Some(chat) = chat {
      let _ = st.events.send(ApiEvent::ChatUpdated(chat));
    }
    sent_msgs.push(copy);
  }
  Ok(Json(sent_msgs))
}

#[derive(Deserialize)]
pub struct SendMediaQuery {
  /// Nome do arquivo (documentos mostram e salvam com ele).
  name: Option<String>,
  caption: Option<String>,
  width: Option<u32>,
  height: Option<u32>,
  seconds: Option<u32>,
}

/// Envia um arquivo: o corpo é o arquivo; `Content-Type` diz o tipo.
pub async fn send_media(
  State(st): State<AppState>,
  Path(jid): Path<String>,
  Query(q): Query<SendMediaQuery>,
  headers: HeaderMap,
  body: Bytes,
) -> ApiResult<Json<Message>> {
  validate_jid(&jid)?;
  if is_read_only(&jid) {
    return Err(ApiError::BadRequest(
      "status e canais são só de leitura".into(),
    ));
  }
  if body.is_empty() {
    return Err(ApiError::BadRequest("arquivo vazio".into()));
  }
  let mimetype = headers
    .get(header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.split(';').next())
    .map(|v| v.trim().to_ascii_lowercase())
    .filter(|v| !v.is_empty())
    .unwrap_or_else(|| "application/octet-stream".into());
  let kind = outgoing_kind(&mimetype);
  let caption = q
    .caption
    .as_deref()
    .map(str::trim)
    .filter(|c| !c.is_empty())
    .map(str::to_string);
  if caption
    .as_ref()
    .is_some_and(|c| c.chars().count() > MAX_TEXT_CHARS)
  {
    return Err(ApiError::BadRequest(format!(
      "legenda acima de {MAX_TEXT_CHARS} caracteres"
    )));
  }
  let file_name = q
    .name
    .as_deref()
    .map(|n| {
      n.trim()
        .chars()
        .take(MAX_FILE_NAME_CHARS)
        .collect::<String>()
    })
    .filter(|n| !n.is_empty());
  ensure_connected(&st)?;
  if !st.limiter.try_acquire() {
    return Err(ApiError::RateLimited);
  }

  let (sent, media) = st
    .wa
    .send_media(
      &jid,
      OutgoingMedia {
        kind,
        bytes: body.to_vec(),
        mimetype,
        file_name: file_name.clone(),
        caption: caption.clone(),
        width: q.width,
        height: q.height,
        seconds: q.seconds,
      },
    )
    .await
    .map_err(ApiError::Upstream)?;
  let msg = Message {
    id: sent.id,
    chat_jid: jid.clone(),
    sender_jid: None,
    sender_name: None,
    from_me: true,
    ts: sent.ts,
    kind,
    // Documento sem legenda mostra o nome do arquivo, como os recebidos.
    body: caption.or(if kind == MessageKind::Doc {
      file_name
    } else {
      None
    }),
    quoted_id: None,
    status: Some(MsgStatus::Sent),
    is_read: true,
    media: Some(media.info()),
    starred: false,
    pinned: false,
    mentions: vec![],
  };
  let m = msg.clone();
  let chat = st
    .store
    .run(move |db| {
      db.insert_media(&m.chat_jid, &m.id, &media)?;
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

/// Como o arquivo vai para o WhatsApp: JPEG/PNG como imagem, MP4 como vídeo; o resto
/// (inclusive GIF e WebP, que o WhatsApp não trata como foto) vai como documento.
fn outgoing_kind(mimetype: &str) -> MessageKind {
  match mimetype {
    "image/jpeg" | "image/png" => MessageKind::Image,
    "video/mp4" => MessageKind::Video,
    _ => MessageKind::Doc,
  }
}

fn is_read_only(jid: &str) -> bool {
  jid == "status@broadcast" || jid.ends_with("@newsletter")
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
