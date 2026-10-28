# WhatsApp Lite

Cliente de WhatsApp pessoal e local que **abre instantaneamente**: o backend fica sempre
conectado e grava tudo num SQLite próprio; o front só lê esse banco pela API local. Nada de
sincronizar o histórico antigo; para isso, use o celular.

Uso previsto: um humano lendo e respondendo. Sem bot, sem automação, sem envio em massa.

```
WhatsApp <──> backend Rust (whatsapp-rust + SQLite) <──HTTP/SSE 127.0.0.1──> front React
```

Plano completo em [`plano.md`](plano.md); o que a biblioteca entrega, em
[`docs/FINDINGS.md`](docs/FINDINGS.md).

## Riscos (leia antes)

- Clientes não oficiais violam os termos do WhatsApp. Existe risco de **suspensão do número**.
  O uso aqui é o de menor risco (leitura e resposta manual), mas não é zero.
- **Desenvolva e teste com um número secundário.** Migre para o principal só depois de estável.
- **Não pareie e despareie repetidamente.** Pareie uma vez; a sessão fica em `data/whatsapp.db`.
  Faça backup dele antes de mexer em código de sessão e **nunca o apague**.
- O recibo de leitura é visível ao contato. Por isso, por padrão, o app só marca como lida
  quando você clica em "Marcar como lida" (a opção "Ler ao abrir" muda isso).

## Requisitos

- Rust estável (testado com 1.97) e Node 20+.
- A feature `simd` da lib exige nightly e está desligada; não é preciso nightly.

## Desenvolvimento

```sh
# backend: cria data/config.toml com um token na primeira execução
RUST_LOG=info,wpp_backend=debug cargo run

# front, em outro terminal (http://127.0.0.1:5173)
cd frontend && npm install && npm run dev
```

No primeiro start o QR aparece no terminal. No celular: **WhatsApp → Aparelhos conectados →
Conectar aparelho**. Nos próximos starts ele reconecta sozinho, sem QR.

O front pede o token uma vez: copie o campo `token` de `data/config.toml`.

CI local (fmt, clippy, testes, lint e build do front): `scripts/check.sh`.

### Configuração

`data/config.toml` (criado automaticamente, permissão 600), sobrescrito por variáveis de ambiente:

| Campo | Env | Padrão | |
| --- | --- | --- | --- |
| — | `WPP_DATA_DIR` | `data` | diretório de config e bancos |
| `port` | `WPP_PORT` | `8787` | porta da API e do front |
| — | `WPP_BIND` | `127.0.0.1` | endereço de escuta; `0.0.0.0` só dentro de container |
| `token` | `WPP_TOKEN` | aleatório | `Authorization: Bearer ...` |
| `front_origin` | `WPP_FRONT_ORIGIN` | `http://127.0.0.1:5173` | única origem aceita no CORS |
| `front_dir` | `WPP_FRONT_DIR` | — | serve o build do front na mesma origem |
| `send_per_sec` | — | `2` | limite de envios por segundo |
| `history_msgs_per_chat` | — | `50` | mensagens por chat guardadas da sincronização inicial |

## Rodar como serviço (systemd --user)

```sh
scripts/install-service.sh
journalctl --user -u wpp-lite -f      # QR do primeiro pareamento aparece aqui
```

Instala em `~/.local/share/wpp-lite/` (binário, build do front e `data/`) e abre em
`http://127.0.0.1:8787`. Para levar um `data/` já pareado do desenvolvimento, pare o serviço e
copie o diretório para `~/.local/share/wpp-lite/data/`.

O serviço reinicia se cair. A própria lib reconecta com backoff quando a rede volta; o estado
aparece em `/api/status` e no topo da lista de chats. Mensagens recebidas com o serviço parado
são entregues pelo WhatsApp na reconexão (dentro do que o servidor guarda).

## Backup

```sh
scripts/backup.sh ~/.local/share/wpp-lite/data ~/backups/wpp-lite
```

Copia `whatsapp.db`, `messages.db` e `config.toml`, mantendo os 10 mais recentes. Com o `sqlite3`
instalado a cópia é consistente com o serviço rodando; sem ele, pare o serviço antes.

## Trocar de número

1. Pare o serviço e faça backup do diretório de dados.
2. No celular antigo, desconecte o aparelho em **Aparelhos conectados**.
3. Mova `whatsapp.db` e `messages.db` para fora do diretório de dados (guarde, não apague: o
   banco de mensagens é do número antigo).
4. Suba o serviço e pareie o novo número pelo QR.

## API

Todas as rotas exigem `Authorization: Bearer <token>`; só `/api/events` aceita `?token=`
(o `EventSource` do navegador não envia cabeçalhos).

| Rota | |
| --- | --- |
| `GET /api/status` | `{ state, connected }` |
| `GET /api/chats?limit=&before_ts=` | chats por atividade |
| `GET /api/chats/{jid}/messages?limit=&before=` | últimas mensagens do banco local + `first_unread_id`; a primeira página sempre inclui todas as não lidas |
| `POST /api/chats/{jid}/read` | recibos de leitura + marca lido no celular |
| `POST /api/chats/{jid}/send` | `{ "text": "..." }`, uma mensagem por requisição |
| `GET /api/events` | SSE: `message.new`, `message.status`, `chat.updated`, `connection.state` (e `resync`) |

## Privacidade

- O banco de mensagens fica só no disco local; logs em nível `info` não têm conteúdo de mensagens.
- O backend escuta só em loopback. Antes de expor na rede, exija TLS e revise a autenticação.
