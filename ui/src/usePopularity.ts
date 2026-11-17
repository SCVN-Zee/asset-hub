import { useEffect, useState } from 'react'

const NONE: Record<string, number> = {}

/**
 * The team's score of each blob SHA of `repo`. It is empty first and while the team backend is off
 * or out of reach, so the first render never waits, and the order stays what it was. The scores are
 * asked for again when the backend comes back, and when `retryKey` (the open branch) changes: the
 * app keeps good scores for 10 minutes, so that only costs a request while the backend is failing,
 * which also lets the app notice that it works again.
 */
export function usePopularity(repo: string | undefined, retryKey?: string): Record<string, number> {
  const [found, setFound] = useState<{ repo: string; scores: Record<string, number> }>()
  const [online, setOnline] = useState(0)
  useEffect(() => window.api.team.onStatus((status) => status === 'online' && setOnline((n) => n + 1)), [])
  useEffect(() => {
    if (!repo) return
    let live = true
    window.api.team
      .popular(repo)
      .then((scores) => live && setFound({ repo, scores }))
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [repo, retryKey, online])
  return found && found.repo === repo ? found.scores : NONE
}
