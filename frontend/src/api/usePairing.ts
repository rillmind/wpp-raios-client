import { useEffect, useState } from 'react'
import { fetchPairing, type Pairing } from './client'

const POLL_MS = 3000

/** Consulta o pareamento a cada poucos segundos (o QR muda sozinho a cada ~20 s). */
export function usePairing(): Pairing | null {
  const [pairing, setPairing] = useState<Pairing | null>(null)
  useEffect(() => {
    let alive = true
    let timer: ReturnType<typeof setTimeout>
    const tick = () => {
      fetchPairing().then(
        (p) => alive && setPairing(p),
        () => alive && setPairing(null),
      )
      timer = setTimeout(tick, POLL_MS)
    }
    tick()
    return () => {
      alive = false
      clearTimeout(timer)
    }
  }, [])
  return pairing
}
