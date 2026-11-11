/**
 * Usage notes and feedback, sent to a private GitHub repo (`feedbackRepo` in
 * package.json). Each person has one issue, `usage: <login>`, with a comment per
 * app session; feedback is an issue of its own. Only counts and names of things
 * are kept (repo names, asset types, error codes), never search text or files.
 */

export const FEEDBACK_KINDS = ['bug', 'idea', 'other'] as const
export type FeedbackKind = (typeof FEEDBACK_KINDS)[number]

export interface FeedbackReport {
  kind: FeedbackKind
  title: string
  body: string
  /** Where the user was, when they chose to include it. */
  context?: { repo?: string; branch?: string; asset?: string }
}

/** Sent: the issue's link. Not sent (offline, no access): kept and retried on a later launch. */
export type FeedbackResult = { sent: true; url: string } | { sent: false }

/** What the renderer reports itself; the rest is seen in the main process. */
export type UsageEvent = { kind: 'view'; type: string } | { kind: 'search' } | { kind: 'feedback' }

export interface UsageSession {
  v: 1
  id: string
  login?: string
  version: string
  os: string
  language: string
  /** ISO times. */
  start: string
  end: string
  /** Time the window was focused and the user not away, in ms. */
  activeMs: number
  /** Repos whose branches were opened. */
  repos: string[]
  /** e.g. `branch.open`, `view.fbx`, `search`, `export`, `export.assets`, `unity`, `ai.chat`, `error.notFound`. */
  counts: Record<string, number>
}

export const USAGE_TITLE_PREFIX = 'usage: '

export function newSession(id: string, now: Date, info: Pick<UsageSession, 'version' | 'os' | 'language'>): UsageSession {
  return { v: 1, id, ...info, start: now.toISOString(), end: now.toISOString(), activeMs: 0, repos: [], counts: {} }
}

export function bump(session: UsageSession, key: string, by = 1): void {
  session.counts[key] = (session.counts[key] ?? 0) + by
}

export function noteEvent(session: UsageSession, event: UsageEvent): void {
  if (event.kind === 'view') bump(session, `view.${String(event.type).replace(/[^\w-]/g, '').slice(0, 20) || 'other'}`)
  else if (event.kind === 'search' || event.kind === 'feedback') bump(session, event.kind)
}

/** Counts what an IPC request did, from its channel, arguments and result. */
export function noteIpc(session: UsageSession, channel: string, args: unknown[], result: unknown): void {
  const r = result as { ok?: boolean; data?: { canceled?: boolean; assets?: number }; error?: { code?: unknown } } | undefined
  if (r && r.ok === false) {
    // Errors from background work (thumbnails, scans) count too: they show what breaks.
    const code = typeof r.error?.code === 'string' ? r.error.code : 'unknown'
    bump(session, `error.${code}`)
    if (channel === 'export:unitypackage') bump(session, 'export.fail')
    if (channel === 'export:to-unity') bump(session, 'unity.fail')
    return
  }
  switch (channel) {
    case 'source:list-tree': {
      bump(session, 'branch.open')
      const repo = args[0]
      if (typeof repo === 'string' && repo && !session.repos.includes(repo)) session.repos.push(repo)
      break
    }
    case 'export:unitypackage':
    case 'export:to-unity': {
      if (!r?.ok || r.data?.canceled) break
      const key = channel === 'export:unitypackage' ? 'export' : 'unity'
      bump(session, key)
      bump(session, `${key}.assets`, r.data?.assets ?? 0)
      break
    }
    case 'ai:chat-send':
      bump(session, 'ai.chat')
      break
  }
}

/** A session worth sending: some use, not just a launch that went straight to quit. */
export function isWorthSending(session: UsageSession): boolean {
  return !!session.login && (session.activeMs >= 60_000 || Object.keys(session.counts).length > 0)
}

export function usageComment(session: UsageSession): string {
  return '```json\n' + JSON.stringify(session) + '\n```'
}

/** Reads a session back out of a comment (the report script does the same). */
export function parseUsageComment(body: string): UsageSession | undefined {
  const m = /```json\n([\s\S]*?)\n```/.exec(body)
  if (!m) return undefined
  try {
    const s = JSON.parse(m[1]) as UsageSession
    return s && s.v === 1 && typeof s.start === 'string' ? s : undefined
  } catch {
    return undefined
  }
}

const KIND_LABEL: Record<FeedbackKind, string> = { bug: 'Bug', idea: 'Idea', other: 'Feedback' }

export function feedbackIssue(report: FeedbackReport, from: { login: string; version: string; os: string; language: string }): { title: string; body: string } {
  const where = report.context && [report.context.repo, report.context.branch && `@ ${report.context.branch}`, report.context.asset && `· \`${report.context.asset}\``].filter(Boolean).join(' ')
  const title = `[${KIND_LABEL[report.kind]}] ${report.title.trim().replace(/\s+/g, ' ').slice(0, 120)}`
  const body = [
    `**From:** @${from.login} · v${from.version} · ${from.os} · ${from.language}`,
    ...(where ? [`**Where:** ${where}`] : []),
    '',
    report.body.trim().slice(0, 20_000)
  ].join('\n')
  return { title, body }
}

/** Checks untrusted IPC input. */
export function sanitizeFeedback(raw: unknown): FeedbackReport | undefined {
  const r = raw as Record<string, unknown> | undefined
  if (!r || !(FEEDBACK_KINDS as readonly unknown[]).includes(r.kind) || typeof r.title !== 'string' || !r.title.trim() || typeof r.body !== 'string') return undefined
  const c = r.context as Record<string, unknown> | undefined
  const str = (v: unknown) => (typeof v === 'string' && v ? v.slice(0, 300) : undefined)
  return {
    kind: r.kind as FeedbackKind,
    title: r.title,
    body: r.body,
    ...(c && typeof c === 'object' ? { context: { repo: str(c.repo), branch: str(c.branch), asset: str(c.asset) } } : {})
  }
}
