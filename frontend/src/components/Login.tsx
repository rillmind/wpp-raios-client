import { useState } from 'react'

export function Login({ onSubmit, error }: { onSubmit: (token: string) => void; error: string | null }) {
  const [token, setToken] = useState('')
  return (
    <main className="login">
      <form
        className="login__card"
        onSubmit={(e) => {
          e.preventDefault()
          if (token.trim()) onSubmit(token.trim())
        }}
      >
        <h1>WhatsApp Lite</h1>
        <p>
          Cole o token da API. Ele está em <code>data/config.toml</code>, campo <code>token</code>.
        </p>
        <input
          type="password"
          autoComplete="off"
          value={token}
          aria-label="Token"
          onChange={(e) => setToken(e.target.value)}
          autoFocus
        />
        {error && (
          <p className="login__error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" className="btn btn--primary">
          Entrar
        </button>
      </form>
    </main>
  )
}
