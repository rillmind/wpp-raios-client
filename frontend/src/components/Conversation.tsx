import { Fragment, useLayoutEffect, useRef } from 'react'
import type { Chat, Message } from '../api/types'
import { Composer } from './Composer'
import { chatName, formatTime, messageText, shortJid } from './format'

export interface ConvState {
  jid: string
  messages: Message[]
  firstUnreadId: string | null
  hasMore: boolean
  loading: boolean
  error: string | null
}

interface Props {
  chat: Chat | undefined
  conv: ConvState
  connected: boolean
  onBack: () => void
  onLoadOlder: () => Promise<void>
  onMarkRead: () => Promise<void>
  onSend: (text: string) => Promise<void>
}

const STATUS_ICON: Record<string, string> = { sent: '✓', delivered: '✓✓', read: '✓✓' }

export function Conversation({ chat, conv, connected, onBack, onLoadOlder, onMarkRead, onSend }: Props) {
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
  const unread = chat?.unread_count ?? 0

  return (
    <section className="conversation" aria-label="Conversa">
      <header className="topbar">
        <button type="button" className="btn btn--ghost topbar__back" onClick={onBack} aria-label="Voltar">
          ←
        </button>
        <h2 className="topbar__title">{chat ? chatName(chat) : shortJid(conv.jid)}</h2>
        {unread > 0 && (
          <button type="button" className="btn" onClick={() => void onMarkRead()} disabled={!connected}>
            Marcar como lida
          </button>
        )}
      </header>

      <div
        className="messages"
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80
        }}
      >
        {conv.hasMore && (
          <button type="button" className="btn btn--ghost load-older" onClick={() => void loadOlder()}>
            Carregar anteriores
          </button>
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
            <div className={m.from_me ? 'bubble bubble--out' : 'bubble bubble--in'}>
              {isGroup && !m.from_me && <span className="bubble__sender">{shortJid(m.sender_jid)}</span>}
              <span className={m.kind === 'text' ? 'bubble__text' : 'bubble__text bubble__text--media'}>
                {messageText(m)}
              </span>
              <span className="bubble__meta">
                {formatTime(m.ts)}
                {m.from_me && m.status && (
                  <span className={m.status === 'read' ? 'tick tick--read' : 'tick'} aria-label={m.status}>
                    {STATUS_ICON[m.status]}
                  </span>
                )}
              </span>
            </div>
          </Fragment>
        ))}
      </div>

      <Composer disabled={!connected} onSend={onSend} />
    </section>
  )
}
