//! Testes dos endpoints com um WhatsApp falso (sem rede).

use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::model::{ConnState, Message, MessageKind};
use crate::store::Db;
use crate::wa::{Sent, UnreadRef};

const TOKEN: &str = "segredo";
const ORIGIN: &str = "http://127.0.0.1:5173";
const A: &str = "111@s.whatsapp.net";

#[derive(Default)]
struct FakeWa {
  sent: Mutex<Vec<(String, String)>>,
  read: Mutex<Vec<(String, Vec<String>)>>,
  offline: bool,
}

#[async_trait]
impl WhatsApp for FakeWa {
  fn state(&self) -> ConnState {
    if self.offline {
      ConnState::Disconnected
    } else {
      ConnState::Connected
    }
  }

  async fn send_text(&self, chat_jid: &str, text: &str) -> Result<Sent> {
    let mut sent = self.sent.lock().unwrap();
    sent.push((chat_jid.into(), text.into()));
    Ok(Sent {
      id: format!("out-{}", sent.len()),
      ts: 1_000,
    })
  }

  async fn mark_read(&self, chat_jid: &str, unread: &[UnreadRef]) -> Result<()> {
    let ids = unread.iter().map(|u| u.id.clone()).collect();
    self.read.lock().unwrap().push((chat_jid.into(), ids));
    Ok(())
  }

  async fn phone_for_lid(&self, _lid_jid: &str) -> Result<Option<String>> {
    Ok(None)
  }

  async fn shutdown(&self) {}
}

struct Harness {
  app: Router,
  wa: Arc<FakeWa>,
  store: Store,
  events: broadcast::Sender<ApiEvent>,
  stop: watch::Sender<bool>,
}

fn harness_with(wa: FakeWa) -> Harness {
  let store = Store::new(Db::open_in_memory().unwrap());
  let wa = Arc::new(wa);
  let (events, _) = broadcast::channel(64);
  let (stop, shutdown) = watch::channel(false);
  let state = AppState {
    store: store.clone(),
    wa: wa.clone(),
    events: events.clone(),
    token: TOKEN.into(),
    limiter: Arc::new(RateLimiter::new(2)),
    shutdown,
  };
  Harness {
    app: router(state, ORIGIN, None).unwrap(),
    wa,
    store,
    events,
    stop,
  }
}

fn harness() -> Harness {
  harness_with(FakeWa::default())
}

fn incoming(id: &str, ts: i64, is_read: bool) -> Message {
  Message {
    id: id.into(),
    chat_jid: A.into(),
    sender_jid: Some(A.into()),
    sender_name: None,
    from_me: false,
    ts,
    kind: MessageKind::Text,
    body: Some(format!("oi {id}")),
    quoted_id: None,
    status: None,
    is_read,
  }
}

async fn seed(h: &Harness, msgs: Vec<Message>) {
  h.store
    .run(move |db| {
      for m in &msgs {
        db.insert_message(m)?;
      }
      Ok(())
    })
    .await
    .unwrap();
}

async fn call(h: &Harness, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
  let mut req = Request::builder()
    .method(method)
    .uri(uri)
    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
  let body = match body {
    Some(v) => {
      req = req.header(header::CONTENT_TYPE, "application/json");
      Body::from(v.to_string())
    }
    None => Body::empty(),
  };
  let resp = h
    .app
    .clone()
    .oneshot(req.body(body).unwrap())
    .await
    .unwrap();
  let status = resp.status();
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
  (status, json)
}

#[tokio::test]
async fn rejects_missing_or_wrong_token() {
  let h = harness();
  let resp = h
    .app
    .clone()
    .oneshot(Request::get("/api/chats").body(Body::empty()).unwrap())
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

  let resp = h
    .app
    .clone()
    .oneshot(
      Request::get("/api/chats")
        .header(header::AUTHORIZATION, "Bearer errado")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

  // Token na query só vale para o SSE.
  let resp = h
    .app
    .clone()
    .oneshot(
      Request::get(format!("/api/chats?token={TOKEN}"))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn healthz_needs_no_token() {
  let h = harness();
  let resp = h
    .app
    .clone()
    .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn status_reports_connection() {
  let h = harness();
  let (code, body) = call(&h, "GET", "/api/status", None).await;
  assert_eq!(code, StatusCode::OK);
  assert_eq!(body, json!({ "state": "connected", "connected": true }));
}

#[tokio::test]
async fn lists_chats_by_activity() {
  let h = harness();
  let mut b = incoming("1", 500, false);
  b.chat_jid = "222@s.whatsapp.net".into();
  seed(&h, vec![incoming("1", 100, false), b]).await;

  let (code, body) = call(&h, "GET", "/api/chats", None).await;
  assert_eq!(code, StatusCode::OK);
  assert_eq!(body[0]["jid"], "222@s.whatsapp.net");
  assert_eq!(body[1]["jid"], A);
  assert_eq!(body[1]["unread_count"], 1);
  assert_eq!(body[1]["last_msg_preview"], "oi 1");

  let (_, page) = call(&h, "GET", "/api/chats?limit=1", None).await;
  assert_eq!(page.as_array().unwrap().len(), 1);
  let (code, _) = call(&h, "GET", "/api/chats?limit=0", None).await;
  assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn messages_include_all_unread_and_marker() {
  let h = harness();
  let mut msgs: Vec<_> = (1..=5)
    .map(|i| incoming(&i.to_string(), i * 10, true))
    .collect();
  msgs.extend((6..=9).map(|i| incoming(&i.to_string(), i * 10, false)));
  seed(&h, msgs).await;

  // limit=2, mas há 4 não lidas: vêm as 4.
  let (code, body) = call(&h, "GET", &format!("/api/chats/{A}/messages?limit=2"), None).await;
  assert_eq!(code, StatusCode::OK);
  assert_eq!(body["first_unread_id"], "6");
  let ids: Vec<_> = body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .map(|m| m["id"].as_str().unwrap())
    .collect();
  assert_eq!(ids, ["6", "7", "8", "9"]);
  assert_eq!(body["has_more"], true);

  let (_, older) = call(
    &h,
    "GET",
    &format!("/api/chats/{A}/messages?before=60&limit=2"),
    None,
  )
  .await;
  let ids: Vec<_> = older["messages"]
    .as_array()
    .unwrap()
    .iter()
    .map(|m| m["id"].as_str().unwrap())
    .collect();
  assert_eq!(ids, ["4", "5"]);
  assert_eq!(older["has_more"], true);

  let (_, oldest) = call(
    &h,
    "GET",
    &format!("/api/chats/{A}/messages?before=20&limit=5"),
    None,
  )
  .await;
  assert_eq!(oldest["messages"].as_array().unwrap().len(), 1);
  assert_eq!(oldest["has_more"], false);
}

#[tokio::test]
async fn messages_unknown_chat_and_bad_jid() {
  let h = harness();
  let (code, _) = call(&h, "GET", "/api/chats/999@s.whatsapp.net/messages", None).await;
  assert_eq!(code, StatusCode::NOT_FOUND);
  let (code, _) = call(&h, "GET", "/api/chats/sem-arroba/messages", None).await;
  assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mark_read_calls_whatsapp_then_clears() {
  let h = harness();
  seed(&h, vec![incoming("1", 10, false), incoming("2", 20, false)]).await;
  let mut rx = h.events.subscribe();

  let (code, body) = call(&h, "POST", &format!("/api/chats/{A}/read"), None).await;
  assert_eq!(code, StatusCode::OK);
  assert_eq!(body["unread_count"], 0);
  assert_eq!(
    *h.wa.read.lock().unwrap(),
    vec![(A.to_string(), vec!["1".to_string(), "2".to_string()])]
  );
  assert!(matches!(rx.recv().await.unwrap(), ApiEvent::ChatUpdated(c) if c.unread_count == 0));
}

#[tokio::test]
async fn send_validates_stores_and_rate_limits() {
  let h = harness();
  seed(&h, vec![incoming("1", 10, true)]).await;
  let uri = format!("/api/chats/{A}/send");

  let (code, _) = call(&h, "POST", &uri, Some(json!({ "text": "   " }))).await;
  assert_eq!(code, StatusCode::BAD_REQUEST);

  let (code, body) = call(&h, "POST", &uri, Some(json!({ "text": "olá" }))).await;
  assert_eq!(code, StatusCode::OK);
  assert_eq!(body["from_me"], true);
  assert_eq!(body["status"], "sent");
  let (_, msgs) = call(&h, "GET", &format!("/api/chats/{A}/messages"), None).await;
  assert_eq!(
    msgs["messages"].as_array().unwrap().last().unwrap()["body"],
    "olá"
  );

  // Limite de 2 por segundo: o terceiro é recusado.
  let (code, _) = call(&h, "POST", &uri, Some(json!({ "text": "2" }))).await;
  assert_eq!(code, StatusCode::OK);
  let (code, _) = call(&h, "POST", &uri, Some(json!({ "text": "3" }))).await;
  assert_eq!(code, StatusCode::TOO_MANY_REQUESTS);
  assert_eq!(h.wa.sent.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn send_and_read_need_connection() {
  let h = harness_with(FakeWa {
    offline: true,
    ..Default::default()
  });
  seed(&h, vec![incoming("1", 10, false)]).await;
  let (code, _) = call(
    &h,
    "POST",
    &format!("/api/chats/{A}/send"),
    Some(json!({ "text": "x" })),
  )
  .await;
  assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
  let (code, _) = call(&h, "POST", &format!("/api/chats/{A}/read"), None).await;
  assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn sse_sends_state_then_live_events() {
  let h = harness();
  let resp = h
    .app
    .clone()
    .oneshot(
      Request::get(format!("/api/events?token={TOKEN}"))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::OK);
  let mut body = resp.into_body();

  let first = next_chunk(&mut body).await;
  assert!(first.contains("event: connection.state"), "{first}");

  // Simula uma mensagem chegando do WhatsApp pelo ingest.
  let out = crate::ingest::apply(
    &h.store,
    crate::wa::WaEvent::Message(incoming("9", 99, false)),
  )
  .await
  .unwrap();
  for ev in out {
    h.events.send(ev).unwrap();
  }
  let second = next_chunk(&mut body).await;
  assert!(second.contains("event: message.new"), "{second}");
  assert!(second.contains("\"id\":\"9\""), "{second}");
}

#[tokio::test]
async fn sse_ends_on_shutdown() {
  let h = harness();
  let resp = h
    .app
    .clone()
    .oneshot(
      Request::get(format!("/api/events?token={TOKEN}"))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  let mut body = resp.into_body();
  next_chunk(&mut body).await;
  h.stop.send(true).unwrap();
  let end = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
    .await
    .expect("SSE não encerrou");
  assert!(end.is_none(), "stream deveria terminar");
}

#[tokio::test]
async fn cors_allows_only_front_origin() {
  let h = harness();
  let preflight = |origin: &str| {
    Request::builder()
      .method("OPTIONS")
      .uri("/api/chats")
      .header(header::ORIGIN, origin)
      .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
      .body(Body::empty())
      .unwrap()
  };
  let ok = h.app.clone().oneshot(preflight(ORIGIN)).await.unwrap();
  assert_eq!(ok.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], ORIGIN);
  let bad = h
    .app
    .clone()
    .oneshot(preflight("http://evil.example"))
    .await
    .unwrap();
  // Com origem fixa, a resposta sempre nomeia o front; o navegador da outra
  // origem compara e bloqueia.
  let allowed = bad.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN);
  assert!(allowed.is_none_or(|v| v == ORIGIN));
}

async fn next_chunk(body: &mut Body) -> String {
  let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
    .await
    .expect("SSE sem evento")
    .expect("SSE terminou")
    .unwrap();
  String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap()
}
