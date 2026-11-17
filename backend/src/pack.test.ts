import { describe, expect, it } from 'vitest'
import { decodePack, encodeEntryHeader, encodePack } from './pack'

const hex = (bytes: Uint8Array) => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('')
const fromHex = (text: string) => new Uint8Array(text.match(/../g)!.map((h) => parseInt(h, 16)))

// The same vectors are asserted in crates/pack/src/lib.rs on the desktop side.
const VECTORS = [
  {
    entries: [{ name: 'ab', bytes: [1, 2, 3] }],
    hex: '00026162' + '00000003' + '010203'
  },
  {
    entries: [
      { name: 'x.fx', bytes: [] },
      { name: 'é', bytes: [255] }
    ],
    hex: '0004782e6678' + '00000000' + '0002c3a9' + '00000001' + 'ff'
  }
]

describe('thumbnail pack codec', () => {
  for (const [i, v] of VECTORS.entries()) {
    it(`encodes test vector ${i}`, () => {
      const encoded = encodePack(v.entries.map((e) => ({ name: e.name, bytes: new Uint8Array(e.bytes) })))
      expect(hex(encoded)).toBe(v.hex)
    })

    it(`decodes test vector ${i}`, () => {
      const decoded = decodePack(fromHex(v.hex))
      expect(decoded.map((e) => ({ name: e.name, bytes: [...e.bytes] }))).toEqual(v.entries)
    })
  }

  it('entry header matches the start of an encoded entry', () => {
    const header = encodeEntryHeader('ab', 3)
    expect(hex(header)).toBe('00026162' + '00000003')
    expect(hex(encodePack([{ name: 'ab', bytes: new Uint8Array([1, 2, 3]) }]).subarray(0, header.length))).toBe(hex(header))
  })

  it('decodes an empty pack to no entries', () => {
    expect(decodePack(new Uint8Array())).toEqual([])
  })

  it('rejects a truncated pack', () => {
    const full = fromHex(VECTORS[0].hex)
    for (let cut = 1; cut < full.length; cut++) expect(() => decodePack(full.subarray(0, cut))).toThrow(RangeError)
  })

  it('rejects a name that is not valid utf-8', () => {
    expect(() => decodePack(fromHex('0001ff00000000'))).toThrow(RangeError)
  })
})
