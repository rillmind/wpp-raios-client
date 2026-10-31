export type MessageKind = 'text' | 'image' | 'audio' | 'video' | 'doc' | 'sticker' | 'other'
export type MsgStatus = 'sent' | 'delivered' | 'read'
export type ConnState = 'connecting' | 'waiting_qr' | 'connected' | 'disconnected' | 'logged_out'

export interface Chat {
  jid: string
  /** Grupo: assunto. Individual: nome da agenda, senão o apelido. */
  name: string | null
  /** Só dígitos, quando conhecido (conversas individuais). */
  phone: string | null
  is_group: boolean
  unread_count: number
  last_msg_ts: number | null
  last_msg_preview: string | null
  archived: boolean
  pinned: boolean
}

export interface Message {
  id: string
  chat_jid: string
  sender_jid: string | null
  /** Agenda, apelido ou "+telefone" do remetente. */
  sender_name?: string
  from_me: boolean
  ts: number
  kind: MessageKind
  body: string | null
  quoted_id: string | null
  status: MsgStatus | null
  is_read: boolean
  /** Presente quando há imagem/figurinha para baixar. */
  media?: MediaInfo
}

export interface MediaInfo {
  mimetype: string | null
  width: number | null
  height: number | null
}

export interface MessagesPage {
  messages: Message[]
  first_unread_id: string | null
  has_more: boolean
}

export type ServerEvent =
  | { type: 'message.new'; data: Message }
  | { type: 'message.status'; data: { chat_jid: string; ids: string[]; status: MsgStatus } }
  | { type: 'chat.updated'; data: Chat }
  | { type: 'connection.state'; data: { state: ConnState } }
