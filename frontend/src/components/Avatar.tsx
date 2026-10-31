import { useRef } from 'react'
import { api } from '../api/client'
import { useImage } from '../api/images'
import { useInView } from '../api/useInView'

interface Props {
  jid: string
  name: string
  size?: 'md' | 'sm'
}

// Mesmas cores para o mesmo contato, da paleta Mocha.
const COLORS = ['mauve', 'blue', 'teal', 'green', 'peach', 'pink', 'sapphire', 'yellow']

function initials(name: string): string {
  const words = name.replace(/[^\p{L}\p{N} ]/gu, '').trim().split(/\s+/).filter(Boolean)
  if (words.length === 0) return '?'
  return (words[0][0] + (words.length > 1 ? words[words.length - 1][0] : '')).toUpperCase()
}

function colorFor(jid: string): string {
  let h = 0
  for (const ch of jid) h = (h * 31 + ch.charCodeAt(0)) | 0
  return COLORS[Math.abs(h) % COLORS.length]
}

/** Foto de perfil, baixada quando aparece na tela; iniciais enquanto isso (ou se não houver). */
export function Avatar({ jid, name, size = 'md' }: Props) {
  const ref = useRef<HTMLSpanElement>(null)
  const visible = useInView(ref)
  const img = useImage(api.avatarPath(jid), visible)
  return (
    <span ref={ref} className={`avatar avatar--${size} avatar--${colorFor(jid)}`} aria-hidden="true">
      {img.status === 'ready' ? <img src={img.url} alt="" /> : initials(name)}
    </span>
  )
}
