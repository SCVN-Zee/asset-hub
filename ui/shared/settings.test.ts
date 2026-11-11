import { describe, expect, it } from 'vitest'
import { sanitizeSettings, type Settings } from './settings'

const defaults: Settings = { language: 'en', cacheDir: '/cache', excludedFolders: ['_ThirdParty'] }

describe('sanitizeSettings', () => {
  it('defaults to English', () => {
    expect(sanitizeSettings(undefined, defaults)).toEqual(defaults)
  })

  it('keeps valid values and drops invalid ones', () => {
    expect(sanitizeSettings({ language: 'vi', cacheDir: '/x', excludedFolders: [' Plugins ', 'Plugins', 3, ''] }, defaults)).toEqual({
      language: 'vi',
      cacheDir: '/x',
      excludedFolders: ['Plugins']
    })
    expect(sanitizeSettings({ language: 'fr', cacheDir: '  ', excludedFolders: 'x', extra: 1 }, defaults)).toEqual(defaults)
  })

  it('allows an empty exclusion list', () => {
    expect(sanitizeSettings({ excludedFolders: [] }, defaults).excludedFolders).toEqual([])
  })
})

describe('last opened repo/branch', () => {
  it('keeps non-empty strings only', () => {
    expect(sanitizeSettings({ lastRepo: 'o/r', lastBranch: 'vn/x' }, defaults)).toMatchObject({ lastRepo: 'o/r', lastBranch: 'vn/x' })
    expect(sanitizeSettings({ lastRepo: '', lastBranch: 3 }, defaults)).not.toHaveProperty('lastRepo')
  })
})

describe('export layout', () => {
  it('keeps known layouts only', () => {
    expect(sanitizeSettings({ exportLayout: 'byType' }, defaults)).toMatchObject({ exportLayout: 'byType' })
    expect(sanitizeSettings({ exportLayout: 'flat' }, defaults)).not.toHaveProperty('exportLayout')
  })
})

describe('playable budget preset', () => {
  it('keeps a known preset and positive custom limits only', () => {
    expect(sanitizeSettings({ budgetPreset: 'tight' }, defaults)).toMatchObject({ budgetPreset: 'tight' })
    // Checks can be turned off: not every asset is made for a playable.
    expect(sanitizeSettings({ budgetPreset: 'off' }, defaults)).toMatchObject({ budgetPreset: 'off' })
    expect(sanitizeSettings({ budgetPreset: 'huge' }, defaults)).not.toHaveProperty('budgetPreset')
    expect(sanitizeSettings({ customBudget: { triangles: 9000, bones: -1, fileBytes: 'x', other: 3 } }, defaults).customBudget).toEqual({ triangles: 9000 })
    expect(sanitizeSettings({ customBudget: { bones: 0 } }, defaults)).not.toHaveProperty('customBudget')
  })
})
