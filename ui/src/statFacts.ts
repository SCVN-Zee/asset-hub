import type { StatsSummary } from '@shared/types'

/**
 * What the screen takes from all measurements together: which models hold clips, which are effects, which are
 * left out of lists. Each `*Key` changes only when its own part changes, so what depends on it is rebuilt then
 * and not on every thumbnail. Kept up to date from the entries that merge, never by scanning every measurement.
 */
export interface StatFacts {
  /** Clip names of the models that hold clips. */
  clipKey: number
  /** The models measured as not rigged (clips no model was found for). */
  unlistedKey: number
  /** Which models hold clips. */
  withClips: number
  /** Which prefabs are effects (particles read), and how many emitters they have. */
  withParticles: number
  /** How many models are measured as not rigged: left out of lists unless shown anyway. */
  unlisted: number
}

export const NO_STAT_FACTS: StatFacts = { clipKey: 0, unlistedKey: 0, withClips: 0, withParticles: 0, unlisted: 0 }

const clipsOf = (s?: StatsSummary) => (s && s.clips > 0 && s.clipNames ? s.clipNames.join(',') : undefined)
const particlesOf = (s?: StatsSummary) => (s && s.particles !== undefined ? s.emitters ?? 0 : undefined)

/**
 * The facts after `after` takes the place of `before` (undefined: the model was not measured). Returns `facts`
 * itself when nothing they hold changed. `clipNames` (clip names by SHA) is updated in place to match.
 */
export function mergeStatFact(facts: StatFacts, clipNames: Map<string, string[]>, sha: string, before: StatsSummary | undefined, after: StatsSummary): StatFacts {
  if (after.clipNames) clipNames.set(sha, after.clipNames)
  else clipNames.delete(sha)
  const wasUnlisted = before?.rigged === false
  const isUnlisted = after.rigged === false
  const hadClips = !!before && before.clips > 0
  const hasClips = after.clips > 0
  const clipKey = clipsOf(before) !== clipsOf(after)
  const withClips = hadClips !== hasClips
  const withParticles = particlesOf(before) !== particlesOf(after)
  const unlistedKey = wasUnlisted !== isUnlisted
  if (!clipKey && !withClips && !withParticles && !unlistedKey) return facts
  return {
    clipKey: facts.clipKey + (clipKey ? 1 : 0),
    withClips: facts.withClips + (withClips ? 1 : 0),
    withParticles: facts.withParticles + (withParticles ? 1 : 0),
    unlistedKey: facts.unlistedKey + (unlistedKey ? 1 : 0),
    unlisted: facts.unlisted + (isUnlisted ? 1 : 0) - (wasUnlisted ? 1 : 0)
  }
}
