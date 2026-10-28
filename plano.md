# Plano: cliente leve de WhatsApp (Rust + React)

Documento para um agente de código implementar. Leia tudo antes de começar e siga as fases na ordem. Cada fase tem critérios de aceite; não avance sem cumpri-los.

## 1. Objetivo

Um cliente de WhatsApp de uso pessoal, rodando localmente, cujo diferencial é **abrir instantaneamente**: ao abrir, mostra as mensagens não lidas atuais mais as que já estavam guardadas no banco local, sem sincronizar nem carregar o histórico antigo do WhatsApp. Quem precisar de histórico anterior ao pareamento usa o celular.

Uso previsto: leitura e resposta **manual** (um humano digitando). Sem automação de envio, sem bot, sem envio em massa.

## 2. Arquitetura

```
 WhatsApp  <──>  [ Backend Rust ]  <──HTTP/SSE──>  [ Front React ]
                   │
                   ├─ whatsapp-rust (sessão, protocolo, eventos)
                   └─ SQLite próprio (chats, mensagens, não lidas)
```

Regras de arquitetura (inegociáveis):

1. O backend fica **sempre conectado** ao WhatsApp como serviço em segundo plano e grava tudo que chega no SQLite local.
2. O front **nunca** fala com o WhatsApp. Ele só consome a API local do backend, que lê do SQLite. É isso que torna a abertura instantânea.
3. O banco da aplicação (mensagens) é **separado** do banco de sessão da biblioteca (`whatsapp.db`). Dois arquivos, dois propósitos.
4. O backend escuta apenas em `127.0.0.1`.

## 3. Stack

Backend:
- Rust (edition 2021 ou mais recente), `tokio`.
- Biblioteca WhatsApp: crate `whatsapp-rust` (repo `github.com/oxidezap/whatsapp-rust`, inspirada em whatsmeow e Baileys; versão 0.7.x quando este plano foi escrito). Fixar a versão exata no `Cargo.toml`.
- Documentação da lib, que o agente deve ler antes de codar: `https://whatsapp-rust.jlucaso.com` (há também `/llms.txt` e `/llms-full.txt`).
- HTTP/SSE: `axum`.
- Banco da aplicação: `rusqlite` (com `bundled`) ou `sqlx` com SQLite. Preferir o mais simples; o volume de dados é pequeno. Se usar `rusqlite`, rodar as queries via `tokio::task::spawn_blocking`.
- Serialização: `serde`, `serde_json`.
- Logs: `tracing` + `tracing-subscriber`.
- Config: arquivo TOML ou variáveis de ambiente.
- Memory safety: no crate da aplicação, `#![forbid(unsafe_code)]` no topo de `main.rs`/`lib.rs`. Dependências podem usar `unsafe`; o código próprio não.

Frontend:
- React + TypeScript + Vite.
- **CSS puro** (sem Tailwind, sem biblioteca de UI). Layout com flexbox/grid e media queries, responsivo para telas diferentes.
- Estado de servidor: `fetch` simples + um pequeno hook próprio, ou TanStack Query se ficar muito repetitivo. Sem Redux.
- Lista longa de mensagens: virtualização só se necessário (por padrão cada consulta já traz poucas mensagens).

## 4. Risco e ressalvas (ler antes de implementar)

- Clientes não oficiais violam os termos do WhatsApp; existe risco de suspensão do número. O uso aqui é o de menor risco (só um humano lendo/respondendo), mas não é zero.
- **Desenvolver e testar com um número secundário.** Só migrar para o número principal depois de estável.
- Durante o desenvolvimento, **não parear e despareiar repetidamente**. Parear uma vez e persistir a sessão (`whatsapp.db`). Fazer backup desse arquivo antes de mexer em código de sessão.
- A biblioteca Rust é mais jovem que a whatsmeow (Go) e a Baileys (TS). Pode haver APIs instáveis, eventos faltando ou bugs. Se a Fase 1 mostrar que a lib não entrega o necessário (ver seção 6), parar e reportar em vez de contornar com gambiarra. O plano de contingência é trocar o módulo de WhatsApp por whatsmeow mantendo o resto (SQLite + API + front) intacto, por isso a Fase 2 define o schema de forma independente da lib.
- Não verificar nada de memória: confirmar nomes de eventos, structs e métodos na documentação e no código-fonte da versão fixada.

## 5. Modelo de dados (SQLite da aplicação)

Ajustar conforme o que a lib realmente expõe, mas manter a ideia:

```sql
CREATE TABLE chats (
  jid            TEXT PRIMARY KEY,
  name           TEXT,              -- nome do contato/grupo
  is_group       INTEGER NOT NULL DEFAULT 0,
  unread_count   INTEGER NOT NULL DEFAULT 0,
  last_msg_ts    INTEGER,           -- unix seconds
  last_msg_preview TEXT,
  archived       INTEGER NOT NULL DEFAULT 0,
  pinned         INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE messages (
  id             TEXT NOT NULL,     -- id da mensagem no WhatsApp
  chat_jid       TEXT NOT NULL REFERENCES chats(jid),
  sender_jid     TEXT,
  from_me        INTEGER NOT NULL,
  ts             INTEGER NOT NULL,
  kind           TEXT NOT NULL,     -- text | image | audio | video | doc | sticker | other
  body           TEXT,              -- texto ou legenda
  quoted_id      TEXT,
  status         TEXT,              -- sent | delivered | read (só from_me)
  is_read        INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (chat_jid, id)
);

CREATE INDEX idx_messages_chat_ts ON messages(chat_jid, ts DESC);
CREATE INDEX idx_chats_last_ts   ON chats(last_msg_ts DESC);
```

Regras:
- Inserções idempotentes (`INSERT ... ON CONFLICT DO NOTHING/UPDATE`), porque o mesmo evento pode chegar mais de uma vez.
- Mídia nesta versão: guardar apenas metadados e o `kind`. Download de mídia fica fora do escopo inicial (ver seção 9).
- **Sem poda nem reset de histórico.** As mensagens ficam guardadas no banco local; o que mantém o app rápido é limitar quanto se carrega por consulta (índice em `(chat_jid, ts DESC)` + `limit`), não apagar dados. Nunca apagar o `whatsapp.db` (sessão).

## 6. Fases

### Fase 0: esqueleto

- Workspace Cargo com um crate binário (`backend/`) e a pasta `frontend/` (Vite).
- `#![forbid(unsafe_code)]`, `tracing` configurado, config carregada de arquivo/env.
- `cargo clippy -- -D warnings` e `cargo fmt --check` passando; CI local via script simples.

Aceite: `cargo run` sobe e loga; `npm run dev` sobe o front.

### Fase 1: conectar e observar (spike)

Objetivo: descobrir o que a biblioteca realmente entrega, antes de desenhar em cima dela.

- Subir o bot com `SqliteStore` em `whatsapp.db`, transport Tokio e HTTP client da lib.
- Exibir QR code no terminal (ou salvar como imagem) no primeiro pareamento; nos próximos starts, reconectar sozinho.
- Logar (apenas em debug, sem conteúdo sensível em nível info) todos os eventos recebidos: mensagem nova, recibos, sincronização de histórico, estado de chats.
- Produzir um arquivo `docs/FINDINGS.md` respondendo, com base no comportamento observado:
  1. A sincronização inicial entrega lista de chats e contadores de não lidas? Em qual evento?
  2. Chega um pacote de histórico recente no pareamento? Dá para controlar o tamanho?
  3. Mensagens recebidas com o backend offline são entregues ao reconectar?
  4. Como marcar uma conversa como lida (e isso sincroniza com o celular)?
  5. Como enviar texto?
  6. Como obter nome de contatos e grupos?

Aceite: pareamento persiste entre reinícios sem novo QR; `FINDINGS.md` existe e responde às 6 perguntas. **Se 1 ou 4 forem impossíveis com a lib, parar e reportar** (ver seção 4).

### Fase 2: persistência

- Camada `store` com migrações (schema da seção 5) e funções: `upsert_chat`, `insert_message`, `set_unread`, `mark_chat_read`, `last_messages(chat, limit)`, `first_unread(chat)`, `list_chats`.
- Handlers de evento da Fase 1 passam a gravar no SQLite.
- Testes unitários da camada `store` com SQLite em memória, incluindo idempotência, paginação por `before` e a localização da primeira não lida.

Aceite: com o backend rodando, mensagens recebidas aparecem no banco; `cargo test` verde.

### Fase 3: API local

Servidor `axum` em `127.0.0.1:<porta>`, com autenticação por token estático (header `Authorization: Bearer ...`, token gerado na primeira execução e salvo na config).

Endpoints (JSON):

- `GET /api/chats` : lista ordenada por `last_msg_ts` desc, cada item com `jid`, `name`, `unread_count`, `last_msg_preview`, `last_msg_ts`. Paginação por `limit`/`before_ts`.
- `GET /api/chats/{jid}/messages` : retorna as últimas `limit` mensagens (padrão 50) do banco local, que já inclui as vistas antes e as não lidas atuais, mais o campo `first_unread_id` para o front posicionar o separador. Se houver mais não lidas do que `limit`, devolve pelo menos todas as não lidas.
- `GET /api/chats/{jid}/messages?before=<ts>&limit=<n>` : carrega mais para cima sob demanda (somente do banco local).
- `POST /api/chats/{jid}/read` : marca como lido (local + WhatsApp).
- `POST /api/chats/{jid}/send` : corpo `{ "text": "..." }`. Envia uma mensagem de texto por requisição, sem fila, sem agendamento.
- `GET /api/events` : **SSE** com eventos `message.new`, `message.status`, `chat.updated`, `connection.state`.
- `GET /api/status` : estado da conexão (conectado, desconectado, aguardando QR).

Regras:
- Validar entradas; limites máximos em `limit`.
- Rate limit simples no `send` (ex.: no máximo algumas mensagens por segundo) para impedir comportamento que pareça automação, mesmo por bug no front.
- CORS restrito à origem do front local.

Aceite: testes de integração dos endpoints com `curl`/`reqwest`; SSE entrega evento ao chegar mensagem real.

### Fase 4: frontend

- Tela única responsiva: lista de chats à esquerda (ou tela cheia no mobile), conversa à direita (ou tela separada no mobile).
- Lista de chats mostra nome, preview da última mensagem, horário e badge de não lidas. Ordenada por atividade.
- Ao abrir um chat: busca as últimas mensagens do banco local (já vistas + não lidas), rola até a primeira não lida, mostra um separador "mensagens não lidas". Botão "carregar anteriores" busca mais via `before`.
- Atualização em tempo real via `EventSource` em `/api/events` (reconexão automática, indicador de estado de conexão).
- Compositor de texto simples (Enter envia, Shift+Enter quebra linha).
- **Marcar como lido**: configurável (automático ao abrir o chat, ou só por ação do usuário). Padrão: só por ação, porque o recibo de leitura é visível ao contato.
- Mídia aparece como placeholder textual ("[imagem]", "[áudio]") nesta versão.
- Tema: **Catppuccin Mocha por padrão**, sempre, sem depender de `prefers-color-scheme`. Ver seção 6.1 para a paleta e o mapeamento de cores.
- Meta de desempenho: primeira renderização útil (lista de chats com não lidas) em menos de 200 ms com o backend já rodando.

Aceite: abrir o front mostra a lista instantaneamente; abrir um chat mostra as últimas mensagens do banco com o separador na primeira não lida; mensagem nova aparece sem recarregar.

### 6.1 Tema: Catppuccin Mocha

Todo o visual do front usa a paleta **Catppuccin Mocha** como padrão. Definir as cores como variáveis CSS em `:root` (sem hex solto espalhado pelos componentes) e consumir só as variáveis.

```css
:root {
  --ctp-rosewater: #f5e0dc;
  --ctp-flamingo:  #f2cdcd;
  --ctp-pink:      #f5c2e7;
  --ctp-mauve:     #cba6f7;
  --ctp-red:       #f38ba8;
  --ctp-maroon:    #eba0ac;
  --ctp-peach:     #fab387;
  --ctp-yellow:    #f9e2af;
  --ctp-green:     #a6e3a1;
  --ctp-teal:      #94e2d5;
  --ctp-sky:       #89dceb;
  --ctp-sapphire:  #74c7ec;
  --ctp-blue:      #89b4fa;
  --ctp-lavender:  #b4befe;
  --ctp-text:      #cdd6f4;
  --ctp-subtext1:  #bac2de;
  --ctp-subtext0:  #a6adc8;
  --ctp-overlay2:  #9399b2;
  --ctp-overlay1:  #7f849c;
  --ctp-overlay0:  #6c7086;
  --ctp-surface2:  #585b70;
  --ctp-surface1:  #45475a;
  --ctp-surface0:  #313244;
  --ctp-base:      #1e1e2e;
  --ctp-mantle:    #181825;
  --ctp-crust:     #11111b;

  color-scheme: dark;
}
```

Mapeamento sugerido para a interface (tokens semânticos em cima da paleta):

| Uso | Cor |
| --- | --- |
| Fundo da conversa | `--ctp-base` |
| Barra lateral / lista de chats | `--ctp-mantle` |
| Barra superior, compositor | `--ctp-crust` ou `--ctp-surface0` |
| Chat selecionado / hover | `--ctp-surface0` e `--ctp-surface1` |
| Texto principal | `--ctp-text` |
| Texto secundário (preview, horário) | `--ctp-subtext0` |
| Bolha de mensagem recebida | `--ctp-surface0` |
| Bolha de mensagem enviada | `--ctp-mauve` com texto `--ctp-crust` (ou `--ctp-blue`) |
| Badge de não lidas, separador de não lidas | `--ctp-mauve` ou `--ctp-lavender` |
| Estado conectado / erro / reconectando | `--ctp-green` / `--ctp-red` / `--ctp-yellow` |
| Links e foco de teclado | `--ctp-blue` |
| Bordas e divisores | `--ctp-surface1` |

Regras:
- Mocha é o padrão e vale mesmo que o sistema esteja em modo claro.
- Verificar contraste de texto sobre as bolhas e badges (mínimo WCAG AA); ajustar a escolha de cor de acento se algum par ficar fraco.
- Opcional (baixa prioridade): um botão que alterna para outro sabor do Catppuccin (por exemplo Latte), guardado em `localStorage`. Se não for implementado, o app fica apenas em Mocha.
- Fora do front, o que for visual segue a mesma paleta quando fizer sentido: o QR code no terminal e eventuais páginas HTML de status do backend usam fundo `--ctp-base` e texto `--ctp-text`.

### Fase 5: robustez e operação

- Reconexão automática com backoff exponencial; estado refletido em `/api/status` e no front.
- Desligamento limpo (SIGINT/SIGTERM): fecha conexão e faz flush do SQLite.
- Empacotar como serviço (unit `systemd --user` e/ou container) para rodar continuamente em homelab/desktop.
- Backup simples de `whatsapp.db` e do banco de mensagens (script).
- Logs sem conteúdo de mensagens em nível info.
- `README.md` com instruções de pareamento, execução, riscos (seção 4) e como trocar de número.

Aceite: matar a rede e restaurar reconecta sozinho; reiniciar o serviço não pede novo QR e não perde mensagens recebidas nesse intervalo (dentro do que o WhatsApp entregar).

## 7. Qualidade e segurança

- `cargo clippy -- -D warnings`, `cargo fmt`, `cargo test` em todo commit.
- Sem `unwrap()`/`expect()` em caminhos de produção; erros com `thiserror`/`anyhow` e propagados até a borda da API.
- Nenhum segredo (token, `whatsapp.db`) versionado; `.gitignore` cobrindo bancos e config local.
- Backend só em loopback. Se um dia for exposto na rede, exigir TLS e revisar a autenticação antes.
- Dados são pessoais e sensíveis: o banco de mensagens fica em disco do usuário; considerar criptografia em repouso apenas se o dispositivo for compartilhado (fora do escopo inicial).

## 8. Estrutura de pastas sugerida

```
whatsapp-lite/
├── PLANO.md
├── README.md
├── backend/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── config.rs
│       ├── wa/          # integração com whatsapp-rust (isolada)
│       ├── store/       # SQLite da aplicação + migrações
│       └── api/         # axum: rotas, SSE, auth
├── frontend/
│   ├── package.json
│   └── src/
│       ├── api/         # cliente HTTP + EventSource
│       ├── components/
│       └── styles/      # CSS puro, variáveis, responsivo
└── docs/
    └── FINDINGS.md      # saída da Fase 1
```

Importante: tudo que depende da biblioteca de WhatsApp fica em `backend/src/wa/` atrás de uma interface própria (trait). O restante do código não importa tipos da lib. Isso permite trocar por whatsmeow ou outra implementação sem reescrever store, API e front.

## 9. Fora do escopo (versão inicial)

- Download/upload de mídia, áudio, figurinhas animadas, chamadas.
- Status/stories, comunidades, canais.
- Múltiplas contas.
- Envio programado, respostas automáticas, qualquer automação.
- Busca em histórico completo.
- Distribuição pública do app (cada usuário assumiria o risco de ban com o próprio número; se isso mudar, reavaliar a Cloud API oficial).

## 10. Definição de pronto

- Pareia uma vez e persiste.
- Abre o front e vê chats com não lidas em tempo imediato.
- Abrir um chat carrega as últimas mensagens guardadas no banco local (vistas antes + não lidas atuais), com opção de carregar mais para cima.
- Recebe mensagens novas em tempo real e consegue responder manualmente.
- Reconecta sozinho, roda como serviço, sem `unsafe` no código próprio e com lint/testes verdes.
