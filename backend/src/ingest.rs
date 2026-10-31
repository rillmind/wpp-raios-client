//! Grava no banco os eventos vindos do WhatsApp e avisa o front (SSE).

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::api::ApiEvent;
use crate::store::Store;
use crate::wa::{WaEvent, WhatsApp};

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

/// Liga ao telefone os `@lid` já gravados (conversas e remetentes), pelo que a sessão conhece.
///
/// Os eventos novos já trazem a ligação; isto cobre o que veio antes dela existir.
pub async fn backfill_lids(store: Store, wa: Arc<dyn WhatsApp>, tx: mpsc::Sender<WaEvent>) {
  let lids = match store.run(|db| db.unmapped_lids()).await {
    Ok(lids) => lids,
    Err(e) => {
      tracing::error!(error = %e, "falha ao listar lids sem telefone");
      return;
    }
  };
  let mut found = 0;
  for lid in &lids {
    match wa.phone_for_lid(lid).await {
      Ok(Some(pn)) => {
        found += 1;
        if tx
          .send(WaEvent::LidMap {
            lid: lid.clone(),
            pn,
          })
          .await
          .is_err()
        {
          return;
        }
      }
      Ok(None) => {}
      Err(e) => tracing::debug!(error = %e, "falha ao buscar telefone do lid"),
    }
  }
  tracing::info!(lids = lids.len(), found, "telefones dos @lid conferidos");
}

/// Tarefa única: pede a agenda de novo assim que conectar (ver migração 4).
pub async fn resync_contacts_once(store: Store, wa: Arc<dyn WhatsApp>) {
  const TASK: &str = "contacts_resync";
  match store.run(|db| db.has_pending_task(TASK)).await {
    Ok(true) => {}
    Ok(false) => return,
    Err(e) => {
      tracing::error!(error = %e, "falha ao ler tarefas pendentes");
      return;
    }
  }
  while wa.state() != crate::model::ConnState::Connected {
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
  }
  tracing::info!("pedindo a agenda de contatos de novo ao WhatsApp");
  if let Err(e) = wa.resync_contacts().await {
    tracing::warn!(error = %e, "falha ao pedir a agenda; tenta de novo no próximo início");
    return;
  }
  // Os contatos chegam como eventos pela fila do ingest; dá um tempo para gravarem.
  tokio::time::sleep(std::time::Duration::from_secs(5)).await;
  let named = store
    .run(|db| {
      let n = db.count_agenda_names()?;
      // Sem nenhum nome, algo falhou: tenta de novo no próximo início.
      if n > 0 {
        db.finish_pending_task(TASK)?;
      }
      Ok(n)
    })
    .await;
  match named {
    Ok(0) => tracing::warn!("a agenda não trouxe nomes; tenta de novo no próximo início"),
    Ok(n) => tracing::info!(contatos_com_nome_da_agenda = n, "agenda recebida"),
    Err(e) => tracing::error!(error = %e, "falha ao concluir a tarefa da agenda"),
  }
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
    WaEvent::Message { mut msg, media } => {
      store
        .run(move |db| {
          if let Some(media) = &media {
            db.insert_media(&msg.chat_jid, &msg.id, media)?;
          }
          if !db.insert_message(&msg)? {
            return Ok(vec![]);
          }
          if let Some(sender) = &msg.sender_jid {
            msg.sender_name = db.person_name(sender)?;
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
      media,
    } => {
      store
        .run(move |db| {
          for (id, m) in &media {
            db.insert_media(&meta.jid, id, m)?;
          }
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
    WaEvent::Contact(contact) => {
      store
        .run(move |db| {
          db.upsert_contact(&contact)?;
          Ok(
            db.chats_of_person(&contact.jid)?
              .into_iter()
              .map(ApiEvent::ChatUpdated)
              .collect(),
          )
        })
        .await?
    }
    WaEvent::LidMap { lid, pn } => {
      store
        .run(move |db| {
          if !db.set_lid_map(&lid, &pn)? {
            return Ok(vec![]);
          }
          chat_updated(db, &lid)
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
