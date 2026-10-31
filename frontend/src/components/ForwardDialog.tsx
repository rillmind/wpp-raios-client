import { useMemo, useState } from 'react'
import type { Chat } from '../api/types'
import { Avatar } from './Avatar'
import { Dialog } from './Dialog'
import { chatName, isReadOnly } from './format'

/** O WhatsApp encaminha para até 5 conversas de uma vez. */
const MAX_TARGETS = 5

interface Props {
  chats: Chat[]
  onSend: (to: string[]) => Promise<void>
  onClose: () => void
}

export function ForwardDialog({ chats, onSend, onClose }: Props) {
  const [query, setQuery] = useState('')
  const [picked, setPicked] = useState<string[]>([])
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const options = useMemo(() => {
    const q = query.trim().toLowerCase()
    return chats
      .filter((c) => !isReadOnly(c.jid))
      .filter((c) => !q || chatName(c).toLowerCase().includes(q) || (c.phone ?? '').includes(q))
      .slice(0, 100)
  }, [chats, query])

  function toggle(jid: string) {
    setPicked((p) => (p.includes(jid) ? p.filter((j) => j !== jid) : p.length < MAX_TARGETS ? [...p, jid] : p))
  }

  async function send() {
    setSending(true)
    setError(null)
    try {
      await onSend(picked)
      onClose()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Falha ao encaminhar')
      setSending(false)
    }
  }

  return (
    <Dialog title="Encaminhar para…" onClose={onClose}>
      <input
        className="dialog__search"
        type="search"
        placeholder="Buscar conversa"
        aria-label="Buscar conversa"
        value={query}
        autoFocus
        onChange={(e) => setQuery(e.target.value)}
      />
      <ul className="forward__list">
        {options.map((c) => (
          <li key={c.jid}>
            <label className="forward__item">
              <input
                type="checkbox"
                checked={picked.includes(c.jid)}
                disabled={!picked.includes(c.jid) && picked.length >= MAX_TARGETS}
                onChange={() => toggle(c.jid)}
              />
              <Avatar jid={c.jid} name={chatName(c)} size="sm" />
              <span className="forward__name">{chatName(c)}</span>
            </label>
          </li>
        ))}
        {options.length === 0 && <li className="empty">Nenhuma conversa encontrada.</li>}
      </ul>
      {error && (
        <p className="composer__error" role="alert">
          {error}
        </p>
      )}
      <div className="dialog__actions">
        <span className="dialog__hint">
          {picked.length}/{MAX_TARGETS}
        </span>
        <button type="button" className="btn btn--ghost" onClick={onClose}>
          Cancelar
        </button>
        <button type="button" className="btn btn--primary" disabled={!picked.length || sending} onClick={() => void send()}>
          {sending ? 'Enviando…' : 'Encaminhar'}
        </button>
      </div>
    </Dialog>
  )
}
