import { useEffect, useState } from 'react'
import type { SharedIndexStatus } from '@shared/ipc'

/** The shared index's status, kept up to date. */
export function useSharedIndex(): SharedIndexStatus | undefined {
  const [status, setStatus] = useState<SharedIndexStatus>()
  useEffect(() => {
    void window.api.sharedIndex.status().then((r) => r.ok && setStatus(r.data))
    return window.api.sharedIndex.onStatus(setStatus)
  }, [])
  return status
}

/** Goes up each time a shared index brought something in, so lists, thumbnails and numbers are read again. */
export function useSharedImports(): number {
  const [count, setCount] = useState(0)
  useEffect(
    () =>
      window.api.sharedIndex.onStatus((s) => {
        const got = s.last && s.last.commits + s.last.facts + s.last.stats + s.last.thumbs > 0
        if (s.state === 'idle' && s.lastAction === 'imported' && got) setCount((n) => n + 1)
      }),
    []
  )
  return count
}
