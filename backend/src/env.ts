export interface Env {
  DB: D1Database
  /** Comma-separated GitHub owners whose repos are served. */
  ALLOWED_OWNERS: string
  UPSTASH_REDIS_REST_URL: string
  UPSTASH_REDIS_REST_TOKEN: string
  /** S3 endpoint of the Backblaze B2 bucket, for example `https://s3.us-west-004.backblazeb2.com`. */
  B2_ENDPOINT: string
  B2_REGION: string
  B2_BUCKET: string
  /** Secrets of an application key limited to the one bucket. */
  B2_KEY_ID: string
  B2_APP_KEY: string
}
