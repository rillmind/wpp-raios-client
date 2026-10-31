import type { Chat, ConnState, Message, MessagesPage } from './types'

// Em dev o Vite (5173) fala com o backend (8787); no build servido pelo
// próprio backend, a API está na mesma origem.
export const API_URL: string =
  import.meta.env.VITE_API_URL ?? (import.meta.env.DEV ? 'http://127.0.0.1:8787' : '')

const TOKEN_KEY = 'wpp.token'

export function getToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY)
  } catch {
    return null
  }
}

export function setToken(token: string | null) {
  try {
    if (token) localStorage.setItem(TOKEN_KEY, token)
    else localStorage.removeItem(TOKEN_KEY)
  } catch {
    // Sem storage: o token vale só nesta aba.
  }
}

export class ApiError extends Error {
  readonly status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const res = await fetch(`${API_URL}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${getToken() ?? ''}`,
      ...(init.body ? { 'Content-Type': 'application/json' } : {}),
    },
  })
  if (!res.ok) {
    const body = await res.json().catch(() => null)
    throw new ApiError(res.status, body?.error ?? res.statusText)
  }
  return res.json() as Promise<T>
}

/** Imagem autenticada; `null` quando não existe (204/404). Outros erros lançam. */
export async function fetchImage(path: string): Promise<Blob | null> {
  const res = await fetch(`${API_URL}${path}`, {
    headers: { Authorization: `Bearer ${getToken() ?? ''}` },
    cache: 'no-store',
  })
  if (res.status === 204 || res.status === 404) return null
  if (!res.ok) throw new ApiError(res.status, res.statusText)
  return res.blob()
}

export interface Pairing {
  state: ConnState
  /** SVG do QR, só enquanto não há celular pareado. */
  qr_svg: string | null
}

/** Estado do pareamento; não precisa de token (é a primeira tela do primeiro uso). */
export async function fetchPairing(): Promise<Pairing> {
  const res = await fetch(`${API_URL}/api/pairing`, { cache: 'no-store' })
  if (!res.ok) throw new ApiError(res.status, res.statusText)
  return res.json() as Promise<Pairing>
}

const enc = encodeURIComponent

export const api = {
  chats: () => request<Chat[]>('/api/chats?limit=500'),
  messages: (jid: string, opts: { before?: number; limit?: number } = {}) => {
    const q = new URLSearchParams()
    if (opts.before != null) q.set('before', String(opts.before))
    if (opts.limit) q.set('limit', String(opts.limit))
    return request<MessagesPage>(`/api/chats/${enc(jid)}/messages${q.size ? `?${q}` : ''}`)
  },
  /** "Carregar mensagens": as guardadas e, se faltar, pede as anteriores ao celular. */
  loadOlder: (jid: string, before?: number) =>
    request<{ messages: Message[]; requested: boolean }>(`/api/chats/${enc(jid)}/load-older`, {
      method: 'POST',
      body: JSON.stringify(before != null ? { before } : {}),
    }),
  /** Conversa individual com uma pessoa: a que já existe ou uma vazia (ainda fora da lista). */
  personChat: (jid: string) => request<Chat>(`/api/people/${enc(jid)}/chat`),
  markRead: (jid: string) => request<Chat>(`/api/chats/${enc(jid)}/read`, { method: 'POST' }),
  /** Arquiva, fixa, silencia (por `hours`, ou para sempre) ou marca como não lida. */
  action: (jid: string, action: string, hours?: number) =>
    request<Chat>(`/api/chats/${enc(jid)}/action`, {
      method: 'POST',
      body: JSON.stringify(hours ? { action, hours } : { action }),
    }),
  send: (jid: string, text: string, replyTo?: string | null) =>
    request<Message>(`/api/chats/${enc(jid)}/send`, {
      method: 'POST',
      body: JSON.stringify(replyTo ? { text, reply_to: replyTo } : { text }),
    }),
  /** Estrela, fixar (`days`: 1, 7, 30), apagar para mim/todos ou denunciar. */
  messageAction: (jid: string, id: string, action: string, days?: number) =>
    request<Message | null>(`/api/chats/${enc(jid)}/messages/${enc(id)}/action`, {
      method: 'POST',
      body: JSON.stringify(days ? { action, days } : { action }),
    }),
  forward: (jid: string, id: string, to: string[]) =>
    request<Message[]>(`/api/chats/${enc(jid)}/messages/${enc(id)}/forward`, {
      method: 'POST',
      body: JSON.stringify({ to }),
    }),
  avatarPath: (jid: string) => `/api/chats/${enc(jid)}/avatar`,
  mediaPath: (jid: string, id: string) => `/api/chats/${enc(jid)}/messages/${enc(id)}/media`,
  /** Envia um arquivo; o tipo (imagem, vídeo ou documento) sai do `Content-Type`. */
  sendMedia: (
    jid: string,
    file: File,
    opts: { caption?: string | null; width?: number; height?: number; seconds?: number },
  ) => {
    const q = new URLSearchParams()
    if (file.name) q.set('name', file.name)
    if (opts.caption) q.set('caption', opts.caption)
    for (const k of ['width', 'height', 'seconds'] as const) {
      const v = opts[k]
      if (v) q.set(k, String(v))
    }
    return fetch(`${API_URL}/api/chats/${enc(jid)}/send-media?${q}`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${getToken() ?? ''}`,
        'Content-Type': file.type || 'application/octet-stream',
      },
      body: file,
    }).then(async (res) => {
      if (!res.ok) {
        const body = await res.json().catch(() => null)
        throw new ApiError(res.status, body?.error ?? res.statusText)
      }
      return res.json() as Promise<Message>
    })
  },
  eventsUrl: () => `${API_URL}/api/events?token=${enc(getToken() ?? '')}`,
}
