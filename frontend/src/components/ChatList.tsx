import type { Chat, MsgStatus } from '../api/types'
import { Avatar } from './Avatar'
import { chatName, formatListTime, formatPerson } from './format'

const TICK_LABEL: Record<MsgStatus, string> = { sent: 'enviada', delivered: 'entregue', read: 'lida' }

interface Props {
  chats: Chat[]
  selected: string | null
  onSelect: (jid: string) => void
}

export function ChatList({ chats, selected, onSelect }: Props) {
  if (chats.length === 0) {
    return <p className="empty">Nenhuma conversa ainda. Elas aparecem aqui assim que chegarem.</p>
  }
  return (
    <ul className="chat-list" role="listbox" aria-label="Conversas">
      {chats.map((c) => (
        <li key={c.jid}>
          <button
            type="button"
            className="chat-item"
            aria-selected={c.jid === selected}
            onClick={() => onSelect(c.jid)}
          >
            <Avatar jid={c.jid} name={chatName(c)} />
            <span className="chat-item__body">
              <span className="chat-item__top">
                <span className="chat-item__name">{chatName(c)}</span>
                <span className={c.unread_count > 0 ? 'chat-item__time chat-item__time--unread' : 'chat-item__time'}>
                  {formatListTime(c.last_msg_ts)}
                </span>
              </span>
              <span className="chat-item__bottom">
                <span className="chat-item__preview">
                  {c.last_status && (
                    <span
                      className={c.last_status === 'read' ? 'chat-item__tick chat-item__tick--read' : 'chat-item__tick'}
                      aria-label={TICK_LABEL[c.last_status]}
                    >
                      {c.last_status === 'sent' ? '✓' : '✓✓'}
                    </span>
                  )}
                  {c.last_sender && c.last_msg_preview && `${formatPerson(c.last_sender)}: `}
                  {c.last_msg_preview ?? ''}
                </span>
                {c.pinned && (
                  <svg className="chat-item__pin" viewBox="0 0 24 24" width="16" height="16" aria-label="Fixada">
                    <path
                      d="M9 4h6l-1 6 3 3v1H7v-1l3-3zM12 14v6"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.8"
                      strokeLinejoin="round"
                      strokeLinecap="round"
                    />
                  </svg>
                )}
                {c.unread_count > 0 && (
                  <span className="badge" aria-label={`${c.unread_count} não lidas`}>
                    {c.unread_count}
                  </span>
                )}
              </span>
            </span>
          </button>
        </li>
      ))}
    </ul>
  )
}
