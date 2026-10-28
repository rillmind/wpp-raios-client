//! Migrações versionadas por `PRAGMA user_version`.

use anyhow::Result;
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
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
