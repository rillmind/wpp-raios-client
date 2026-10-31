import type { Chat, Message } from '../api/types'

const PLACEHOLDER: Record<string, string> = {
  image: '[imagem]',
  audio: '[áudio]',
  video: '[vídeo]',
  doc: '[documento]',
  sticker: '[figurinha]',
  other: '[mensagem]',
}

export function messageText(m: Message): string {
  if (m.kind === 'text') return m.body ?? ''
  const tag = PLACEHOLDER[m.kind] ?? '[mensagem]'
  return m.body ? `${tag} ${m.body}` : tag
}

export function chatName(c: Chat): string {
  if (c.name) return c.name
  if (c.phone) return formatPhone(c.phone)
  if (c.jid.endsWith('@broadcast')) return 'Status'
  if (c.jid.endsWith('@newsletter')) return 'Canal'
  // Sem nome nem telefone, o id interno (@lid) não diz nada a ninguém.
  return c.is_group ? 'Grupo sem nome' : 'Contato sem nome'
}

/** Telefone legível; números do Brasil como "+55 11 98888-7777". */
export function formatPhone(digits: string): string {
  const br = /^55(\d{2})(\d{4,5})(\d{4})$/.exec(digits)
  return br ? `+55 ${br[1]} ${br[2]}-${br[3]}` : `+${digits}`
}

export function senderName(m: Message): string {
  if (!m.sender_name) return 'Desconhecido'
  return m.sender_name.startsWith('+') ? formatPhone(m.sender_name.slice(1)) : m.sender_name
}

const decimal = new Intl.NumberFormat('pt-BR', { maximumFractionDigits: 1 })

/** "820 B", "340 KB", "2,4 MB". */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`
  return `${decimal.format(n / (1024 * 1024))} MB`
}

const time = new Intl.DateTimeFormat('pt-BR', { hour: '2-digit', minute: '2-digit' })
const day = new Intl.DateTimeFormat('pt-BR', { day: '2-digit', month: '2-digit', year: '2-digit' })

export function formatTime(ts: number): string {
  return time.format(new Date(ts * 1000))
}

/** Hora se for hoje; data caso contrário. */
export function formatListTime(ts: number | null): string {
  if (ts == null) return ''
  const d = new Date(ts * 1000)
  return d.toDateString() === new Date().toDateString() ? time.format(d) : day.format(d)
}
