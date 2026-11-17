import { createHash } from 'node:crypto'
import { AwsClient } from 'aws4fetch'
import { describe, expect, it } from 'vitest'
import { B2_APP_KEY, B2_BUCKET, B2_ENDPOINT, B2_KEY_ID, B2_REGION, FakeB2, sigV4Authorization } from './fake-b2'

const url = `${B2_ENDPOINT}/${B2_BUCKET}/repos/o/r/s1/catalog/one.json.gz`
const signer = (secretAccessKey: string) =>
  new AwsClient({ accessKeyId: B2_KEY_ID, secretAccessKey, service: 's3', region: B2_REGION, retries: 0 })

describe('fake B2 signature check', () => {
  it('accepts a request signed with the right secret', async () => {
    const b2 = new FakeB2()
    const put = await b2.handle(await signer(B2_APP_KEY).sign(url, { method: 'PUT', body: new Uint8Array([1, 2, 3]) }))
    expect(put.status).toBe(200)
    expect(b2.badSignatures).toEqual([])
    expect(b2.keys()).toEqual(['repos/o/r/s1/catalog/one.json.gz'])
  })

  it('rejects a request signed with a wrong secret', async () => {
    const b2 = new FakeB2()
    const res = await b2.handle(await signer('wrong-secret').sign(url, { method: 'PUT', body: new Uint8Array([1, 2, 3]) }))
    expect(res.status).toBe(403)
    expect(b2.badSignatures).toEqual([expect.stringContaining('signature mismatch')])
    expect(b2.keys()).toEqual([])
  })

  it('rejects a request whose URL changed after signing', async () => {
    const b2 = new FakeB2()
    const signed = await signer(B2_APP_KEY).sign(url, { method: 'GET' })
    const moved = new Request(`${url}?versionId=v9`, { method: 'GET', headers: signed.headers })
    expect((await b2.handle(moved)).status).toBe(403)
    expect(b2.badSignatures).toHaveLength(1)
  })

  it('rejects an unsigned request and a request for another key id', async () => {
    const b2 = new FakeB2()
    expect((await b2.handle(new Request(url))).status).toBe(403)
    const other = new AwsClient({ accessKeyId: 'other-key', secretAccessKey: B2_APP_KEY, service: 's3', region: B2_REGION, retries: 0 })
    expect((await b2.handle(await other.sign(url, { method: 'GET' }))).status).toBe(403)
    expect(b2.badSignatures).toEqual([expect.stringContaining('no SigV4'), expect.stringContaining('wrong access key id')])
  })

  const hex = (bytes: Uint8Array): string => createHash('sha256').update(bytes).digest('hex')
  const stamp = (at: Date): string => at.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '')

  /** A PUT signed by the independent signer, with the headers and clock the test picks. */
  function signed(opts: { body: Uint8Array; payloadHash?: string; at?: Date; extra?: Record<string, string>; method?: string }): Request {
    const method = opts.method ?? 'PUT'
    const datetime = stamp(opts.at ?? new Date())
    const payloadHash = opts.payloadHash ?? hex(opts.body)
    const headers: Record<string, string> = { 'x-amz-date': datetime, 'x-amz-content-sha256': payloadHash, ...opts.extra }
    const authorization = sigV4Authorization({
      method,
      url,
      headers,
      signedHeaders: ['host', ...Object.keys(headers)],
      payloadHash,
      datetime,
      keyId: B2_KEY_ID,
      secret: B2_APP_KEY,
      region: B2_REGION
    })
    return new Request(url, { method, headers: { ...headers, authorization }, body: method === 'GET' ? undefined : opts.body })
  }

  it('accepts a signature over the real body hash and over UNSIGNED-PAYLOAD', async () => {
    const b2 = new FakeB2()
    expect((await b2.handle(signed({ body: new Uint8Array([1, 2, 3]) }))).status).toBe(200)
    expect((await b2.handle(signed({ body: new Uint8Array([4]), payloadHash: 'UNSIGNED-PAYLOAD' }))).status).toBe(200)
    expect(b2.badSignatures).toEqual([])
  })

  it('rejects a payload hash that is not the hash of the body', async () => {
    const b2 = new FakeB2()
    const wrong = hex(new Uint8Array([9]))
    expect((await b2.handle(signed({ body: new Uint8Array([1, 2, 3]), payloadHash: wrong }))).status).toBe(403)
    expect((await b2.handle(signed({ body: new Uint8Array([1, 2, 3]), payloadHash: 'deadbeef' }))).status).toBe(403)
    // The same through the client the Worker uses: it signs whatever hash header it is given.
    const client = await signer(B2_APP_KEY).sign(url, { method: 'PUT', body: new Uint8Array([1, 2, 3]), headers: { 'x-amz-content-sha256': wrong } })
    expect((await b2.handle(client)).status).toBe(403)
    expect(b2.badSignatures).toEqual([
      expect.stringContaining('payload hash differs'),
      expect.stringContaining('neither UNSIGNED-PAYLOAD nor a SHA-256'),
      expect.stringContaining('payload hash differs')
    ])
    expect(b2.keys()).toEqual([])
  })

  it('rejects a date more than 15 minutes from now, in the past or the future', async () => {
    const b2 = new FakeB2()
    const body = new Uint8Array([1])
    expect((await b2.handle(signed({ body, at: new Date(Date.now() - 20 * 60_000) }))).status).toBe(403)
    expect((await b2.handle(signed({ body, at: new Date(Date.now() + 20 * 60_000) }))).status).toBe(403)
    expect((await b2.handle(signed({ body, at: new Date(0) }))).status).toBe(403)
    expect(b2.badSignatures).toHaveLength(3)
    expect(b2.badSignatures.every((p) => p.includes('15 minutes'))).toBe(true)
    expect((await b2.handle(signed({ body, at: new Date(Date.now() - 5 * 60_000) }))).status).toBe(200)
  })

  it('accepts a signature that also signs content-type and content-length', async () => {
    const b2 = new FakeB2()
    const body = new Uint8Array([1, 2, 3])
    const res = await b2.handle(signed({ body, extra: { 'content-type': 'application/octet-stream', 'content-length': '3' } }))
    expect(res.status).toBe(200)
    expect(b2.badSignatures).toEqual([])
  })

  it('rejects a signature that leaves out a required header', async () => {
    const b2 = new FakeB2()
    const datetime = stamp(new Date())
    const headers = { 'x-amz-date': datetime, 'x-amz-content-sha256': 'UNSIGNED-PAYLOAD' }
    const authorization = sigV4Authorization({
      method: 'GET', url, headers, signedHeaders: ['host', 'x-amz-date'], payloadHash: 'UNSIGNED-PAYLOAD', datetime, keyId: B2_KEY_ID, secret: B2_APP_KEY, region: B2_REGION
    })
    expect((await b2.handle(new Request(url, { headers: { ...headers, authorization } }))).status).toBe(403)
    expect(b2.badSignatures).toEqual([expect.stringContaining('x-amz-content-sha256 is not signed')])
  })
})
