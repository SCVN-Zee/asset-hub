import { vi, type MockInstance } from 'vitest'

export const FAKE_B2_ENDPOINT = 'https://b2.fake.invalid'
export const FAKE_B2_BUCKET = 'test-bucket'

interface Version {
  id: string
  bytes: Uint8Array
}

export interface B2Call {
  method: string
  key: string
  versionId: string | null
}

/**
 * In-memory stand-in for the B2 S3 API: versioned PUT, GET of the latest version and DELETE.
 * It answers by intercepting `fetch` for the fake endpoint and passes every other request on.
 * Hooks let a test hold a request, fail it, or watch how many run at once.
 */
export class FakeB2 {
  private versions = new Map<string, Version[]>()
  private counter = 0
  private spy: MockInstance<typeof fetch> | null = null
  calls: B2Call[] = []
  /** Status to answer a request with instead of serving it; `null` serves normally. */
  failWith: ((call: B2Call) => number | null) | null = null
  /** Runs before a request is served. A test can hold it here. */
  hold: ((call: B2Call, signal: AbortSignal) => Promise<void>) | null = null
  inflightGets = 0
  maxInflightGets = 0
  /** Requests that arrived without a SigV4 `Authorization` header. */
  unsigned = 0

  install(): void {
    const original = globalThis.fetch.bind(globalThis)
    this.spy = vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
      const req = input instanceof Request && init === undefined ? input : new Request(input, init)
      return req.url.startsWith(FAKE_B2_ENDPOINT) ? this.serve(req) : original(input, init)
    })
  }

  uninstall(): void {
    this.spy?.mockRestore()
    this.spy = null
  }

  reset(): void {
    this.versions.clear()
    this.calls = []
    this.failWith = null
    this.hold = null
    this.inflightGets = 0
    this.maxInflightGets = 0
    this.unsigned = 0
  }

  /** Every stored version of `key`, oldest first. */
  versionsOf(key: string): string[] {
    return (this.versions.get(key) ?? []).map((v) => v.id)
  }

  keys(): string[] {
    return [...this.versions.entries()].filter(([, v]) => v.length > 0).map(([k]) => k).sort()
  }

  totalBytes(): number {
    let total = 0
    for (const list of this.versions.values()) for (const v of list) total += v.bytes.length
    return total
  }

  /** Removes every version of `key`, as a lifecycle rule would. */
  dropAll(key: string): void {
    this.versions.set(key, [])
  }

  /** Stores a version directly, bypassing the Worker. */
  seed(key: string, bytes: Uint8Array): string {
    return this.store(key, bytes)
  }

  callsTo(method: string): B2Call[] {
    return this.calls.filter((c) => c.method === method)
  }

  private store(key: string, bytes: Uint8Array): string {
    const id = `v${++this.counter}`
    const list = this.versions.get(key) ?? []
    list.push({ id, bytes })
    this.versions.set(key, list)
    return id
  }

  private async serve(req: Request): Promise<Response> {
    const url = new URL(req.url)
    const prefix = `/${FAKE_B2_BUCKET}/`
    if (!url.pathname.startsWith(prefix)) return new Response(null, { status: 404 })
    const key = url.pathname.slice(prefix.length).split('/').map(decodeURIComponent).join('/')
    const call: B2Call = { method: req.method, key, versionId: url.searchParams.get('versionId') }
    this.calls.push(call)
    if (!req.headers.get('authorization')?.startsWith('AWS4-HMAC-SHA256 ')) this.unsigned += 1

    const isGet = req.method === 'GET'
    if (isGet) this.maxInflightGets = Math.max(this.maxInflightGets, ++this.inflightGets)
    try {
      await this.hold?.(call, req.signal)
      const status = this.failWith?.(call) ?? null
      if (status !== null) return new Response('fail', { status })
      switch (req.method) {
        case 'PUT': {
          const id = this.store(key, new Uint8Array(await req.arrayBuffer()))
          return new Response(null, { status: 200, headers: { 'x-amz-version-id': id } })
        }
        case 'GET': {
          const stored = this.versions.get(key)
          const latest = call.versionId ? stored?.find((v) => v.id === call.versionId) : stored?.at(-1)
          return latest ? new Response(latest.bytes, { headers: { 'content-length': String(latest.bytes.length) } }) : new Response(null, { status: 404 })
        }
        case 'DELETE': {
          const list = this.versions.get(key)
          const at = list?.findIndex((v) => v.id === call.versionId) ?? -1
          if (!list || at < 0) return new Response(null, { status: 404 })
          list.splice(at, 1)
          return new Response(null, { status: 204 })
        }
        default:
          return new Response(null, { status: 405 })
      }
    } finally {
      if (isGet) this.inflightGets -= 1
    }
  }
}

export const fakeB2 = new FakeB2()
