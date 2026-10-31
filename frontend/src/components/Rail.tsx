import { SECTION_TITLE, type Section } from './format'

interface Props {
  section: Section
  /** Conversas com mensagens não lidas, por seção. */
  unread: Record<Section, number>
  onSection: (s: Section) => void
}

const ICON: Record<Section, string> = {
  chats: 'M4 4h16a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H8l-4 4V5a1 1 0 0 1 1-1z',
  status: 'M12 3a9 9 0 1 1-6.4 2.6M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
  channels: 'M3 10v4h3l6 4V6L6 10zM16 9a4 4 0 0 1 0 6M18.5 6.5a8 8 0 0 1 0 11',
  archived: 'M3 4h18v4H3zM5 8h14v12H5zM10 12h4',
}

/** Barra lateral estreita, com as seções do app. */
export function Rail({ section, unread, onSection }: Props) {
  const item = (s: Section) => (
    <button
      key={s}
      type="button"
      className="rail__btn"
      aria-current={section === s}
      title={SECTION_TITLE[s]}
      aria-label={unread[s] > 0 ? `${SECTION_TITLE[s]}, ${unread[s]} não lidas` : SECTION_TITLE[s]}
      onClick={() => onSection(s)}
    >
      <Icon d={ICON[s]} />
      {unread[s] > 0 &&
        (s === 'chats' ? (
          <span className="rail__badge">{unread[s] > 99 ? '99+' : unread[s]}</span>
        ) : (
          <span className="rail__dot" aria-hidden="true" />
        ))}
    </button>
  )

  return (
    <nav className="rail" aria-label="Seções">
      {item('chats')}
      {item('status')}
      {item('channels')}
      <span className="rail__sep" aria-hidden="true" />
      {item('archived')}

    </nav>
  )
}

function Icon({ d }: { d: string }) {
  return (
    <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
      <path d={d} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  )
}
