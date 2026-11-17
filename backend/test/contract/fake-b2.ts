import { createHash, createHmac } from 'node:crypto'

export const B2_HOST = 'b2.fake.test'
export const B2_ENDPOINT = `https://${B2_HOST}`
export const B2_BUCKET = 'test-bucket'
export const B2_REGION = 'us-west-000'
export const B2_KEY_ID = 'test-key-id'
export const B2_APP_KEY = 'test-app-key'

interface Version {
  id: string
  bytes: Uint8Array
}

export interface B2Call {
  method: string
  key: string
  versionId: string | null
}

const MAX_SKEW_MS = 15 * 60 * 1000
const UNSIGNED_PAYLOAD = 'UNSIGNED-PAYLOAD'
const REQUIRED_SIGNED = ['host', 'x-amz-date', 'x-amz-content-sha256']

const sha256Hex = (data: string | Uint8Array): string => createHash('sha256').update(data).digest('hex')
const hmac = (key: string | Buffer, data: string): Buffer => createHmac('sha256', key).update(data).digest()
const encodeRfc3986 = (s: string): string => encodeURIComponent(s).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`)

export interface SigV4Input {
  method: string
  url: string
  /** Header values by lower-case name. Must hold every name in `signedHeaders`; `host` may be left out. */
  headers: Record<string, string>
  signedHeaders: string[]
  payloadHash: string
  /** `YYYYMMDDTHHMMSSZ` */
  datetime: string
  keyId: string
  secret: string
  region: string
}

/**
 * The SigV4 `Authorization` value for a request, built straight from the specification: canonical
 * request over exactly the headers in `signedHeaders`, then the derived key. It shares no code with
 * the client that the Worker uses.
 */
export function sigV4Authorization(input: SigV4Input): string {
  const url = new URL(input.url)
  const canonicalPath = url.pathname.split('/').map((seg) => encodeRfc3986(decodeURIComponent(seg))).join('/')
  const query = [...url.searchParams.entries()]
    .map(([k, v]) => [encodeRfc3986(k), encodeRfc3986(v)] as const)
    .sort(([ak, av], [bk, bv]) => (ak === bk ? (av < bv ? -1 : av > bv ? 1 : 0) : ak < bk ? -1 : 1))
    .map(([k, v]) => `${k}=${v}`)
    .join('&')
  const names = [...input.signedHeaders].sort()
  const canonicalHeaders = names
    .map((n) => `${n}:${(n === 'host' ? (input.headers.host ?? url.host) : input.headers[n]).trim().replace(/\s+/g, ' ')}\n`)
    .join('')
  const canonical = [input.method, canonicalPath, query, canonicalHeaders, names.join(';'), input.payloadHash].join('\n')
  const day = input.datetime.slice(0, 8)
  const scope = `${day}/${input.region}/s3/aws4_request`
  const toSign = ['AWS4-HMAC-SHA256', input.datetime, scope, sha256Hex(canonical)].join('\n')
  const key = hmac(hmac(hmac(hmac(`AWS4${input.secret}`, day), input.region), 's3'), 'aws4_request')
  const signature = createHmac('sha256', key).update(toSign).digest('hex')
  return `AWS4-HMAC-SHA256 Credential=${input.keyId}/${scope}, SignedHeaders=${names.join(';')}, Signature=${signature}`
}

/**
 * Checks the SigV4 `Authorization` header of a request against the secret the test knows. The
 * expected signature covers the headers the client lists in `SignedHeaders`, so any correct signer
 * passes. The payload hash must be `UNSIGNED-PAYLOAD` (what the Worker sends over TLS) or the SHA-256
 * of the body. The date must be within 15 minutes of this machine's clock. Returns the reason for a
 * failure, or `null` when the request is valid.
 */
export async function signatureProblem(
  req: Request,
  expected: { keyId: string; secret: string; region: string }
): Promise<string | null> {
  const auth = req.headers.get('authorization') ?? ''
  const match = /^AWS4-HMAC-SHA256 Credential=([^/]+)\/(\d{8})\/([^/]+)\/s3\/aws4_request, SignedHeaders=([^,]+), Signature=([0-9a-f]{64})$/.exec(auth)
  if (!match) return 'no SigV4 Authorization header'
  const [, keyId, , region, signedHeaders] = match
  if (keyId !== expected.keyId) return 'wrong access key id'
  if (region !== expected.region) return 'wrong region'
  const datetime = req.headers.get('x-amz-date')
  if (!datetime || !/^\d{8}T\d{6}Z$/.test(datetime)) return 'no x-amz-date'
  if (!datetime.startsWith(match[2])) return 'credential date differs from x-amz-date'
  const at = Date.UTC(+datetime.slice(0, 4), +datetime.slice(4, 6) - 1, +datetime.slice(6, 8), +datetime.slice(9, 11), +datetime.slice(11, 13), +datetime.slice(13, 15))
  if (Math.abs(at - Date.now()) > MAX_SKEW_MS) return `x-amz-date ${datetime} is more than 15 minutes from now`
  const names = signedHeaders.split(';')
  for (const required of REQUIRED_SIGNED) {
    if (!names.includes(required)) return `${required} is not signed`
  }
  const host = req.headers.get('host')
  if (host !== null && host !== new URL(req.url).host) return 'host header differs from the URL'
  const headers: Record<string, string> = {}
  for (const name of names) {
    if (name === 'host') continue
    const value = req.headers.get(name)
    if (value === null) return `signed header ${name} is missing`
    headers[name] = value
  }
  const payloadHash = req.headers.get('x-amz-content-sha256') ?? ''
  const wanted = sigV4Authorization({
    method: req.method,
    url: req.url,
    headers,
    signedHeaders: names,
    payloadHash,
    datetime,
    keyId: expected.keyId,
    secret: expected.secret,
    region: expected.region
  })
  if (wanted !== auth) return 'signature mismatch'
  if (payloadHash === UNSIGNED_PAYLOAD) return null
  if (!/^[0-9a-f]{64}$/.test(payloadHash)) return 'x-amz-content-sha256 is neither UNSIGNED-PAYLOAD nor a SHA-256'
  const body = req.method === 'GET' || req.method === 'HEAD' ? new Uint8Array() : new Uint8Array(await req.clone().arrayBuffer())
  return sha256Hex(body) === payloadHash ? null : 'payload hash differs from the body'
}

/**
 * In-memory stand-in for the B2 S3 API: versioned PUT, GET of a version and DELETE of a version.
 * It verifies the SigV4 signature of every request. Hooks let a test hold a request, fail it, or
 * watch how many run at once.
 */
export class FakeB2 {
  private versions = new Map<string, Version[]>()
  private counter = 0
  calls: B2Call[] = []
  /** Status to answer a request with instead of serving it; `null` serves normally. */
  failWith: ((call: B2Call) => number | null) | null = null
  /** Body and extra headers of a `failWith` reply. A test can plant a secret here to see whether the Worker logs it. */
  failReply: { body: string; headers: Record<string, string> } = { body: 'fail', headers: {} }
  /** Runs before a request is served. A test can hold it here, or throw to drop the connection. */
  hold: ((call: B2Call, signal: AbortSignal) => Promise<void>) | null = null
  inflightGets = 0
  maxInflightGets = 0
  /** One entry for each request whose signature did not verify. The suite fails on any. */
  badSignatures: string[] = []
  /** The secret the fake verifies against. A self-test changes it. */
  secret = B2_APP_KEY

  reset(): void {
    this.versions.clear()
    this.calls = []
    this.failWith = null
    this.failReply = { body: 'fail', headers: {} }
    this.hold = null
    this.inflightGets = 0
    this.maxInflightGets = 0
    this.badSignatures = []
    this.secret = B2_APP_KEY
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

  async handle(req: Request): Promise<Response> {
    const url = new URL(req.url)
    const prefix = `/${B2_BUCKET}/`
    if (!url.pathname.startsWith(prefix)) return new Response(null, { status: 404 })
    const key = url.pathname.slice(prefix.length).split('/').map(decodeURIComponent).join('/')
    const call: B2Call = { method: req.method, key, versionId: url.searchParams.get('versionId') }
    this.calls.push(call)

    const problem = await signatureProblem(req, { keyId: B2_KEY_ID, secret: this.secret, region: B2_REGION })
    if (problem) {
      this.badSignatures.push(`${req.method} ${url.pathname}: ${problem}`)
      return new Response('<Error><Code>SignatureDoesNotMatch</Code></Error>', { status: 403 })
    }

    const isGet = req.method === 'GET'
    if (isGet) this.maxInflightGets = Math.max(this.maxInflightGets, ++this.inflightGets)
    try {
      await this.hold?.(call, req.signal)
      const status = this.failWith?.(call) ?? null
      if (status !== null) return new Response(this.failReply.body, { status, headers: this.failReply.headers })
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
