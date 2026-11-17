import { expect } from 'vitest'

export const JSON_TYPE = 'application/json; charset=utf-8'

/** The contract sends no `access-control-*` header on any reply. */
export function expectNoCors(res: Response): void {
  const cors = [...res.headers.keys()].filter((name) => name.startsWith('access-control-'))
  expect(cors).toEqual([])
}

/** Names of the headers a reply carries, without the ones the HTTP layer adds on its own. */
export function headerNames(res: Response): string[] {
  const transport = new Set(['date', 'content-length', 'connection', 'keep-alive', 'transfer-encoding'])
  // `mf-*` headers come from Miniflare's own proxy, not from the Worker.
  return [...res.headers.keys()].filter((name) => !transport.has(name) && !name.startsWith('mf-')).sort()
}

export interface Snapshot {
  status: number
  text: string
  type: string | null
  cors: string[]
}

/** Reads a reply at once, so its body is never left open while another request runs. */
export async function snapshot(res: Response): Promise<Snapshot> {
  return {
    status: res.status,
    text: await res.text(),
    type: res.headers.get('content-type'),
    cors: [...res.headers.keys()].filter((name) => name.startsWith('access-control-'))
  }
}

/** Checks a list of `[reply, status, exact body]` cases: JSON content type and no CORS header on each. */
export function expectErrorReplies(cases: Array<[Snapshot, number, string]>): void {
  for (const [res, status, body] of cases) {
    expect(res.status, body).toBe(status)
    expect(res.text).toBe(body)
    expect(res.type).toBe(JSON_TYPE)
    expect(res.cors).toEqual([])
  }
}
