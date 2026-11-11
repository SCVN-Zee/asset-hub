/**
 * The AI pet's face: a 16×16 LED matrix with an expression per state of the chat
 * (see the AI Pet Pixel Faces prototype). Pure, so the frames and the choice of
 * expression can be tested; components/PixelFace draws them.
 */

export const EXPRESSIONS = ['idle', 'listening', 'thinking', 'searching', 'found', 'asking', 'error', 'sad', 'sleeping'] as const
export type Expression = (typeof EXPRESSIONS)[number]

/** One frame: 16 rows of 16 characters ('.' off, '#' lit, '*' highlight, 'o' dim, 'r' blush, 'b' water, 'z' sleep), and a nudge of the whole face in cells. */
export interface FaceFrame {
  rows: string[]
  dx: number
  dy: number
}

/** The face fills the round avatar: only cells inside this circle exist (lit or off). */
export function inFace(x: number, y: number): boolean {
  return (x - 7.5) ** 2 + (y - 7.5) ** 2 <= 8.2 ** 2
}

/** A frame lasts this long; `drawFace` takes the frame count. */
export const FACE_TICK_MS = 120

const SMILE: Array<[number, number]> = [[5, 11], [6, 12], [7, 12], [8, 12], [9, 12], [10, 11]]

export function drawFace(expr: Expression, t: number): FaceFrame {
  const g = Array.from({ length: 16 }, () => new Array<string>(16).fill('.'))
  const px = (x: number, y: number, c = '#') => {
    if (x >= 0 && x < 16 && y >= 0 && y < 16) g[y][x] = c
  }
  const rect = (x: number, y: number, w: number, h: number, c = '#') => {
    for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) px(x + i, y + j, c)
  }
  const pts = (list: Array<[number, number]>, c = '#', dx = 0, dy = 0) => list.forEach(([x, y]) => px(x + dx, y + dy, c))
  let dx = 0
  let dy = 0

  switch (expr) {
    case 'idle': {
      // Blinks, now and then glances to either side.
      const c = t % 40
      if (c < 2) {
        rect(4, 7, 2, 1)
        rect(10, 7, 2, 1)
      } else {
        const look = c >= 14 && c < 22 ? 1 : c >= 27 && c < 34 ? -1 : 0
        rect(4 + look, 5, 2, 4)
        rect(10 + look, 5, 2, 4)
        px(4 + look, 5, '*')
        px(10 + look, 5, '*')
      }
      pts(SMILE)
      break
    }
    case 'listening': {
      if (t % 30 < 2) {
        rect(3, 6, 3, 1)
        rect(10, 6, 3, 1)
      } else {
        rect(3, 4, 3, 4)
        rect(10, 4, 3, 4)
        px(3, 4, '*')
        px(10, 4, '*')
      }
      rect(7, 11, 2, 2)
      dy = Math.floor(t / 4) % 2 ? -1 : 0
      break
    }
    case 'thinking': {
      // Looking up and away, the dots filling in.
      rect(5, 4, 2, 3)
      rect(11, 4, 2, 3)
      px(6, 4, '*')
      px(12, 4, '*')
      pts([[7, 12], [8, 12], [9, 12], [10, 11]])
      const dots = Math.floor((t % 16) / 4)
      for (let k = 0; k < dots; k++) px(9 + k * 2, 2, '*')
      break
    }
    case 'searching': {
      // Eyes scanning left and right.
      const seq = [0, 1, 2, 2, 1, 0, -1, -2, -2, -1]
      const off = seq[Math.floor(t / 2) % seq.length]
      rect(4 + off, 6, 2, 2)
      rect(10 + off, 6, 2, 2)
      rect(3, 4, 4, 1, 'o')
      rect(9, 4, 4, 1, 'o')
      rect(6, 12, 4, 1)
      break
    }
    case 'found': {
      pts([[3, 7], [4, 6], [5, 6], [6, 7]])
      pts([[9, 7], [10, 6], [11, 6], [12, 7]])
      rect(4, 10, 8, 1)
      rect(5, 11, 6, 1)
      rect(6, 12, 4, 1)
      rect(6, 11, 4, 1, 'r')
      pts([[2, 9], [3, 9], [12, 9], [13, 9]], 'r')
      if (t % 8 < 4) pts([[2, 3], [13, 2], [14, 5]], '*')
      else pts([[3, 2], [12, 3], [1, 6]], '*')
      dy = Math.floor(t / 3) % 2 ? -1 : 0
      break
    }
    case 'asking': {
      rect(4, 5, 2, 4)
      px(4, 5, '*')
      rect(10, 6, 2, 3)
      px(10, 6, '*')
      rect(10, 4, 3, 1, 'o')
      pts([[6, 12], [7, 11], [8, 12], [9, 11]])
      // A small "?" that blinks, kept inside the round face.
      if (t % 10 < 7) pts([[11, 1], [12, 1], [13, 2], [12, 3]], '*')
      px(12, 5, t % 10 < 7 ? '*' : 'o')
      break
    }
    case 'error': {
      pts([[3, 5], [5, 5], [4, 6], [3, 7], [5, 7]])
      pts([[10, 5], [12, 5], [11, 6], [10, 7], [12, 7]])
      pts([[5, 12], [6, 11], [7, 12], [8, 11], [9, 12], [10, 11]])
      const sy = 3 + Math.floor((t % 12) / 2)
      px(14, sy, 'b')
      px(14, sy + 1, 'b')
      dx = t % 20 < 6 ? (t % 2 ? 1 : -1) : 0
      break
    }
    case 'sad': {
      pts([[3, 5], [4, 5], [5, 4]], 'o')
      pts([[10, 4], [11, 5], [12, 5]], 'o')
      rect(4, 7, 2, 2)
      rect(10, 7, 2, 2)
      pts([[5, 12], [6, 11], [7, 11], [8, 11], [9, 11], [10, 12]])
      px(4, 9 + Math.floor((t % 12) / 2), 'b')
      break
    }
    case 'sleeping': {
      const breathe = Math.floor(t / 8) % 2
      rect(3, 7 + breathe, 3, 1)
      rect(10, 7 + breathe, 3, 1)
      rect(7, 11 + breathe, 2, 1)
      const phase = t % 24
      pts([[0, 0], [1, 0], [2, 0], [1, 1], [0, 2], [1, 2], [2, 2]], 'z', 10 + (phase >= 12 ? 1 : 0), 4 - Math.floor(phase / 8))
      break
    }
  }
  return { rows: g.map((r) => r.join('')), dx, dy }
}

/** What the pet knows of the chat when it picks a face. */
export interface PetMood {
  open: boolean
  /** Something typed in the box. */
  typing: boolean
  busy: boolean
  /** A tool is running (search, words…). */
  working: boolean
  /** The latest answer, once written. */
  last?: { state?: 'pending' | 'stopped' | 'failed'; reported?: boolean; choices?: boolean; text: boolean }
  /** The latest answer came in (or was reported) moments ago: its reaction still shows with the bubble closed. */
  fresh: boolean
  /** Nothing has happened for a long while. */
  dozing: boolean
}

export function petExpression(m: PetMood): Expression {
  if (m.busy) return m.working ? 'searching' : 'thinking'
  if (m.open && m.typing) return 'listening'
  const l = m.last
  if (l && (m.open || m.fresh)) {
    if (l.state === 'failed') return 'error'
    if (l.reported || l.state === 'stopped') return 'sad'
    if (l.choices) return 'asking'
    if (l.text) return m.fresh ? 'found' : 'listening'
  }
  if (m.open) return 'listening'
  return m.dozing ? 'sleeping' : 'idle'
}
