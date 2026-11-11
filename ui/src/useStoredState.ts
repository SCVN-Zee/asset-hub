import { useCallback, useState } from 'react'

/**
 * A yes/no choice kept between launches (the app's localStorage), such as the
 * Filters and chest ticks. Falls back to `initial` when nothing is stored or
 * storage cannot be read; a failed write keeps it for this session.
 */
export function useStoredFlag(key: string, initial: boolean): [boolean, (value: boolean) => void] {
  const [value, setValue] = useState(() => {
    try {
      const stored = localStorage.getItem(key)
      return stored === null ? initial : stored === '1'
    } catch {
      return initial
    }
  })
  const set = useCallback(
    (next: boolean) => {
      setValue(next)
      try {
        localStorage.setItem(key, next ? '1' : '0')
      } catch {
        // kept for this session
      }
    },
    [key]
  )
  return [value, set]
}
