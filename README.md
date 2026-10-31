# WhatsApp Lite

Cliente de WhatsApp pessoal que roda na sua máquina ou no seu servidor e abre no navegador.

## O que faz

- Fica conectado ao WhatsApp como um aparelho vinculado (como o WhatsApp Web) e grava as
  conversas num banco SQLite próprio. A interface lê desse banco, por isso abre na hora.
- Lista as conversas por atividade, com as fixadas no celular no topo (com um alfinete), o
  contador de não lidas, o estado da conexão no topo e, quando a última mensagem é sua, os tiques
  dela (✓ enviada, ✓✓ entregue, ✓✓ azul lida).
- Uma barra lateral separa **Conversas**, **Status**, **Canais** e **Arquivadas** (com o número de
  conversas não lidas, ou um ponto nas outras seções). Status e
  canais são só de leitura: daqui não dá para postar um status nem escrever num canal. Os canais
  aparecem como "Canal", sem nome nem foto (a biblioteca ainda não consegue buscá-los).
- Mostra o nome de cada contato como está na agenda do seu celular; sem nome salvo, o nome que a
  pessoa definiu no WhatsApp; sem nenhum dos dois, o número. Em grupos, mostra quem mandou cada
  mensagem.
- Mostra as fotos de perfil, que carregam sozinhas.
- Imagens e figurinhas aparecem como um botão com o tamanho do arquivo; só são baixadas do
  WhatsApp quando você clica. Ficam só na memória da aba: nada vai para o disco nem para o banco
  (o banco guarda só a referência para baixar). Fechou a aba, sumiram. Figurinhas animadas
  vetoriais aparecem como `[figurinha]`, e imagens recebidas antes desta versão como `[imagem]`.
- Abre a conversa na primeira mensagem não lida, com um separador, e com a caixa de mensagem já
  selecionada (em telas de toque não, para não abrir o teclado). Esc fecha a conversa e volta à
  tela inicial; com uma imagem ampliada, Esc fecha só a imagem.
- Envia mensagens de texto (Enter envia, Shift+Enter quebra linha). Cada conversa tem o seu
  rascunho: trocar de conversa não leva o texto junto.
- Ao abrir uma conversa, marca as mensagens como lidas (o contato vê os tiques azuis e o contador
  zera no celular também). Com a opção **"Não visualizar ao abrir"** marcada, no topo da lista,
  abrir não marca nada: só o botão "Marcar como lida" faz isso.
- Guarda só o que chega a partir do pareamento (e as últimas mensagens de cada conversa na
  sincronização inicial). O histórico antigo continua no celular.
- Áudio, vídeo e documentos aparecem como `[áudio]`, `[vídeo]`, `[documento]`.
- Mensagens de visualização única aparecem como um aviso para abrir no celular: o WhatsApp não
  entrega o conteúdo delas a aparelhos conectados (o mesmo acontece no WhatsApp Web).

## Por que existe

O WhatsApp Web demora para abrir e sincronizar. Aqui o backend fica sempre conectado e a
interface é leve, então abrir para ler e responder é instantâneo. É para uma pessoa lendo e
respondendo: sem bot, sem automação, sem envio em massa (há limite de envios por segundo).

## Antes de usar

- Clientes não oficiais violam os termos do WhatsApp; existe risco de **suspensão do número**.
  O uso aqui (ler e responder à mão) é o de menor risco, mas não é zero. Se puder, teste antes
  com um número secundário.
- **Pareie uma vez.** A sessão fica em `data/whatsapp.db`: faça backup e **nunca o apague**.
  Parear e desparear repetidamente chama atenção.
- **Nunca rode duas instâncias com o mesmo `data/`** (ex.: o serviço e o `cargo run`): o
  WhatsApp pode derrubar a sessão.

## Como rodar

Em todos os jeitos abaixo, no primeiro início o QR de pareamento aparece na primeira tela da
interface (e também no terminal ou nos logs). No celular: **WhatsApp → Aparelhos conectados →
Conectar aparelho**. Depois disso ele reconecta sozinho e o QR não aparece mais. A interface pede
um token uma vez: é o campo `token` de `data/config.toml`.

### Docker (servidor)

No servidor basta o [`docker-compose.yaml`](docker-compose.yaml); a imagem vem do Docker Hub
([`softfrost/wpp-lite`](https://hub.docker.com/r/softfrost/wpp-lite), amd64).

```sh
mkdir -p data && sudo chown 1000:1000 data
docker compose up -d
docker compose logs -f                       # QR do primeiro pareamento
docker compose pull && docker compose up -d  # atualizar
```

Abre em `http://127.0.0.1:8787` no servidor. A porta só escuta em loopback; para acessar de
outra máquina use um túnel SSH (`ssh -L 8787:127.0.0.1:8787 servidor` e abra
`http://127.0.0.1:8787`) ou um proxy com TLS / Tailscale. Não publique a porta sem TLS: o token
vai em toda requisição.

**Podman rootless:** o usuário 1000 do container não é o seu no host e ele falha com
`Permission denied`. Rode como root do container (que no rootless é o seu próprio usuário, sem
privilégio extra), sem `chown`:

```sh
printf 'WPP_UID=0\nWPP_GID=0\n' > .env
podman compose up -d
```

Variáveis do compose: `WPP_TAG` (`latest`), `WPP_HOST_PORT` (`8787`), `WPP_UID`/`WPP_GID`
(`1000`), `RUST_LOG` (`info`).

**Levar uma sessão já pareada** (sem novo QR): pare a instância antiga, copie o diretório
inteiro (`rsync -a data/ servidor:wpp-lite/data/`), ajuste o dono (`sudo chown -R 1000:1000
data`) e suba.

### Serviço do usuário (systemd)

Precisa de Rust estável (testado com 1.97) e Node 20+.

```sh
scripts/install-service.sh
journalctl --user -u wpp-lite -f      # QR do primeiro pareamento
```

Instala em `~/.local/share/wpp-lite/` (binário, interface e `data/`) e abre em
`http://127.0.0.1:8787`. Reinicia sozinho se cair.

### Desenvolvimento

```sh
RUST_LOG=info,wpp_backend=debug cargo run          # backend, cria data/config.toml
cd frontend && npm install && npm run dev          # interface em http://127.0.0.1:5173
scripts/check.sh                                   # fmt, clippy, testes, lint e build
```

### Configuração

`data/config.toml` é criado no primeiro início (permissão 600). Variáveis de ambiente
sobrescrevem:

| Campo | Env | Padrão | |
| --- | --- | --- | --- |
| — | `WPP_DATA_DIR` | `data` | diretório da config e dos bancos |
| `port` | `WPP_PORT` | `8787` | porta da API e da interface |
| — | `WPP_BIND` | `127.0.0.1` | endereço de escuta; `0.0.0.0` só dentro de container |
| `token` | `WPP_TOKEN` | aleatório | token de acesso |
| `front_origin` | `WPP_FRONT_ORIGIN` | `http://127.0.0.1:5173` | origem aceita no CORS (dev) |
| `front_dir` | `WPP_FRONT_DIR` | — | serve o build da interface na mesma porta |
| `send_per_sec` | — | `2` | limite de envios por segundo |
| `history_msgs_per_chat` | — | `50` | mensagens por conversa guardadas da sincronização inicial |

Backup, troca de número, API e CI: [`docs/OPERACAO.md`](docs/OPERACAO.md).
