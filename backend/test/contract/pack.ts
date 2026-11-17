/**
 * Test copy of the thumbnail pack codec: repeated `[u16 nameLength][name utf8][u32 byteLength][bytes]`,
 * big-endian lengths. It is written from the wire format so the suite never imports Worker code.
 */
export interface PackEntry {
  name: string
  bytes: Uint8Array
}

const encoder = new TextEncoder()

export function encodePack(entries: readonly PackEntry[]): Uint8Array {
  const named = entries.map((e) => ({ name: encoder.encode(e.name), bytes: e.bytes }))
  const out = new Uint8Array(named.reduce((sum, e) => sum + 2 + e.name.length + 4 + e.bytes.length, 0))
  const view = new DataView(out.buffer)
  let at = 0
  for (const e of named) {
    view.setUint16(at, e.name.length)
    out.set(e.name, at + 2)
    view.setUint32(at + 2 + e.name.length, e.bytes.length)
    out.set(e.bytes, at + 6 + e.name.length)
    at += 6 + e.name.length + e.bytes.length
  }
  return out
}

export function decodePack(data: Uint8Array): PackEntry[] {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength)
  const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: false })
  const entries: PackEntry[] = []
  let at = 0
  while (at < data.length) {
    const nameLength = view.getUint16(at)
    const name = decoder.decode(data.subarray(at + 2, at + 2 + nameLength))
    const byteLength = view.getUint32(at + 2 + nameLength)
    const start = at + 6 + nameLength
    if (start + byteLength > data.length) throw new RangeError('truncated pack')
    entries.push({ name, bytes: data.slice(start, start + byteLength) })
    at = start + byteLength
  }
  return entries
}
