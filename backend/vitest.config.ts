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
          // The harness in test/helpers.ts clears D1 and the fake B2 before every test instead.
          isolatedStorage: false,
          singleWorker: true,
          // No `main` or wrangler config: the pool must not load the Worker before a test mocks the
          // Redis wrapper. Tests call the exported handler directly.
          miniflare: {
            compatibilityDate: '2026-03-01',
            compatibilityFlags: ['nodejs_compat'],
            d1Databases: ['DB'],
            bindings: {
              ALLOWED_OWNERS: 'supercent-io,SCVN-Dan',
              TEST_MIGRATIONS: migrations,
              UPSTASH_REDIS_REST_URL: 'https://redis.invalid',
              UPSTASH_REDIS_REST_TOKEN: 'test-redis-token',
              B2_ENDPOINT: 'https://b2.fake.invalid',
              B2_REGION: 'us-west-000',
              B2_BUCKET: 'test-bucket',
              B2_KEY_ID: 'test-key-id',
              B2_APP_KEY: 'test-app-key'
            }
          }
        }
      }
    }
  }
})
