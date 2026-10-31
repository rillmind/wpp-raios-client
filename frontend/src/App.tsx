import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { api, ApiError, getToken, setToken } from './api/client'
import type { Chat, ConnState, Message, ServerEvent } from './api/types'
import { useEvents } from './api/useEvents'
import { ChatList } from './components/ChatList'
import { ChatMenu, type MenuAction } from './components/ChatMenu'
import type { FileMeta } from './components/Composer'
import { Confirm, type ConfirmRequest } from './components/Dialog'
import { ForwardDialog } from './components/ForwardDialog'
import type { MessagePick } from './components/MessageMenu'
import { copyText, securityCode, shuffledPalette, withMentionNames } from './components/format'
import { Conversation, type ConvState } from './components/Conversation'
import { Login } from './components/Login'
import { usePairing } from './api/usePairing'
import { PairingQr } from './components/PairingQr'
import { chatSection, SECTION_TITLE, type Section } from './components/format'
import { Rail } from './components/Rail'

// Marcada: abrir a conversa não envia recibo de leitura. Desmarcada (padrão): envia.
const NO_READ_KEY = 'wpp.noRead'
/** Quanto esperar o celular mandar as mensagens antigas. */
const HISTORY_TIMEOUT_MS = 30_000

/** Junta mensagens antigas às da tela, sem repetir e em ordem. */
function withOlder(c: ConvState, older: Message[]): ConvState {
  const known = new Set(c.messages.map((m) => m.id))
  const fresh = older.filter((m) => !known.has(m.id))
  return { ...c, messages: [...fresh, ...c.messages].sort((a, b) => a.ts - b.ts) }
}
const STATUS_JID = 'status@broadcast'

function loadNoRead(): boolean {
  try {
    return localStorage.getItem(NO_READ_KEY) === '1'
  } catch {
    return false
  }
}

/** Mesma ordem do backend: fixadas primeiro, depois por atividade. */
function byActivity(a: Chat, b: Chat): number {
  if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
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
  const [menu, setMenu] = useState<{ chat: Chat; x: number; y: number } | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  // Conversa aberta que ainda não está na lista (sem mensagens); entra na primeira.
  const [draftChat, setDraftChat] = useState<Chat | null>(null)
  const [replyTo, setReplyTo] = useState<Message | null>(null)
  const [forwarding, setForwarding] = useState<Message | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [noRead, setNoRead] = useState(loadNoRead)
  const autoRead = !noRead
  const selectedRef = useRef(selected)
  const convRef = useRef<ConvState | null>(null)
  useLayoutEffect(() => {
    selectedRef.current = selected
  }, [selected])
  useLayoutEffect(() => {
    convRef.current = conv
  }, [conv])

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
      const palette = shuffledPalette()
      setConv({
        jid,
        messages: [],
        firstUnreadId: null,
        hasMore: false,
        loading: true,
        error: null,
        pending: false,
        palette,
      })
      try {
        // Abre com o que está guardado e visível: na primeira vez, só as não lidas.
        const page = await api.messages(jid)
        if (selectedRef.current !== jid) return
        setConv({
          jid,
          messages: page.messages,
          firstUnreadId: page.first_unread_id,
          hasMore: page.has_more,
          loading: false,
          error: null,
          pending: false,
          palette,
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

  // Chegaram as mensagens antigas pedidas ao celular: busca as que ficaram antes da tela.
  const onHistoryLoaded = useCallback(
    async (chatJid: string, count: number) => {
      if (selectedRef.current !== chatJid) return
      try {
        const before = convRef.current?.jid === chatJid ? convRef.current.messages[0]?.ts : undefined
        const page = await api.messages(chatJid, { before, limit: 50 })
        setConv((c) =>
          c?.jid === chatJid
            ? { ...withOlder(c, page.messages), pending: false, hasMore: count > 0 && c.hasMore }
            : c,
        )
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
        case 'history.loaded':
          void onHistoryLoaded(ev.data.chat_jid, ev.data.count)
          break
        case 'message.updated': {
          const m = ev.data
          setConv((c) =>
            c && c.jid === m.chat_jid ? { ...c, messages: c.messages.map((x) => (x.id === m.id ? m : x)) } : c,
          )
          break
        }
        case 'message.deleted': {
          const { chat_jid, id } = ev.data
          setConv((c) => (c && c.jid === chat_jid ? { ...c, messages: c.messages.filter((x) => x.id !== id) } : c))
          setReplyTo((r) => (r?.id === id ? null : r))
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
    [autoRead, markRead, onHistoryLoaded],
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
    setReplyTo(null)
  }, [])

  // Esc dentro de uma conversa, um passo por vez: desmarca a resposta (o foco fica na
  // caixa); depois sai da caixa de mensagem; depois volta para a tela inicial.
  const replying = replyTo !== null
  useEffect(() => {
    if (!selected) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.isComposing) return
      const typing = document.activeElement?.classList.contains('composer__input') ?? false
      if (replying) {
        setReplyTo(null)
        // Extensões de teclado (ex.: Vimium) tiram o foco da caixa no mesmo Esc, antes
        // daqui: devolve o cursor, que é o que este primeiro Esc deve deixar.
        setTimeout(() => document.querySelector<HTMLTextAreaElement>('.composer__input')?.focus(), 0)
      } else if (typing) (document.activeElement as HTMLElement).blur()
      else closeChat()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [selected, replying, closeChat])

  const closeMenu = useCallback(() => setMenu(null), [])

  // Aviso curto quando uma ação do menu falha (ex.: WhatsApp desconectado).
  useEffect(() => {
    if (!notice) return
    const t = setTimeout(() => setNotice(null), 4000)
    return () => clearTimeout(t)
  }, [notice])

  async function pickAction(chat: Chat, pick: MenuAction) {
    if (pick.action === 'read') return markRead(chat.jid)
    try {
      const updated = await api.action(chat.jid, pick.action, pick.action === 'mute' ? pick.hours : undefined)
      setChats((cs) => cs.map((c) => (c.jid === updated.jid ? updated : c)).sort(byActivity))
    } catch (e) {
      setNotice(fail(e))
    }
  }

  // Status é uma conversa só: o botão abre ela direto.
  function chooseSection(s: Section) {
    setSection(s)
    if (s === 'status' && chats.some((c) => c.jid === STATUS_JID)) select(STATUS_JID)
  }

  function select(jid: string) {
    if (jid !== selectedRef.current) setReplyTo(null)
    selectedRef.current = jid
    setSelected(jid)
    void openChat(jid)
    const chat = chats.find((c) => c.jid === jid)
    if (autoRead && chat && chat.unread_count > 0 && conn === 'connected') void markRead(jid)
  }

  /** Abre a conversa com uma pessoa (clicada num grupo) ou com um telefone escrito numa
   *  mensagem, mesmo que ainda não exista. */
  async function openPerson(who: { jid: string } | { phone: string }) {
    try {
      const chat = await ('jid' in who ? api.personChat(who.jid) : api.phoneChat(who.phone))
      if (!chats.some((c) => c.jid === chat.jid)) setDraftChat(chat)
      select(chat.jid)
    } catch (e) {
      setNotice(fail(e))
    }
  }

  async function loadOlder() {
    if (!conv || conv.pending) return
    const jid = conv.jid
    try {
      const out = await api.loadOlder(jid, conv.messages[0]?.ts)
      setConv((c) =>
        c?.jid === jid
          ? {
              ...withOlder(c, out.messages),
              pending: out.requested,
              // Nada guardado e nada a pedir: chegou ao começo.
              hasMore: out.requested || out.messages.length > 0,
            }
          : c,
      )
      if (out.requested) {
        // Celular desligado ou sem rede: não espera para sempre.
        setTimeout(() => {
          const c = convRef.current
          if (c?.jid !== jid || !c.pending) return
          setNotice('O celular não respondeu. Tente de novo com ele ligado e com internet.')
          setConv((cur) => (cur?.jid === jid ? { ...cur, pending: false } : cur))
        }, HISTORY_TIMEOUT_MS)
      }
    } catch (e) {
      setNotice(fail(e))
    }
  }



  /** Aplica a versão nova da mensagem (ou a remove) na conversa aberta. */
  function applyMessage(chatJid: string, id: string, updated: Message | null) {
    setConv((c) =>
      c?.jid === chatJid
        ? {
            ...c,
            messages: updated
              ? c.messages.map((x) => (x.id === id ? updated : x))
              : c.messages.filter((x) => x.id !== id),
          }
        : c,
    )
  }

  async function runMessageAction(m: Message, action: string, days?: number) {
    try {
      const updated = await api.messageAction(m.chat_jid, m.id, action, days)
      applyMessage(m.chat_jid, m.id, updated)
    } catch (e) {
      setNotice(fail(e))
    }
  }

  async function copy(text: string, what: string) {
    try {
      await copyText(text)
      setNotice(`${what} copiado`)
    } catch (e) {
      setNotice(e instanceof Error ? e.message : 'Não foi possível copiar')
    }
  }

  function messagePick(m: Message, pick: MessagePick) {
    switch (pick) {
      case 'reply':
        return setReplyTo(m)
      case 'copy':
        return void copy(withMentionNames(m.body ?? '', m.mentions), 'Texto')
      case 'copy_code': {
        const code = securityCode(m.body)
        return code ? void copy(code, 'Código') : undefined
      }
      case 'forward':
        return setForwarding(m)
      case 'pin1':
        return void runMessageAction(m, 'pin', 1)
      case 'pin7':
        return void runMessageAction(m, 'pin', 7)
      case 'pin30':
        return void runMessageAction(m, 'pin', 30)
      case 'unpin':
      case 'star':
      case 'unstar':
        return void runMessageAction(m, pick)
      case 'delete_me':
        return setConfirm({
          title: 'Excluir para mim?',
          text: 'A mensagem some daqui e do seu celular. Os outros continuam vendo.',
          confirmLabel: 'Excluir para mim',
          onConfirm: () => void runMessageAction(m, 'delete_me'),
        })
      case 'delete_all':
        return setConfirm({
          title: 'Excluir para todos?',
          text: 'A mensagem é apagada para todos na conversa. Não dá para desfazer.',
          confirmLabel: 'Excluir para todos',
          onConfirm: () => void runMessageAction(m, 'delete_all'),
        })
      case 'report':
        return setConfirm({
          title: 'Denunciar ao WhatsApp?',
          text: 'Esta mensagem é enviada ao WhatsApp para análise. Quem mandou não é avisado.',
          confirmLabel: 'Denunciar',
          onConfirm: () =>
            void api.messageAction(m.chat_jid, m.id, 'report').then(
              () => setNotice('Mensagem denunciada'),
              (e) => setNotice(fail(e)),
            ),
        })
    }
  }

  async function forwardTo(to: string[]) {
    if (!forwarding) return
    try {
      await api.forward(forwarding.chat_jid, forwarding.id, to)
      setNotice(to.length > 1 ? `Encaminhada para ${to.length} conversas` : 'Encaminhada')
    } catch (e) {
      throw new Error(fail(e))
    }
  }

  async function sendFile(file: File, caption: string | null, meta: FileMeta) {
    if (!selected) return
    const jid = selected
    try {
      const m = await api.sendMedia(jid, file, { caption, ...meta })
      setConv((c) =>
        c?.jid === jid && !c.messages.some((x) => x.id === m.id) ? { ...c, messages: [...c.messages, m] } : c,
      )
    } catch (e) {
      throw new Error(fail(e))
    }
  }

  async function send(text: string) {
    if (!selected) return
    const jid = selected
    try {
      const m = await api.send(jid, text, replyTo?.chat_jid === jid ? replyTo.id : null)
      setReplyTo(null)
      setConv((c) =>
        c?.jid === jid && !c.messages.some((x) => x.id === m.id) ? { ...c, messages: [...c.messages, m] } : c,
      )
    } catch (e) {
      throw new Error(fail(e))
    }
  }

  function toggleNoRead() {
    const next = !noRead
    setNoRead(next)
    try {
      localStorage.setItem(NO_READ_KEY, next ? '1' : '0')
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
        onSection={chooseSection}
      />
      <aside className="sidebar">
        <header className="topbar">
          <h1 className="topbar__title">{SECTION_TITLE[section]}</h1>
          <label
            className="toggle"
            title="Marcado: abrir a conversa não marca as mensagens como lidas (o contato não vê os tiques azuis)"
          >
            <input type="checkbox" checked={noRead} onChange={toggleNoRead} />
            Não visualizar ao abrir
          </label>
        </header>
        <p className={`conn conn--${connClass}`} role="status">
          <span className="conn__dot" aria-hidden="true" />
          {connText}
        </p>
        <ChatList
          chats={visible}
          selected={selected}
          onSelect={select}
          onMenu={(chat, x, y) => setMenu({ chat, x, y })}
        />
        {menu && (
          <ChatMenu
            chat={menu.chat}
            x={menu.x}
            y={menu.y}
            onClose={closeMenu}
            onPick={(pick) => void pickAction(menu.chat, pick)}
          />
        )}
        {forwarding && (
          <ForwardDialog chats={chats} onSend={forwardTo} onClose={() => setForwarding(null)} />
        )}
        {confirm && <Confirm req={confirm} onClose={() => setConfirm(null)} />}
        {notice && (
          <p className="notice" role="alert">
            {notice}
          </p>
        )}
      </aside>

      <main className="main">
        {conv ? (
          <Conversation
            chat={chats.find((c) => c.jid === conv.jid) ?? (draftChat?.jid === conv.jid ? draftChat : undefined)}
            conv={conv}
            connected={conn === 'connected' && stream === 'open'}
            onBack={closeChat}
            onLoadOlder={loadOlder}
            onMarkRead={() => markRead(conv.jid)}
            onSend={send}
            onSendFile={sendFile}
            replyTo={replyTo?.chat_jid === conv.jid ? replyTo : null}
            onCancelReply={() => setReplyTo(null)}
            onMessagePick={messagePick}
            onOpenPerson={(jid) => void openPerson({ jid })}
            onOpenPhone={(phone) => void openPerson({ phone })}
          />
        ) : conn === 'waiting_qr' ? (
          <MainPairing />
        ) : (
          <p className="empty empty--center">Selecione uma conversa.</p>
        )}
      </main>
    </div>
  )
}

function MainPairing() {
  const pairing = usePairing()
  return (
    <div className="main__center">
      <PairingQr svg={pairing?.qr_svg ?? null} />
    </div>
  )
}
