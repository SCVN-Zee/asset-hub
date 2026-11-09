import path from 'node:path'
import { defineWorkersConfig, readD1Migrations } from '@cloudflare/vitest-pool-workers/config'

export default defineWorkersConfig(async () => {
  const migrations = await readD1Migrations(path.join(import.meta.dirname, 'migrations'))
  return {
    test: {
      include: ['src/**/*.test.ts'],
      setupFiles: ['./test/apply-migrations.ts'],
      poolOptions: {
        workers: {
          // Isolated storage cannot pop R2 state (it trips over SQLite sidecar files), so the
          // harness in test/helpers.ts clears D1 and R2 before every test instead.
          isolatedStorage: false,
          singleWorker: true,
          // No `main` or wrangler config: the pool must not load the Worker before a test mocks the
          // Redis wrapper. Tests call the exported handler directly.
          miniflare: {
            compatibilityDate: '2026-03-01',
            compatibilityFlags: ['nodejs_compat'],
            d1Databases: ['DB'],
            r2Buckets: ['CACHE'],
            bindings: {
              ALLOWED_OWNERS: 'supercent-io,SCVN-Dan',
              TEST_MIGRATIONS: migrations,
              UPSTASH_REDIS_REST_URL: 'https://redis.invalid',
              UPSTASH_REDIS_REST_TOKEN: 'test-redis-token'
            }
          }
        }
      }
    }
  }
})
