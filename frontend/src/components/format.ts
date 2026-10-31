import type { Chat, Message } from '../api/types'

const PLACEHOLDER: Record<string, string> = {
  image: '[imagem]',
  audio: '[áudio]',
  video: '[vídeo]',
  doc: '[documento]',
  sticker: '[figurinha]',
  // O WhatsApp não entrega o conteúdo a aparelhos conectados.
  view_once: '📷 Visualização única — abra no celular',
  revoked: '🚫 Mensagem apagada',
  other: '[mensagem]',
}

export function messageText(m: Message): string {
  if (m.kind === 'text') return m.body ?? ''
  const tag = PLACEHOLDER[m.kind] ?? '[mensagem]'
  return m.body ? `${tag} ${m.body}` : tag
}

export type Section = 'chats' | 'archived' | 'status' | 'channels'

export const SECTION_TITLE: Record<Section, string> = {
  chats: 'Conversas',
  archived: 'Arquivadas',
  status: 'Status',
  channels: 'Canais',
}

/** Seção da barra lateral onde a conversa aparece. */
export function chatSection(c: Chat): Section {
  if (c.jid === 'status@broadcast') return 'status'
  if (c.jid.endsWith('@newsletter')) return 'channels'
  return c.archived ? 'archived' : 'chats'
}

/** Status e canais: só leitura (enviar no status publicaria um status). */
export function isReadOnly(jid: string): boolean {
  return jid === 'status@broadcast' || jid.endsWith('@newsletter')
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

/** Nome vindo do backend: "+5511..." (sem nome) vira telefone formatado. */
export function formatPerson(name: string): string {
  return name.startsWith('+') ? formatPhone(name.slice(1)) : name
}

export function senderName(m: Message): string {
  return m.sender_name ? formatPerson(m.sender_name) : 'Desconhecido'
}

const decimal = new Intl.NumberFormat('pt-BR', { maximumFractionDigits: 1 })

/** "820 B", "340 KB", "2,4 MB". */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`
  return `${decimal.format(n / (1024 * 1024))} MB`
}

/** "0:07", "12:34", "1:02:03". */
export function formatDuration(total: number): string {
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = String(Math.floor(total % 60)).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`
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

/** Código de verificação na mensagem (4 a 8 dígitos, com ou sem hífen/espaço no meio). */
export function securityCode(text: string | null): string | null {
  const m = text?.match(/(?<![\d])(\d{3}[- ]?\d{3}|\d{4,8})(?![\d])/)
  return m ? m[1].replace(/[- ]/g, '') : null
}

/**
 * Copia texto. A API de área de transferência só existe em contexto seguro (HTTPS ou
 * localhost); pelo IP do Tailscale em HTTP cai no método antigo.
 */
export async function copyText(text: string): Promise<void> {
  if (window.isSecureContext && navigator.clipboard) {
    await navigator.clipboard.writeText(text)
    return
  }
  const area = document.createElement('textarea')
  area.value = text
  area.setAttribute('readonly', '')
  area.style.position = 'fixed'
  area.style.opacity = '0'
  document.body.appendChild(area)
  area.select()
  const ok = document.execCommand('copy')
  area.remove()
  if (!ok) throw new Error('Não foi possível copiar')
}

/** Cores de destaque do Catppuccin Mocha para os nomes em grupos. */
export const SENDER_COLORS = [
  'rosewater',
  'flamingo',
  'pink',
  'mauve',
  'red',
  'maroon',
  'peach',
  'yellow',
  'green',
  'teal',
  'sky',
  'sapphire',
  'blue',
  'lavender',
]

/** As cores em ordem aleatória: cada abertura da conversa sorteia de novo. */
export function shuffledPalette(): number[] {
  const order = SENDER_COLORS.map((_, i) => i)
  for (let i = order.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    ;[order[i], order[j]] = [order[j], order[i]]
  }
  return order
}
