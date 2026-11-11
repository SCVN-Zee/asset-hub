import { describe, expect, it } from 'vitest'
import { drawFace, EXPRESSIONS, inFace, petExpression, type PetMood } from './petFace'

const base: PetMood = { open: false, typing: false, busy: false, working: false, fresh: false, dozing: false }

describe('pet face', () => {
  it('draws every expression on a 16×16 grid, every frame', () => {
    for (const e of EXPRESSIONS)
      for (let t = 0; t < 60; t++) {
        const f = drawFace(e, t)
        expect(f.rows).toHaveLength(16)
        expect(f.rows.every((r) => r.length === 16 && /^[.#*orbz]+$/.test(r))).toBe(true)
        expect(f.rows.join('')).toMatch(/[#*]/)
        // Nothing lit falls outside the round avatar.
        f.rows.forEach((r, y) => [...r].forEach((c, x) => c !== '.' && expect(inFace(x, y), `${e} t=${t} (${x},${y})`).toBe(true)))
      }
  })

  it('blinks when idle', () => {
    const lit = (t: number) => drawFace('idle', t).rows.join('').replace(/[^#*]/g, '').length
    expect(lit(0)).toBeLessThan(lit(5))
  })

  it('picks the face from the chat', () => {
    expect(petExpression(base)).toBe('idle')
    expect(petExpression({ ...base, dozing: true })).toBe('sleeping')
    expect(petExpression({ ...base, open: true })).toBe('listening')
    expect(petExpression({ ...base, open: true, typing: true })).toBe('listening')
    expect(petExpression({ ...base, busy: true })).toBe('thinking')
    expect(petExpression({ ...base, busy: true, working: true })).toBe('searching')
    const answered = { ...base, fresh: true, last: { text: true } }
    expect(petExpression(answered)).toBe('found')
    expect(petExpression({ ...answered, last: { text: true, choices: true } })).toBe('asking')
    expect(petExpression({ ...answered, last: { text: false, state: 'failed' } })).toBe('error')
    expect(petExpression({ ...answered, last: { text: true, reported: true } })).toBe('sad')
    expect(petExpression({ ...answered, last: { text: true, state: 'stopped' } })).toBe('sad')
    // A reaction fades once it is old and the bubble is closed.
    expect(petExpression({ ...answered, fresh: false })).toBe('idle')
    // With the bubble open, an old answer leaves the pet listening for more.
    expect(petExpression({ ...answered, fresh: false, open: true })).toBe('listening')
  })
})
