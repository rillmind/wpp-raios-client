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
  // 3: referência para baixar imagens e figurinhas na hora (o arquivo nunca é guardado).
  "CREATE TABLE media (
     chat_jid        TEXT NOT NULL,
     msg_id          TEXT NOT NULL,
     kind            TEXT NOT NULL,
     direct_path     TEXT NOT NULL,
     media_key       BLOB,
     file_sha256     BLOB NOT NULL,
     file_enc_sha256 BLOB,
     file_length     INTEGER NOT NULL,
     mimetype        TEXT,
     width           INTEGER,
     height          INTEGER,
     PRIMARY KEY (chat_jid, msg_id)
   );",
  // 4: os nomes por telefone migrados na versão 2 eram, quase todos, apelidos (o histórico
  // os gravou por cima dos nomes da agenda). Voltam a ser apelido, e a agenda é pedida de
  // novo ao WhatsApp uma vez (tarefa pendente `contacts_resync`).
  "UPDATE contacts SET push_name = COALESCE(push_name, contact_name), contact_name = NULL
     WHERE jid LIKE '%@s.whatsapp.net';
   CREATE TABLE pending_tasks (name TEXT PRIMARY KEY);
   INSERT INTO pending_tasks (name) VALUES ('contacts_resync');",
  // 5: vídeo e áudio também são baixados na hora; a duração vai junto com a referência.
  "ALTER TABLE media ADD COLUMN seconds INTEGER;",
  // 6: silenciada até (ms desde 1970); -1 = para sempre, NULL/0 = não silenciada.
  "ALTER TABLE chats ADD COLUMN muted_until INTEGER;",
  // 7: documentos também são baixados na hora; o nome do arquivo vai junto.
  "ALTER TABLE media ADD COLUMN file_name TEXT;",
  // 8: estrela e fixação (ms até quando) por mensagem.
  "ALTER TABLE messages ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;
   ALTER TABLE messages ADD COLUMN pinned_until INTEGER;",
  // 9: mensagem guardada mas ainda não mostrada (a âncora da sincronização inicial, que
  // aparece no primeiro "carregar mensagens" e fica visível daí em diante).
  "ALTER TABLE messages ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;",
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
