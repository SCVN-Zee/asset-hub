import { AwsClient } from 'aws4fetch'
import type { Env } from './env'

/** B2 answered with an error status, so the request is known not to have changed the bucket. */
/** The S3 `<Code>` of a B2 error reply, plus its message when it is plain words (never a key). */
async function errorCode(res: Response): Promise<string> {
  const text = await res.text().catch(() => '')
  const code = /<Code>([A-Za-z]{1,64})<\/Code>/.exec(text)?.[1] ?? 'unknown'
  // A cap reply says so only in its message ("Transaction cap exceeded"); keep at most 160 characters.
  const message = /<Message>([^<]{1,160})<\/Message>/.exec(text)?.[1]
  return message ? `${code} (${message})` : code
}

export class B2HttpError extends Error {
  constructor(
    readonly status: number,
    operation: string
  ) {
    super(`b2 ${operation} ${status}`)
    this.name = 'B2HttpError'
  }
}

/**
 * Minimal signed client for one Backblaze B2 bucket over its S3 API. It never lists the bucket (D1
 * holds every index) and never retries: a hidden retry would multiply requests and mask failures.
 */
export class B2Client {
  private readonly aws: AwsClient
  private readonly base: string

  constructor(env: Pick<Env, 'B2_ENDPOINT' | 'B2_REGION' | 'B2_BUCKET' | 'B2_KEY_ID' | 'B2_APP_KEY'>) {
    this.aws = new AwsClient({
      accessKeyId: env.B2_KEY_ID,
      secretAccessKey: env.B2_APP_KEY,
      service: 's3',
      region: env.B2_REGION,
      retries: 0
    })
    this.base = `${env.B2_ENDPOINT.replace(/\/+$/, '')}/${env.B2_BUCKET}`
  }

  private url(key: string, query = ''): string {
    return `${this.base}/${key.split('/').map(encodeURIComponent).join('/')}${query}`
  }

  /**
   * One version of `key` (the latest when `versionId` is omitted), or `null` when B2 has none.
   * The caller must consume or cancel the body.
   */
  async get(key: string, signal?: AbortSignal, versionId?: string): Promise<Response | null> {
    const query = versionId ? `?versionId=${encodeURIComponent(versionId)}` : ''
    const res = await this.aws.fetch(this.url(key, query), { method: 'GET', signal })
    if (res.status === 404) {
      await res.body?.cancel().catch(() => {})
      return null
    }
    if (!res.ok) throw new B2HttpError(res.status, `get ${await errorCode(res)}`)
    return res
  }

  /** Stores `body` under `key` and returns the version id B2 assigned. */
  async put(key: string, body: Uint8Array, contentType: string): Promise<string> {
    // No payload hash: for service s3 aws4fetch signs `UNSIGNED-PAYLOAD`, and TLS protects the
    // body. Hashing a 20 MB body in the isolate would cost about 10 ms of CPU, the whole budget of
    // a Workers Free request.
    const res = await this.aws.fetch(this.url(key), { method: 'PUT', body, headers: { 'content-type': contentType } })
    await res.body?.cancel().catch(() => {})
    if (!res.ok) throw new B2HttpError(res.status, 'put')
    const versionId = res.headers.get('x-amz-version-id')
    // Without the id the version could never be deleted, so its bytes could never be freed.
    if (!versionId) throw new Error('b2 put returned no version id')
    return versionId
  }

  /**
   * Deletes one version. A delete without `versionId` only adds a hide marker and frees no
   * storage, so the id is required. Returns `false` when the version was already gone: this call
   * freed nothing, so the caller must not release bytes for it.
   */
  async delete(key: string, versionId: string): Promise<boolean> {
    if (!versionId) throw new Error('b2 delete needs a version id')
    const res = await this.aws.fetch(this.url(key, `?versionId=${encodeURIComponent(versionId)}`), { method: 'DELETE' })
    await res.body?.cancel().catch(() => {})
    if (res.status === 404) return false
    if (!res.ok) throw new B2HttpError(res.status, 'delete')
    return true
  }
}
