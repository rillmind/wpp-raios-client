//! Grava no banco os eventos vindos do WhatsApp e avisa o front (SSE).

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::api::ApiEvent;
use crate::store::Store;
use crate::wa::WaEvent;

/// Consome eventos até `drain` disparar; aí grava o que ainda estiver na fila e sai.
pub async fn run(
  store: Store,
  mut rx: mpsc::Receiver<WaEvent>,
  events: broadcast::Sender<ApiEvent>,
  mut drain: oneshot::Receiver<()>,
) {
  loop {
    let ev = tokio::select! {
      ev = rx.recv() => ev,
      _ = &mut drain => break,
    };
    let Some(ev) = ev else { return };
    handle(&store, &events, ev).await;
  }
  let mut pending = 0;
  while let Ok(ev) = rx.try_recv() {
    handle(&store, &events, ev).await;
    pending += 1;
  }
  tracing::info!(pending, "fila de eventos drenada");
}

async fn handle(store: &Store, events: &broadcast::Sender<ApiEvent>, ev: WaEvent) {
  match apply(store, ev).await {
    Ok(out) => {
      for ev in out {
        // Sem assinantes (front fechado) não é erro.
        let _ = events.send(ev);
      }
    }
    Err(e) => tracing::error!(error = %e, "falha ao gravar evento"),
  }
}

/// Aplica o evento no banco e devolve o que deve ser repassado ao front.
pub async fn apply(store: &Store, ev: WaEvent) -> Result<Vec<ApiEvent>> {
  let out = match ev {
    WaEvent::Connection(state) => vec![ApiEvent::ConnectionState { state }],
    WaEvent::Message(msg) => {
      store
        .run(move |db| {
          if !db.insert_message(&msg)? {
            return Ok(vec![]);
          }
          let mut out = vec![];
          if let Some(chat) = db.get_chat(&msg.chat_jid)? {
            out.push(ApiEvent::ChatUpdated(chat));
          }
          out.insert(0, ApiEvent::MessageNew(msg));
          Ok(out)
        })
        .await?
    }
    WaEvent::HistoryChat {
      meta,
      last_ts,
      messages,
    } => {
      store
        .run(move |db| {
          db.import_history_chat(&meta, last_ts, &messages)?;
          chat_updated(db, &meta.jid)
        })
        .await?
    }
    WaEvent::Status {
      chat_jid,
      ids,
      status,
    } => {
      store
        .run(move |db| {
          let ids = db.set_status(&chat_jid, &ids, status)?;
          Ok(if ids.is_empty() {
            vec![]
          } else {
            vec![ApiEvent::MessageStatus {
              chat_jid,
              ids,
              status,
            }]
          })
        })
        .await?
    }
    WaEvent::ReadElsewhere { chat_jid, ids } => {
      store
        .run(move |db| {
          db.mark_messages_read(&chat_jid, &ids)?;
          chat_updated(db, &chat_jid)
        })
        .await?
    }
    WaEvent::ChatRead { chat_jid, read } => {
      store
        .run(move |db| {
          if read {
            db.mark_chat_read(&chat_jid)?;
          } else {
            // "Marcar como não lida" no celular: sem mensagem específica, contador >= 1.
            let current = db.get_chat(&chat_jid)?.map_or(0, |c| c.unread_count);
            db.set_unread(&chat_jid, current.max(1))?;
          }
          chat_updated(db, &chat_jid)
        })
        .await?
    }
    WaEvent::ChatMeta(meta) => {
      store
        .run(move |db| {
          db.upsert_chat(&meta)?;
          chat_updated(db, &meta.jid)
        })
        .await?
    }
  };
  Ok(out)
}

fn chat_updated(db: &crate::store::Db, jid: &str) -> Result<Vec<ApiEvent>> {
  Ok(
    db.get_chat(jid)?
      .map(ApiEvent::ChatUpdated)
      .into_iter()
      .collect(),
  )
}
