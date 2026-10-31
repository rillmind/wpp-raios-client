import { useEffect, useState, type RefObject } from 'react'

/** Vira `true` (e fica) quando o elemento chega perto da área visível. */
export function useInView(ref: RefObject<Element | null>, margin = '300px'): boolean {
  // Sem IntersectionObserver (navegador antigo), carrega direto.
  const [seen, setSeen] = useState(() => typeof IntersectionObserver === 'undefined')
  useEffect(() => {
    const el = ref.current
    if (seen || !el) return
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setSeen(true)
          io.disconnect()
        }
      },
      { rootMargin: margin },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [ref, margin, seen])
  return seen
}
