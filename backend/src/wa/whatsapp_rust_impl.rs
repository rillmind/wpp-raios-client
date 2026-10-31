//! Implementação de [`WhatsApp`] com a crate `whatsapp-rust` (0.7.0).
//!
//! Único arquivo do projeto que importa tipos da biblioteca.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio::sync::{mpsc, watch, Semaphore};
use whatsapp_rust::buffa;
use whatsapp_rust::download::{DownloadParams, MediaType};
use whatsapp_rust::http::HttpRequest;
use whatsapp_rust::media::{DocumentOptions, ImageOptions, VideoOptions};
use whatsapp_rust::prelude::*;
use whatsapp_rust::send::{PinDuration, RevokeType};
use whatsapp_rust::sync_task::MajorSyncTask;
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::wacore::appstate::hash::HashState;
use whatsapp_rust::wacore::appstate::patch_decode::WAPatchName;
use whatsapp_rust::wacore::iq::mex_operations::fetch_all_newsletters_metadata;
use whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info;
use whatsapp_rust::wacore::store::traits::AppSyncStore;
use whatsapp_rust::wacore::types::events::{LazyHistorySync, UnavailableType};
use whatsapp_rust::wacore::types::presence::ReceiptType;
use whatsapp_rust::wacore::types::spam_report::{SpamFlow, SpamReportRequest};
use whatsapp_rust::wacore_binary::jid::JidExt;
use whatsapp_rust::MexRequest;

use super::{qr, OutgoingMedia, Sent, UnreadRef, WaEvent, WhatsApp};
use crate::model::{
  ChatAction, ChatMeta, ConnState, Contact, MediaRef, Message, MessageAction, MessageKind,
  MsgStatus, Quote,
};

pub struct WaOptions {
  pub session_db: PathBuf,
}

/// Fotos de perfil buscadas ao mesmo tempo (a lista pede várias de uma vez).
const AVATAR_CONCURRENCY: usize = 4;
/// Sem resposta nesse tempo, desiste (sem isso uma consulta presa segura a fila).
const AVATAR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

pub struct RustWa {
  client: Arc<Client>,
  handle: tokio::sync::Mutex<Option<BotHandle>>,
  state: watch::Receiver<ConnState>,
  avatar_slots: Semaphore,
  /// Mesma sessão que a lib usa (o `SqliteStore` é um handle clonável).
  session: SqliteStore,
  /// QR atual, compartilhado com o [`Translator`].
  qr: Arc<Mutex<Option<String>>>,
  /// Canais assinados (jid → dados), buscados uma vez por execução.
  channels: tokio::sync::Mutex<Option<HashMap<String, Channel>>>,
}

#[derive(Debug, Clone)]
struct Channel {
  name: String,
  /// Caminho da foto (miniatura) no CDN.
  picture: Option<String>,
}

/// Host das fotos de canal (a lista traz só o caminho).
const CHANNEL_PICTURE_HOST: &str = "https://mmg.whatsapp.net";

/// Conecta (ou pareia, mostrando o QR no terminal) e passa a emitir eventos em `tx`.
pub async fn start(opts: WaOptions, tx: mpsc::Sender<WaEvent>) -> Result<Arc<dyn WhatsApp>> {
  let session = opts.session_db.to_string_lossy().into_owned();
  let store = SqliteStore::new(&session)
    .await
    .map_err(|e| anyhow!("abrindo sessão {session}: {e}"))?;

  let (state_tx, state_rx) = watch::channel(ConnState::Connecting);
  let state_tx = Arc::new(state_tx);
  let qr = Arc::new(Mutex::new(None));
  let ctx = Arc::new(Translator {
    qr: Arc::clone(&qr),
    tx,
    state: state_tx,
    groups_named: Mutex::new(HashSet::new()),
    lids_seen: Mutex::new(HashSet::new()),
  });

  let session = store.clone();
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
    avatar_slots: Semaphore::new(AVATAR_CONCURRENCY),
    session,
    qr,
    channels: tokio::sync::Mutex::new(None),
  }))
}

impl RustWa {
  /// Canais assinados, da memória ou do WhatsApp (uma consulta traz todos).
  async fn channel_map(&self) -> Result<HashMap<String, Channel>> {
    let mut cached = self.channels.lock().await;
    if let Some(map) = cached.as_ref() {
      return Ok(map.clone());
    }
    // A 0.7.0 manda as variáveis vazias e o servidor responde 400; o WhatsApp Web
    // (e a lib a partir da próxima versão) manda estas duas como `false`.
    let vars = fetch_all_newsletters_metadata::Variables {
      fetch_status_metadata: Some(false),
      fetch_wamo_sub: Some(false),
    };
    let response = self
      .client
      .mex()
      .query(MexRequest::new(
        fetch_all_newsletters_metadata::NAME,
        fetch_all_newsletters_metadata::DOC_ID,
        vars,
      ))
      .await
      .map_err(|e| anyhow!("listando canais: {e:?}"))?;
    let list = response
      .data
      .as_ref()
      .and_then(|d| d["xwa2_newsletter_subscribed"].as_array())
      .ok_or_else(|| anyhow!("listando canais: resposta sem a lista"))?;
    let mut map = HashMap::new();
    for item in list {
      let (Some(id), Some(name)) = (
        normalize(item["id"].as_str()),
        item["thread_metadata"]["name"]["text"]
          .as_str()
          .and_then(non_empty),
      ) else {
        continue;
      };
      let thread = &item["thread_metadata"];
      let picture = thread["preview"]["direct_path"]
        .as_str()
        .or(thread["picture"]["direct_path"].as_str())
        .map(str::to_string);
      map.insert(id, Channel { name, picture });
    }
    *cached = Some(map.clone());
    Ok(map)
  }

  /// Nosso jid (o de telefone; o `@lid` se ainda não houver).
  fn own_jid(&self) -> Result<Jid> {
    self
      .client
      .pn()
      .or_else(|| self.client.lid())
      .map(|j| j.to_non_ad())
      .ok_or_else(|| anyhow!("sessão sem jid próprio"))
  }
}

#[async_trait]
impl WhatsApp for RustWa {
  fn state(&self) -> ConnState {
    *self.state.borrow()
  }

  fn pairing_qr(&self) -> Option<String> {
    if self.state() != ConnState::WaitingQr {
      return None;
    }
    self.qr.lock().ok().and_then(|q| q.clone())
  }

  async fn send_text(&self, chat_jid: &str, text: &str, quote: Option<&Quote>) -> Result<Sent> {
    let jid: Jid = chat_jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;
    let message = match quote {
      None => wa::Message::text(text),
      Some(q) => {
        // A citação leva o texto da original (não guardamos a mensagem crua).
        let sender = match (&q.sender_jid, q.from_me) {
          (_, true) => self.own_jid()?,
          (Some(s), false) => parse_jid(s)?,
          (None, false) => jid.clone(),
        };
        let context =
          build_quote_context_with_info(&q.id, &sender, &jid, &jid, &wa::Message::text(&q.text));
        wa::Message::text_with_context(text, context)
      }
    };
    let sent = self
      .client
      .send_message(jid, message)
      .await
      .map_err(|e| anyhow!("falha ao enviar: {e}"))?;
    Ok(Sent {
      id: sent.message_id,
      ts: now_secs(),
    })
  }

  async fn message_action(&self, msg: &Message, action: MessageAction) -> Result<()> {
    let chat = parse_jid(&msg.chat_jid)?;
    // Em grupo, quem mandou identifica a mensagem; nas nossas e em conversa
    // individual não vai participante.
    let participant = if (chat.is_group() || chat.is_status_broadcast()) && !msg.from_me {
      msg.sender_jid.as_deref().map(parse_jid).transpose()?
    } else {
      None
    };
    let key = wa::MessageKey {
      remote_jid: Some(chat.to_string()),
      from_me: Some(msg.from_me),
      id: Some(msg.id.clone()),
      participant: participant.as_ref().map(Jid::to_string),
    };
    let actions = self.client.chat_actions();
    match action {
      MessageAction::Star(true) => actions
        .star_message(&chat, participant.as_ref(), &msg.id, msg.from_me)
        .await
        .map_err(|e| anyhow!("{e}")),
      MessageAction::Star(false) => actions
        .unstar_message(&chat, participant.as_ref(), &msg.id, msg.from_me)
        .await
        .map_err(|e| anyhow!("{e}")),
      MessageAction::Pin(days) => {
        let duration = match days {
          1 => PinDuration::Hours24,
          30 => PinDuration::Days30,
          _ => PinDuration::Days7,
        };
        self
          .client
          .pin_message(chat, key, duration)
          .await
          .map_err(|e| anyhow!("{e}"))
      }
      MessageAction::Unpin => self
        .client
        .unpin_message(chat, key)
        .await
        .map_err(|e| anyhow!("{e}")),
      MessageAction::DeleteForMe => actions
        .delete_message_for_me(
          &chat,
          participant.as_ref(),
          &msg.id,
          msg.from_me,
          true,
          Some(msg.ts * 1000),
        )
        .await
        .map_err(|e| anyhow!("{e}")),
      MessageAction::DeleteForAll => self
        .client
        .revoke_message(chat, msg.id.clone(), RevokeType::Sender)
        .await
        .map_err(|e| anyhow!("{e}")),
      MessageAction::Report => {
        let sender = match &msg.sender_jid {
          Some(s) => parse_jid(s)?,
          None => chat.clone(),
        };
        let group = chat.is_group();
        self
          .client
          .send_spam_report(SpamReportRequest {
            message_id: msg.id.clone(),
            message_timestamp: u64::try_from(msg.ts).unwrap_or(0),
            from_jid: Some(if group { chat.clone() } else { sender.clone() }),
            participant_jid: group.then_some(sender),
            group_jid: group.then(|| chat.clone()),
            spam_flow: SpamFlow::MessageMenu,
            ..Default::default()
          })
          .await
          .map(|_| ())
          .map_err(|e| anyhow!("{e}"))
      }
    }
  }

  async fn forward(&self, to_jid: &str, msg: &Message, media: Option<&MediaRef>) -> Result<Sent> {
    let to = parse_jid(to_jid)?;
    let ctx = || {
      buffa::MessageField::some(wa::ContextInfo {
        is_forwarded: Some(true),
        forwarding_score: Some(1),
        ..Default::default()
      })
    };
    let caption = msg.body.clone();
    let message = match media {
      // Mídia: reaproveita o arquivo que já está no CDN (mesmo caminho e chave).
      Some(m) => match m.kind {
        MessageKind::Image => wa::Message {
          image_message: buffa::MessageField::some(wa::message::ImageMessage {
            direct_path: Some(m.direct_path.clone()),
            media_key: m.media_key.clone(),
            file_sha256: Some(m.file_sha256.clone()),
            file_enc_sha256: m.file_enc_sha256.clone(),
            file_length: Some(m.file_length),
            mimetype: m.mimetype.clone(),
            width: m.width,
            height: m.height,
            caption,
            context_info: ctx(),
            ..Default::default()
          }),
          ..Default::default()
        },
        MessageKind::Video => wa::Message {
          video_message: buffa::MessageField::some(wa::message::VideoMessage {
            direct_path: Some(m.direct_path.clone()),
            media_key: m.media_key.clone(),
            file_sha256: Some(m.file_sha256.clone()),
            file_enc_sha256: m.file_enc_sha256.clone(),
            file_length: Some(m.file_length),
            mimetype: m.mimetype.clone(),
            width: m.width,
            height: m.height,
            seconds: m.seconds,
            caption,
            context_info: ctx(),
            ..Default::default()
          }),
          ..Default::default()
        },
        MessageKind::Audio => wa::Message {
          audio_message: buffa::MessageField::some(wa::message::AudioMessage {
            direct_path: Some(m.direct_path.clone()),
            media_key: m.media_key.clone(),
            file_sha256: Some(m.file_sha256.clone()),
            file_enc_sha256: m.file_enc_sha256.clone(),
            file_length: Some(m.file_length),
            mimetype: m.mimetype.clone(),
            seconds: m.seconds,
            context_info: ctx(),
            ..Default::default()
          }),
          ..Default::default()
        },
        MessageKind::Sticker => wa::Message {
          sticker_message: buffa::MessageField::some(wa::message::StickerMessage {
            direct_path: Some(m.direct_path.clone()),
            media_key: m.media_key.clone(),
            file_sha256: Some(m.file_sha256.clone()),
            file_enc_sha256: m.file_enc_sha256.clone(),
            file_length: Some(m.file_length),
            mimetype: m.mimetype.clone(),
            width: m.width,
            height: m.height,
            context_info: ctx(),
            ..Default::default()
          }),
          ..Default::default()
        },
        _ => wa::Message {
          document_message: buffa::MessageField::some(wa::message::DocumentMessage {
            direct_path: Some(m.direct_path.clone()),
            media_key: m.media_key.clone(),
            file_sha256: Some(m.file_sha256.clone()),
            file_enc_sha256: m.file_enc_sha256.clone(),
            file_length: Some(m.file_length),
            mimetype: m.mimetype.clone(),
            file_name: m.file_name.clone(),
            title: m.file_name.clone(),
            caption: caption.filter(|c| Some(c) != m.file_name.as_ref()),
            context_info: ctx(),
            ..Default::default()
          }),
          ..Default::default()
        },
      },
      None => {
        let text = msg
          .body
          .clone()
          .ok_or_else(|| anyhow!("mensagem sem texto para encaminhar"))?;
        wa::Message::text_with_context(
          text,
          wa::ContextInfo {
            is_forwarded: Some(true),
            forwarding_score: Some(1),
            ..Default::default()
          },
        )
      }
    };
    let sent = self
      .client
      .send_message(to, message)
      .await
      .map_err(|e| anyhow!("falha ao encaminhar: {e}"))?;
    Ok(Sent {
      id: sent.message_id,
      ts: now_secs(),
    })
  }

  async fn request_history(&self, anchor: &Message, count: u32) -> Result<()> {
    let chat = parse_jid(&anchor.chat_jid)?;
    self
      .client
      .fetch_message_history(
        &chat,
        &anchor.id,
        anchor.from_me,
        anchor.ts * 1000,
        i32::try_from(count).unwrap_or(20),
      )
      .await
      .map(|_| ())
      .map_err(|e| anyhow!("pedindo mensagens ao celular: {e}"))
  }

  async fn chat_action(&self, chat_jid: &str, action: ChatAction) -> Result<()> {
    let jid: Jid = chat_jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;
    let actions = self.client.chat_actions();
    let res = match action {
      ChatAction::Archive(true) => actions.archive_chat(&jid, None).await,
      ChatAction::Archive(false) => actions.unarchive_chat(&jid, None).await,
      ChatAction::Pin(true) => actions.pin_chat(&jid).await,
      ChatAction::Pin(false) => actions.unpin_chat(&jid).await,
      ChatAction::Mute(None) => actions.mute_chat(&jid).await,
      ChatAction::Mute(Some(until_ms)) => actions.mute_chat_until(&jid, until_ms).await,
      ChatAction::Unmute => actions.unmute_chat(&jid).await,
      ChatAction::MarkUnread => actions.mark_chat_as_read(&jid, false, None).await,
    };
    res.map_err(|e| anyhow!("{e}"))
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

  async fn send_media(&self, chat_jid: &str, out: OutgoingMedia) -> Result<(Sent, MediaRef)> {
    let jid: Jid = chat_jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;
    let media_type = match out.kind {
      MessageKind::Image => MediaType::Image,
      MessageKind::Video => MediaType::Video,
      _ => MediaType::Document,
    };
    let up = self
      .client
      .upload(out.bytes, media_type, UploadOptions::default())
      .await
      .map_err(|e| anyhow!("subindo arquivo: {e}"))?;
    let media = MediaRef {
      kind: out.kind,
      direct_path: up.direct_path.clone(),
      media_key: Some(up.media_key.to_vec()),
      file_sha256: up.file_sha256.to_vec(),
      file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
      file_length: up.file_length,
      mimetype: Some(out.mimetype.clone()),
      width: out.width,
      height: out.height,
      seconds: out.seconds,
      file_name: out.file_name.clone(),
    };
    let mut msg = match out.kind {
      MessageKind::Image => whatsapp_rust::media::image_message(
        up,
        ImageOptions {
          caption: out.caption,
          mimetype: Some(out.mimetype),
          ..Default::default()
        },
      ),
      MessageKind::Video => whatsapp_rust::media::video_message(
        up,
        VideoOptions {
          caption: out.caption,
          mimetype: Some(out.mimetype),
          duration_seconds: out.seconds,
          ..Default::default()
        },
      ),
      _ => whatsapp_rust::media::document_message(
        up,
        DocumentOptions {
          caption: out.caption,
          mimetype: Some(out.mimetype),
          title: out.file_name.clone(),
          file_name: out.file_name,
          ..Default::default()
        },
      ),
    };
    // Os construtores da lib não preenchem as dimensões; o celular usa para o layout.
    if let Some(img) = msg.image_message.as_option_mut() {
      img.width = out.width;
      img.height = out.height;
    }
    if let Some(vid) = msg.video_message.as_option_mut() {
      vid.width = out.width;
      vid.height = out.height;
    }
    let sent = self
      .client
      .send_message(jid, msg)
      .await
      .map_err(|e| anyhow!("falha ao enviar: {e}"))?;
    Ok((
      Sent {
        id: sent.message_id,
        ts: now_secs(),
      },
      media,
    ))
  }

  async fn download_media(&self, media: &MediaRef) -> Result<Vec<u8>> {
    let params = DownloadParams {
      direct_path: media.direct_path.clone(),
      media_key: media.media_key.clone(),
      file_sha256: media.file_sha256.clone(),
      file_enc_sha256: media.file_enc_sha256.clone(),
      file_length: media.file_length,
      media_type: match media.kind {
        MessageKind::Sticker => MediaType::Sticker,
        MessageKind::Video => MediaType::Video,
        MessageKind::Audio => MediaType::Audio,
        MessageKind::Doc => MediaType::Document,
        _ => MediaType::Image,
      },
    };
    self
      .client
      .download_from_params(&params)
      .await
      .map_err(|e| anyhow!("baixando mídia: {e}"))
  }

  async fn profile_picture(&self, jid: &str) -> Result<Option<Vec<u8>>> {
    let jid: Jid = jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;
    let _slot = self.avatar_slots.acquire().await?;
    let url = if jid.is_newsletter() {
      let key = jid.to_non_ad_string();
      match self
        .channel_map()
        .await?
        .get(&key)
        .and_then(|c| c.picture.clone())
      {
        Some(path) => format!("{CHANNEL_PICTURE_HOST}{path}"),
        None => return Ok(None),
      }
    } else if jid.is_pn() || jid.is_lid() || jid.is_group() {
      match self
        .client
        .contacts()
        .get_profile_picture_with_timeout(&jid, true, Some(AVATAR_TIMEOUT))
        .await
        .map_err(|e| anyhow!("buscando foto de perfil: {e}"))?
      {
        Some(pic) => pic.url,
        None => return Ok(None),
      }
    } else {
      // Status não tem foto.
      return Ok(None);
    };
    let resp = tokio::time::timeout(
      AVATAR_TIMEOUT,
      self.client.http_client.execute(HttpRequest::get(url)),
    )
    .await
    .map_err(|_| anyhow!("foto de perfil: tempo esgotado"))?
    .map_err(|e| anyhow!("baixando foto de perfil: {e}"))?;
    if resp.status_code != 200 {
      return Err(anyhow!("foto de perfil: HTTP {}", resp.status_code));
    }
    Ok(Some(resp.body))
  }

  async fn resync_contacts(&self) -> Result<()> {
    // Os contatos ficam na coleção `critical_unblock_low`. A lib só aplica um
    // snapshot mais novo que a versão guardada, então a versão volta a 0 (como
    // num aparelho recém-pareado) e o snapshot inteiro é pedido. Ao aplicá-lo a
    // lib refaz o estado da coleção do zero (ltHash e MACs) e só então grava a
    // versão nova.
    let name = WAPatchName::CriticalUnblockLow;
    self
      .session
      .set_version(name.as_str(), HashState::default())
      .await
      .map_err(|e| anyhow!("zerando a versão dos contatos: {e}"))?;
    self
      .client
      .process_sync_task(MajorSyncTask::AppStateSync {
        name,
        full_sync: true,
      })
      .await;
    Ok(())
  }

  async fn chat_title(&self, jid: &str) -> Result<Option<String>> {
    let jid: Jid = jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;
    if jid.is_newsletter() {
      let key = jid.to_non_ad_string();
      return Ok(self.channel_map().await?.get(&key).map(|c| c.name.clone()));
    }
    chat_title(&self.client, &jid).await
  }

  async fn channels(&self) -> Result<Vec<(String, String)>> {
    Ok(
      self
        .channel_map()
        .await?
        .into_iter()
        .map(|(jid, c)| (jid, c.name))
        .collect(),
    )
  }

  async fn phone_for_lid(&self, lid_jid: &str) -> Result<Option<String>> {
    let jid: Jid = lid_jid.parse().map_err(|e| anyhow!("jid inválido: {e}"))?;
    Ok(
      self
        .client
        .get_lid_pn_entry(&jid)
        .await?
        .map(|e| pn_jid(&e.phone_number)),
    )
  }

  async fn shutdown(&self) {
    if let Some(handle) = self.handle.lock().await.take() {
      handle.shutdown().await;
    }
  }
}

/// Converte eventos da biblioteca em [`WaEvent`].
struct Translator {
  /// Último QR de pareamento (para a interface); some ao parear.
  qr: Arc<Mutex<Option<String>>>,
  tx: mpsc::Sender<WaEvent>,
  state: Arc<watch::Sender<ConnState>>,
  /// Grupos cujo nome já foi buscado nesta execução.
  groups_named: Mutex<HashSet<String>>,
  /// `@lid` cujo telefone já foi procurado nesta execução.
  lids_seen: Mutex<HashSet<String>>,
}

impl Translator {
  async fn handle(&self, event: &Event, client: &Arc<Client>) {
    // `Debug` de `Event` imprime só o nome da variante, sem conteúdo.
    tracing::debug!(event = ?event, "evento do WhatsApp");

    match event {
      Event::Connected(_) => {
        self.set_qr(None);
        self.set_state(ConnState::Connected).await
      }
      Event::Disconnected(_) | Event::StreamError(_) | Event::ConnectFailure(_) => {
        self.set_state(ConnState::Disconnected).await
      }
      Event::LoggedOut(_) => {
        tracing::warn!("sessão encerrada pelo WhatsApp: é preciso parear de novo");
        self.set_state(ConnState::LoggedOut).await
      }
      Event::PairingQrCode(qr) => {
        self.set_qr(Some(qr.code.to_string()));
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
      Event::PairSuccess(_) => {
        self.set_qr(None);
        tracing::info!("pareamento concluído")
      }
      Event::Messages(batch) => {
        for inbound in batch.iter() {
          // Apagar para todos e fixar chegam como mensagens de controle.
          if let Some(ev) = control_event(&inbound.message, &inbound.info) {
            self.send(ev).await;
            continue;
          }
          if let Some((msg, media)) = convert_live(&inbound.message, &inbound.info) {
            let src = &inbound.info.source;
            self.ensure_group_name(client, &src.chat).await;
            if !msg.from_me {
              if let (Some(jid), Some(name)) =
                (&msg.sender_jid, clean_name(&inbound.info.push_name))
              {
                self
                  .send(WaEvent::Contact(Contact {
                    jid: jid.clone(),
                    push_name: Some(name),
                    ..Default::default()
                  }))
                  .await;
              }
              self
                .learn_lid(client, &src.sender, src.sender_alt.as_ref())
                .await;
            }
            if !src.is_group {
              let alt = if msg.from_me {
                src.recipient_alt.as_ref()
              } else {
                src.sender_alt.as_ref()
              };
              self.learn_lid(client, &src.chat, alt).await;
            }
            self
              .send(WaEvent::Message {
                msg,
                media: media.map(Box::new),
              })
              .await;
          }
        }
      }
      // Visualização única chega a aparelhos conectados só como aviso, sem conteúdo
      // (nem chave de mídia): registra a mensagem para a conversa mostrar o aviso.
      Event::UndecryptableMessage(u) if u.unavailable_type == UnavailableType::ViewOnce => {
        let src = &u.info.source;
        self
          .send(WaEvent::Message {
            msg: Message {
              id: u.info.id.to_string(),
              chat_jid: src.chat.to_non_ad_string(),
              sender_jid: Some(src.sender.to_non_ad_string()),
              sender_name: None,
              from_me: src.is_from_me,
              ts: u.info.timestamp.timestamp(),
              kind: MessageKind::ViewOnce,
              body: None,
              quoted_id: None,
              status: src.is_from_me.then_some(MsgStatus::Sent),
              is_read: src.is_from_me,
              media: None,
              starred: false,
              pinned: false,
            },
            media: None,
          })
          .await;
      }
      Event::Receipt(r) => {
        let ids: Vec<String> = r.message_ids.iter().map(|id| id.to_string()).collect();
        let chat_jid = r.source.chat.to_non_ad_string();
        let status = match r.r#type {
          ReceiptType::Delivered => Some(MsgStatus::Delivered),
          ReceiptType::Read | ReceiptType::Played => Some(MsgStatus::Read),
          ReceiptType::ReadSelf | ReceiptType::PlayedSelf => {
            // Em conversa individual o recibo vem do nosso próprio jid, com a conversa no
            // atributo `recipient`, que a whatsapp-rust 0.7 não repassa.
            let chat = r.source.chat.to_non_ad();
            let own = [client.pn(), client.lid()]
              .into_iter()
              .flatten()
              .any(|j| j.to_non_ad() == chat);
            let chat_jid = (!own).then_some(chat_jid);
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
      Event::MuteUpdate(u) => {
        let until = if u.action.muted == Some(true) {
          u.action.mute_end_timestamp.unwrap_or(-1)
        } else {
          0
        };
        self
          .send(WaEvent::ChatMeta(ChatMeta {
            jid: u.jid.to_non_ad_string(),
            muted_until: Some(until),
            ..Default::default()
          }))
          .await
      }
      Event::StarUpdate(u) => {
        self
          .send(WaEvent::Starred {
            chat_jid: u.chat_jid.to_non_ad_string(),
            id: u.message_id.clone(),
            starred: u.action.starred.unwrap_or(false),
          })
          .await
      }
      Event::DeleteMessageForMeUpdate(u) => {
        self
          .send(WaEvent::DeletedForMe {
            chat_jid: u.chat_jid.to_non_ad_string(),
            id: u.message_id.clone(),
          })
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
        let jid = u.jid.to_non_ad_string();
        let contact_name = u
          .action
          .full_name
          .as_deref()
          .or(u.action.first_name.as_deref())
          .and_then(clean_name);
        if contact_name.is_some() {
          self
            .send(WaEvent::Contact(Contact {
              jid: jid.clone(),
              contact_name,
              ..Default::default()
            }))
            .await;
        }
        let lid = if u.jid.is_lid() {
          Some(jid.clone())
        } else {
          normalize(u.action.lid_jid.as_deref())
        };
        let pn = if u.jid.is_pn() {
          Some(jid)
        } else {
          normalize(u.action.pn_jid.as_deref())
        };
        if let (Some(lid), Some(pn)) = (lid, pn) {
          self.send(WaEvent::LidMap { lid, pn }).await;
        }
      }
      Event::HistorySync(lazy) => self.handle_history(lazy).await,
      _ => {}
    }
  }

  async fn handle_history(&self, lazy: &LazyHistorySync) {
    use wa::history_sync::HistorySyncType;
    let sync_type = lazy.sync_type();
    // A sincronização inicial e a recente, e as respostas a "carregar mensagens"
    // (sob demanda). O histórico completo (FULL) fica fora, por desenho do app.
    let wanted = [
      HistorySyncType::INITIAL_BOOTSTRAP as i32,
      HistorySyncType::RECENT as i32,
      HistorySyncType::PUSH_NAME as i32,
      HistorySyncType::ON_DEMAND as i32,
    ];
    let on_demand = sync_type == HistorySyncType::ON_DEMAND as i32;
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
      for ev in conversation_people(conv) {
        self.send(ev).await;
      }
      if let Some(ev) = convert_conversation(conv, on_demand) {
        self.send(ev).await;
      }
    }
    for pn in &hs.pushnames {
      if let (Some(jid), Some(name)) = (
        normalize(pn.id.as_deref()),
        pn.pushname.as_deref().and_then(clean_name),
      ) {
        self
          .send(WaEvent::Contact(Contact {
            jid,
            push_name: Some(name),
            ..Default::default()
          }))
          .await;
      }
    }
  }

  /// Liga um `@lid` ao telefone, uma vez por execução: pelo `alt` da mensagem
  /// ou, sem ele, pelo que a sessão já aprendeu.
  async fn learn_lid(&self, client: &Arc<Client>, jid: &Jid, alt: Option<&Jid>) {
    if !jid.is_lid() {
      return;
    }
    let lid = jid.to_non_ad_string();
    {
      let Ok(mut seen) = self.lids_seen.lock() else {
        return;
      };
      if !seen.insert(lid.clone()) {
        return;
      }
    }
    let pn = match alt.filter(|a| a.is_pn()) {
      Some(alt) => Some(alt.to_non_ad_string()),
      None => match client.get_lid_pn_entry(jid).await {
        Ok(entry) => entry.map(|e| pn_jid(&e.phone_number)),
        Err(e) => {
          tracing::debug!(error = %e, "falha ao buscar telefone do lid");
          None
        }
      },
    };
    if let Some(pn) = pn {
      self.send(WaEvent::LidMap { lid, pn }).await;
    }
  }

  /// Busca o nome do grupo uma vez por execução.
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
      match chat_title(&client, &chat).await {
        Ok(name) => {
          let _ = tx
            .send(WaEvent::ChatMeta(ChatMeta {
              jid: key,
              name,
              is_group: true,
              ..Default::default()
            }))
            .await;
        }
        Err(e) => tracing::debug!(error = %e, "falha ao buscar nome do grupo"),
      }
    });
  }

  fn set_qr(&self, code: Option<String>) {
    if let Ok(mut q) = self.qr.lock() {
      *q = code;
    }
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

/// Assunto do grupo (consulta ao WhatsApp). Canais vêm de [`RustWa::channel_map`].
async fn chat_title(client: &Client, jid: &Jid) -> Result<Option<String>> {
  if !jid.is_group() {
    return Ok(None);
  }
  let meta = client
    .groups()
    .get_metadata(jid)
    .await
    .map_err(|e| anyhow!("{e}"))?;
  Ok(non_empty(&meta.subject))
}

/// Fixação padrão quando a mensagem não diz a duração (a do WhatsApp).
const DEFAULT_PIN_SECS: i64 = 7 * 24 * 3600;

/// Apagar para todos e fixar/desafixar mensagem.
fn control_event(message: &wa::Message, info: &MessageInfo) -> Option<WaEvent> {
  let m = message.get_base_message();
  let chat_jid = info.source.chat.to_non_ad_string();
  if let Some(p) = m.protocol_message.as_option() {
    use wa::message::protocol_message::Type;
    if p.r#type == Some(Type::REVOKE) {
      let id = p.key.as_option()?.id.clone()?;
      return Some(WaEvent::Revoked { chat_jid, id });
    }
    return None;
  }
  if let Some(pin) = m.pin_in_chat_message.as_option() {
    use wa::message::pin_in_chat_message::Type;
    let id = pin.key.as_option()?.id.clone()?;
    let until_ms = match pin.r#type {
      Some(Type::PIN_FOR_ALL) => {
        let secs = message
          .message_context_info
          .as_option()
          .and_then(|c| c.message_add_on_duration_in_secs)
          .map_or(DEFAULT_PIN_SECS, i64::from);
        Some((now_secs() + secs) * 1000)
      }
      _ => None,
    };
    return Some(WaEvent::Pinned {
      chat_jid,
      id,
      until_ms,
    });
  }
  None
}

fn convert_live(message: &wa::Message, info: &MessageInfo) -> Option<(Message, Option<MediaRef>)> {
  let Classified {
    kind,
    body,
    quoted_id,
    media,
  } = classify(message)?;
  let src = &info.source;
  let msg = Message {
    id: info.id.to_string(),
    chat_jid: src.chat.to_non_ad_string(),
    sender_jid: Some(src.sender.to_non_ad_string()),
    sender_name: None,
    from_me: src.is_from_me,
    ts: info.timestamp.timestamp(),
    kind,
    body,
    quoted_id,
    status: src.is_from_me.then_some(MsgStatus::Sent),
    // Mensagens nossas (enviadas de outro aparelho) já nascem lidas.
    is_read: src.is_from_me,
    media: media.as_ref().map(MediaRef::info),
    starred: false,
    pinned: false,
  };
  Some((msg, media))
}

/// Conversa do histórico. Na sincronização inicial guarda só as não lidas e a mais nova
/// (ponto de partida para pedir as anteriores ao celular); sob demanda, tudo o que veio.
fn convert_conversation(conv: &wa::Conversation, on_demand: bool) -> Option<WaEvent> {
  let jid: Jid = conv.id.parse().ok()?;
  if jid.is_status_broadcast() || jid.is_newsletter() {
    return None;
  }
  let chat_jid = jid.to_non_ad_string();
  let unread = conv.unread_count.unwrap_or(0);
  let marked_unread = conv.marked_as_unread.unwrap_or(false);

  let mut converted: Vec<(Message, Option<MediaRef>)> = conv
    .messages
    .iter()
    .filter_map(|hm| hm.message.as_option())
    .filter_map(|wmi| convert_history_msg(wmi, &chat_jid, jid.is_group()))
    .collect();
  converted.sort_by_key(|(m, _)| std::cmp::Reverse(m.ts));

  if on_demand {
    // Mensagens antigas pedidas pela pessoa: já lidas, e o resto do chat não muda.
    let mut media = Vec::new();
    let mut messages = Vec::with_capacity(converted.len());
    for (mut m, r) in converted {
      m.is_read = true;
      if let Some(r) = r {
        media.push((m.id.clone(), r));
      }
      messages.push(m);
    }
    return Some(WaEvent::HistoryChat {
      meta: ChatMeta {
        jid: chat_jid,
        is_group: jid.is_group(),
        ..Default::default()
      },
      last_ts: None,
      messages,
      media,
      on_demand: true,
      hidden_id: None,
    });
  }

  // As `unread` mensagens recebidas mais recentes são as não lidas.
  let mut left = unread as usize;
  for (m, _) in converted.iter_mut().filter(|(m, _)| !m.from_me) {
    if left == 0 {
      m.is_read = true;
    } else {
      m.is_read = false;
      left -= 1;
    }
  }
  let mut media = Vec::new();
  let mut messages = Vec::new();
  // A mais nova fica de âncora; se já foi lida, guardada sem aparecer (a conversa abre
  // vazia na primeira vez, e ela surge no primeiro "carregar mensagens").
  let mut hidden_id = None;
  for (i, (m, r)) in converted.into_iter().enumerate() {
    if m.is_read && i > 0 {
      continue;
    }
    if m.is_read {
      hidden_id = Some(m.id.clone());
    }
    if let Some(r) = r {
      media.push((m.id.clone(), r));
    }
    messages.push(m);
  }

  let last_ts = conv
    .conversation_timestamp
    .or(conv.last_msg_timestamp)
    .and_then(|t| i64::try_from(t).ok())
    .or_else(|| messages.first().map(|m| m.ts));

  Some(WaEvent::HistoryChat {
    meta: ChatMeta {
      jid: chat_jid,
      // Conversa individual: o nome vai para `contacts` (ver `conversation_people`).
      name: if jid.is_group() {
        conv.name.as_deref().and_then(non_empty)
      } else {
        None
      },
      is_group: jid.is_group(),
      unread_count: Some(if unread == 0 && marked_unread {
        1
      } else {
        unread
      }),
      archived: conv.archived,
      pinned: conv.pinned.map(|p| p > 0),
      muted_until: conv.mute_end_time.map(history_mute_ms),
    },
    last_ts,
    messages,
    media,
    on_demand: false,
    hidden_id,
  })
}

/// `muteEndTime` do histórico em ms; aceita segundos (valores pequenos demais para ms).
fn history_mute_ms(t: u64) -> i64 {
  let t = i64::try_from(t).unwrap_or(-1);
  if t > 0 && t < 100_000_000_000 {
    t * 1000
  } else {
    t
  }
}

/// Nomes e ligação `@lid` <-> telefone vindos de uma conversa do histórico.
fn conversation_people(conv: &wa::Conversation) -> Vec<WaEvent> {
  let Some(jid) = normalize(Some(&conv.id)) else {
    return vec![];
  };
  if jid.ends_with("@g.us") {
    return group_push_names(conv);
  }
  let mut out = vec![];
  let name = conv
    .name
    .as_deref()
    .or(conv.display_name.as_deref())
    .and_then(clean_name);
  if name.is_some() {
    out.push(WaEvent::Contact(Contact {
      jid: jid.clone(),
      push_name: name,
      ..Default::default()
    }));
  }
  let lid = if jid.ends_with("@lid") {
    Some(jid.clone())
  } else {
    normalize(conv.lid_jid.as_deref())
  };
  let pn = if jid.ends_with("@s.whatsapp.net") {
    Some(jid)
  } else {
    normalize(conv.pn_jid.as_deref())
  };
  if let (Some(lid), Some(pn)) = (lid, pn) {
    out.push(WaEvent::LidMap { lid, pn });
  }
  out
}

/// Apelido de cada participante, pelo `pushName` das mensagens do grupo.
fn group_push_names(conv: &wa::Conversation) -> Vec<WaEvent> {
  let mut seen = HashSet::new();
  conv
    .messages
    .iter()
    .filter_map(|hm| hm.message.as_option())
    .filter_map(|wmi| {
      let sender = wmi
        .key
        .as_option()
        .and_then(|k| k.participant.as_deref())
        .or(wmi.participant.as_deref());
      Some((
        normalize(sender)?,
        wmi.push_name.as_deref().and_then(clean_name)?,
      ))
    })
    .filter(|(jid, _)| seen.insert(jid.clone()))
    .map(|(jid, name)| {
      WaEvent::Contact(Contact {
        jid,
        push_name: Some(name),
        ..Default::default()
      })
    })
    .collect()
}

fn convert_history_msg(
  wmi: &wa::WebMessageInfo,
  chat_jid: &str,
  is_group: bool,
) -> Option<(Message, Option<MediaRef>)> {
  let key = wmi.key.as_option()?;
  let id = key.id.clone()?;
  let from_me = key.from_me.unwrap_or(false);
  let Classified {
    kind,
    body,
    quoted_id,
    media,
  } = classify(wmi.message.as_option()?)?;
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
  let msg = Message {
    id,
    chat_jid: chat_jid.to_string(),
    sender_jid,
    sender_name: None,
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
    media: media.as_ref().map(MediaRef::info),
    starred: false,
    pinned: false,
  };
  Some((msg, media))
}

struct Classified {
  kind: MessageKind,
  /// Texto ou legenda.
  body: Option<String>,
  quoted_id: Option<String>,
  /// Imagem ou figurinha que dá para baixar depois.
  media: Option<MediaRef>,
}

/// `None` para mensagens que não aparecem na conversa (reações, edições,
/// apagamentos, controle de protocolo).
fn classify(message: &wa::Message) -> Option<Classified> {
  let m = message.get_base_message();
  if m.protocol_message.is_set() || m.reaction_message.is_set() {
    return None;
  }

  let caption = m.get_caption().map(str::to_string);
  let mut media = None;
  let (kind, body, ctx) = if let Some(text) = m.conversation.as_deref() {
    (MessageKind::Text, Some(text.to_string()), None)
  } else if let Some(ext) = m.extended_text_message.as_option() {
    (
      MessageKind::Text,
      ext.text.clone(),
      ext.context_info.as_option(),
    )
  } else if let Some(x) = m.image_message.as_option() {
    media = media_ref(
      MessageKind::Image,
      x.direct_path.as_deref(),
      x.media_key.as_deref(),
      x.file_sha256.as_deref(),
      x.file_enc_sha256.as_deref(),
      x.file_length,
      x.mimetype.as_deref(),
      (x.width, x.height, None),
    );
    (MessageKind::Image, caption, x.context_info.as_option())
  } else if let Some(x) = m
    .video_message
    .as_option()
    // Vídeo redondo (PTV) usa o mesmo formato de vídeo.
    .or(m.ptv_message.as_option())
  {
    media = media_ref(
      MessageKind::Video,
      x.direct_path.as_deref(),
      x.media_key.as_deref(),
      x.file_sha256.as_deref(),
      x.file_enc_sha256.as_deref(),
      x.file_length,
      x.mimetype.as_deref(),
      (x.width, x.height, x.seconds),
    );
    (MessageKind::Video, caption, x.context_info.as_option())
  } else if let Some(x) = m.audio_message.as_option() {
    media = media_ref(
      MessageKind::Audio,
      x.direct_path.as_deref(),
      x.media_key.as_deref(),
      x.file_sha256.as_deref(),
      x.file_enc_sha256.as_deref(),
      x.file_length,
      x.mimetype.as_deref(),
      (None, None, x.seconds),
    );
    (MessageKind::Audio, None, x.context_info.as_option())
  } else if let Some(x) = m.document_message.as_option() {
    media = media_ref(
      MessageKind::Doc,
      x.direct_path.as_deref(),
      x.media_key.as_deref(),
      x.file_sha256.as_deref(),
      x.file_enc_sha256.as_deref(),
      x.file_length,
      x.mimetype.as_deref(),
      (None, None, None),
    )
    .map(|r| MediaRef {
      file_name: x.file_name.clone(),
      ..r
    });
    let body = caption.or_else(|| x.file_name.clone());
    (MessageKind::Doc, body, x.context_info.as_option())
  } else if let Some(x) = m.sticker_message.as_option() {
    // Figurinhas Lottie (animação vetorial) não são imagem; ficam como texto.
    if x.is_lottie != Some(true) {
      media = media_ref(
        MessageKind::Sticker,
        x.direct_path.as_deref(),
        x.media_key.as_deref(),
        x.file_sha256.as_deref(),
        x.file_enc_sha256.as_deref(),
        x.file_length,
        x.mimetype.as_deref(),
        (x.width, x.height, None),
      );
    }
    (MessageKind::Sticker, None, x.context_info.as_option())
  } else if has_visible_content(m) {
    (MessageKind::Other, None, None)
  } else {
    return None;
  };
  let quoted_id = ctx.and_then(|c| c.stanza_id.clone());
  Some(Classified {
    kind,
    body,
    quoted_id,
    media,
  })
}

/// Referência de download; `None` se faltar o que o CDN exige.
#[allow(clippy::too_many_arguments)]
fn media_ref(
  kind: MessageKind,
  direct_path: Option<&str>,
  media_key: Option<&[u8]>,
  file_sha256: Option<&[u8]>,
  file_enc_sha256: Option<&[u8]>,
  file_length: Option<u64>,
  mimetype: Option<&str>,
  (width, height, seconds): (Option<u32>, Option<u32>, Option<u32>),
) -> Option<MediaRef> {
  Some(MediaRef {
    kind,
    direct_path: direct_path.filter(|p| !p.is_empty())?.to_string(),
    media_key: media_key.map(<[u8]>::to_vec),
    file_sha256: file_sha256?.to_vec(),
    file_enc_sha256: file_enc_sha256.map(<[u8]>::to_vec),
    file_length: file_length?,
    mimetype: mimetype.map(str::to_string),
    width,
    height,
    seconds,
    file_name: None,
  })
}

/// Outros tipos que aparecem na conversa (localização, contato, enquete...).
fn has_visible_content(m: &wa::Message) -> bool {
  m.location_message.is_set()
    || m.live_location_message.is_set()
    || m.contact_message.is_set()
    || m.contacts_array_message.is_set()
    || m.poll_creation_message.is_set()
    || m.poll_creation_message_v3.is_set()
    || m.event_message.is_set()
}

fn parse_jid(s: &str) -> Result<Jid> {
  s.parse().map_err(|e| anyhow!("jid inválido: {e}"))
}

fn non_empty(s: &str) -> Option<String> {
  let t = s.trim();
  (!t.is_empty()).then(|| t.to_string())
}

/// Nome de pessoa utilizável; descarta vazios e números mascarados ("+55∙∙∙65").
fn clean_name(s: &str) -> Option<String> {
  non_empty(s).filter(|n| !n.contains('\u{2219}'))
}

/// Jid sem dispositivo (`5511...@s.whatsapp.net`, `123...@lid`), se válido.
fn normalize(jid: Option<&str>) -> Option<String> {
  jid?.parse::<Jid>().ok().map(|j| j.to_non_ad_string())
}

fn pn_jid(phone: &str) -> String {
  format!("{phone}@s.whatsapp.net")
}

fn now_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}
