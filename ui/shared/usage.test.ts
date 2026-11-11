import { describe, expect, it } from 'vitest'
import { feedbackIssue, isWorthSending, newSession, noteEvent, noteIpc, parseUsageComment, sanitizeFeedback, usageComment } from './usage'

const fresh = () => newSession('s1', new Date('2026-10-31T10:00:00Z'), { version: '0.1.3', os: 'macOS 15.3', language: 'en' })

describe('usage notes', () => {
  it('counts opened branches, exports and errors from IPC results', () => {
    const s = fresh()
    noteIpc(s, 'source:list-tree', ['org/game', 'main'], { ok: true, data: [] })
    noteIpc(s, 'source:list-tree', ['org/game', 'dev'], { ok: true, data: [] })
    noteIpc(s, 'export:unitypackage', [], { ok: true, data: { canceled: false, assets: 3 } })
    noteIpc(s, 'export:unitypackage', [], { ok: true, data: { canceled: true } })
    noteIpc(s, 'export:to-unity', [], { ok: false, error: { code: 'network' } })
    noteIpc(s, 'thumbs:get', [], { some: 'value' })
    expect(s.repos).toEqual(['org/game'])
    expect(s.counts).toEqual({ 'branch.open': 2, export: 1, 'export.assets': 3, 'error.network': 1, 'unity.fail': 1 })
  })

  it('keeps asset types to a safe key', () => {
    const s = fresh()
    noteEvent(s, { kind: 'view', type: 'fbx' })
    noteEvent(s, { kind: 'view', type: '../../x y' })
    noteEvent(s, { kind: 'search' })
    expect(s.counts).toEqual({ 'view.fbx': 1, 'view.xy': 1, search: 1 })
  })

  it('sends only sessions with a user and some use', () => {
    const s = fresh()
    expect(isWorthSending(s)).toBe(false)
    s.login = 'dan'
    expect(isWorthSending(s)).toBe(false)
    s.activeMs = 120_000
    expect(isWorthSending(s)).toBe(true)
  })

  it('reads a session back from its comment', () => {
    const s = { ...fresh(), login: 'dan', counts: { search: 2 } }
    expect(parseUsageComment(usageComment(s))).toEqual(s)
    expect(parseUsageComment('hello')).toBeUndefined()
  })
})

describe('feedback', () => {
  it('rejects malformed reports', () => {
    expect(sanitizeFeedback({ kind: 'rant', title: 'x', body: '' })).toBeUndefined()
    expect(sanitizeFeedback({ kind: 'bug', title: '  ', body: '' })).toBeUndefined()
    expect(sanitizeFeedback({ kind: 'bug', title: 'Crash', body: 'when…', context: { repo: 'org/game', branch: 3 } })).toEqual({
      kind: 'bug',
      title: 'Crash',
      body: 'when…',
      context: { repo: 'org/game', branch: undefined, asset: undefined }
    })
  })

  it('writes the issue with who and where', () => {
    const issue = feedbackIssue({ kind: 'idea', title: ' Dark\nmode ', body: 'please', context: { repo: 'org/game', branch: 'main' } }, { login: 'dan', version: '0.1.3', os: 'macOS 15', language: 'vi' })
    expect(issue.title).toBe('[Idea] Dark mode')
    expect(issue.body).toBe('**From:** @dan · v0.1.3 · macOS 15 · vi\n**Where:** org/game @ main\n\nplease')
  })
})
