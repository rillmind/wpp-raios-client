import { useEffect, type ReactNode } from 'react'

interface Props {
  title: string
  onClose: () => void
  children: ReactNode
}

/** Janela modal simples; Esc ou clicar fora fecha (sem fechar a conversa). */
export function Dialog({ title, onClose, children }: Props) {
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
    <div className="dialog__backdrop" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <section className="dialog" role="dialog" aria-modal="true" aria-label={title}>
        <h2 className="dialog__title">{title}</h2>
        {children}
      </section>
    </div>
  )
}

export interface ConfirmRequest {
  title: string
  text: string
  confirmLabel: string
  onConfirm: () => void
}

export function Confirm({ req, onClose }: { req: ConfirmRequest; onClose: () => void }) {
  return (
    <Dialog title={req.title} onClose={onClose}>
      <p className="dialog__text">{req.text}</p>
      <div className="dialog__actions">
        <button type="button" className="btn btn--ghost" onClick={onClose}>
          Cancelar
        </button>
        <button
          type="button"
          className="btn btn--danger"
          autoFocus
          onClick={() => {
            onClose()
            req.onConfirm()
          }}
        >
          {req.confirmLabel}
        </button>
      </div>
    </Dialog>
  )
}
