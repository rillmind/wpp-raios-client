/** QR para conectar o celular, enquanto o app ainda não está pareado. */
export function PairingQr({ svg }: { svg: string | null }) {
  return (
    <section className="pairing" aria-label="Conectar celular">
      <h2>Conecte seu WhatsApp</h2>
      <ol>
        <li>Abra o WhatsApp no celular.</li>
        <li>
          Vá em <strong>Aparelhos conectados → Conectar aparelho</strong>.
        </li>
        <li>Aponte a câmera para o código.</li>
      </ol>
      {svg ? (
        <img className="pairing__qr" src={`data:image/svg+xml;utf8,${encodeURIComponent(svg)}`} alt="QR de pareamento" />
      ) : (
        <p className="pairing__wait">Gerando o código…</p>
      )}
      <p className="pairing__hint">O código muda a cada poucos segundos; esta tela acompanha sozinha.</p>
    </section>
  )
}
