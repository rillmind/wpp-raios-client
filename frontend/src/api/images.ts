import { useEffect, useState } from 'react'
import { fetchImage } from './client'

// Imagens baixadas ficam só na memória desta aba (URLs de blob): somem ao
// fechar ou recarregar, e nada vai para o disco nem para o banco.
const cache = new Map<string, Promise<string | null>>()

export function loadImage(path: string): Promise<string | null> {
  let p = cache.get(path)
  if (!p) {
    p = fetchImage(path).then((blob) => (blob ? URL.createObjectURL(blob) : null))
    // Falha (rede, WhatsApp desconectado) não fica no cache: tenta de novo depois.
    p.catch(() => cache.delete(path))
    cache.set(path, p)
  }
  return p
}

export type ImageState = { status: 'idle' | 'loading' | 'none' | 'error' } | { status: 'ready'; url: string }

/** Baixa `path` quando `enabled` (ex.: o elemento apareceu na tela ou a pessoa clicou). */
export function useImage(path: string, enabled: boolean): ImageState {
  const [state, setState] = useState<{ path: string; value: ImageState }>({ path, value: { status: 'idle' } })
  useEffect(() => {
    if (!enabled) return
    let alive = true
    loadImage(path).then(
      (url) => alive && setState({ path, value: url ? { status: 'ready', url } : { status: 'none' } }),
      () => alive && setState({ path, value: { status: 'error' } }),
    )
    return () => {
      alive = false
    }
  }, [path, enabled])
  if (state.path !== path) return { status: enabled ? 'loading' : 'idle' }
  return state.value.status === 'idle' && enabled ? { status: 'loading' } : state.value
}
