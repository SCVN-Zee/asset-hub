import { useEffect, useRef } from 'react'
import { drawFace, FACE_TICK_MS, inFace, type Expression } from '../petFace'

/** Amber LEDs, as the app's accent; off LEDs stay faintly visible, like a real matrix. */
const COLORS: Record<string, string> = { '#': '#ffb733', '*': '#fff4cf', o: 'rgba(255,183,51,0.45)', r: '#ff6f91', b: '#5ec8ff', z: '#ffd98a' }
const OFF = 'rgba(255,183,51,0.08)'
const GLOW = 'rgba(255,170,40,0.85)'

/** The pet's face: a round 16×16 LED matrix filling `size` px, animated per expression (still with reduced motion). */
export function PixelFace({ expression, size }: { expression: Expression; size: number }) {
  const canvas = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    const cv = canvas.current
    const g = cv?.getContext('2d')
    if (!cv || !g) return
    const dpr = Math.min(3, window.devicePixelRatio || 1)
    cv.width = cv.height = Math.round(size * dpr)
    g.setTransform(dpr, 0, 0, dpr, 0, 0)
    const cell = size / 16
    const dot = cell * 0.44
    const draw = (t: number) => {
      const f = drawFace(expression, t)
      g.clearRect(0, 0, size, size)
      for (let y = 0; y < 16; y++)
        for (let x = 0; x < 16; x++) {
          if (!inFace(x, y)) continue
          // The face nudges as a whole (a shake, a bounce); the matrix itself stays put.
          const sx = x - f.dx
          const sy = y - f.dy
          const c = sx >= 0 && sx < 16 && sy >= 0 && sy < 16 ? f.rows[sy][sx] : '.'
          const lit = c !== '.'
          g.shadowBlur = lit ? cell * 1.1 : 0
          g.shadowColor = GLOW
          g.fillStyle = lit ? COLORS[c] : OFF
          g.beginPath()
          g.arc((x + 0.5) * cell, (y + 0.5) * cell, dot, 0, Math.PI * 2)
          g.fill()
        }
    }
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      // A still face: a frame with eyes open.
      draw(5)
      return
    }
    let t = 0
    draw(t)
    const timer = window.setInterval(() => draw(++t), FACE_TICK_MS)
    return () => window.clearInterval(timer)
  }, [expression, size])
  return <canvas ref={canvas} className="pixel-face" style={{ width: size, height: size }} aria-hidden="true" />
}
