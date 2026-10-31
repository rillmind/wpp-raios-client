//! Integração com o WhatsApp, isolada atrás de [`WhatsApp`] e [`WaEvent`].
//!
//! Só `whatsapp_rust_impl` importa tipos da biblioteca; o resto do backend
//! conversa com este módulo pelos tipos de [`crate::model`].

mod qr;
mod whatsapp_rust_impl;

use anyhow::Result;
use async_trait::async_trait;

use crate::model::{ChatMeta, ConnState, Contact, MediaRef, Message, MsgStatus};

pub use whatsapp_rust_impl::{start, WaOptions};

/// Eventos já normalizados que o backend grava no banco e repassa ao front.
#[derive(Debug, Clone)]
pub enum WaEvent {
  Connection(ConnState),
  /// Mensagem nova (ao vivo ou entregue na reconexão), com a referência da mídia.
  Message {
    msg: Message,
    media: Option<Box<MediaRef>>,
  },
  /// Chat vindo da sincronização inicial, com as mensagens recentes dele.
  HistoryChat {
    meta: ChatMeta,
    last_ts: Option<i64>,
    messages: Vec<Message>,
    /// Referências de mídia por id de mensagem.
    media: Vec<(String, MediaRef)>,
  },
  /// Recibo de mensagem enviada por nós.
  Status {
    chat_jid: String,
    ids: Vec<String>,
    status: MsgStatus,
  },
  /// Mensagens recebidas foram lidas em outro aparelho nosso (celular).
  ReadElsewhere {
    chat_jid: String,
    ids: Vec<String>,
  },
  /// Chat marcado como lido/não lido em outro aparelho.
  ChatRead {
    chat_jid: String,
    read: bool,
  },
  /// Nome de grupo, arquivamento ou fixação alterados.
  ChatMeta(ChatMeta),
  /// Nome da agenda ou apelido de uma pessoa.
  Contact(Contact),
  /// O `@lid` corresponde a este jid de telefone (`@s.whatsapp.net`).
  LidMap {
    lid: String,
    pn: String,
  },
}

/// Referência a uma mensagem recebida, para recibo de leitura.
#[derive(Debug, Clone)]
pub struct UnreadRef {
  pub id: String,
  /// Remetente; necessário em grupos.
  pub sender_jid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Sent {
  pub id: String,
  pub ts: i64,
}

#[async_trait]
pub trait WhatsApp: Send + Sync {
  fn state(&self) -> ConnState;

  /// Envia uma mensagem de texto.
  async fn send_text(&self, chat_jid: &str, text: &str) -> Result<Sent>;

  /// Envia recibos de leitura e marca o chat como lido nos outros aparelhos.
  async fn mark_read(&self, chat_jid: &str, unread: &[UnreadRef]) -> Result<()>;

  /// Baixa e descriptografa uma imagem ou figurinha do CDN (só em memória).
  async fn download_media(&self, media: &MediaRef) -> Result<Vec<u8>>;

  /// Foto de perfil (miniatura) de uma pessoa ou grupo; `None` se não houver.
  async fn profile_picture(&self, jid: &str) -> Result<Option<Vec<u8>>>;

  /// Pede ao WhatsApp a agenda inteira de novo; cada contato salvo chega como
  /// [`WaEvent::Contact`] com `contact_name`.
  async fn resync_contacts(&self) -> Result<()>;

  /// Jid de telefone de um `@lid`, pelo que a sessão já aprendeu (sem rede).
  async fn phone_for_lid(&self, lid_jid: &str) -> Result<Option<String>>;

  /// Fecha a conexão e grava o estado pendente da sessão.
  async fn shutdown(&self);
}
