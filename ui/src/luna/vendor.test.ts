import { createHash } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { LUNA_VERSION } from './runtime-info'

const dir = resolve(__dirname, '../../../vendor/luna', LUNA_VERSION)

function listFiles(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory() ? listFiles(join(root, entry.name)) : [relative(dir, join(root, entry.name))]
  )
}

function readSums(): Map<string, string> {
  const sums = new Map<string, string>()
  for (const line of readFileSync(join(dir, 'SHA256SUMS'), 'utf8').split('\n')) {
    if (!line.trim()) continue
    const match = /^([0-9a-f]{64})  (.+)$/.exec(line)
    if (!match) throw new Error(`Malformed SHA256SUMS line: ${line}`)
    sums.set(match[2], match[1])
  }
  return sums
}

describe('bundled Luna runtime files', () => {
  it('has a VERSION file that matches the pinned version', () => {
    expect(readFileSync(join(dir, 'VERSION'), 'utf8').trim()).toBe(LUNA_VERSION)
    expect(JSON.parse(readFileSync(join(dir, 'config.json'), 'utf8')).version).toBe(LUNA_VERSION)
  })

  it('matches every hash in SHA256SUMS', () => {
    const sums = readSums()
    expect(sums.size).toBeGreaterThan(0)
    for (const [rel, expected] of sums) {
      const file = join(dir, rel)
      expect(existsSync(file) && statSync(file).isFile(), `${rel} is missing`).toBe(true)
      const actual = createHash('sha256').update(readFileSync(file)).digest('hex')
      expect(actual, `${rel} was edited; a Luna change must be a version bump`).toBe(expected)
    }
  })

  it('has no file outside SHA256SUMS', () => {
    const listed = new Set(readSums().keys())
    const extra = listFiles(dir).filter((rel) => rel !== 'SHA256SUMS' && rel !== 'VERSION' && !listed.has(rel))
    expect(extra).toEqual([])
  })
})
