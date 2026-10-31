import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { api } from '../api/client'
import { useImage } from '../api/images'
import type { Message } from '../api/types'
import { formatBytes, formatDuration } from './format'

const MAX_W = 280
const MAX_H = 320
const STICKER = 140
const AUDIO_W = 260

const LABEL: Record<string, string> = {
  image: 'Imagem',
  sticker: 'Figurinha',
  video: 'Vídeo',
  audio: 'Áudio',
  doc: 'Documento',
}

/** Caixa com a proporção da mídia, para a conversa não pular quando ela chegar. */
function boxSize(m: Message): CSSProperties {
  if (m.kind === 'sticker') return { width: STICKER, height: STICKER }
  if (m.kind === 'audio' || m.kind === 'doc') return { width: AUDIO_W }
  const w = m.media?.width ?? 0
  const h = m.media?.height ?? 0
  if (!w || !h) return { width: MAX_W, height: Math.round(MAX_W * 0.75) }
  const scale = Math.min(MAX_W / w, MAX_H / h, 1)
  return { width: Math.max(Math.round(w * scale), 160), height: Math.max(Math.round(h * scale), 90) }
}

/**
 * Imagem, figurinha, vídeo ou áudio de uma mensagem. Só baixa quando a pessoa clica;
 * o arquivo fica na memória da aba (ver `api/images.ts`).
 */
export function MediaView({ chatJid, message }: { chatJid: string; message: Message }) {
  // Cada tentativa remonta o download (uma falha não fica em cache).
  const [attempt, setAttempt] = useState(0)
  const kind = message.kind
  const label = kind === 'doc' ? (message.media?.file_name ?? LABEL.doc) : (LABEL[kind] ?? 'Mídia')
  const details = [
    message.media?.seconds ? formatDuration(message.media.seconds) : null,
    message.media?.size ? formatBytes(message.media.size) : null,
  ].filter(Boolean)

  return (
    <div className={`media media--${kind}`} style={boxSize(message)}>
      {attempt === 0 ? (
        <button type="button" className="media__download" onClick={() => setAttempt(1)}>
          <span className="media__icon" aria-hidden="true">
            {kind === 'audio' || kind === 'video' ? '▶' : kind === 'doc' ? '📄' : '↓'}
          </span>
          <span className="media__label">{[label, ...details].join(' · ')}</span>
          {kind !== 'audio' && kind !== 'doc' && <span className="media__hint">Clique para baixar</span>}
        </button>
      ) : (
        <Download
          key={attempt}
          path={api.mediaPath(chatJid, message.id)}
          kind={kind}
          label={label}
          onRetry={() => setAttempt((a) => a + 1)}
        />
      )}
    </div>
  )
}

interface DownloadProps {
  path: string
  kind: string
  label: string
  onRetry: () => void
}

function Download({ path, kind, label, onRetry }: DownloadProps) {
  const file = useImage(path, true)
  const [zoom, setZoom] = useState(false)
  const save = useRef<HTMLAnchorElement>(null)

  // Documento: assim que baixa, entrega ao navegador para salvar (uma vez).
  const readyUrl = file.status === 'ready' ? file.url : null
  useEffect(() => {
    if (kind === 'doc' && readyUrl) save.current?.click()
  }, [kind, readyUrl])

  if (file.status === 'ready') {
    // Vídeo e áudio já tocam: o clique em "baixar" foi o pedido para ver/ouvir.
    if (kind === 'video') return <video src={file.url} controls autoPlay playsInline />
    if (kind === 'audio') return <audio src={file.url} controls autoPlay />
    if (kind === 'doc')
      return (
        <a ref={save} className="media__download media__saved" href={file.url} download={label}>
          <span className="media__icon" aria-hidden="true">
            📄
          </span>
          <span className="media__label">{label}</span>
          <span className="media__hint">Salvar de novo</span>
        </a>
      )
    if (kind === 'sticker') return <img src={file.url} alt={label} />
    return (
      <>
        <button type="button" className="media__open" onClick={() => setZoom(true)} aria-label="Ampliar imagem">
          <img src={file.url} alt={label} />
        </button>
        {zoom && <Lightbox url={file.url} onClose={() => setZoom(false)} />}
      </>
    )
  }
  if (file.status === 'none' || file.status === 'error') {
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
  // Em captura e sem propagar: com a imagem ampliada, Esc só fecha a imagem (não a conversa).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.stopPropagation()
      onClose()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onClose])
  return (
    <div className="lightbox" role="dialog" aria-label="Imagem ampliada" onClick={onClose}>
      <img src={url} alt="Imagem ampliada" />
    </div>
  )
}
