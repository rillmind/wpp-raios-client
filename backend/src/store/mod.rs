//! Banco da aplicação (SQLite): chats, mensagens e não lidas.
//!
//! Separado do `whatsapp.db` da biblioteca. As funções síncronas ficam em
//! [`Db`]; [`Store`] as roda em `spawn_blocking` para uso no runtime async.

mod migrations;

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::model::{
  ChatAction, ChatMeta, Contact, MediaInfo, MediaRef, Message, MessageKind, MsgStatus,
};

/// Item da lista de chats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatSummary {
  pub jid: String,
  /// Grupo: assunto. Conversa individual: nome da agenda, senão o apelido.
  pub name: Option<String>,
  /// Telefone (só dígitos) da conversa individual, quando conhecido.
  pub phone: Option<String>,
  pub is_group: bool,
  pub unread_count: u32,
  pub last_msg_ts: Option<i64>,
  pub last_msg_preview: Option<String>,
  /// Status (tiques) da última mensagem, quando ela é nossa.
  pub last_status: Option<MsgStatus>,
  /// Grupos: quem mandou a última mensagem ("Você", agenda, apelido ou `+telefone`).
  pub last_sender: Option<String>,
  pub archived: bool,
  pub pinned: bool,
  /// Silenciada agora (notificações desligadas no celular).
  pub muted: bool,
  /// Jid de quem mandou a última mensagem do grupo (`""` = nós); vira `last_sender`.
  #[serde(skip)]
  last_sender_jid: Option<String>,
}

/// Mensagem recebida ainda não lida, para recibo de leitura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadMsg {
  pub id: String,
  pub sender_jid: Option<String>,
}

pub struct Db {
  conn: Connection,
}

impl Db {
  pub fn open(path: &Path) -> Result<Self> {
    Self::init(Connection::open(path)?)
  }

  #[cfg(test)]
  pub fn open_in_memory() -> Result<Self> {
    Self::init(Connection::open_in_memory()?)
  }

  fn init(conn: Connection) -> Result<Self> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let mut db = Db { conn };
    migrations::run(&mut db.conn)?;
    Ok(db)
  }

  /// Cria o chat se não existir; campos `None` não sobrescrevem valores salvos.
  pub fn upsert_chat(&self, meta: &ChatMeta) -> Result<()> {
    upsert_chat(&self.conn, meta)
  }

  /// Atualiza `last_msg_ts` só se `ts` for mais novo (preview fica com a mensagem mais nova).
  fn touch_chat(&self, chat_jid: &str, ts: i64, preview: Option<&str>) -> Result<()> {
    self.conn.execute(
      "UPDATE chats SET last_msg_ts = ?2, last_msg_preview = ?3
       WHERE jid = ?1 AND (last_msg_ts IS NULL OR last_msg_ts <= ?2)",
      params![chat_jid, ts, preview],
    )?;
    Ok(())
  }

  /// Insere a mensagem (idempotente). Devolve `true` se ela era nova.
  ///
  /// Mensagem nova recebida e não lida incrementa `unread_count` do chat.
  pub fn insert_message(&self, msg: &Message) -> Result<bool> {
    self.upsert_chat(&ChatMeta {
      jid: msg.chat_jid.clone(),
      is_group: msg.chat_jid.ends_with("@g.us"),
      ..Default::default()
    })?;
    let inserted = insert_row(&self.conn, msg)?;
    if inserted {
      self.touch_chat(&msg.chat_jid, msg.ts, Some(&preview(msg)))?;
      if !msg.from_me && !msg.is_read {
        self.conn.execute(
          "UPDATE chats SET unread_count = unread_count + 1 WHERE jid = ?1",
          params![msg.chat_jid],
        )?;
      }
    }
    Ok(inserted)
  }

  /// Grava um chat da sincronização inicial com suas mensagens recentes.
  ///
  /// O contador vem do WhatsApp (`meta.unread_count`), então as inserções não o incrementam.
  pub fn import_history_chat(
    &mut self,
    meta: &ChatMeta,
    last_ts: Option<i64>,
    msgs: &[Message],
  ) -> Result<()> {
    let tx = self.conn.transaction()?;
    upsert_chat(&tx, meta)?;
    for m in msgs {
      insert_row(&tx, m)?;
    }
    let newest = msgs.iter().max_by_key(|m| m.ts);
    if let Some(ts) = newest.map(|m| m.ts).or(last_ts) {
      tx.execute(
        "UPDATE chats SET last_msg_ts = ?2, last_msg_preview = COALESCE(?3, last_msg_preview)
         WHERE jid = ?1 AND (last_msg_ts IS NULL OR last_msg_ts <= ?2)",
        params![meta.jid, ts, newest.map(preview)],
      )?;
    }
    tx.commit()?;
    Ok(())
  }

  /// Reflete no banco uma ação já feita no WhatsApp.
  pub fn apply_action(&self, chat_jid: &str, action: ChatAction) -> Result<()> {
    let mut meta = ChatMeta {
      jid: chat_jid.to_string(),
      ..Default::default()
    };
    match action {
      ChatAction::Archive(archived) => {
        meta.archived = Some(archived);
        // O WhatsApp não deixa conversa arquivada fixada.
        if archived {
          meta.pinned = Some(false);
        }
      }
      ChatAction::Pin(pinned) => meta.pinned = Some(pinned),
      ChatAction::Mute(until) => meta.muted_until = Some(until.unwrap_or(-1)),
      ChatAction::Unmute => meta.muted_until = Some(0),
      ChatAction::MarkUnread => {
        let current = self.get_chat(chat_jid)?.map_or(0, |c| c.unread_count);
        return self.set_unread(chat_jid, current.max(1));
      }
    }
    self.upsert_chat(&meta)
  }

  pub fn set_unread(&self, chat_jid: &str, count: u32) -> Result<()> {
    self.conn.execute(
      "UPDATE chats SET unread_count = ?2 WHERE jid = ?1",
      params![chat_jid, count],
    )?;
    Ok(())
  }

  /// Mensagens recebidas não lidas, da mais antiga para a mais nova.
  pub fn unread_messages(&self, chat_jid: &str) -> Result<Vec<UnreadMsg>> {
    let mut stmt = self.conn.prepare(
      "SELECT id, sender_jid FROM messages
       WHERE chat_jid = ?1 AND from_me = 0 AND is_read = 0 ORDER BY ts, rowid",
    )?;
    let rows = stmt.query_map(params![chat_jid], |r| {
      Ok(UnreadMsg {
        id: r.get(0)?,
        sender_jid: r.get(1)?,
      })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// Marca tudo como lido. Devolve as mensagens que estavam não lidas.
  pub fn mark_chat_read(&mut self, chat_jid: &str) -> Result<Vec<UnreadMsg>> {
    let unread = self.unread_messages(chat_jid)?;
    let tx = self.conn.transaction()?;
    tx.execute(
      "UPDATE messages SET is_read = 1 WHERE chat_jid = ?1 AND is_read = 0",
      params![chat_jid],
    )?;
    tx.execute(
      "UPDATE chats SET unread_count = 0 WHERE jid = ?1",
      params![chat_jid],
    )?;
    tx.commit()?;
    Ok(unread)
  }

  /// Mensagens lidas em outro aparelho: marca como lidas e recalcula o contador.
  pub fn mark_messages_read(&mut self, chat_jid: &str, ids: &[String]) -> Result<()> {
    let tx = self.conn.transaction()?;
    let mut changed = 0;
    for id in ids {
      changed += tx.execute(
        "UPDATE messages SET is_read = 1 WHERE chat_jid = ?1 AND id = ?2 AND is_read = 0",
        params![chat_jid, id],
      )?;
    }
    if changed > 0 {
      tx.execute(
        "UPDATE chats SET unread_count = MAX(unread_count - ?2, 0) WHERE jid = ?1",
        params![chat_jid, changed as i64],
      )?;
    }
    tx.commit()?;
    Ok(())
  }

  /// Atualiza o status de mensagens enviadas, sem regredir (read > delivered > sent).
  pub fn set_status(
    &self,
    chat_jid: &str,
    ids: &[String],
    status: MsgStatus,
  ) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    for id in ids {
      let current: Option<Option<String>> = self
        .conn
        .query_row(
          "SELECT status FROM messages WHERE chat_jid = ?1 AND id = ?2 AND from_me = 1",
          params![chat_jid, id],
          |r| r.get(0),
        )
        .optional()?;
      let Some(current) = current else { continue };
      if current
        .as_deref()
        .and_then(MsgStatus::parse)
        .is_some_and(|c| c >= status)
      {
        continue;
      }
      self.conn.execute(
        "UPDATE messages SET status = ?3 WHERE chat_jid = ?1 AND id = ?2",
        params![chat_jid, id, status.as_str()],
      )?;
      changed.push(id.clone());
    }
    Ok(changed)
  }

  /// Guarda a referência de download da mídia de uma mensagem (não o arquivo).
  pub fn insert_media(&self, chat_jid: &str, msg_id: &str, m: &MediaRef) -> Result<()> {
    self.conn.execute(
      "INSERT INTO media (chat_jid, msg_id, kind, direct_path, media_key, file_sha256,
         file_enc_sha256, file_length, mimetype, width, height, seconds)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
       ON CONFLICT(chat_jid, msg_id) DO NOTHING",
      params![
        chat_jid,
        msg_id,
        m.kind.as_str(),
        m.direct_path,
        m.media_key,
        m.file_sha256,
        m.file_enc_sha256,
        m.file_length as i64,
        m.mimetype,
        m.width,
        m.height,
        m.seconds,
      ],
    )?;
    Ok(())
  }

  pub fn get_media(&self, chat_jid: &str, msg_id: &str) -> Result<Option<MediaRef>> {
    Ok(
      self
        .conn
        .query_row(
          "SELECT kind, direct_path, media_key, file_sha256, file_enc_sha256, file_length,
             mimetype, width, height, seconds
           FROM media WHERE chat_jid = ?1 AND msg_id = ?2",
          params![chat_jid, msg_id],
          |r| {
            let kind: String = r.get(0)?;
            let len: i64 = r.get(5)?;
            Ok(MediaRef {
              kind: MessageKind::parse(&kind),
              direct_path: r.get(1)?,
              media_key: r.get(2)?,
              file_sha256: r.get(3)?,
              file_enc_sha256: r.get(4)?,
              file_length: u64::try_from(len).unwrap_or(0),
              mimetype: r.get(6)?,
              width: r.get(7)?,
              height: r.get(8)?,
              seconds: r.get(9)?,
            })
          },
        )
        .optional()?,
    )
  }

  /// Tarefa única ainda não feita (criada por migração).
  pub fn has_pending_task(&self, name: &str) -> Result<bool> {
    Ok(
      self
        .conn
        .query_row(
          "SELECT 1 FROM pending_tasks WHERE name = ?1",
          params![name],
          |_| Ok(()),
        )
        .optional()?
        .is_some(),
    )
  }

  pub fn count_agenda_names(&self) -> Result<u32> {
    Ok(self.conn.query_row(
      "SELECT COUNT(*) FROM contacts WHERE contact_name IS NOT NULL",
      [],
      |r| r.get(0),
    )?)
  }

  pub fn finish_pending_task(&self, name: &str) -> Result<()> {
    self
      .conn
      .execute("DELETE FROM pending_tasks WHERE name = ?1", params![name])?;
    Ok(())
  }

  /// Grava nomes de uma pessoa; campos `None` não apagam os salvos.
  pub fn upsert_contact(&self, c: &Contact) -> Result<()> {
    self.conn.execute(
      "INSERT INTO contacts (jid, contact_name, push_name) VALUES (?1, ?2, ?3)
       ON CONFLICT(jid) DO UPDATE SET
         contact_name = COALESCE(excluded.contact_name, contacts.contact_name),
         push_name    = COALESCE(excluded.push_name, contacts.push_name)",
      params![c.jid, c.contact_name, c.push_name],
    )?;
    Ok(())
  }

  /// Liga um `@lid` ao jid de telefone. Devolve `true` se mudou algo.
  pub fn set_lid_map(&self, lid: &str, pn: &str) -> Result<bool> {
    let n = self.conn.execute(
      "INSERT INTO lid_map (lid, pn) VALUES (?1, ?2)
       ON CONFLICT(lid) DO UPDATE SET pn = excluded.pn WHERE lid_map.pn <> excluded.pn",
      params![lid, pn],
    )?;
    Ok(n > 0)
  }

  /// `@lid` (conversas individuais e remetentes em grupos) sem telefone conhecido.
  pub fn unmapped_lids(&self) -> Result<Vec<String>> {
    let mut stmt = self.conn.prepare(
      "SELECT jid FROM chats WHERE jid LIKE '%@lid'
       UNION
       SELECT DISTINCT sender_jid FROM messages WHERE sender_jid LIKE '%@lid'
       EXCEPT
       SELECT lid FROM lid_map",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// Grupos ainda sem nome.
  pub fn untitled_groups(&self) -> Result<Vec<String>> {
    let mut stmt = self
      .conn
      .prepare("SELECT jid FROM chats WHERE name IS NULL AND jid LIKE '%@g.us'")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// Chats cujo nome depende dos nomes de `jid` (ele mesmo e o outro lado do mapa).
  pub fn chats_of_person(&self, jid: &str) -> Result<Vec<ChatSummary>> {
    let mut stmt = self.conn.prepare(&format!(
      "{CHAT_SELECT}
       WHERE c.jid = ?1
          OR c.jid IN (SELECT lid FROM lid_map WHERE pn = ?1)
          OR c.jid IN (SELECT pn FROM lid_map WHERE lid = ?1)"
    ))?;
    let rows = stmt.query_map(params![jid], chat_from_row)?;
    self.name_senders(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// Preenche `last_sender` dos grupos com o nome de exibição de quem mandou.
  fn name_senders(&self, mut chats: Vec<ChatSummary>) -> Result<Vec<ChatSummary>> {
    for c in &mut chats {
      c.last_sender = match c.last_sender_jid.take().as_deref() {
        None => None,
        Some("") => Some("Você".into()),
        Some(jid) => self.person_name(jid)?,
      };
    }
    Ok(chats)
  }

  /// Nome de exibição de uma pessoa: agenda, apelido ou `+telefone`.
  pub fn person_name(&self, jid: &str) -> Result<Option<String>> {
    Ok(
      self
        .conn
        .query_row(
          &format!(
            "SELECT {}
             FROM (SELECT ?1 AS jid) m
             LEFT JOIN lid_map sm ON sm.lid = m.jid
             LEFT JOIN contacts sd ON sd.jid = m.jid
             LEFT JOIN contacts sp ON sp.jid = sm.pn",
            sender_name_sql("m.jid")
          ),
          params![jid],
          |r| Ok(display_name(r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .flatten(),
    )
  }

  pub fn get_chat(&self, jid: &str) -> Result<Option<ChatSummary>> {
    let chat = self
      .conn
      .query_row(
        &format!("{CHAT_SELECT} WHERE c.jid = ?1"),
        params![jid],
        chat_from_row,
      )
      .optional()?;
    Ok(self.name_senders(chat.into_iter().collect())?.pop())
  }

  /// Fixados primeiro, depois por atividade, do mais recente; `before_ts` pagina.
  pub fn list_chats(&self, limit: u32, before_ts: Option<i64>) -> Result<Vec<ChatSummary>> {
    let mut stmt = self.conn.prepare(&format!(
      "{CHAT_SELECT}
       WHERE (?1 IS NULL OR c.last_msg_ts < ?1)
       ORDER BY c.pinned DESC, c.last_msg_ts IS NULL, c.last_msg_ts DESC
       LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![before_ts, limit], chat_from_row)?;
    self.name_senders(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// Últimas `limit` mensagens antes de `before` (exclusivo), em ordem cronológica.
  pub fn last_messages(
    &self,
    chat_jid: &str,
    limit: u32,
    before: Option<i64>,
  ) -> Result<Vec<Message>> {
    let mut stmt = self.conn.prepare(&format!(
      "{} WHERE m.chat_jid = ?1 AND (?2 IS NULL OR m.ts < ?2)
       ORDER BY m.ts DESC, m.rowid DESC
       LIMIT ?3",
      msg_select()
    ))?;
    let rows = stmt.query_map(params![chat_jid, before, limit], message_from_row)?;
    let mut msgs = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    msgs.reverse();
    Ok(msgs)
  }

  /// Primeira (mais antiga) mensagem recebida não lida.
  pub fn first_unread(&self, chat_jid: &str) -> Result<Option<Message>> {
    Ok(
      self
        .conn
        .query_row(
          &format!(
            "{} WHERE m.chat_jid = ?1 AND m.from_me = 0 AND m.is_read = 0
             ORDER BY m.ts ASC, m.rowid ASC
             LIMIT 1",
            msg_select()
          ),
          params![chat_jid],
          message_from_row,
        )
        .optional()?,
    )
  }

  /// Quantas mensagens existem a partir de `ts` (inclusive), para cobrir todas as não lidas.
  pub fn count_since(&self, chat_jid: &str, ts: i64) -> Result<u32> {
    Ok(self.conn.query_row(
      "SELECT COUNT(*) FROM messages WHERE chat_jid = ?1 AND ts >= ?2",
      params![chat_jid, ts],
      |r| r.get(0),
    )?)
  }

  /// Força a gravação do WAL no arquivo principal (desligamento limpo).
  pub fn checkpoint(&self) -> Result<()> {
    self
      .conn
      .pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
    Ok(())
  }
}

fn upsert_chat(conn: &Connection, meta: &ChatMeta) -> Result<()> {
  conn.execute(
    "INSERT INTO chats (jid, name, is_group, unread_count, archived, pinned, muted_until)
     VALUES (?1, ?2, ?3, COALESCE(?4, 0), COALESCE(?5, 0), COALESCE(?6, 0), ?7)
     ON CONFLICT(jid) DO UPDATE SET
       name         = COALESCE(excluded.name, chats.name),
       is_group     = MAX(chats.is_group, excluded.is_group),
       unread_count = COALESCE(?4, chats.unread_count),
       archived     = COALESCE(?5, chats.archived),
       pinned       = COALESCE(?6, chats.pinned),
       muted_until  = COALESCE(?7, chats.muted_until)",
    params![
      meta.jid,
      meta.name,
      meta.is_group,
      meta.unread_count,
      meta.archived,
      meta.pinned,
      meta.muted_until
    ],
  )?;
  Ok(())
}

/// Insere a linha da mensagem; `false` se ela já existia.
fn insert_row(conn: &Connection, m: &Message) -> Result<bool> {
  let n = conn.execute(
    "INSERT INTO messages
       (id, chat_jid, sender_jid, from_me, ts, kind, body, quoted_id, status, is_read)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
     ON CONFLICT(chat_jid, id) DO NOTHING",
    params![
      m.id,
      m.chat_jid,
      m.sender_jid,
      m.from_me,
      m.ts,
      m.kind.as_str(),
      m.body,
      m.quoted_id,
      m.status.map(MsgStatus::as_str),
      m.is_read,
    ],
  )?;
  Ok(n == 1)
}

// Nome de conversa individual: agenda (pelo telefone, senão pelo próprio jid), depois
// apelido (o do jid da conversa é o mais recente), depois o que veio do histórico.
const CHAT_SELECT: &str = "SELECT c.jid,
     CASE WHEN c.is_group THEN c.name
          ELSE COALESCE(pc.contact_name, dc.contact_name, dc.push_name, pc.push_name, c.name)
     END,
     COALESCE(lm.pn, CASE WHEN c.jid LIKE '%@s.whatsapp.net' THEN c.jid END),
     c.is_group, c.unread_count, c.last_msg_ts, c.last_msg_preview, c.archived, c.pinned,
     (SELECT CASE WHEN lm2.from_me THEN lm2.status END FROM messages lm2
       WHERE lm2.chat_jid = c.jid ORDER BY lm2.ts DESC, lm2.rowid DESC LIMIT 1),
     CASE WHEN c.is_group THEN
       (SELECT CASE WHEN lm3.from_me THEN '' ELSE lm3.sender_jid END FROM messages lm3
         WHERE lm3.chat_jid = c.jid ORDER BY lm3.ts DESC, lm3.rowid DESC LIMIT 1)
     END,
     c.muted_until
   FROM chats c
   LEFT JOIN lid_map lm ON lm.lid = c.jid
   LEFT JOIN contacts dc ON dc.jid = c.jid
   LEFT JOIN contacts pc ON pc.jid = lm.pn";

/// Duas colunas: nome (mesma precedência de [`CHAT_SELECT`]) e jid de telefone, para
/// [`display_name`]. Espera os joins `sm` (lid_map), `sd` (contato do jid) e `sp`
/// (contato do telefone).
fn sender_name_sql(jid: &str) -> String {
  format!(
    "COALESCE(sp.contact_name, sd.contact_name, sd.push_name, sp.push_name),
     COALESCE(sm.pn, CASE WHEN {jid} LIKE '%@s.whatsapp.net' THEN {jid} END)"
  )
}

/// Nome de exibição; sem nome, `+telefone` quando conhecido.
fn display_name(name: Option<String>, pn: Option<String>) -> Option<String> {
  name.or_else(|| pn.map(|p| format!("+{}", jid_user(&p))))
}

fn msg_select() -> String {
  format!(
    "SELECT m.id, m.chat_jid, m.sender_jid, m.from_me, m.ts, m.kind, m.body, m.quoted_id,
       m.status, m.is_read, {},
       md.msg_id IS NOT NULL, md.mimetype, md.width, md.height, md.file_length, md.seconds
     FROM messages m
     LEFT JOIN media md ON md.chat_jid = m.chat_jid AND md.msg_id = m.id
     LEFT JOIN lid_map sm ON sm.lid = m.sender_jid
     LEFT JOIN contacts sd ON sd.jid = m.sender_jid
     LEFT JOIN contacts sp ON sp.jid = sm.pn",
    sender_name_sql("m.sender_jid")
  )
}

/// Parte antes do `@` (os dígitos do telefone num jid `@s.whatsapp.net`).
fn jid_user(jid: &str) -> &str {
  jid.split_once('@').map_or(jid, |(user, _)| user)
}

fn chat_from_row(r: &Row<'_>) -> rusqlite::Result<ChatSummary> {
  let pn: Option<String> = r.get(2)?;
  Ok(ChatSummary {
    jid: r.get(0)?,
    name: r.get(1)?,
    phone: pn.as_deref().map(|p| jid_user(p).to_string()),
    is_group: r.get(3)?,
    unread_count: r.get(4)?,
    last_msg_ts: r.get(5)?,
    last_msg_preview: r.get(6)?,
    archived: r.get(7)?,
    pinned: r.get(8)?,
    last_status: r
      .get::<_, Option<String>>(9)?
      .as_deref()
      .and_then(MsgStatus::parse),
    last_sender: None,
    last_sender_jid: r.get(10)?,
    muted: is_muted(r.get(11)?),
  })
}

/// `muted_until`: -1 = para sempre; no futuro = silenciada.
fn is_muted(until_ms: Option<i64>) -> bool {
  let now_ms = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis() as i64)
    .unwrap_or(0);
  matches!(until_ms, Some(u) if u == -1 || u > now_ms)
}

fn message_from_row(r: &Row<'_>) -> rusqlite::Result<Message> {
  let kind: String = r.get(5)?;
  let status: Option<String> = r.get(8)?;
  Ok(Message {
    id: r.get(0)?,
    chat_jid: r.get(1)?,
    sender_jid: r.get(2)?,
    sender_name: display_name(r.get(10)?, r.get(11)?),
    from_me: r.get(3)?,
    ts: r.get(4)?,
    kind: MessageKind::parse(&kind),
    body: r.get(6)?,
    quoted_id: r.get(7)?,
    status: status.as_deref().and_then(MsgStatus::parse),
    is_read: r.get(9)?,
    media: if r.get(12)? {
      let size: i64 = r.get(16)?;
      Some(MediaInfo {
        size: u64::try_from(size).unwrap_or(0),
        mimetype: r.get(13)?,
        width: r.get(14)?,
        height: r.get(15)?,
        seconds: r.get(17)?,
      })
    } else {
      None
    },
  })
}

/// Texto curto para a lista de chats.
fn preview(msg: &Message) -> String {
  let base = match (msg.kind, msg.body.as_deref()) {
    (MessageKind::Text, Some(b)) => b.to_string(),
    (kind, body) => {
      let tag = match kind {
        MessageKind::Image => "[imagem]",
        MessageKind::Audio => "[áudio]",
        MessageKind::Video => "[vídeo]",
        MessageKind::Doc => "[documento]",
        MessageKind::Sticker => "[figurinha]",
        MessageKind::ViewOnce => "[visualização única]",
        _ => "[mensagem]",
      };
      match body {
        Some(b) if !b.is_empty() => format!("{tag} {b}"),
        _ => tag.to_string(),
      }
    }
  };
  base.chars().take(120).collect()
}

/// Handle assíncrono e clonável para o banco.
#[derive(Clone)]
pub struct Store {
  db: Arc<Mutex<Db>>,
}

impl Store {
  pub fn new(db: Db) -> Self {
    Self {
      db: Arc::new(Mutex::new(db)),
    }
  }

  /// Roda `f` com o banco em uma thread de bloqueio.
  pub async fn run<T, F>(&self, f: F) -> Result<T>
  where
    T: Send + 'static,
    F: FnOnce(&mut Db) -> Result<T> + Send + 'static,
  {
    let db = Arc::clone(&self.db);
    tokio::task::spawn_blocking(move || {
      let mut guard = db
        .lock()
        .map_err(|_| anyhow!("mutex do banco envenenado"))?;
      f(&mut guard)
    })
    .await?
  }
}

#[cfg(test)]
mod tests;
