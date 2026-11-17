/**
 * Binary pack of named blobs: repeated `[u16 nameLength][name utf8][u32 byteLength][bytes]`,
 * big-endian lengths. The same codec lives in `crates/pack/src/lib.rs` for the desktop; keep
 * both files and their test vectors identical.
 */
export interface PackEntry {
  name: string
  bytes: Uint8Array
}

const encoder = new TextEncoder()
const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: false })

/** The bytes that precede an entry's data: `[u16 nameLength][name utf8][u32 byteLength]`. */
export function encodeEntryHeader(name: string, byteLength: number): Uint8Array {
  const nameBytes = encoder.encode(name)
  if (nameBytes.length > 0xffff) throw new RangeError('pack name too long')
  const out = new Uint8Array(2 + nameBytes.length + 4)
  const view = new DataView(out.buffer)
  view.setUint16(0, nameBytes.length)
  out.set(nameBytes, 2)
  view.setUint32(2 + nameBytes.length, byteLength)
  return out
}

export function encodePack(entries: readonly PackEntry[]): Uint8Array {
  const named = entries.map((e) => ({ name: encoder.encode(e.name), bytes: e.bytes }))
  let total = 0
  for (const e of named) {
    if (e.name.length > 0xffff) throw new RangeError('pack name too long')
    total += 2 + e.name.length + 4 + e.bytes.length
  }
  const out = new Uint8Array(total)
  const view = new DataView(out.buffer)
  let at = 0
  for (const e of named) {
    view.setUint16(at, e.name.length)
    at += 2
    out.set(e.name, at)
    at += e.name.length
    view.setUint32(at, e.bytes.length)
    at += 4
    out.set(e.bytes, at)
    at += e.bytes.length
  }
  return out
}

/** Throws `RangeError` on truncated or malformed input. */
export function decodePack(data: Uint8Array): PackEntry[] {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength)
  const entries: PackEntry[] = []
  let at = 0
  while (at < data.length) {
    if (at + 2 > data.length) throw new RangeError('truncated pack')
    const nameLength = view.getUint16(at)
    at += 2
    if (at + nameLength + 4 > data.length) throw new RangeError('truncated pack')
    let name: string
    try {
      name = decoder.decode(data.subarray(at, at + nameLength))
    } catch {
      throw new RangeError('invalid pack name')
    }
    at += nameLength
    const byteLength = view.getUint32(at)
    at += 4
    if (at + byteLength > data.length) throw new RangeError('truncated pack')
    entries.push({ name, bytes: data.slice(at, at + byteLength) })
    at += byteLength
  }
  return entries
}
