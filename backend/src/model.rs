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
  /// Visualização única: o WhatsApp não entrega o conteúdo a aparelhos conectados.
  ViewOnce,
  /// Apagada para todos por quem mandou.
  Revoked,
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
      Self::ViewOnce => "view_once",
      Self::Revoked => "revoked",
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
      "view_once" => Self::ViewOnce,
      "revoked" => Self::Revoked,
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
  /// Nome do remetente para exibição (agenda, apelido ou telefone); não é gravado.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub sender_name: Option<String>,
  pub from_me: bool,
  /// Unix seconds.
  pub ts: i64,
  pub kind: MessageKind,
  pub body: Option<String>,
  pub quoted_id: Option<String>,
  pub status: Option<MsgStatus>,
  pub is_read: bool,
  /// Imagem ou figurinha que pode ser baixada (`GET .../media`).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub media: Option<MediaInfo>,
  /// Favoritada (estrela).
  #[serde(default)]
  pub starred: bool,
  /// Fixada na conversa (para todos), e ainda dentro do prazo.
  #[serde(default)]
  pub pinned: bool,
}

/// O que o front mostra antes de baixar a mídia (só baixa quando pedem).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaInfo {
  /// Tamanho do arquivo em bytes.
  pub size: u64,
  pub mimetype: Option<String>,
  pub width: Option<u32>,
  pub height: Option<u32>,
  /// Duração (vídeo e áudio).
  pub seconds: Option<u32>,
  /// Nome do arquivo (documento).
  pub file_name: Option<String>,
}

/// Referência para baixar uma mídia do CDN do WhatsApp na hora em que for vista.
///
/// Só isto vai para o banco (caminho, chave e hashes); o arquivo nunca é guardado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
  /// `Image`, `Sticker`, `Video`, `Audio` ou `Doc`.
  pub kind: MessageKind,
  pub direct_path: String,
  pub media_key: Option<Vec<u8>>,
  pub file_sha256: Vec<u8>,
  pub file_enc_sha256: Option<Vec<u8>>,
  pub file_length: u64,
  pub mimetype: Option<String>,
  pub width: Option<u32>,
  pub height: Option<u32>,
  pub seconds: Option<u32>,
  pub file_name: Option<String>,
}

impl MediaRef {
  pub fn info(&self) -> MediaInfo {
    MediaInfo {
      size: self.file_length,
      mimetype: self.mimetype.clone(),
      width: self.width,
      height: self.height,
      seconds: self.seconds,
      file_name: self.file_name.clone(),
    }
  }
}

/// Nomes de uma pessoa (chave: jid `@s.whatsapp.net` ou `@lid`). `None` não sobrescreve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contact {
  pub jid: String,
  /// Nome salvo na agenda do celular.
  pub contact_name: Option<String>,
  /// Nome que a própria pessoa definiu no WhatsApp.
  pub push_name: Option<String>,
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
  /// Silenciada até (ms); `Some(-1)` = para sempre, `Some(0)` = não silenciada.
  pub muted_until: Option<i64>,
}

/// Mensagem citada numa resposta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
  pub id: String,
  pub from_me: bool,
  /// Quem mandou (obrigatório em grupo).
  pub sender_jid: Option<String>,
  /// Texto (ou "[imagem]" etc.) que aparece na citação para quem recebe.
  pub text: String,
}

/// Ação numa mensagem pedida pela interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAction {
  Star(bool),
  /// Fixar para todos por tantos dias (1, 7 ou 30).
  Pin(u32),
  Unpin,
  DeleteForMe,
  /// Apagar para todos (só as nossas).
  DeleteForAll,
  /// Denunciar ao WhatsApp (só as recebidas).
  Report,
}

/// Ação na conversa pedida pela interface (sincroniza com o celular).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAction {
  Archive(bool),
  Pin(bool),
  /// `None` = para sempre; `Some(ms)` = até esse instante.
  Mute(Option<i64>),
  Unmute,
  MarkUnread,
}
