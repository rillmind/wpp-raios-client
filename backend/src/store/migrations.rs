//! Migrações versionadas por `PRAGMA user_version`.

use anyhow::Result;
use rusqlite::Connection;

pub(super) const MIGRATIONS: &[&str] = &[
  // 1: schema inicial (plano, seção 5).
  "CREATE TABLE chats (
     jid              TEXT PRIMARY KEY,
     name             TEXT,
     is_group         INTEGER NOT NULL DEFAULT 0,
     unread_count     INTEGER NOT NULL DEFAULT 0,
     last_msg_ts      INTEGER,
     last_msg_preview TEXT,
     archived         INTEGER NOT NULL DEFAULT 0,
     pinned           INTEGER NOT NULL DEFAULT 0
   );
   CREATE TABLE messages (
     id         TEXT NOT NULL,
     chat_jid   TEXT NOT NULL REFERENCES chats(jid),
     sender_jid TEXT,
     from_me    INTEGER NOT NULL,
     ts         INTEGER NOT NULL,
     kind       TEXT NOT NULL,
     body       TEXT,
     quoted_id  TEXT,
     status     TEXT,
     is_read    INTEGER NOT NULL DEFAULT 0,
     PRIMARY KEY (chat_jid, id)
   );
   CREATE INDEX idx_messages_chat_ts ON messages(chat_jid, ts DESC);
   CREATE INDEX idx_chats_last_ts ON chats(last_msg_ts DESC);",
  // 2: nomes de contatos separados dos chats, e a ligação @lid -> telefone.
  // Os nomes de conversas individuais saem de `chats.name` (que fica só para grupos):
  // os de chats por telefone eram, na maioria, nomes da agenda; os de chats @lid,
  // apelidos. Nomes mascarados ("+55∙∙∙65") são descartados, e as entradas por
  // telefone sem nenhuma mensagem (só existiam para guardar o nome) saem da lista.
  "CREATE TABLE contacts (
     jid          TEXT PRIMARY KEY,
     contact_name TEXT,
     push_name    TEXT
   );
   CREATE TABLE lid_map (
     lid TEXT PRIMARY KEY,
     pn  TEXT NOT NULL
   );
   CREATE INDEX idx_lid_map_pn ON lid_map(pn);
   INSERT INTO contacts (jid, contact_name)
     SELECT jid, name FROM chats
     WHERE jid LIKE '%@s.whatsapp.net' AND name IS NOT NULL;
   INSERT INTO contacts (jid, push_name)
     SELECT jid, name FROM chats
     WHERE jid NOT LIKE '%@s.whatsapp.net' AND is_group = 0
       AND name IS NOT NULL AND name NOT LIKE '%∙%';
   UPDATE chats SET name = NULL WHERE is_group = 0;
   DELETE FROM chats
     WHERE jid LIKE '%@s.whatsapp.net' AND last_msg_ts IS NULL
       AND NOT EXISTS (SELECT 1 FROM messages WHERE chat_jid = chats.jid);",
];

pub fn run(conn: &mut Connection) -> Result<()> {
  let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
  for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
    let tx = conn.transaction()?;
    tx.execute_batch(sql)?;
    tx.pragma_update(None, "user_version", (i + 1) as i64)?;
    tx.commit()?;
    tracing::info!(version = i + 1, "migração aplicada");
  }
  Ok(())
}
