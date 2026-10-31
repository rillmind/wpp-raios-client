import { useState } from 'react'
import type { Message } from '../api/types'
import { isReadOnly, securityCode } from './format'
import { Menu, type MenuItem } from './Menu'

export type MessagePick =
  | 'reply'
  | 'copy'
  | 'copy_code'
  | 'forward'
  | 'pin1'
  | 'pin7'
  | 'pin30'
  | 'unpin'
  | 'star'
  | 'unstar'
  | 'delete_me'
  | 'delete_all'
  | 'report'

/** O WhatsApp só deixa apagar para todos até ~2,5 dias depois (o backend confere). */
const REVOKE_WINDOW_SECS = 60 * 3600

interface Props {
  message: Message
  chatJid: string
  x: number
  y: number
  onPick: (pick: MessagePick) => void
  onClose: () => void
}

/** Menu do botão direito numa mensagem da conversa. */
export function MessageMenu({ message: m, chatJid, x, y, onPick, onClose }: Props) {
  // O menu é montado a cada abertura: a hora de agora vale para ela inteira.
  const [now] = useState(() => Date.now() / 1000)
  const item = (label: string, pick: MessagePick, danger = false): MenuItem => ({
    label,
    onPick: () => onPick(pick),
    danger,
  })
  const items: MenuItem[] = []
  const readOnly = isReadOnly(chatJid)

  if (m.kind === 'revoked') {
    items.push(item('Excluir para mim', 'delete_me', true))
    return <Menu x={x} y={y} items={items} onClose={onClose} />
  }
  if (!readOnly) items.push(item('Responder', 'reply'))
  if (m.body) items.push(item('Copiar', 'copy'))
  if (securityCode(m.body)) items.push(item('Copiar código', 'copy_code'))
  if ((m.kind === 'text' && m.body) || m.media) items.push(item('Encaminhar', 'forward'))
  if (!readOnly) {
    if (m.pinned) items.push(item('Desafixar', 'unpin'))
    else items.push(item('Fixar por 24 horas', 'pin1'), item('Fixar por 7 dias', 'pin7'), item('Fixar por 30 dias', 'pin30'))
  }
  items.push(m.starred ? item('Desfavoritar', 'unstar') : item('Favoritar', 'star'))
  items.push(item('Excluir para mim', 'delete_me', true))
  if (m.from_me && now - m.ts <= REVOKE_WINDOW_SECS) {
    items.push(item('Excluir para todos', 'delete_all', true))
  }
  if (!m.from_me && !readOnly) items.push(item('Denunciar', 'report', true))
  return <Menu x={x} y={y} items={items} onClose={onClose} />
}
