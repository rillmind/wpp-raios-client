import type { Chat } from '../api/types'
import { Menu, type MenuItem } from './Menu'

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
  const isStatus = chat.jid === 'status@broadcast'
  const isChannel = chat.jid.endsWith('@newsletter')
  const item = (label: string, pick: MenuAction): MenuItem => ({ label, onPick: () => onPick(pick) })
  const items: MenuItem[] = []
  if (!isStatus && !isChannel) {
    items.push(
      chat.archived
        ? item('Desarquivar conversa', { action: 'unarchive' })
        : item('Arquivar conversa', { action: 'archive' }),
    )
    if (!chat.archived) {
      items.push(
        chat.pinned ? item('Desafixar conversa', { action: 'unpin' }) : item('Fixar conversa', { action: 'pin' }),
      )
    }
  }
  if (!isStatus) {
    if (chat.muted) items.push(item('Reativar som', { action: 'unmute' }))
    else
      items.push(
        item('Silenciar por 8 horas', { action: 'mute', hours: HOURS_8 }),
        item('Silenciar por 1 semana', { action: 'mute', hours: WEEK }),
        item('Silenciar para sempre', { action: 'mute' }),
      )
  }
  items.push(
    chat.unread_count > 0
      ? item('Marcar como lida', { action: 'read' })
      : item('Marcar como não lida', { action: 'unread' }),
  )
  return <Menu x={x} y={y} items={items} onClose={onClose} />
}
