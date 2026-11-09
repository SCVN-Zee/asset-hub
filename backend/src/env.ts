export interface Env {
  DB: D1Database
  CACHE: R2Bucket
  /** Comma-separated GitHub owners whose repos are served. */
  ALLOWED_OWNERS: string
  UPSTASH_REDIS_REST_URL: string
  UPSTASH_REDIS_REST_TOKEN: string
}
