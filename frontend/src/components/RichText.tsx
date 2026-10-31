import { Fragment } from 'react'
import type { Mention } from '../api/types'
import { splitMentions } from './format'

/** Texto de mensagem com as marcações (`@Nome`) destacadas. */
export function RichText({ text, mentions }: { text: string; mentions?: Mention[] }) {
  return (
    <>
      {splitMentions(text, mentions).map((p, i) =>
        p.mention ? (
          <span key={i} className={p.mention.me ? 'mention mention--me' : 'mention'}>
            {p.text}
          </span>
        ) : (
          <Fragment key={i}>{p.text}</Fragment>
        ),
      )}
    </>
  )
}
