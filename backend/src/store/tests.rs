use super::*;

fn db() -> Db {
  Db::open_in_memory().expect("banco em memória")
}

fn msg(chat: &str, id: &str, ts: i64, from_me: bool, is_read: bool) -> Message {
  Message {
    id: id.to_string(),
    chat_jid: chat.to_string(),
    sender_jid: (!from_me).then(|| chat.to_string()),
    sender_name: None,
    from_me,
    ts,
    kind: MessageKind::Text,
    body: Some(format!("msg {id}")),
    quoted_id: None,
    status: from_me.then_some(MsgStatus::Sent),
    is_read,
    media: None,
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

const LID: &str = "999@lid";
const PN: &str = "5511988887777@s.whatsapp.net";
const GROUP: &str = "123-456@g.us";

fn contact(jid: &str, contact_name: Option<&str>, push_name: Option<&str>) -> Contact {
  Contact {
    jid: jid.into(),
    contact_name: contact_name.map(Into::into),
    push_name: push_name.map(Into::into),
  }
}

#[test]
fn agenda_name_wins_and_reaches_lid_chat_through_phone() {
  let db = db();
  db.insert_message(&msg(LID, "1", 10, false, true)).unwrap();

  // Sem mapa: só o apelido, e telefone desconhecido.
  db.upsert_contact(&contact(LID, None, Some("Zé"))).unwrap();
  let chat = db.get_chat(LID).unwrap().unwrap();
  assert_eq!(chat.name.as_deref(), Some("Zé"));
  assert_eq!(chat.phone, None);

  // Nome da agenda chega pelo telefone; com o mapa ele vence o apelido.
  db.upsert_contact(&contact(PN, Some("José da Silva"), None))
    .unwrap();
  assert!(db.set_lid_map(LID, PN).unwrap());
  assert!(
    !db.set_lid_map(LID, PN).unwrap(),
    "mapa igual não muda nada"
  );
  let chat = db.get_chat(LID).unwrap().unwrap();
  assert_eq!(chat.name.as_deref(), Some("José da Silva"));
  assert_eq!(chat.phone.as_deref(), Some("5511988887777"));

  // Apelido novo não apaga o nome da agenda.
  db.upsert_contact(&contact(LID, None, Some("Zezinho")))
    .unwrap();
  db.upsert_contact(&contact(PN, None, Some("Zezinho")))
    .unwrap();
  let chat = db.get_chat(LID).unwrap().unwrap();
  assert_eq!(chat.name.as_deref(), Some("José da Silva"));

  let affected: Vec<String> = db
    .chats_of_person(PN)
    .unwrap()
    .into_iter()
    .map(|c| c.jid)
    .collect();
  assert_eq!(affected, vec![LID.to_string()]);
}

#[test]
fn group_keeps_subject_and_names_senders() {
  let db = db();
  db.upsert_chat(&ChatMeta {
    jid: GROUP.into(),
    name: Some("Família".into()),
    is_group: true,
    ..Default::default()
  })
  .unwrap();
  let from = |id: &str, sender: &str| Message {
    sender_jid: Some(sender.into()),
    ..msg(GROUP, id, 10, false, true)
  };
  db.insert_message(&from("1", LID)).unwrap();
  db.insert_message(&from("2", "888@lid")).unwrap();
  db.insert_message(&from("3", "5521911112222@s.whatsapp.net"))
    .unwrap();
  let mut unmapped = db.unmapped_lids().unwrap();
  unmapped.sort();
  assert_eq!(unmapped, vec!["888@lid".to_string(), LID.to_string()]);
  db.set_lid_map(LID, PN).unwrap();
  assert_eq!(db.unmapped_lids().unwrap(), vec!["888@lid".to_string()]);
  db.upsert_contact(&contact(PN, Some("José"), None)).unwrap();
  db.upsert_contact(&contact(GROUP, Some("não deve aparecer"), None))
    .unwrap();

  assert_eq!(
    db.get_chat(GROUP).unwrap().unwrap().name.as_deref(),
    Some("Família")
  );
  let names: Vec<Option<String>> = db
    .last_messages(GROUP, 10, None)
    .unwrap()
    .into_iter()
    .map(|m| m.sender_name)
    .collect();
  assert_eq!(
    names,
    vec![Some("José".into()), None, Some("+5521911112222".into())]
  );
  assert_eq!(db.person_name(LID).unwrap().as_deref(), Some("José"));
  assert_eq!(db.person_name("888@lid").unwrap(), None);
}

#[test]
fn migration_moves_dm_names_to_contacts() {
  let dir = std::env::temp_dir().join(format!("wpp-mig2-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let path = dir.join("m.db");
  {
    // Banco na versão 1, como estava antes desta migração.
    let mut conn = Connection::open(&path).unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute_batch(migrations::MIGRATIONS[0]).unwrap();
    tx.pragma_update(None, "user_version", 1).unwrap();
    tx.commit().unwrap();
    conn
      .execute_batch(&format!(
        "INSERT INTO chats (jid, name, is_group, last_msg_ts) VALUES
           ('{PN}', 'José da Silva', 0, NULL),
           ('{LID}', 'Zé', 0, 10),
           ('777@lid', '+55∙∙∙∙∙∙∙∙65', 0, 20),
           ('{GROUP}', 'Família', 1, 30);"
      ))
      .unwrap();
  }
  let db = Db::open(&path).unwrap();
  db.set_lid_map(LID, PN).unwrap();
  let chats = db.list_chats(10, None).unwrap();
  let view: Vec<(&str, Option<&str>)> = chats
    .iter()
    .map(|c| (c.jid.as_str(), c.name.as_deref()))
    .collect();
  assert_eq!(
    view,
    vec![
      (GROUP, Some("Família")),
      ("777@lid", None),
      (LID, Some("José da Silva"))
    ],
    "entrada só de telefone some; nome mascarado é descartado"
  );
  std::fs::remove_dir_all(&dir).unwrap();
}
