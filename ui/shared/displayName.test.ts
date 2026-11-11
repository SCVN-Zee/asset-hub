import { describe, expect, it } from 'vitest'
import { displayName } from './displayName'

describe('displayName', () => {
  it('leaves the company prefix off', () => {
    expect(displayName('SCVN-Dan')).toBe('Dan')
    expect(displayName('SC-Minh')).toBe('Minh')
    expect(displayName('scvn_linh')).toBe('linh')
  })
  it('keeps other logins as they are', () => {
    expect(displayName('octocat')).toBe('octocat')
    expect(displayName('Scott')).toBe('Scott')
    expect(displayName('SCVN-')).toBe('SCVN-')
  })
})
