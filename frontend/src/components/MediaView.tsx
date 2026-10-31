import { useEffect, useRef, useState } from 'react'
import { api } from '../api/client'
import { useImage } from '../api/images'
import { useInView } from '../api/useInView'
import type { Message } from '../api/types'

const MAX_W = 280
const MAX_H = 320
const STICKER = 140

/** Caixa com a proporção da imagem, para a conversa não pular quando ela chegar. */
function boxSize(m: Message): { width: number; height: number } {
  if (m.kind === 'sticker') return { width: STICKER, height: STICKER }
  const w = m.media?.width ?? 0
  const h = m.media?.height ?? 0
  if (!w || !h) return { width: MAX_W, height: Math.round(MAX_W * 0.75) }
  const scale = Math.min(MAX_W / w, MAX_H / h, 1)
  return { width: Math.round(w * scale), height: Math.round(h * scale) }
}

/** Imagem ou figurinha de uma mensagem, baixada quando aparece na tela. */
export function MediaView({ chatJid, message }: { chatJid: string; message: Message }) {
  const ref = useRef<HTMLDivElement>(null)
  const visible = useInView(ref)
  const img = useImage(api.mediaPath(chatJid, message.id), visible)
  const [zoom, setZoom] = useState(false)
  const size = boxSize(message)
  const sticker = message.kind === 'sticker'
  const label = sticker ? 'Figurinha' : 'Imagem'

  return (
    <div ref={ref} className={sticker ? 'media media--sticker' : 'media'} style={size}>
      {img.status === 'ready' ? (
        sticker ? (
          <img src={img.url} alt={label} />
        ) : (
          <button type="button" className="media__open" onClick={() => setZoom(true)} aria-label="Ampliar imagem">
            <img src={img.url} alt={label} />
          </button>
        )
      ) : (
        <span className="media__placeholder">
          {img.status === 'none' || img.status === 'error' ? `${label} indisponível` : 'Carregando…'}
        </span>
      )}
      {zoom && img.status === 'ready' && <Lightbox url={img.url} onClose={() => setZoom(false)} />}
    </div>
  )
}

function Lightbox({ url, onClose }: { url: string; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div className="lightbox" role="dialog" aria-label="Imagem ampliada" onClick={onClose}>
      <img src={url} alt="Imagem ampliada" />
    </div>
  )
}
