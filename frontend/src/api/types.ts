export type MessageKind =
  | 'text'
  | 'image'
  | 'audio'
  | 'video'
  | 'doc'
  | 'sticker'
  | 'view_once'
  | 'revoked'
  | 'other'
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
  /** Tiques da última mensagem, quando ela é nossa. */
  last_status: MsgStatus | null
  /** Grupos: quem mandou a última mensagem ("Você", nome ou "+telefone"). */
  last_sender: string | null
  archived: boolean
  pinned: boolean
  /** Silenciada no celular. */
  muted: boolean
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
  /** Favoritada. */
  starred: boolean
  /** Fixada na conversa. */
  pinned: boolean
  /** Pessoas marcadas; no texto aparecem como `@` + a parte do jid antes do `@`. */
  mentions?: Mention[]
}

export interface Mention {
  jid: string
  /** Agenda, apelido, "+telefone" ou "Você". */
  name?: string
  /** A pessoa marcada é você. */
  me?: boolean
}

export interface MediaInfo {
  /** Tamanho do arquivo em bytes. */
  size: number
  mimetype: string | null
  width: number | null
  height: number | null
  /** Duração em segundos (vídeo e áudio). */
  seconds: number | null
  /** Nome do arquivo (documento). */
  file_name: string | null
}

export interface MessagesPage {
  messages: Message[]
  first_unread_id: string | null
  has_more: boolean
}

export type ServerEvent =
  | { type: 'message.new'; data: Message }
  | { type: 'message.status'; data: { chat_jid: string; ids: string[]; status: MsgStatus } }
  | { type: 'message.updated'; data: Message }
  | { type: 'message.deleted'; data: { chat_jid: string; id: string } }
  | { type: 'chat.updated'; data: Chat }
  | { type: 'history.loaded'; data: { chat_jid: string; count: number } }
  | { type: 'connection.state'; data: { state: ConnState } }
