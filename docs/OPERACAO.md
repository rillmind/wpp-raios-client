# Operação

## Backup

```sh
scripts/backup.sh ~/.local/share/wpp-lite/data ~/backups/wpp-lite   # serviço
scripts/backup.sh ./data ~/backups/wpp-lite                          # docker
```

Copia `whatsapp.db`, `messages.db` e `config.toml`, mantendo os 10 mais recentes. Com o
`sqlite3` instalado a cópia é consistente com o app rodando; sem ele, pare o app antes.

## Trocar de número

1. Pare o app e faça backup do diretório de dados.
2. No celular antigo, desconecte o aparelho em **Aparelhos conectados**.
3. Mova `whatsapp.db` e `messages.db` para fora do diretório de dados (guarde, não apague: o
   banco de mensagens é do número antigo).
4. Suba o app e pareie o novo número pelo QR.

## Conexão

A biblioteca reconecta com backoff quando a rede volta; o estado aparece em `/api/status` e no
topo da lista de conversas. Mensagens recebidas com o app parado chegam na reconexão (dentro do
que o servidor do WhatsApp guarda). Parar o app (`docker compose down`, `systemctl --user stop`,
Ctrl+C) grava a fila e fecha a sessão antes de sair.

## API

Tudo em `127.0.0.1`. As rotas exigem `Authorization: Bearer <token>`; só `/api/events` aceita
`?token=` (o `EventSource` do navegador não envia cabeçalhos).

| Rota | |
| --- | --- |
| `GET /healthz` | sem token; `ok` se o processo responde |
| `GET /api/pairing` | sem token; `{ state, qr_svg }`, com o QR só enquanto não há celular pareado |
| `GET /api/status` | `{ state, connected }` |
| `GET /api/chats?limit=&before_ts=` | conversas por atividade, com `name` e `phone` |
| `GET /api/chats/{jid}/messages?limit=&before=` | últimas mensagens do banco local + `first_unread_id`; a primeira página sempre inclui todas as não lidas. Mensagens com imagem/figurinha trazem `media` |
| `GET /api/chats/{jid}/messages/{id}/media` | baixa a imagem, figurinha, vídeo ou áudio do WhatsApp na hora (`Cache-Control: no-store`; até 32 MB, vídeo até 100 MB) |
| `GET /api/chats/{jid}/avatar` | foto de perfil, buscada na hora; `204` se não houver |
| `POST /api/chats/{jid}/read` | recibos de leitura + marca lido no celular |
| `POST /api/chats/{jid}/action` | `{ "action": "archive" \| "unarchive" \| "pin" \| "unpin" \| "mute" \| "unmute" \| "unread", "hours"? }` (`hours` só em `mute`; sem ele, para sempre) |
| `POST /api/chats/{jid}/send` | `{ "text": "..." }`, uma mensagem por requisição |
| `GET /api/events` | SSE: `message.new`, `message.status`, `chat.updated`, `connection.state` (e `resync`) |

Logs em nível `info` não têm conteúdo de mensagens.

## CI e publicação

`.github/workflows/ci.yml` roda fmt, clippy, testes, lint e build do front, compila a imagem e
faz um teste com ela no ar (API, front e desligamento limpo). Em PR para por aí. Em push na
`main` publica `softfrost/wpp-lite:latest` e `:sha-<commit>`; numa tag `v1.2.3`, também `:1.2.3`
e `:1.2`.

No GitHub (Settings → Secrets and variables → Actions): variável `DOCKERHUB_USERNAME` e secret
`DOCKERHUB_TOKEN` (access token do Docker Hub, Read & Write).

Para gerar a imagem localmente: `docker build -t softfrost/wpp-lite:dev .` e
`WPP_TAG=dev docker compose up -d`.
