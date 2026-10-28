use super::*;

fn db() -> Db {
  Db::open_in_memory().expect("banco em memória")
}

fn msg(chat: &str, id: &str, ts: i64, from_me: bool, is_read: bool) -> Message {
  Message {
    id: id.to_string(),
    chat_jid: chat.to_string(),
    sender_jid: (!from_me).then(|| chat.to_string()),
    from_me,
    ts,
    kind: MessageKind::Text,
    body: Some(format!("msg {id}")),
    quoted_id: None,
    status: from_me.then_some(MsgStatus::Sent),
    is_read,
  }
}

const A: &str = "111@s.whatsapp.net";
const B: &str = "222@s.whatsapp.net";

#[test]
fn insert_is_idempotent() {
  let db = db();
  let m = msg(A, "1", 100, false, false);
  assert!(db.insert_message(&m).unwrap());
  assert!(!db.insert_message(&m).unwrap());
  let chat = db.get_chat(A).unwrap().unwrap();
  assert_eq!(chat.unread_count, 1, "repetição não conta de novo");
  assert_eq!(db.last_messages(A, 50, None).unwrap().len(), 1);
}

#[test]
fn unread_count_and_preview_follow_new_messages() {
  let db = db();
  db.insert_message(&msg(A, "1", 100, false, false)).unwrap();
  db.insert_message(&msg(A, "2", 200, false, false)).unwrap();
  db.insert_message(&msg(A, "3", 300, true, true)).unwrap();
  // Mensagem antiga chegando atrasada não troca o preview.
  db.insert_message(&msg(A, "0", 50, false, true)).unwrap();
  let chat = db.get_chat(A).unwrap().unwrap();
  assert_eq!(chat.unread_count, 2);
  assert_eq!(chat.last_msg_ts, Some(300));
  assert_eq!(chat.last_msg_preview.as_deref(), Some("msg 3"));
}

#[test]
fn upsert_chat_keeps_existing_values() {
  let db = db();
  db.upsert_chat(&ChatMeta {
    jid: A.into(),
    name: Some("Ana".into()),
    unread_count: Some(3),
    ..Default::default()
  })
  .unwrap();
  db.upsert_chat(&ChatMeta {
    jid: A.into(),
    ..Default::default()
  })
  .unwrap();
  let chat = db.get_chat(A).unwrap().unwrap();
  assert_eq!(chat.name.as_deref(), Some("Ana"));
  assert_eq!(chat.unread_count, 3);
}

#[test]
fn list_chats_orders_by_activity_and_paginates() {
  let db = db();
  db.insert_message(&msg(A, "1", 100, false, true)).unwrap();
  db.insert_message(&msg(B, "1", 200, false, true)).unwrap();
  db.upsert_chat(&ChatMeta {
    jid: "333@s.whatsapp.net".into(),
    ..Default::default()
  })
  .unwrap();

  let all = db.list_chats(10, None).unwrap();
  let jids: Vec<_> = all.iter().map(|c| c.jid.as_str()).collect();
  assert_eq!(
    jids,
    [B, A, "333@s.whatsapp.net"],
    "sem atividade vai pro fim"
  );

  let page = db.list_chats(10, Some(200)).unwrap();
  assert_eq!(page[0].jid, A);
}

#[test]
fn last_messages_paginates_with_before() {
  let db = db();
  for i in 1..=10 {
    db.insert_message(&msg(A, &i.to_string(), i * 10, false, true))
      .unwrap();
  }
  let last = db.last_messages(A, 3, None).unwrap();
  let ts: Vec<_> = last.iter().map(|m| m.ts).collect();
  assert_eq!(ts, [80, 90, 100], "mais novas, em ordem cronológica");

  let older = db.last_messages(A, 3, Some(80)).unwrap();
  let ts: Vec<_> = older.iter().map(|m| m.ts).collect();
  assert_eq!(ts, [50, 60, 70]);

  assert!(db.last_messages(A, 3, Some(10)).unwrap().is_empty());
}

#[test]
fn first_unread_is_oldest_unread_received() {
  let db = db();
  db.insert_message(&msg(A, "1", 100, false, true)).unwrap();
  db.insert_message(&msg(A, "2", 200, true, true)).unwrap();
  db.insert_message(&msg(A, "3", 300, false, false)).unwrap();
  db.insert_message(&msg(A, "4", 400, false, false)).unwrap();
  assert_eq!(db.first_unread(A).unwrap().unwrap().id, "3");
  assert_eq!(db.count_since(A, 300).unwrap(), 2);
  assert!(db.first_unread(B).unwrap().is_none());
}

#[test]
fn mark_chat_read_returns_unread_and_clears() {
  let mut db = db();
  db.insert_message(&msg(A, "1", 100, false, false)).unwrap();
  db.insert_message(&msg(A, "2", 200, false, false)).unwrap();
  let unread = db.mark_chat_read(A).unwrap();
  let ids: Vec<_> = unread.iter().map(|u| u.id.as_str()).collect();
  assert_eq!(ids, ["1", "2"]);
  assert_eq!(db.get_chat(A).unwrap().unwrap().unread_count, 0);
  assert!(db.first_unread(A).unwrap().is_none());
  assert!(db.mark_chat_read(A).unwrap().is_empty());
}

#[test]
fn mark_messages_read_from_other_device() {
  let mut db = db();
  db.insert_message(&msg(A, "1", 100, false, false)).unwrap();
  db.insert_message(&msg(A, "2", 200, false, false)).unwrap();
  db.mark_messages_read(A, &["1".into(), "1".into(), "x".into()])
    .unwrap();
  assert_eq!(db.get_chat(A).unwrap().unwrap().unread_count, 1);
  assert_eq!(db.first_unread(A).unwrap().unwrap().id, "2");
}

#[test]
fn status_never_regresses() {
  let db = db();
  db.insert_message(&msg(A, "1", 100, true, true)).unwrap();
  let ids = vec!["1".to_string()];
  assert_eq!(db.set_status(A, &ids, MsgStatus::Read).unwrap(), ids);
  assert!(db
    .set_status(A, &ids, MsgStatus::Delivered)
    .unwrap()
    .is_empty());
  let m = &db.last_messages(A, 1, None).unwrap()[0];
  assert_eq!(m.status, Some(MsgStatus::Read));
}

#[test]
fn history_import_uses_server_unread_count() {
  let mut db = db();
  let meta = ChatMeta {
    jid: A.into(),
    name: Some("Ana".into()),
    unread_count: Some(1),
    ..Default::default()
  };
  let msgs = [
    msg(A, "1", 100, false, true),
    msg(A, "2", 200, false, false),
  ];
  db.import_history_chat(&meta, Some(150), &msgs).unwrap();
  db.import_history_chat(&meta, Some(150), &msgs).unwrap();
  let chat = db.get_chat(A).unwrap().unwrap();
  assert_eq!(chat.unread_count, 1);
  assert_eq!(chat.last_msg_ts, Some(200));
  assert_eq!(db.first_unread(A).unwrap().unwrap().id, "2");
}

#[test]
fn migrations_are_reentrant() {
  let dir = std::env::temp_dir().join(format!("wpp-test-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let path = dir.join("m.db");
  Db::open(&path)
    .unwrap()
    .insert_message(&msg(A, "1", 1, false, true))
    .unwrap();
  let db = Db::open(&path).unwrap();
  assert_eq!(db.last_messages(A, 10, None).unwrap().len(), 1);
  std::fs::remove_dir_all(&dir).unwrap();
}
