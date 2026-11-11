import { readdirSync, readFileSync } from 'fs'
import { join } from 'path'
import { describe, expect, it } from 'vitest'
import en from './locales/en.json'
import vi from './locales/vi.json'
import ko from './locales/ko.json'

function keys(obj: object, prefix = ''): string[] {
  return Object.entries(obj).flatMap(([k, v]) => (v && typeof v === 'object' ? keys(v, `${prefix}${k}.`) : [`${prefix}${k}`]))
}

// Plural suffixes differ by language (Vietnamese and Korean only have `_other`), so compare base keys.
const base = (list: string[]) => new Set(list.map((k) => k.replace(/_(zero|one|two|few|many|other)$/, '')))

describe('locales', () => {
  it.each([
    ['Vietnamese', vi],
    ['Korean', ko]
  ])('%s defines the same keys as English', (_, locale) => {
    expect([...base(keys(locale))].sort()).toEqual([...base(keys(en))].sort())
  })

  it('has every key the code asks for by name (a missing one shows as its raw key)', () => {
    const src = join(__dirname, '..')
    const files = readdirSync(src, { recursive: true, encoding: 'utf8' }).filter((f) => /\.tsx?$/.test(f) && !f.endsWith('.test.ts'))
    const known = base(keys(en))
    const missing = files.flatMap((f) => [...readFileSync(join(src, f), 'utf8').matchAll(/\bt\(\s*'([A-Za-z0-9_.]+)'/g)].map((m) => m[1]).filter((k) => !known.has(k) && !known.has(k.replace(/_(one|other)$/, ''))).map((k) => `${k} (${f})`))
    expect(missing).toEqual([])
  })
})
