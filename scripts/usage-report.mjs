// Builds usage-report.html from the feedback repo (package.json "feedbackRepo"): who uses the app,
// for how long, what they do, what breaks, and the feedback sent from it (see ui/shared/usage.ts).
// Needs gh signed in with read access to that repo. The page is one file with the data inside:
// it holds GitHub names, so keep it within the team.
//   node scripts/usage-report.mjs [out.html]   real data
//   node scripts/usage-report.mjs --demo        made-up data, to see the page before any comes in
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const root = new URL('..', import.meta.url).pathname
const { feedbackRepo } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
const demo = process.argv.includes('--demo')
const out = resolve(process.argv.slice(2).find((a) => !a.startsWith('--')) ?? (demo ? 'usage-report-demo.html' : 'usage-report.html'))

const gh = (path) => JSON.parse(execFileSync('gh', ['api', path], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 }))
function pages(path) {
  const all = []
  for (let page = 1; ; page++) {
    const list = gh(`${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`)
    all.push(...list)
    if (list.length < 100) return all
  }
}
const parse = (body) => {
  const m = /```json\n([\s\S]*?)\n```/.exec(body ?? '')
  try {
    const s = m && JSON.parse(m[1])
    return s && s.v === 1 ? s : undefined
  } catch {
    return undefined
  }
}

function fetchData() {
  const issues = pages(`repos/${feedbackRepo}/issues?state=all`).filter((i) => !i.pull_request)
  const sessions = []
  const feedback = []
  for (const issue of issues) {
    if (issue.title.startsWith('usage: ')) {
      for (const c of pages(`repos/${feedbackRepo}/issues/${issue.number}/comments`)) {
        const s = parse(c.body)
        // The comment's author is who sent it, whatever the session says.
        if (s) sessions.push({ ...s, login: c.user.login })
      }
    } else {
      const kind = /^\[(Bug|Idea|Feedback)\]/.exec(issue.title)?.[1]?.toLowerCase()
      feedback.push({ kind: kind === 'feedback' ? 'other' : kind ?? 'other', title: issue.title.replace(/^\[\w+\]\s*/, ''), url: issue.html_url, login: issue.user.login, at: issue.created_at, state: issue.state, comments: issue.comments })
    }
  }
  return { sessions, feedback }
}

function demoData() {
  let seed = 7
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647)
  const pick = (list) => list[Math.floor(rnd() * list.length)]
  const people = ['SCVN-Dan', 'minh-art', 'linh-ta', 'kim-jisoo', 'park-dev', 'tuan-vfx', 'hoa-ui']
  const repos = ['supercent-io/hb_creative_xp_hero', 'supercent-io/pa_burger', 'supercent-io/pa_farm', 'supercent-io/common-assets']
  const types = ['fbx', 'prefab', 'texture', 'material', 'vfx', 'animation', 'audio', 'sprite', 'font']
  const errors = ['network', 'notFound', 'rateLimited', 'lfsMissing']
  const sessions = []
  const now = Date.now()
  for (const login of people) {
    const often = 0.25 + rnd() * 0.6
    for (let d = 45; d >= 0; d--) {
      if (rnd() > often || new Date(now - d * 864e5).getDay() % 6 === 0) continue
      const start = new Date(now - d * 864e5 - rnd() * 8 * 3600e3)
      const activeMs = Math.round((5 + rnd() * 110) * 60e3)
      const counts = { 'branch.open': 1 + Math.floor(rnd() * 6), search: Math.floor(rnd() * 12) }
      for (let i = Math.floor(rnd() * 25); i > 0; i--) counts[`view.${pick(types)}`] = (counts[`view.${pick(types)}`] ?? 0) + 1
      if (rnd() < 0.5) (counts.export = 1 + Math.floor(rnd() * 3)), (counts['export.assets'] = counts.export * (1 + Math.floor(rnd() * 6)))
      if (rnd() < 0.3) (counts.unity = 1), (counts['unity.assets'] = 1 + Math.floor(rnd() * 4))
      if (rnd() < 0.2) counts['ai.chat'] = 1 + Math.floor(rnd() * 5)
      if (rnd() < 0.25) counts[`error.${pick(errors)}`] = 1 + Math.floor(rnd() * 3)
      sessions.push({ v: 1, id: `${login}-${d}`, login, version: d > 20 ? '0.1.2' : '0.1.3', os: 'macOS 15.3', language: pick(['en', 'vi', 'ko']), start: start.toISOString(), end: new Date(+start + activeMs * 1.3).toISOString(), activeMs, repos: [...new Set([pick(repos), pick(repos)])], counts })
    }
  }
  const feedback = [
    ['bug', 'Prefab preview turns black after switching branch', 'minh-art', 2, 'open'],
    ['idea', 'Filter by polygon count on the grid', 'linh-ta', 5, 'open'],
    ['bug', 'Import into Unity does nothing when Unity is minimised', 'park-dev', 9, 'closed'],
    ['other', 'Love the chest animation 🙂', 'hoa-ui', 12, 'closed'],
    ['idea', 'Export several branches into one package', 'tuan-vfx', 20, 'open']
  ].map(([kind, title, login, d, state], i) => ({ kind, title, login, state, url: `https://github.com/${feedbackRepo}/issues/${i + 1}`, at: new Date(now - d * 864e5).toISOString(), comments: i % 3 }))
  return { sessions, feedback }
}

const data = { repo: feedbackRepo, generated: new Date().toISOString(), demo, ...(demo ? demoData() : fetchData()) }
const html = readFileSync(join(root, 'scripts/usage-report.html'), 'utf8').replace('/*DATA*/null', JSON.stringify(data).replace(/</g, '\\u003c'))
writeFileSync(out, html)
console.log(`${data.sessions.length} sessions, ${data.feedback.length} feedback → ${out}`)
