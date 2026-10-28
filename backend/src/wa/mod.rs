//! Integração com o WhatsApp, isolada atrás de [`WhatsApp`] e [`WaEvent`].
//!
//! Só `whatsapp_rust_impl` importa tipos da biblioteca; o resto do backend
//! conversa com este módulo pelos tipos de [`crate::model`].

mod qr;
mod whatsapp_rust_impl;

use anyhow::Result;
use async_trait::async_trait;

use crate::model::{ChatMeta, ConnState, Message, MsgStatus};

pub use whatsapp_rust_impl::{start, WaOptions};

/// Eventos já normalizados que o backend grava no banco e repassa ao front.
#[derive(Debug, Clone)]
pub enum WaEvent {
  Connection(ConnState),
  /// Mensagem nova (ao vivo ou entregue na reconexão).
  Message(Message),
  /// Chat vindo da sincronização inicial, com as mensagens recentes dele.
  HistoryChat {
    meta: ChatMeta,
    last_ts: Option<i64>,
    messages: Vec<Message>,
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
  /// Nome de contato ou grupo descoberto/alterado.
  ChatMeta(ChatMeta),
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

  /// Fecha a conexão e grava o estado pendente da sessão.
  async fn shutdown(&self);
}
