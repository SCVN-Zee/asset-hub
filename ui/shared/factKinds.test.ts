import { readFileSync } from 'fs'
import { resolve } from 'path'
import { describe, expect, it } from 'vitest'
import { CACHE_OBJECT_NAMES, FACT_KINDS } from '../../backend/src/kinds'

/** Reads `SHARED_FACT_KINDS` from the Rust core source, resolving each `*_KIND` constant it lists. */
function rustSharedFactKinds(): string[] {
  const source = readFileSync(resolve(__dirname, '../../crates/core/src/github/source.rs'), 'utf8')
  const list = source.match(/pub const SHARED_FACT_KINDS: \[&str; \d+\] = \[([^\]]*)\];/)
  if (!list) throw new Error('SHARED_FACT_KINDS not found in source.rs')
  return list[1].split(',').map((name) => {
    const constant = name.trim()
    const value = source.match(new RegExp(`pub const ${constant}: &str = "([^"]*)";`))
    if (!value) throw new Error(`${constant} not found in source.rs`)
    return value[1]
  })
}

describe('backend fact kinds', () => {
  const sharedFactKinds = rustSharedFactKinds()

  it('match SHARED_FACT_KINDS', () => {
    expect([...FACT_KINDS]).toEqual(sharedFactKinds)
  })

  it('name one cache object per kind plus catalog and stats', () => {
    expect([...CACHE_OBJECT_NAMES].sort()).toEqual(['catalog', 'stats', ...sharedFactKinds.map((k) => `facts-${k}`)].sort())
  })
})
