//! Eventos enviados ao front por SSE (`GET /api/events`).

use serde::Serialize;

use crate::model::{ConnState, Message, MsgStatus};
use crate::store::ChatSummary;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum ApiEvent {
  #[serde(rename = "message.new")]
  MessageNew(Message),
  #[serde(rename = "message.status")]
  MessageStatus {
    chat_jid: String,
    ids: Vec<String>,
    status: MsgStatus,
  },
  /// A mensagem mudou (estrela, fixação, apagada para todos).
  #[serde(rename = "message.updated")]
  MessageUpdated(Message),
  /// A mensagem foi apagada para nós.
  #[serde(rename = "message.deleted")]
  MessageDeleted { chat_jid: String, id: String },
  #[serde(rename = "chat.updated")]
  ChatUpdated(ChatSummary),
  #[serde(rename = "connection.state")]
  ConnectionState { state: ConnState },
}

impl ApiEvent {
  pub fn name(&self) -> &'static str {
    match self {
      Self::MessageNew(_) => "message.new",
      Self::MessageStatus { .. } => "message.status",
      Self::MessageUpdated(_) => "message.updated",
      Self::MessageDeleted { .. } => "message.deleted",
      Self::ChatUpdated(_) => "chat.updated",
      Self::ConnectionState { .. } => "connection.state",
    }
  }
}
