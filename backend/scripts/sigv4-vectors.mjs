// Prints the SigV4 signatures that `aws4fetch` (the production signer) gives for the three B2 request
// shapes at a fixed time. The Rust signer in `src/sigv4.rs` must reproduce them byte for byte.
// Run: node scripts/sigv4-vectors.mjs
import { AwsV4Signer } from 'aws4fetch'

const creds = { accessKeyId: 'AKIDEXAMPLE', secretAccessKey: 'wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY', service: 's3', region: 'us-west-004' }
const datetime = '20260101T000000Z'
const base = 'https://s3.us-west-004.backblazeb2.com/asset-hub-cache'
const key = 'repos/supercent-io/game/s1/catalog/0b9a4f1e-1111-4222-8333-444455556666.json.gz'

// A key with every character that `encodeURIComponent` keeps but SigV4 encodes, plus a space and a non-ASCII letter.
const special = 'repos/o/g/s1/a b!\'()*~\u00e9/x.json.gz'
const specialUrl = `${base}/${special.split('/').map(encodeURIComponent).join('/')}`

const cases = [
  { name: 'put-special', method: 'PUT', url: specialUrl, headers: {}, body: new Uint8Array([1]) },
  { name: 'get', method: 'GET', url: `${base}/${key}`, headers: {} },
  { name: 'get-version', method: 'GET', url: `${base}/${key}?versionId=4_zabc_d2026_m01`, headers: {} },
  { name: 'put', method: 'PUT', url: `${base}/${key}`, headers: { 'content-type': 'application/gzip' }, body: new Uint8Array([0x1f, 0x8b, 8]) },
  { name: 'delete-version', method: 'DELETE', url: `${base}/${key}?versionId=4_zabc_d2026_m01`, headers: {} }
]

for (const c of cases) {
  const signer = new AwsV4Signer({ ...creds, method: c.method, url: c.url, headers: c.headers, body: c.body, datetime })
  const { headers } = await signer.sign()
  console.log(JSON.stringify({ name: c.name, method: c.method, url: c.url, contentType: c.headers['content-type'] ?? null, authorization: headers.get('authorization'), contentSha: headers.get('x-amz-content-sha256'), date: headers.get('x-amz-date'), all: [...headers.keys()] }))
}
