import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { api } from './client'
import type { ServerEvent } from './types'

export type StreamState = 'open' | 'connecting' | 'closed'

const NAMES = [
  'message.new',
  'message.status',
  'message.updated',
  'message.deleted',
  'chat.updated',
  'connection.state',
] as const

/**
 * Assina `/api/events`. O EventSource reconecta sozinho; `onResync` é chamado
 * ao reabrir (ou quando o servidor avisa que eventos foram perdidos), para
 * recarregar do banco o que possa ter passado.
 */
export function useEvents(onEvent: (ev: ServerEvent) => void, onResync: () => void): StreamState {
  const [state, setState] = useState<StreamState>('connecting')
  const handlers = useRef({ onEvent, onResync })
  useLayoutEffect(() => {
    handlers.current = { onEvent, onResync }
  })

  useEffect(() => {
    const es = new EventSource(api.eventsUrl())
    let opened = false
    es.onopen = () => {
      setState('open')
      if (opened) handlers.current.onResync()
      opened = true
    }
    es.onerror = () => setState(es.readyState === EventSource.CLOSED ? 'closed' : 'connecting')
    for (const name of NAMES) {
      es.addEventListener(name, (e) => {
        try {
          handlers.current.onEvent(JSON.parse((e as MessageEvent<string>).data) as ServerEvent)
        } catch {
          // Evento malformado: ignora.
        }
      })
    }
    es.addEventListener('resync', () => handlers.current.onResync())
    return () => es.close()
  }, [])

  return state
}
