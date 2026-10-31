import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { api, ApiError, getToken, setToken } from './api/client'
import type { Chat, ConnState, ServerEvent } from './api/types'
import { useEvents } from './api/useEvents'
import { ChatList } from './components/ChatList'
import { Conversation, type ConvState } from './components/Conversation'
import { Login } from './components/Login'
import { chatSection, SECTION_TITLE, type Section } from './components/format'
import { Rail } from './components/Rail'

const AUTO_READ_KEY = 'wpp.autoRead'

function loadAutoRead(): boolean {
  try {
    return localStorage.getItem(AUTO_READ_KEY) === '1'
  } catch {
    return false
  }
}

function byActivity(a: Chat, b: Chat): number {
  return (b.last_msg_ts ?? -1) - (a.last_msg_ts ?? -1)
}

const CONN_LABEL: Record<ConnState, string> = {
  connecting: 'Conectando…',
  waiting_qr: 'Aguardando pareamento (QR no terminal)',
  connected: 'Conectado',
  disconnected: 'Reconectando…',
  logged_out: 'Sessão encerrada: pareie de novo',
}

export default function App() {
  const [token, setTokenState] = useState(getToken())
  const [authError, setAuthError] = useState<string | null>(null)

  if (!token) {
    return (
      <Login
        error={authError}
        onSubmit={(t) => {
          setToken(t)
          setAuthError(null)
          setTokenState(t)
        }}
      />
    )
  }
  return (
    <Client
      key={token}
      onUnauthorized={() => {
        setToken(null)
        setAuthError('Token inválido.')
        setTokenState(null)
      }}
    />
  )
}

function Client({ onUnauthorized }: { onUnauthorized: () => void }) {
  const [chats, setChats] = useState<Chat[]>([])
  const [conn, setConn] = useState<ConnState>('connecting')
  const [selected, setSelected] = useState<string | null>(null)
  const [conv, setConv] = useState<ConvState | null>(null)
  const [section, setSection] = useState<Section>('chats')
  const [autoRead, setAutoRead] = useState(loadAutoRead)
  const selectedRef = useRef(selected)
  useLayoutEffect(() => {
    selectedRef.current = selected
  }, [selected])

  const fail = useCallback(
    (e: unknown) => {
      if (e instanceof ApiError && e.status === 401) onUnauthorized()
      return e instanceof Error ? e.message : 'Erro inesperado'
    },
    [onUnauthorized],
  )

  const loadChats = useCallback(() => {
    api.chats().then(setChats, fail)
  }, [fail])

  const openChat = useCallback(
    async (jid: string) => {
      setConv({ jid, messages: [], firstUnreadId: null, hasMore: false, loading: true, error: null })
      try {
        const page = await api.messages(jid)
        if (selectedRef.current !== jid) return
        setConv({
          jid,
          messages: page.messages,
          firstUnreadId: page.first_unread_id,
          hasMore: page.has_more,
          loading: false,
          error: null,
        })
      } catch (e) {
        const error = fail(e)
        setConv((c) => (c?.jid === jid ? { ...c, loading: false, error } : c))
      }
    },
    [fail],
  )

  useEffect(loadChats, [loadChats])

  const markRead = useCallback(
    async (jid: string) => {
      try {
        const chat = await api.markRead(jid)
        setChats((cs) => cs.map((c) => (c.jid === jid ? chat : c)))
      } catch (e) {
        fail(e)
      }
    },
    [fail],
  )

  const onEvent = useCallback(
    (ev: ServerEvent) => {
      switch (ev.type) {
        case 'connection.state':
          setConn(ev.data.state)
          break
        case 'chat.updated':
          setChats((cs) => [ev.data, ...cs.filter((c) => c.jid !== ev.data.jid)].sort(byActivity))
          break
        case 'message.new': {
          const m = ev.data
          setConv((c) => {
            if (!c || c.jid !== m.chat_jid || c.messages.some((x) => x.id === m.id)) return c
            return { ...c, messages: [...c.messages, m] }
          })
          if (autoRead && !m.from_me && m.chat_jid === selectedRef.current && document.visibilityState === 'visible') {
            void markRead(m.chat_jid)
          }
          break
        }
        case 'message.status': {
          const { chat_jid, ids, status } = ev.data
          setConv((c) =>
            c && c.jid === chat_jid
              ? { ...c, messages: c.messages.map((m) => (ids.includes(m.id) ? { ...m, status } : m)) }
              : c,
          )
          break
        }
      }
    },
    [autoRead, markRead],
  )

  const onResync = useCallback(() => {
    loadChats()
    if (selectedRef.current) void openChat(selectedRef.current)
  }, [loadChats, openChat])

  const stream = useEvents(onEvent, onResync)

  const closeChat = useCallback(() => {
    selectedRef.current = null
    setSelected(null)
    setConv(null)
  }, [])

  // Esc dentro de uma conversa volta para a tela inicial (inclusive digitando).
  useEffect(() => {
    if (!selected) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.isComposing) closeChat()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [selected, closeChat])

  function select(jid: string) {
    selectedRef.current = jid
    setSelected(jid)
    void openChat(jid)
    const chat = chats.find((c) => c.jid === jid)
    if (autoRead && chat && chat.unread_count > 0 && conn === 'connected') void markRead(jid)
  }

  async function loadOlder() {
    if (!conv || conv.messages.length === 0) return
    const jid = conv.jid
    try {
      const page = await api.messages(jid, conv.messages[0].ts)
      setConv((c) =>
        c?.jid === jid
          ? {
              ...c,
              messages: [...page.messages.filter((m) => !c.messages.some((x) => x.id === m.id)), ...c.messages],
              hasMore: page.has_more,
            }
          : c,
      )
    } catch (e) {
      fail(e)
    }
  }

  async function send(text: string) {
    if (!selected) return
    const jid = selected
    try {
      const m = await api.send(jid, text)
      setConv((c) =>
        c?.jid === jid && !c.messages.some((x) => x.id === m.id) ? { ...c, messages: [...c.messages, m] } : c,
      )
    } catch (e) {
      throw new Error(fail(e))
    }
  }

  function toggleAutoRead() {
    const next = !autoRead
    setAutoRead(next)
    try {
      localStorage.setItem(AUTO_READ_KEY, next ? '1' : '0')
    } catch {
      // Preferência vale só nesta aba.
    }
  }

  const visible = useMemo(() => chats.filter((c) => chatSection(c) === section), [chats, section])
  const unread = useMemo(() => {
    const n: Record<Section, number> = { chats: 0, archived: 0, status: 0, channels: 0 }
    for (const c of chats) if (c.unread_count > 0) n[chatSection(c)] += 1
    return n
  }, [chats])
  const connClass = stream !== 'open' ? 'warn' : conn === 'connected' ? 'ok' : conn === 'disconnected' || conn === 'connecting' ? 'warn' : 'err'
  const connText = stream !== 'open' ? 'Sem conexão com o backend…' : CONN_LABEL[conn]

  return (
    <div className={selected ? 'app app--chat-open' : 'app'}>
      <Rail
        section={section}
        unread={unread}
        autoRead={autoRead}
        onSection={setSection}
        onToggleAutoRead={toggleAutoRead}
      />
      <aside className="sidebar">
        <header className="topbar">
          <h1 className="topbar__title">{SECTION_TITLE[section]}</h1>
        </header>
        <p className={`conn conn--${connClass}`} role="status">
          <span className="conn__dot" aria-hidden="true" />
          {connText}
        </p>
        <ChatList chats={visible} selected={selected} onSelect={select} />
      </aside>

      <main className="main">
        {conv ? (
          <Conversation
            chat={chats.find((c) => c.jid === conv.jid)}
            conv={conv}
            connected={conn === 'connected' && stream === 'open'}
            onBack={closeChat}
            onLoadOlder={loadOlder}
            onMarkRead={() => markRead(conv.jid)}
            onSend={send}
          />
        ) : (
          <p className="empty empty--center">Selecione uma conversa.</p>
        )}
      </main>
    </div>
  )
}
