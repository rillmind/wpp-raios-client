import { Fragment, useLayoutEffect, useRef, useState } from 'react'
import type { Chat, Message } from '../api/types'
import { Avatar } from './Avatar'
import { Composer, type FileMeta } from './Composer'
import { MediaView } from './MediaView'
import { MessageMenu, type MessagePick } from './MessageMenu'
import {
  chatName,
  formatPhone,
  formatTime,
  isReadOnly,
  messageText,
  SENDER_COLORS,
  senderName,
} from './format'

export interface ConvState {
  jid: string
  messages: Message[]
  firstUnreadId: string | null
  hasMore: boolean
  loading: boolean
  error: string | null
  /** Esperando o celular mandar mensagens antigas. */
  pending: boolean
  /** Ordem das cores de nome nesta abertura da conversa (sorteada ao abrir). */
  palette: number[]
}


interface Props {
  chat: Chat | undefined
  conv: ConvState
  connected: boolean
  onBack: () => void
  onLoadOlder: () => Promise<void>
  onMarkRead: () => Promise<void>
  onSend: (text: string) => Promise<void>
  onSendFile: (file: File, caption: string | null, meta: FileMeta) => Promise<void>
  /** Mensagem sendo respondida (aparece acima da caixa). */
  replyTo: Message | null
  onCancelReply: () => void
  onMessagePick: (m: Message, pick: MessagePick) => void
}

/** Dentro disso o duplo clique é do próprio controle (mídia, citação, links). */
const INTERACTIVE = 'button, a, audio, video, input, textarea'

/** Rola até a mensagem e pisca para achar. */
function jumpTo(id: string) {
  const el = document.getElementById(`msg-${id}`)
  if (!el) return
  el.scrollIntoView({ block: 'center', behavior: 'smooth' })
  el.classList.add('bubble--flash')
  setTimeout(() => el.classList.remove('bubble--flash'), 1200)
}

function quoteText(m: Message): string {
  return m.kind === 'text' ? (m.body ?? '') : messageText(m)
}

const STATUS_ICON: Record<string, string> = { sent: '✓', delivered: '✓✓', read: '✓✓' }

export function Conversation({
  chat,
  conv,
  connected,
  onBack,
  onLoadOlder,
  onMarkRead,
  onSend,
  onSendFile,
  replyTo,
  onCancelReply,
  onMessagePick,
}: Props) {
  const [menu, setMenu] = useState<{ m: Message; x: number; y: number } | null>(null)

  // Cor de cada remetente, fixa enquanto a conversa está aberta. Quem aparece primeiro
  // (das mais novas para as mais antigas) pega a próxima cor da ordem sorteada; uma cor
  // só se repete quando há mais pessoas do que cores.
  const [colors, setColors] = useState<{ jid: string; bySender: Record<string, number> }>({
    jid: conv.jid,
    bySender: {},
  })
  const assigned = colors.jid === conv.jid ? colors.bySender : {}
  const missing: string[] = []
  for (let i = conv.messages.length - 1; i >= 0; i--) {
    const s = conv.messages[i].sender_jid
    if (!conv.messages[i].from_me && s && !(s in assigned) && !missing.includes(s)) missing.push(s)
  }
  if (colors.jid !== conv.jid || missing.length > 0) {
    const next = { ...assigned }
    for (const s of missing) next[s] = Object.keys(next).length
    setColors({ jid: conv.jid, bySender: next })
  }
  const senderColor = (s: string | null) => {
    const n = s != null ? assigned[s] : undefined
    if (n == null || conv.palette.length === 0) return undefined
    return `var(--ctp-${SENDER_COLORS[conv.palette[n % conv.palette.length]]})`
  }
  const scroller = useRef<HTMLDivElement>(null)
  const positioned = useRef<string | null>(null)
  const restore = useRef<number | null>(null)
  const stick = useRef(true)

  // Ao abrir: rola até a primeira não lida (ou o fim). Depois: mantém no fim
  // se o usuário já estava lá, e preserva a posição ao carregar anteriores.
  useLayoutEffect(() => {
    const el = scroller.current
    if (!el || conv.loading) return
    if (positioned.current !== conv.jid) {
      positioned.current = conv.jid
      const sep = el.querySelector<HTMLElement>('.unread-sep')
      if (sep) el.scrollTop = sep.offsetTop - el.clientHeight / 3
      else el.scrollTop = el.scrollHeight
      return
    }
    if (restore.current != null) {
      el.scrollTop = el.scrollHeight - restore.current
      restore.current = null
    } else if (stick.current) {
      el.scrollTop = el.scrollHeight
    }
  }, [conv])

  async function loadOlder() {
    const el = scroller.current
    if (el) restore.current = el.scrollHeight - el.scrollTop
    await onLoadOlder()
  }

  const isGroup = chat?.is_group ?? conv.jid.endsWith('@g.us')
  // No status cada mensagem é de uma pessoa: mostra quem postou, como em grupo.
  const showSender = isGroup || conv.jid === 'status@broadcast'
  const readOnly = isReadOnly(conv.jid)
  const unread = chat?.unread_count ?? 0
  const byId = new Map(conv.messages.map((m) => [m.id, m]))
  const pinned = conv.messages.filter((m) => m.pinned).at(-1)
  const canReply = (m: Message) => !readOnly && m.kind !== 'revoked'
  const who = (m: Message) => (m.from_me ? 'Você' : showSender ? senderName(m) : chat ? chatName(chat) : '')

  return (
    <section className="conversation" aria-label="Conversa">
      <header className="topbar">
        <button type="button" className="btn btn--ghost topbar__back" onClick={onBack} aria-label="Voltar">
          ←
        </button>
        {chat && <Avatar jid={chat.jid} name={chatName(chat)} size="sm" />}
        <div className="topbar__title">
          <h2 className="topbar__name">{chat ? chatName(chat) : 'Conversa'}</h2>
          {chat?.name && chat.phone && <p className="topbar__sub">{formatPhone(chat.phone)}</p>}
        </div>
        {unread > 0 && (
          <button type="button" className="btn" onClick={() => void onMarkRead()} disabled={!connected}>
            Marcar como lida
          </button>
        )}
      </header>
      {pinned && (
        <button type="button" className="pinned-bar" onClick={() => jumpTo(pinned.id)}>
          <span aria-hidden="true">📌</span>
          <span className="pinned-bar__text">{quoteText(pinned)}</span>
        </button>
      )}

      <div
        className="messages"
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80
        }}
      >
        {conv.hasMore && !conv.loading && (
          <button
            type="button"
            className="btn btn--ghost load-older"
            disabled={conv.pending}
            onClick={() => void loadOlder()}
          >
            {conv.pending
              ? 'Pedindo ao celular…'
              : conv.messages.length === 0
                ? 'Carregar mensagens'
                : 'Carregar anteriores'}
          </button>
        )}
        {!conv.hasMore && !conv.loading && conv.messages.length === 0 && (
          <p className="empty">Nenhuma mensagem.</p>
        )}
        {conv.loading && <p className="empty">Carregando…</p>}
        {conv.error && (
          <p className="empty empty--error" role="alert">
            {conv.error}
          </p>
        )}
        {conv.messages.map((m) => (
          <Fragment key={m.id}>
            {m.id === conv.firstUnreadId && (
              <div className="unread-sep" role="separator">
                <span>mensagens não lidas</span>
              </div>
            )}
            <div
              className={m.from_me ? 'msg-row msg-row--out' : 'msg-row'}
              // Dois cliques na mensagem ou no espaço ao lado dela: responder.
              onDoubleClick={(e) => {
                if (!canReply(m) || (e.target as HTMLElement).closest(INTERACTIVE)) return
                window.getSelection()?.removeAllRanges()
                onMessagePick(m, 'reply')
              }}
              // Evita que o duplo clique selecione a palavra.
              onMouseDown={(e) => {
                if (e.detail > 1 && canReply(m)) e.preventDefault()
              }}
            >
              <div
                id={`msg-${m.id}`}
                className={[
                  'bubble',
                  m.from_me ? 'bubble--out' : 'bubble--in',
                  m.media && m.kind === 'sticker' ? 'bubble--sticker' : '',
                ].join(' ')}
                onContextMenu={(e) => {
                  e.preventDefault()
                  setMenu({ m, x: e.clientX, y: e.clientY })
                }}
              >
                {showSender && !m.from_me && (
                <span className="bubble__sender" style={{ color: senderColor(m.sender_jid) }}>
                  {senderName(m)}
                </span>
              )}
                {m.quoted_id && (
                  <button type="button" className="quote" onClick={() => jumpTo(m.quoted_id!)}>
                    {byId.has(m.quoted_id) ? (
                      <>
                        <span className="quote__who">{who(byId.get(m.quoted_id)!)}</span>
                        <span className="quote__text">{quoteText(byId.get(m.quoted_id)!)}</span>
                      </>
                    ) : (
                      <span className="quote__text">Mensagem citada</span>
                    )}
                  </button>
                )}
                {m.media ? (
                  <>
                    <MediaView chatJid={conv.jid} message={m} />
                    {/* Documento já mostra o nome no botão; o corpo só se for legenda. */}
                    {m.body && m.body !== m.media.file_name && <span className="bubble__text">{m.body}</span>}
                  </>
                ) : (
                  <span className={m.kind === 'text' ? 'bubble__text' : 'bubble__text bubble__text--media'}>
                    {messageText(m)}
                  </span>
                )}
                <span className="bubble__meta">
                  {m.starred && <span aria-label="Favoritada">★</span>}
                  {formatTime(m.ts)}
                  {m.from_me && m.status && (
                    <span className={m.status === 'read' ? 'tick tick--read' : 'tick'} aria-label={m.status}>
                      {STATUS_ICON[m.status]}
                    </span>
                  )}
                </span>
              </div>
            </div>
          </Fragment>
        ))}
      </div>

      {readOnly ? (
        <p className="readonly-note">Status e canais são só de leitura aqui.</p>
      ) : (
        <Composer
          key={conv.jid}
          disabled={!connected}
          onSend={onSend}
          onSendFile={onSendFile}
          reply={replyTo ? { who: who(replyTo), text: quoteText(replyTo) } : null}
          onCancelReply={onCancelReply}
        />
      )}
      {menu && (
        <MessageMenu
          message={menu.m}
          chatJid={conv.jid}
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          onPick={(pick) => onMessagePick(menu.m, pick)}
        />
      )}
    </section>
  )
}
