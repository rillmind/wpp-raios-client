import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { Chat } from '../api/types'

export type MenuAction =
  | { action: 'archive' | 'unarchive' | 'pin' | 'unpin' | 'unmute' | 'unread' }
  | { action: 'mute'; hours?: number }
  | { action: 'read' }

interface Props {
  chat: Chat
  x: number
  y: number
  onPick: (a: MenuAction) => void
  onClose: () => void
}

const HOURS_8 = 8
const WEEK = 24 * 7

/** Menu do botão direito numa conversa da lista. */
export function ChatMenu({ chat, x, y, onPick, onClose }: Props) {
  const ref = useRef<HTMLUListElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })

  // Mantém o menu dentro da tela.
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { width, height } = el.getBoundingClientRect()
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - height - 4)),
    })
  }, [x, y])

  useEffect(() => {
    // Esc fecha só o menu (em captura, antes de fechar a conversa).
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.stopPropagation()
      onClose()
    }
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose()
    }
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('resize', onClose)
    window.addEventListener('blur', onClose)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('pointerdown', onDown, true)
      window.removeEventListener('resize', onClose)
      window.removeEventListener('blur', onClose)
    }
  }, [onClose])

  const isStatus = chat.jid === 'status@broadcast'
  const isChannel = chat.jid.endsWith('@newsletter')
  const items: { label: string; pick: MenuAction }[] = []
  if (!isStatus && !isChannel) {
    items.push(
      chat.archived
        ? { label: 'Desarquivar conversa', pick: { action: 'unarchive' } }
        : { label: 'Arquivar conversa', pick: { action: 'archive' } },
    )
    if (!chat.archived) {
      items.push(
        chat.pinned
          ? { label: 'Desafixar conversa', pick: { action: 'unpin' } }
          : { label: 'Fixar conversa', pick: { action: 'pin' } },
      )
    }
  }
  if (!isStatus) {
    if (chat.muted) items.push({ label: 'Reativar som', pick: { action: 'unmute' } })
    else
      items.push(
        { label: 'Silenciar por 8 horas', pick: { action: 'mute', hours: HOURS_8 } },
        { label: 'Silenciar por 1 semana', pick: { action: 'mute', hours: WEEK } },
        { label: 'Silenciar para sempre', pick: { action: 'mute' } },
      )
  }
  items.push(
    chat.unread_count > 0
      ? { label: 'Marcar como lida', pick: { action: 'read' } }
      : { label: 'Marcar como não lida', pick: { action: 'unread' } },
  )

  return (
    <ul ref={ref} className="menu" role="menu" style={pos} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it) => (
        <li key={it.label} role="none">
          <button
            type="button"
            role="menuitem"
            className="menu__item"
            onClick={() => {
              onClose()
              onPick(it.pick)
            }}
          >
            {it.label}
          </button>
        </li>
      ))}
    </ul>
  )
}
