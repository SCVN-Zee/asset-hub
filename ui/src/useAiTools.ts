import { useEffect, useRef } from 'react'
import type { AiToolName, AiToolResult } from '@shared/aiTools'

export type AiToolHandlers = Record<AiToolName, (args: Record<string, unknown>) => Promise<unknown>>

/** Answers the AI pet's tool calls with these handlers (the latest ones on each call). */
export function useAiTools(handlers: AiToolHandlers): void {
  const latest = useRef(handlers)
  latest.current = handlers
  useEffect(
    () =>
      window.api.ai.onToolCall(async ({ callId, tool, args }) => {
        let result: AiToolResult
        try {
          const handler = latest.current[tool as AiToolName]
          result = handler ? { ok: true, data: await handler(args && typeof args === 'object' ? (args as Record<string, unknown>) : {}) } : { ok: false, error: `Unknown tool ${tool}` }
        } catch (err) {
          result = { ok: false, error: err instanceof Error ? err.message : String(err) }
        }
        void window.api.ai.toolResult(callId, result)
      }),
    []
  )
}

/** Resolves once `ready()` holds (checked every 100 ms), or with false after `ms`. */
export function waitFor(ready: () => boolean, ms = 12_000): Promise<boolean> {
  const start = Date.now()
  return new Promise((resolve) => {
    const tick = () => (ready() ? resolve(true) : Date.now() - start > ms ? resolve(false) : window.setTimeout(tick, 100))
    tick()
  })
}
