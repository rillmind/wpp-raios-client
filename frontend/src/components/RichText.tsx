import { Fragment } from 'react'
import type { Mention } from '../api/types'
import { splitMentions } from './format'

interface Props {
  text: string
  mentions?: Mention[]
  /** Clique numa marcação (de outra pessoa): abre a conversa com ela. */
  onMention?: (jid: string) => void
}

/** Texto de mensagem com as marcações (`@Nome`) destacadas e clicáveis. */
export function RichText({ text, mentions, onMention }: Props) {
  return (
    <>
      {splitMentions(text, mentions).map((p, i) => {
        const m = p.mention
        if (!m) return <Fragment key={i}>{p.text}</Fragment>
        if (m.me || !onMention) {
          return (
            <span key={i} className={m.me ? 'mention mention--me' : 'mention'}>
              {p.text}
            </span>
          )
        }
        return (
          <button
            key={i}
            type="button"
            className="mention mention--link"
            title="Abrir conversa"
            onClick={() => onMention(m.jid)}
            onDoubleClick={(e) => e.stopPropagation()}
          >
            {p.text}
          </button>
        )
      })}
    </>
  )
}
