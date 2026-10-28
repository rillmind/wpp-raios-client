//! Grava no banco os eventos vindos do WhatsApp.

use anyhow::Result;
use tokio::sync::mpsc;

use crate::store::Store;
use crate::wa::WaEvent;

pub async fn run(store: Store, mut rx: mpsc::Receiver<WaEvent>) {
  while let Some(ev) = rx.recv().await {
    if let Err(e) = apply(&store, ev).await {
      tracing::error!(error = %e, "falha ao gravar evento");
    }
  }
}

pub async fn apply(store: &Store, ev: WaEvent) -> Result<()> {
  match ev {
    WaEvent::Connection(_) => {}
    WaEvent::Message(msg) => {
      let inserted = store.run(move |db| db.insert_message(&msg)).await?;
      tracing::debug!(inserted, "mensagem gravada");
    }
    WaEvent::HistoryChat {
      meta,
      last_ts,
      messages,
    } => {
      store
        .run(move |db| db.import_history_chat(&meta, last_ts, &messages))
        .await?;
    }
    WaEvent::Status {
      chat_jid,
      ids,
      status,
    } => {
      store
        .run(move |db| db.set_status(&chat_jid, &ids, status))
        .await?;
    }
    WaEvent::ReadElsewhere { chat_jid, ids } => {
      store
        .run(move |db| db.mark_messages_read(&chat_jid, &ids))
        .await?;
    }
    WaEvent::ChatRead { chat_jid, read } => {
      store
        .run(move |db| {
          if read {
            db.mark_chat_read(&chat_jid).map(drop)
          } else {
            // "Marcar como não lida" no celular: sem mensagem específica, contador >= 1.
            let current = db.get_chat(&chat_jid)?.map_or(0, |c| c.unread_count);
            db.set_unread(&chat_jid, current.max(1))
          }
        })
        .await?;
    }
    WaEvent::ChatMeta(meta) => store.run(move |db| db.upsert_chat(&meta)).await?,
  }
  Ok(())
}
