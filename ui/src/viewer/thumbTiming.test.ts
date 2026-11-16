import { afterEach, describe, expect, it, vi } from 'vitest'
import { createThumbTimer, thumbTimingEnabled } from './thumbTiming'

describe('createThumbTimer', () => {
  it('records the time between marks and formats one line', () => {
    let t = 100
    const timer = createThumbTimer(() => t)
    t = 220
    timer.mark('queue')
    t = 620
    timer.mark('fetch')
    expect(timer.reached()).toBe('fetch')
    expect(timer.line('three', 'Assets/x.fbx')).toBe('[thumb] three Assets/x.fbx total=520 queue=120 fetch=400')
  })

  it('reports start before any stage and counts from an earlier start', () => {
    let t = 50
    const timer = createThumbTimer(() => t, 10)
    expect(timer.reached()).toBe('start')
    t = 60
    timer.mark('queue')
    expect(timer.line('image', 'a.png')).toBe('[thumb] image a.png total=50 queue=50')
  })
})

describe('thumbTimingEnabled', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('is on when the flag is set', () => {
    vi.stubGlobal('localStorage', { getItem: () => '1' })
    expect(thumbTimingEnabled()).toBe(true)
  })

  it('does not throw when storage is unavailable', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('blocked')
      }
    })
    expect(() => thumbTimingEnabled()).not.toThrow()
  })
})
