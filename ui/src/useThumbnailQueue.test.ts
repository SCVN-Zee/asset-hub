import { describe, expect, it } from 'vitest'
import { afterCancel, takeNext } from './useThumbnailQueue'

const jobs = (...keys: string[]) => keys.map((key) => ({ key }))

describe('takeNext', () => {
  it('starts the priority job first even when its order is larger', () => {
    expect(takeNext(jobs('a', 'b', 'c'), 'c')).toBe(2)
  })

  it('keeps the queue order without a priority key', () => {
    expect(takeNext(jobs('a', 'b'))).toBe(0)
  })

  it('ignores a priority key that is not queued', () => {
    expect(takeNext(jobs('a', 'b'), 'gone')).toBe(0)
  })

  it('returns -1 for an empty queue', () => {
    expect(takeNext([], 'a')).toBe(-1)
  })
})

describe('afterCancel', () => {
  it('drops a cancelled card', () => {
    expect(afterCancel(jobs('a', 'b'), 'a', 'b')).toEqual(jobs('b'))
  })

  it('keeps the priority card', () => {
    expect(afterCancel(jobs('a', 'b'), 'b', 'b')).toEqual(jobs('a', 'b'))
  })
})
