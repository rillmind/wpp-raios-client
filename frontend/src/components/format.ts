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
  return c.name ?? c.jid.split('@')[0]
}

export function shortJid(jid: string | null): string {
  return jid ? jid.split('@')[0].split(':')[0] : ''
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
