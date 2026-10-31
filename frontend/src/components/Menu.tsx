import { useEffect, useLayoutEffect, useRef, useState } from 'react'

export interface MenuItem {
  label: string
  onPick: () => void
  /** Ação destrutiva (aparece em vermelho). */
  danger?: boolean
}

interface Props {
  x: number
  y: number
  items: MenuItem[]
  onClose: () => void
}

/** Menu flutuante do botão direito, preso dentro da tela. */
export function Menu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLUListElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { width, height } = el.getBoundingClientRect()
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - height - 4)),
    })
  }, [x, y])

  useEffect(() => {
    // Esc fecha só o menu (em captura, antes de fechar a conversa).
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.stopPropagation()
      onClose()
    }
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose()
    }
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('resize', onClose)
    window.addEventListener('blur', onClose)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('pointerdown', onDown, true)
      window.removeEventListener('resize', onClose)
      window.removeEventListener('blur', onClose)
    }
  }, [onClose])

  return (
    <ul ref={ref} className="menu" role="menu" style={pos} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it) => (
        <li key={it.label} role="none">
          <button
            type="button"
            role="menuitem"
            className={it.danger ? 'menu__item menu__item--danger' : 'menu__item'}
            onClick={() => {
              onClose()
              it.onPick()
            }}
          >
            {it.label}
          </button>
        </li>
      ))}
    </ul>
  )
}
