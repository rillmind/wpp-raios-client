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
  messages: (jid: string, before?: number) =>
    request<MessagesPage>(
      `/api/chats/${enc(jid)}/messages${before != null ? `?before=${before}&limit=50` : ''}`,
    ),
  markRead: (jid: string) => request<Chat>(`/api/chats/${enc(jid)}/read`, { method: 'POST' }),
  send: (jid: string, text: string) =>
    request<Message>(`/api/chats/${enc(jid)}/send`, {
      method: 'POST',
      body: JSON.stringify({ text }),
    }),
  avatarPath: (jid: string) => `/api/chats/${enc(jid)}/avatar`,
  mediaPath: (jid: string, id: string) => `/api/chats/${enc(jid)}/messages/${enc(id)}/media`,
  eventsUrl: () => `${API_URL}/api/events?token=${enc(getToken() ?? '')}`,
}
