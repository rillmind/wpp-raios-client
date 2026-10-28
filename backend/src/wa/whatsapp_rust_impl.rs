//! Implementação de [`WhatsApp`] com a crate `whatsapp-rust` (0.7.0).
//!
//! Único arquivo do projeto que importa tipos da biblioteca.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore::types::events::LazyHistorySync;
use whatsapp_rust::wacore::types::presence::ReceiptType;
use whatsapp_rust::wacore_binary::jid::JidExt;

use super::{qr, Sent, UnreadRef, WaEvent, WhatsApp};
use crate::model::{ChatMeta, ConnState, Message, MessageKind, MsgStatus};

pub struct WaOptions {
  pub session_db: PathBuf,
  /// Máximo de mensagens guardadas por chat vindas da sincronização inicial.
  pub history_msgs_per_chat: usize,
}

pub struct RustWa {
  client: Arc<Client>,
  handle: tokio::sync::Mutex<Option<BotHandle>>,
  state: watch::Receiver<ConnState>,
}

/// Conecta (ou pareia, mostrando o QR no terminal) e passa a emitir eventos em `tx`.
pub async fn start(opts: WaOptions, tx: mpsc::Sender<WaEvent>) -> Result<Arc<dyn WhatsApp>> {
  let session = opts.session_db.to_string_lossy().into_owned();
  let store = SqliteStore::new(&session)
    .await
    .map_err(|e| anyhow!("abrindo sessão {session}: {e}"))?;

  let (state_tx, state_rx) = watch::channel(ConnState::Connecting);
  let state_tx = Arc::new(state_tx);
  let ctx = Arc::new(Translator {
    tx,
    state: state_tx,
    history_limit: opts.history_msgs_per_chat,
    groups_named: Mutex::new(HashSet::new()),
  });

  let bot = Bot::builder()
    .with_backend(store)
    .on_event(move |event, client| {
      let ctx = Arc::clone(&ctx);
      async move { ctx.handle(&event, &client).await }
    })
    .build()
    .await
    .context("iniciando cliente do WhatsApp")?;

  let client = bot.client();
  let handle = bot.spawn();
  Ok(Arc::new(RustWa {
    client,
    handle: tokio::sync::Mutex::new(Some(handle)),
    state: state_rx,
  }))
}

#[async_trait]
impl WhatsApp for RustWa {
  fn state(&self) -> ConnState {
    *self.state.borrow()
  }

  async fn send_text(&self, chat_jid: &str, text: &str) -> Result<Sent> {
    let jid: Jid = chat_jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;
    let sent = self
      .client
      .send_message(jid, wa::Message::text(text))
      .await
      .map_err(|e| anyhow!("falha ao enviar: {e}"))?;
    Ok(Sent {
      id: sent.message_id,
      ts: now_secs(),
    })
  }

  async fn mark_read(&self, chat_jid: &str, unread: &[UnreadRef]) -> Result<()> {
    let chat: Jid = chat_jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;

    // Recibos de leitura (o contato vê os "tiques azuis"), agrupados por
    // remetente porque em grupo cada recibo leva o `participant`.
    let mut by_sender: Vec<(Option<Jid>, Vec<&str>)> = Vec::new();
    for msg in unread {
      let sender = match &msg.sender_jid {
        Some(s) if chat.is_group() => {
          Some(s.parse::<Jid>().map_err(|e| anyhow!("jid inválido: {e}"))?)
        }
        _ => None,
      };
      match by_sender.iter_mut().find(|(s, _)| *s == sender) {
        Some((_, ids)) => ids.push(&msg.id),
        None => by_sender.push((sender, vec![&msg.id])),
      }
    }
    for (sender, ids) in &by_sender {
      self
        .client
        .mark_as_read(&chat, sender.as_ref(), ids)
        .await?;
    }

    // Sincroniza o estado "lido" com o celular e outros aparelhos.
    self
      .client
      .chat_actions()
      .mark_chat_as_read(&chat, true, None)
      .await
      .map_err(|e| anyhow!("marcando chat como lido: {e}"))?;
    Ok(())
  }

  async fn shutdown(&self) {
    if let Some(handle) = self.handle.lock().await.take() {
      handle.shutdown().await;
    }
  }
}

/// Converte eventos da biblioteca em [`WaEvent`].
struct Translator {
  tx: mpsc::Sender<WaEvent>,
  state: Arc<watch::Sender<ConnState>>,
  history_limit: usize,
  /// Grupos cujo nome já foi buscado nesta execução.
  groups_named: Mutex<HashSet<String>>,
}

impl Translator {
  async fn handle(&self, event: &Event, client: &Arc<Client>) {
    // `Debug` de `Event` imprime só o nome da variante, sem conteúdo.
    tracing::debug!(event = ?event, "evento do WhatsApp");

    match event {
      Event::Connected(_) => self.set_state(ConnState::Connected).await,
      Event::Disconnected(_) | Event::StreamError(_) | Event::ConnectFailure(_) => {
        self.set_state(ConnState::Disconnected).await
      }
      Event::LoggedOut(_) => {
        tracing::warn!("sessão encerrada pelo WhatsApp: é preciso parear de novo");
        self.set_state(ConnState::LoggedOut).await
      }
      Event::PairingQrCode(qr) => {
        self.set_state(ConnState::WaitingQr).await;
        match qr::render(&qr.code) {
          Some(art) => {
            // stderr: o QR é para o humano no terminal, não para os logs.
            #[allow(clippy::print_stderr)]
            {
              eprintln!(
                "\nEscaneie no celular: WhatsApp > Aparelhos conectados > Conectar aparelho\n"
              );
              eprintln!("{art}\n");
            }
          }
          None => tracing::error!("não foi possível gerar o QR code"),
        }
      }
      Event::PairSuccess(_) => tracing::info!("pareamento concluído"),
      Event::Messages(batch) => {
        for inbound in batch.iter() {
          if let Some(msg) = convert_live(&inbound.message, &inbound.info) {
            self
              .ensure_group_name(client, &inbound.info.source.chat)
              .await;
            if !msg.from_me && !inbound.info.source.is_group {
              if let Some(name) = non_empty(&inbound.info.push_name) {
                self
                  .send(WaEvent::ChatMeta(ChatMeta {
                    jid: msg.chat_jid.clone(),
                    name: Some(name),
                    ..Default::default()
                  }))
                  .await;
              }
            }
            self.send(WaEvent::Message(msg)).await;
          }
        }
      }
      Event::Receipt(r) => {
        let ids: Vec<String> = r.message_ids.iter().map(|id| id.to_string()).collect();
        let chat_jid = r.source.chat.to_non_ad_string();
        let status = match r.r#type {
          ReceiptType::Delivered => Some(MsgStatus::Delivered),
          ReceiptType::Read | ReceiptType::Played => Some(MsgStatus::Read),
          ReceiptType::ReadSelf | ReceiptType::PlayedSelf => {
            self.send(WaEvent::ReadElsewhere { chat_jid, ids }).await;
            return;
          }
          _ => None,
        };
        if let Some(status) = status {
          self
            .send(WaEvent::Status {
              chat_jid,
              ids,
              status,
            })
            .await;
        }
      }
      Event::MarkChatAsReadUpdate(u) => {
        if let Some(read) = u.action.read {
          self
            .send(WaEvent::ChatRead {
              chat_jid: u.jid.to_non_ad_string(),
              read,
            })
            .await;
        }
      }
      Event::ArchiveUpdate(u) => {
        self
          .send(WaEvent::ChatMeta(ChatMeta {
            jid: u.jid.to_non_ad_string(),
            archived: u.action.archived,
            ..Default::default()
          }))
          .await
      }
      Event::PinUpdate(u) => {
        self
          .send(WaEvent::ChatMeta(ChatMeta {
            jid: u.jid.to_non_ad_string(),
            pinned: u.action.pinned,
            ..Default::default()
          }))
          .await
      }
      Event::ContactUpdate(u) => {
        let name = u
          .action
          .full_name
          .as_deref()
          .or(u.action.first_name.as_deref())
          .and_then(non_empty);
        if name.is_some() {
          self
            .send(WaEvent::ChatMeta(ChatMeta {
              jid: u.jid.to_non_ad_string(),
              name,
              ..Default::default()
            }))
            .await
        }
      }
      Event::HistorySync(lazy) => self.handle_history(lazy).await,
      _ => {}
    }
  }

  async fn handle_history(&self, lazy: &LazyHistorySync) {
    use wa::history_sync::HistorySyncType;
    let sync_type = lazy.sync_type();
    // Só a sincronização inicial e a recente; histórico completo (FULL) e
    // sob demanda ficam fora, por desenho do app.
    let wanted = [
      HistorySyncType::INITIAL_BOOTSTRAP as i32,
      HistorySyncType::RECENT as i32,
      HistorySyncType::PUSH_NAME as i32,
    ];
    if !wanted.contains(&sync_type) {
      tracing::debug!(sync_type, "pacote de histórico ignorado");
      return;
    }
    let Some(hs) = lazy.get() else {
      tracing::warn!(sync_type, "pacote de histórico ilegível");
      return;
    };
    tracing::info!(
      sync_type,
      conversations = hs.conversations.len(),
      pushnames = hs.pushnames.len(),
      "pacote de histórico recebido"
    );

    for conv in &hs.conversations {
      if let Some(ev) = convert_conversation(conv, self.history_limit) {
        self.send(ev).await;
      }
    }
    for pn in &hs.pushnames {
      if let (Some(id), Some(name)) = (pn.id.as_deref(), pn.pushname.as_deref().and_then(non_empty))
      {
        self
          .send(WaEvent::ChatMeta(ChatMeta {
            jid: id.to_string(),
            name: Some(name),
            ..Default::default()
          }))
          .await;
      }
    }
  }

  /// Busca o nome (assunto) do grupo uma vez por execução.
  async fn ensure_group_name(&self, client: &Arc<Client>, chat: &Jid) {
    if !chat.is_group() {
      return;
    }
    let key = chat.to_non_ad_string();
    {
      let Ok(mut seen) = self.groups_named.lock() else {
        return;
      };
      if !seen.insert(key.clone()) {
        return;
      }
    }
    let client = Arc::clone(client);
    let chat = chat.clone();
    let tx = self.tx.clone();
    tokio::spawn(async move {
      match client.groups().get_metadata(&chat).await {
        Ok(meta) => {
          let _ = tx
            .send(WaEvent::ChatMeta(ChatMeta {
              jid: key,
              name: non_empty(&meta.subject),
              is_group: true,
              ..Default::default()
            }))
            .await;
        }
        Err(e) => tracing::debug!(error = %e, "falha ao buscar dados do grupo"),
      }
    });
  }

  async fn set_state(&self, state: ConnState) {
    if *self.state.borrow() != state {
      tracing::info!(?state, "estado da conexão");
      self.state.send_replace(state);
      self.send(WaEvent::Connection(state)).await;
    }
  }

  async fn send(&self, ev: WaEvent) {
    if self.tx.send(ev).await.is_err() {
      tracing::warn!("canal de eventos fechado");
    }
  }
}

fn convert_live(message: &wa::Message, info: &MessageInfo) -> Option<Message> {
  let (kind, body, quoted_id) = classify(message)?;
  let src = &info.source;
  Some(Message {
    id: info.id.to_string(),
    chat_jid: src.chat.to_non_ad_string(),
    sender_jid: Some(src.sender.to_non_ad_string()),
    from_me: src.is_from_me,
    ts: info.timestamp.timestamp(),
    kind,
    body,
    quoted_id,
    status: src.is_from_me.then_some(MsgStatus::Sent),
    // Mensagens nossas (enviadas de outro aparelho) já nascem lidas.
    is_read: src.is_from_me,
  })
}

fn convert_conversation(conv: &wa::Conversation, limit: usize) -> Option<WaEvent> {
  let jid: Jid = conv.id.parse().ok()?;
  if jid.is_status_broadcast() || jid.is_newsletter() {
    return None;
  }
  let chat_jid = jid.to_non_ad_string();
  let unread = conv.unread_count.unwrap_or(0);
  let marked_unread = conv.marked_as_unread.unwrap_or(false);

  let mut messages: Vec<Message> = conv
    .messages
    .iter()
    .filter_map(|hm| hm.message.as_option())
    .filter_map(|wmi| convert_history_msg(wmi, &chat_jid, jid.is_group()))
    .collect();
  messages.sort_by_key(|m| std::cmp::Reverse(m.ts));
  messages.truncate(limit);

  // As `unread` mensagens recebidas mais recentes são as não lidas.
  let mut left = unread as usize;
  for m in messages.iter_mut().filter(|m| !m.from_me) {
    if left == 0 {
      m.is_read = true;
    } else {
      m.is_read = false;
      left -= 1;
    }
  }

  let last_ts = conv
    .conversation_timestamp
    .or(conv.last_msg_timestamp)
    .and_then(|t| i64::try_from(t).ok())
    .or_else(|| messages.first().map(|m| m.ts));

  Some(WaEvent::HistoryChat {
    meta: ChatMeta {
      jid: chat_jid,
      name: conv
        .name
        .as_deref()
        .or(conv.display_name.as_deref())
        .and_then(non_empty),
      is_group: jid.is_group(),
      unread_count: Some(if unread == 0 && marked_unread {
        1
      } else {
        unread
      }),
      archived: conv.archived,
      pinned: conv.pinned.map(|p| p > 0),
    },
    last_ts,
    messages,
  })
}

fn convert_history_msg(
  wmi: &wa::WebMessageInfo,
  chat_jid: &str,
  is_group: bool,
) -> Option<Message> {
  let key = wmi.key.as_option()?;
  let id = key.id.clone()?;
  let from_me = key.from_me.unwrap_or(false);
  let (kind, body, quoted_id) = classify(wmi.message.as_option()?)?;
  use wa::web_message_info::Status;
  let status = from_me.then_some(match wmi.status {
    Some(Status::READ | Status::PLAYED) => MsgStatus::Read,
    Some(Status::DELIVERY_ACK) => MsgStatus::Delivered,
    _ => MsgStatus::Sent,
  });
  let sender_jid = if from_me {
    None
  } else if is_group {
    key.participant.clone().or_else(|| wmi.participant.clone())
  } else {
    Some(chat_jid.to_string())
  };
  Some(Message {
    id,
    chat_jid: chat_jid.to_string(),
    sender_jid,
    from_me,
    ts: wmi
      .message_timestamp
      .and_then(|t| i64::try_from(t).ok())
      .unwrap_or(0),
    kind,
    body,
    quoted_id,
    status,
    is_read: true,
  })
}

/// Tipo, texto/legenda e id citado. `None` para mensagens que não aparecem
/// na conversa (reações, edições, apagamentos, controle de protocolo).
fn classify(message: &wa::Message) -> Option<(MessageKind, Option<String>, Option<String>)> {
  let m = message.get_base_message();
  if m.protocol_message.is_set() || m.reaction_message.is_set() {
    return None;
  }

  let caption = m.get_caption().map(str::to_string);
  let (kind, body, ctx) = if let Some(text) = m.conversation.as_deref() {
    (MessageKind::Text, Some(text.to_string()), None)
  } else if let Some(ext) = m.extended_text_message.as_option() {
    (
      MessageKind::Text,
      ext.text.clone(),
      ext.context_info.as_option(),
    )
  } else if let Some(x) = m.image_message.as_option() {
    (MessageKind::Image, caption, x.context_info.as_option())
  } else if let Some(x) = m.video_message.as_option() {
    (MessageKind::Video, caption, x.context_info.as_option())
  } else if let Some(x) = m.audio_message.as_option() {
    (MessageKind::Audio, None, x.context_info.as_option())
  } else if let Some(x) = m.document_message.as_option() {
    let body = caption.or_else(|| x.file_name.clone());
    (MessageKind::Doc, body, x.context_info.as_option())
  } else if let Some(x) = m.sticker_message.as_option() {
    (MessageKind::Sticker, None, x.context_info.as_option())
  } else if has_visible_content(m) {
    (MessageKind::Other, None, None)
  } else {
    return None;
  };
  let quoted_id = ctx.and_then(|c| c.stanza_id.clone());
  Some((kind, body, quoted_id))
}

/// Outros tipos que aparecem na conversa (localização, contato, enquete...).
fn has_visible_content(m: &wa::Message) -> bool {
  m.location_message.is_set()
    || m.live_location_message.is_set()
    || m.contact_message.is_set()
    || m.contacts_array_message.is_set()
    || m.ptv_message.is_set()
    || m.poll_creation_message.is_set()
    || m.poll_creation_message_v3.is_set()
    || m.event_message.is_set()
}

fn non_empty(s: &str) -> Option<String> {
  let t = s.trim();
  (!t.is_empty()).then(|| t.to_string())
}

fn now_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}
