//! Tipos de domínio compartilhados entre `wa`, `store` e `api`.
//!
//! Nada aqui depende da biblioteca do WhatsApp: é o contrato que permite
//! trocar a implementação de `wa` sem mexer no resto.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
  Text,
  Image,
  Audio,
  Video,
  Doc,
  Sticker,
  Other,
}

impl MessageKind {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Text => "text",
      Self::Image => "image",
      Self::Audio => "audio",
      Self::Video => "video",
      Self::Doc => "doc",
      Self::Sticker => "sticker",
      Self::Other => "other",
    }
  }

  pub fn parse(s: &str) -> Self {
    match s {
      "text" => Self::Text,
      "image" => Self::Image,
      "audio" => Self::Audio,
      "video" => Self::Video,
      "doc" => Self::Doc,
      "sticker" => Self::Sticker,
      _ => Self::Other,
    }
  }
}

/// Estado de entrega de uma mensagem enviada por nós.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgStatus {
  Sent,
  Delivered,
  Read,
}

impl MsgStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Sent => "sent",
      Self::Delivered => "delivered",
      Self::Read => "read",
    }
  }

  pub fn parse(s: &str) -> Option<Self> {
    match s {
      "sent" => Some(Self::Sent),
      "delivered" => Some(Self::Delivered),
      "read" => Some(Self::Read),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnState {
  Connecting,
  WaitingQr,
  Connected,
  Disconnected,
  LoggedOut,
}

/// Mensagem normalizada, vinda ao vivo ou do pacote de histórico recente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
  pub id: String,
  pub chat_jid: String,
  pub sender_jid: Option<String>,
  pub from_me: bool,
  /// Unix seconds.
  pub ts: i64,
  pub kind: MessageKind,
  pub body: Option<String>,
  pub quoted_id: Option<String>,
  pub status: Option<MsgStatus>,
  pub is_read: bool,
}

/// Metadados de chat. Campos `None` não sobrescrevem o que já está no banco.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatMeta {
  pub jid: String,
  pub name: Option<String>,
  pub is_group: bool,
  pub unread_count: Option<u32>,
  pub archived: Option<bool>,
  pub pinned: Option<bool>,
}
