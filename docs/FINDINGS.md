# Achados da Fase 1: `whatsapp-rust` 0.7.0

Versão fixada: `whatsapp-rust = "=0.7.0"` (com `default-features = false`, ver "Outros achados").
Fontes: o código da crate e das dependências (`wacore`, `waproto`, `whatsapp-rust-sqlite-storage`)
em `~/.cargo/registry/src/*/`, mais uma execução real do backend.

## O que foi observado e o que ainda falta

| Item | Situação |
| --- | --- |
| Conectar, handshake, emitir QR no terminal | **Observado** (2026-10-28): `Handshake complete (XX)` → `PairingQrCode` → estado `WaitingQr`. |
| Desligamento limpo em SIGTERM | **Observado**: `handle.shutdown()` encerra o loop do cliente. |
| Pareamento persistir entre reinícios | **Falta observar**: exige escanear o QR com um número secundário. |
| Perguntas 1 a 6 abaixo | **Lidas no código-fonte**; confirmar no primeiro pareamento real (ver checklist no final). |

Nenhuma das respostas abaixo depende de memória: todas apontam para o arquivo onde estão na 0.7.0.

## 1. A sincronização inicial entrega a lista de chats e os contadores de não lidas?

**Sim, pelo código.** Chega como `Event::HistorySync(Box<LazyHistorySync>)`
(`wacore/src/types/events.rs`). `lazy.get()` devolve um `wa::HistorySync` cujo `sync_type`
`INITIAL_BOOTSTRAP` (e depois `RECENT`) traz `conversations: Vec<Conversation>`
(`waproto/src/whatsapp.proto`, `message Conversation`), cada uma com:

- `id` (JID), `name` / `displayName`;
- `unreadCount`, `markedAsUnread`;
- `archived`, `pinned` (timestamp, > 0 = fixado);
- `conversationTimestamp` / `lastMsgTimestamp`;
- `messages` (mensagens recentes, ver item 2).

O backend converte cada conversa em `WaEvent::HistoryChat` (`backend/src/wa/whatsapp_rust_impl.rs`,
`convert_conversation`). Um chat com `unreadCount = 0` e `markedAsUnread = true` vira contador 1.

## 2. Chega um pacote de histórico recente no pareamento? Dá para controlar o tamanho?

**Sim e sim.** As mensagens recentes vêm em `Conversation.messages` (`HistorySyncMsg` →
`WebMessageInfo`). Formas de controlar:

- `Bot::builder().skip_history_sync()`: descarta todos os pacotes (perderíamos também os
  contadores do item 1, então **não** usamos).
- `Bot::builder().with_device_props(DevicePropsOverride { history_sync_config, .. })`
  (`wacore/src/store/device.rs`): o proto `HistorySyncConfig` tem `recentSyncDaysLimit`,
  `initialSyncMaxMessagesPerChat`, `fullSyncDaysLimit` etc. Pedido ao servidor; não está claro
  quanto ele respeita. Não usado por enquanto.
- No nosso lado (o que está implementado): só aceitamos `INITIAL_BOOTSTRAP`, `RECENT` e
  `PUSH_NAME`; `FULL` e `ON_DEMAND` são ignorados; e guardamos no máximo
  `history_msgs_per_chat` (padrão 50) mensagens por chat. As `unreadCount` recebidas mais
  recentes ficam marcadas como não lidas.

## 3. Mensagens recebidas com o backend offline são entregues ao reconectar?

**Sim, pela fila offline do servidor.** Na reconexão chegam `Event::OfflineSyncPreview`
(contagens), depois `Event::Messages(MessageBatch)` com `origin = BatchOrigin::OfflineDrain` e
`info.is_offline = true` em cada mensagem, e por fim `Event::OfflineSyncCompleted`. O tratamento é
o mesmo de mensagem ao vivo (inserção idempotente). Quanto tempo o servidor guarda a fila não está
na lib; é política do WhatsApp.

A própria lib reconecta sozinha (`enable_auto_reconnect`, contador de erros com backoff em
`whatsapp-rust/src/client/lifecycle.rs`).

## 4. Como marcar uma conversa como lida (e isso sincroniza com o celular)?

**Possível, em duas chamadas** (implementado em `RustWa::mark_read`):

1. `client.mark_as_read(&chat, sender, &ids)` (`whatsapp-rust/src/receipt.rs`): recibo de leitura
   para o contato (tiques azuis). Em grupo, um recibo por remetente (`participant`).
2. `client.chat_actions().mark_chat_as_read(&chat, true, None)`
   (`whatsapp-rust/src/features/chat_actions.rs`): mutação de app-state `MARK_CHAT_AS_READ`, que é o
   que zera o contador no celular e nos outros aparelhos. O `message_range` é opcional
   (`None` é aceito segundo o comentário da lib).

No sentido inverso (lido no celular), chegam `Event::MarkChatAsReadUpdate` (com `action.read`) e
`Event::Receipt` do tipo `ReadSelf`; os dois atualizam o banco local.

## 5. Como enviar texto?

`client.send_message(jid, wa::Message::text(texto))` (`whatsapp-rust/src/send/mod.rs`), que devolve
`SendResult { message_id, to }`. Recibos posteriores chegam em `Event::Receipt` com
`ReceiptType::Delivered` / `Read` / `Played`.

## 6. Como obter nomes de contatos e grupos?

Várias fontes, todas tratadas como `WaEvent::ChatMeta` (só sobrescrevem quando trazem valor):

- `Conversation.name` / `displayName` da sincronização inicial;
- `HistorySync.pushnames` (`Pushname { id, pushname }`);
- `MessageInfo.push_name` em cada mensagem recebida em conversa individual;
- `Event::ContactUpdate` (nome salvo na agenda: `action.full_name` / `first_name`);
- grupos: `client.groups().get_metadata(&jid)` → `GroupMetadata.subject`. O backend busca uma vez
  por grupo por execução, na primeira mensagem dele.

## Outros achados

- **Nightly:** a feature padrão `simd` liga `#![feature(portable_simd)]` em `wacore-binary` e não
  compila no Rust estável. Usamos as demais features padrão explicitamente: `sqlite-storage`,
  `tokio-transport`, `tokio-runtime`, `ureq-client`, `tokio-native`, `signal`.
- **Logs:** a lib usa a crate `log`; o `tracing-subscriber` repassa. O `Debug` de `Event` imprime
  só o nome da variante, então dá para logar todos os eventos em `debug` sem vazar conteúdo.
- **LID x telefone (visto no pareamento real):** todas as conversas individuais e quase todos os
  remetentes em grupos chegam como `@lid`, enquanto os nomes da agenda (`Event::ContactUpdate`)
  chegam pelo jid de telefone. Por isso os nomes ficam em `contacts` (agenda e apelido separados) e
  a ligação em `lid_map`, alimentada por `MessageSource.sender_alt`/`recipient_alt`,
  `ContactAction.lidJid`/`pnJid`, `Conversation.lidJid`/`pnJid` e, para o que já estava gravado,
  `Client::get_lid_pn_entry` (tabela `lid_pn_mapping` da sessão, consultada sem rede). Não houve
  chats duplicados: as entradas por telefone eram só nomes, sem mensagens.
- **Canais (newsletters):** `newsletter().get_metadata()` e `list_subscribed()` falham na 0.7.0
  com `MEX fatal error (... code=400): Bad Request` (as consultas persistidas da lib não são mais
  aceitas pelo servidor). Sem nome nem foto de canal até a lib corrigir; as mensagens chegam
  normalmente. Grupos de que a pessoa saiu respondem `403 forbidden` a `groups().get_metadata()`.
- **Agenda x apelido:** no pareamento, os apelidos do histórico (`PUSH_NAME`) chegaram depois dos
  nomes da agenda e, quando os dois iam para o mesmo campo, ficavam por cima. Para recuperar a
  agenda: `AppSyncStore::set_version("critical_unblock_low", HashState::default())` e
  `Client::process_sync_task(AppStateSync { CriticalUnblockLow, full_sync: true })`. Só pedir o
  snapshot não basta: a lib descarta snapshot com versão `<=` à guardada ("Skipping stale
  snapshot"); com a versão em 0 ela o aplica do zero (limpa e regrava os MACs, depois a versão),
  e cada contato salvo chega como `Event::ContactUpdate` (317 no pareamento real).
- **Desligamento:** `BotHandle::shutdown()` grava o estado pendente da sessão antes de fechar.
  Depois do desligamento intencional a lib ainda loga "reconnecting immediately", mas o loop termina.
- A lib também tem `shutdown_signal()` (SIGINT + SIGTERM), usado no `main`.

## Checklist do primeiro pareamento (número secundário)

1. Fazer backup de `data/` se já existir. Rodar `RUST_LOG=info,wpp_backend=debug cargo run`.
2. Escanear o QR. Conferir no log `pacote de histórico recebido` com `conversations > 0`.
3. Reiniciar o backend: **não** pode aparecer QR novo.
4. Mandar mensagem de outro número com o backend parado; ao subir, ela deve chegar.
5. Marcar como lido pelo app e conferir que o contador some no celular.
6. Procurar chats duplicados (`@lid` e `@s.whatsapp.net` do mesmo contato).

Se o item 2 ou o 5 falhar na prática, a regra do plano vale: parar e avaliar a troca por whatsmeow.
