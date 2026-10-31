import { useEffect, useState } from 'react'
import { api } from '../api/client'
import { useImage } from '../api/images'
import type { Message } from '../api/types'
import { formatBytes } from './format'

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
  return { width: Math.max(Math.round(w * scale), 160), height: Math.max(Math.round(h * scale), 90) }
}

/**
 * Imagem ou figurinha de uma mensagem. Só baixa quando a pessoa clica; o arquivo
 * fica na memória da aba (ver `api/images.ts`).
 */
export function MediaView({ chatJid, message }: { chatJid: string; message: Message }) {
  // Cada tentativa remonta o download (uma falha não fica em cache).
  const [attempt, setAttempt] = useState(0)
  const sticker = message.kind === 'sticker'
  const label = sticker ? 'Figurinha' : 'Imagem'
  const size = boxSize(message)

  return (
    <div className={sticker ? 'media media--sticker' : 'media'} style={size}>
      {attempt === 0 ? (
        <button type="button" className="media__download" onClick={() => setAttempt(1)}>
          <span className="media__icon" aria-hidden="true">
            ↓
          </span>
          <span>
            {label}
            {message.media?.size ? ` · ${formatBytes(message.media.size)}` : ''}
          </span>
          <span className="media__hint">Clique para baixar</span>
        </button>
      ) : (
        <Download
          key={attempt}
          path={api.mediaPath(chatJid, message.id)}
          label={label}
          sticker={sticker}
          onRetry={() => setAttempt((a) => a + 1)}
        />
      )}
    </div>
  )
}

interface DownloadProps {
  path: string
  label: string
  sticker: boolean
  onRetry: () => void
}

function Download({ path, label, sticker, onRetry }: DownloadProps) {
  const img = useImage(path, true)
  const [zoom, setZoom] = useState(false)

  if (img.status === 'ready') {
    if (sticker) return <img src={img.url} alt={label} />
    return (
      <>
        <button type="button" className="media__open" onClick={() => setZoom(true)} aria-label="Ampliar imagem">
          <img src={img.url} alt={label} />
        </button>
        {zoom && <Lightbox url={img.url} onClose={() => setZoom(false)} />}
      </>
    )
  }
  if (img.status === 'none' || img.status === 'error') {
    return (
      <button type="button" className="media__download" onClick={onRetry}>
        <span>{label} indisponível</span>
        <span className="media__hint">Clique para tentar de novo</span>
      </button>
    )
  }
  return <span className="media__placeholder">Baixando…</span>
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
