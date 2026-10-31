//! Integração com o WhatsApp, isolada atrás de [`WhatsApp`] e [`WaEvent`].
//!
//! Só `whatsapp_rust_impl` importa tipos da biblioteca; o resto do backend
//! conversa com este módulo pelos tipos de [`crate::model`].

mod qr;
mod whatsapp_rust_impl;

use anyhow::Result;
use async_trait::async_trait;

use crate::model::{
  ChatAction, ChatMeta, ConnState, Contact, MediaRef, Message, MessageAction, MsgStatus, Quote,
};

pub use qr::svg as qr_svg;
pub use whatsapp_rust_impl::{start, WaOptions};

/// Eventos já normalizados que o backend grava no banco e repassa ao front.
#[derive(Debug, Clone)]
pub enum WaEvent {
  Connection(ConnState),
  /// Mensagem apagada para todos por quem mandou.
  Revoked {
    chat_jid: String,
    id: String,
  },
  /// Mensagem apagada para nós em outro aparelho.
  DeletedForMe {
    chat_jid: String,
    id: String,
  },
  /// Estrela posta/tirada em outro aparelho.
  Starred {
    chat_jid: String,
    id: String,
    starred: bool,
  },
  /// Mensagem fixada (até `until_ms`) ou desafixada (`None`).
  Pinned {
    chat_jid: String,
    id: String,
    until_ms: Option<i64>,
  },
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

/// Arquivo para enviar (imagem, vídeo ou documento).
#[derive(Debug, Clone)]
pub struct OutgoingMedia {
  /// `Image`, `Video` ou `Doc`.
  pub kind: crate::model::MessageKind,
  pub bytes: Vec<u8>,
  pub mimetype: String,
  pub file_name: Option<String>,
  pub caption: Option<String>,
  pub width: Option<u32>,
  pub height: Option<u32>,
  pub seconds: Option<u32>,
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

  /// Conteúdo do QR de pareamento atual, enquanto não há celular pareado.
  fn pairing_qr(&self) -> Option<String>;

  /// Envia uma mensagem de texto, opcionalmente respondendo (citando) outra.
  async fn send_text(&self, chat_jid: &str, text: &str, quote: Option<&Quote>) -> Result<Sent>;

  /// Favorita, fixa, apaga ou denuncia uma mensagem (também no celular).
  async fn message_action(&self, msg: &Message, action: MessageAction) -> Result<()>;

  /// Encaminha uma mensagem (texto ou mídia, sem subir o arquivo de novo).
  async fn forward(&self, to_jid: &str, msg: &Message, media: Option<&MediaRef>) -> Result<Sent>;

  /// Arquiva, fixa, silencia ou marca como não lida, também no celular.
  async fn chat_action(&self, chat_jid: &str, action: ChatAction) -> Result<()>;

  /// Envia recibos de leitura e marca o chat como lido nos outros aparelhos.
  async fn mark_read(&self, chat_jid: &str, unread: &[UnreadRef]) -> Result<()>;

  /// Sobe o arquivo (criptografado) para o CDN e envia a mensagem. Devolve também a
  /// referência para baixar de novo depois.
  async fn send_media(&self, chat_jid: &str, media: OutgoingMedia) -> Result<(Sent, MediaRef)>;

  /// Baixa e descriptografa uma imagem ou figurinha do CDN (só em memória).
  async fn download_media(&self, media: &MediaRef) -> Result<Vec<u8>>;

  /// Foto de perfil (miniatura) de uma pessoa ou grupo; `None` se não houver.
  async fn profile_picture(&self, jid: &str) -> Result<Option<Vec<u8>>>;

  /// Pede ao WhatsApp a agenda inteira de novo; cada contato salvo chega como
  /// [`WaEvent::Contact`] com `contact_name`.
  async fn resync_contacts(&self) -> Result<()>;

  /// Canais assinados: (jid, nome).
  async fn channels(&self) -> Result<Vec<(String, String)>>;

  /// Assunto do grupo; `None` para outros jids.
  async fn chat_title(&self, jid: &str) -> Result<Option<String>>;

  /// Jid de telefone de um `@lid`, pelo que a sessão já aprendeu (sem rede).
  async fn phone_for_lid(&self, lid_jid: &str) -> Result<Option<String>>;

  /// Fecha a conexão e grava o estado pendente da sessão.
  async fn shutdown(&self);
}
