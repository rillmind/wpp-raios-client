import { useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent } from 'react'
import { formatBytes } from './format'

/** Maior arquivo aceito pelo backend (o limite do WhatsApp para mídia). */
const MAX_FILE_BYTES = 100 * 1024 * 1024

export interface FileMeta {
  width?: number
  height?: number
  seconds?: number
}

interface Props {
  disabled: boolean
  onSend: (text: string) => Promise<void>
  /** Um arquivo por vez; `caption` só no primeiro de um lote. */
  onSendFile: (file: File, caption: string | null, meta: FileMeta) => Promise<void>
}

interface Pending {
  id: number
  file: File
  /** URL local (memória da aba) para a miniatura de imagens. */
  preview: string | null
}

function isTouch(): boolean {
  return window.matchMedia('(pointer: coarse)').matches
}

/** Dimensões (e duração) que o celular de quem recebe usa para montar a mensagem. */
async function probe(file: File): Promise<FileMeta> {
  const url = URL.createObjectURL(file)
  try {
    if (file.type.startsWith('image/')) {
      const img = new Image()
      img.src = url
      await img.decode()
      return { width: img.naturalWidth, height: img.naturalHeight }
    }
    if (file.type.startsWith('video/')) {
      const video = document.createElement('video')
      video.preload = 'metadata'
      video.src = url
      await new Promise<void>((resolve, reject) => {
        video.onloadedmetadata = () => resolve()
        video.onerror = () => reject(new Error('vídeo ilegível'))
      })
      return {
        width: video.videoWidth || undefined,
        height: video.videoHeight || undefined,
        seconds: Number.isFinite(video.duration) ? Math.round(video.duration) : undefined,
      }
    }
  } catch {
    // Sem dimensões o envio funciona igual.
  } finally {
    URL.revokeObjectURL(url)
  }
  return {}
}

let nextId = 1

export function Composer({ disabled, onSend, onSendFile }: Props) {
  const [text, setText] = useState('')
  const [files, setFiles] = useState<Pending[]>([])
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const input = useRef<HTMLTextAreaElement>(null)
  const picker = useRef<HTMLInputElement>(null)
  const filesRef = useRef(files)
  useEffect(() => {
    filesRef.current = files
  }, [files])

  // Ao abrir a conversa (o componente é montado por conversa), já deixa pronto para
  // escrever. Em tela de toque não: abriria o teclado só para ler.
  useEffect(() => {
    if (!disabled && !isTouch()) input.current?.focus()
  }, [disabled])

  // Solta as miniaturas da memória ao sair da conversa.
  useEffect(() => () => filesRef.current.forEach((p) => p.preview && URL.revokeObjectURL(p.preview)), [])

  function addFiles(list: FileList | File[]) {
    const added: Pending[] = []
    for (const file of Array.from(list)) {
      if (file.size > MAX_FILE_BYTES) {
        setError(`${file.name || 'Arquivo'} passa de ${formatBytes(MAX_FILE_BYTES)}`)
        continue
      }
      added.push({
        id: nextId++,
        file,
        preview: file.type.startsWith('image/') ? URL.createObjectURL(file) : null,
      })
    }
    if (added.length) setFiles((fs) => [...fs, ...added])
  }

  function removeFile(id: number) {
    setFiles((fs) => {
      const gone = fs.find((p) => p.id === id)
      if (gone?.preview) URL.revokeObjectURL(gone.preview)
      return fs.filter((p) => p.id !== id)
    })
  }

  async function submit() {
    const value = text.trim()
    if ((!value && files.length === 0) || sending || disabled) return
    setSending(true)
    setError(null)
    try {
      if (files.length === 0) {
        await onSend(value)
      } else {
        // O texto vira a legenda do primeiro arquivo.
        let caption: string | null = value || null
        for (const p of files) {
          await onSendFile(p.file, caption, await probe(p.file))
          caption = null
          removeFile(p.id)
        }
      }
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

  // Colar uma imagem (print, imagem copiada) ou arquivo anexa em vez de virar texto.
  function onPaste(e: ClipboardEvent<HTMLTextAreaElement>) {
    if (e.clipboardData.files.length === 0) return
    e.preventDefault()
    addFiles(e.clipboardData.files)
  }

  const canSend = !disabled && !sending && (text.trim() !== '' || files.length > 0)

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
      {files.length > 0 && (
        <ul className="attachments" aria-label="Arquivos para enviar">
          {files.map((p) => (
            <li key={p.id} className="attachment">
              {p.preview ? (
                <img className="attachment__thumb" src={p.preview} alt="" />
              ) : (
                <span className="attachment__thumb attachment__thumb--file" aria-hidden="true">
                  📄
                </span>
              )}
              <span className="attachment__name">{p.file.name || 'Imagem colada'}</span>
              <span className="attachment__size">{formatBytes(p.file.size)}</span>
              <button
                type="button"
                className="attachment__remove"
                aria-label={`Remover ${p.file.name || 'arquivo'}`}
                disabled={sending}
                onClick={() => removeFile(p.id)}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="composer__row">
        <input
          ref={picker}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            if (e.target.files) addFiles(e.target.files)
            e.target.value = ''
          }}
        />
        <button
          type="button"
          className="btn btn--ghost composer__attach"
          aria-label="Anexar arquivo"
          title="Anexar imagem, vídeo ou documento"
          disabled={disabled || sending}
          onPointerDown={(e) => e.preventDefault()}
          onClick={() => picker.current?.click()}
        >
          📎
        </button>
        <textarea
          ref={input}
          className="composer__input"
          rows={1}
          value={text}
          placeholder={disabled ? 'Desconectado' : files.length ? 'Legenda (opcional)' : 'Mensagem'}
          aria-label="Mensagem"
          disabled={disabled}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          enterKeyHint="enter"
        />
        <button
          type="submit"
          className="btn btn--primary"
          disabled={!canSend}
          // Não tira o foco da caixa: no celular, isso fecharia o teclado a cada envio.
          onPointerDown={(e) => e.preventDefault()}
        >
          {sending && files.length ? 'Enviando…' : 'Enviar'}
        </button>
      </div>
    </form>
  )
}
