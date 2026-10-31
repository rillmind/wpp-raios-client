import type { Chat } from '../api/types'
import { Avatar } from './Avatar'
import { chatName, formatListTime } from './format'

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
                <span className="chat-item__preview">{c.last_msg_preview ?? ''}</span>
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
