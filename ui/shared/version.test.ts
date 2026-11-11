import { describe, expect, it } from 'vitest'
import { isNewer } from './version'

describe('isNewer', () => {
  it('compares numerically, part by part', () => {
    expect(isNewer('0.1.10', '0.1.9')).toBe(true)
    expect(isNewer('0.2.0', '0.1.99')).toBe(true)
    expect(isNewer('1.0', '0.9.9')).toBe(true)
  })
  it('is false for the same or an older version', () => {
    expect(isNewer('0.1.0', '0.1.0')).toBe(false)
    expect(isNewer('v0.1.0', '0.1.0')).toBe(false)
    expect(isNewer('0.1.0', '0.1.1')).toBe(false)
  })
})
