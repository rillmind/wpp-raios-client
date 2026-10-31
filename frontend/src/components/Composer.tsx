import { useEffect, useRef, useState, type KeyboardEvent } from 'react'

interface Props {
  disabled: boolean
  onSend: (text: string) => Promise<void>
}

function isTouch(): boolean {
  return window.matchMedia('(pointer: coarse)').matches
}

export function Composer({ disabled, onSend }: Props) {
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const input = useRef<HTMLTextAreaElement>(null)

  // Ao abrir a conversa (o componente é montado por conversa), já deixa pronto para
  // escrever. Em tela de toque não: abriria o teclado só para ler.
  useEffect(() => {
    if (!disabled && !isTouch()) input.current?.focus()
  }, [disabled])

  async function submit() {
    const value = text.trim()
    if (!value || sending || disabled) return
    setSending(true)
    setError(null)
    try {
      await onSend(value)
      setText('')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Falha ao enviar')
    } finally {
      setSending(false)
    }
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    // Computador: Enter envia, Shift+Enter quebra linha (e não interfere em composição IME).
    // Tela de toque: Enter sempre quebra linha; só o botão envia.
    if (isTouch()) return
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault()
      void submit()
    }
  }

  return (
    <form
      className="composer"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      {error && (
        <p className="composer__error" role="alert">
          {error}
        </p>
      )}
      <div className="composer__row">
        <textarea
          ref={input}
          className="composer__input"
          rows={1}
          value={text}
          placeholder={disabled ? 'Desconectado' : 'Mensagem'}
          aria-label="Mensagem"
          disabled={disabled}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          enterKeyHint="enter"
        />
        <button
          type="submit"
          className="btn btn--primary"
          disabled={disabled || sending || !text.trim()}
          // Não tira o foco da caixa: no celular, isso fecharia o teclado a cada envio.
          onPointerDown={(e) => e.preventDefault()}
        >
          Enviar
        </button>
      </div>
    </form>
  )
}
