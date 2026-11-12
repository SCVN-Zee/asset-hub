#!/usr/bin/env node
// Headless smoke test of the Luna preview host. It builds the app into a temporary folder, serves it, opens a harness
// page in headless Chrome that embeds `luna-host.html` in an iframe, sends a particle effect with a texture and checks
// that the runtime draws it: draw calls, particles, non-background pixels, animation, commands, isolation and CSP.
// It then draws one emitter per ported shader, one after another, checks that each draws and that additive and alpha
// blending differ, checks the fallback shader and its `unsupported` event, and times a 1024x1024 texture.
// Usage: bun scripts/luna-smoke.mjs [--no-build] [--dist <dir>] [--screenshot <png>] [--sheet <png>] [--shots <dir>] [--linear] [--only <case ids>] [--eval <js>]
// Needs Bun, because it runs the TypeScript bridge on real prefabs. The prefab part needs References/LunaParticlePreview and cargo, and is skipped without the samples.
// Env: CHROME_PATH (default: the first Chrome or Chromium found in the usual places), LUNA_SMOKE_NO_SAMPLES=1 (skip the prefab part as if References/ were absent). Ports are fixed: 47320 (server) and 47321 (Chrome DevTools).
// Exits non-zero on failure. Chrome, the server and the temporary folders are always removed.
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { extname, join, normalize, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const SERVER_PORT = 47320
const DEBUG_PORT = 47321
const CHANNEL = 'asset-hub-luna'
/** Background of the host camera, as 0..255. Keep in step with ui/src/luna/host/effect.ts. */
const BACKGROUND = [26, 26, 51]

const root = resolve(fileURLToPath(import.meta.url), '../..')
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
/** Chrome or Chromium: CHROME_PATH, else the first of the usual install places that exists (GitHub's macOS and Linux runners have Chrome). */
const CHROME_CANDIDATES = [
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium',
  '/usr/bin/google-chrome',
  '/usr/bin/google-chrome-stable',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser'
]
const chromePath = process.env.CHROME_PATH ?? CHROME_CANDIDATES.find((p) => existsSync(p)) ?? CHROME_CANDIDATES[0]
const screenshotPath = resolve(option('--screenshot') ?? join(tmpdir(), 'asset-hub-luna-smoke.png'))
const sheetPath = resolve(option('--sheet') ?? join(tmpdir(), 'asset-hub-luna-shaders.png'))

const cleanups = []
const cleanup = () => {
  while (cleanups.length > 0) {
    try {
      cleanups.pop()()
    } catch {
      /* best effort */
    }
  }
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => (cleanup(), process.exit(130)))

const failures = []
const check = (ok, label, detail = '') => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${detail ? '  ' + detail : ''}`)
  if (!ok) failures.push(label)
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// ---- build and serve -------------------------------------------------------------------------------------------
function buildDist() {
  const dir = mkdtempSync(join(tmpdir(), 'asset-hub-luna-dist-'))
  cleanups.push(() => rmSync(dir, { recursive: true, force: true }))
  console.log(`Building the app into ${dir} ...`)
  const result = spawnSync('bun', ['run', 'vite', 'build', '--outDir', dir, '--emptyOutDir'], { cwd: root, encoding: 'utf8' })
  if (result.status !== 0) throw new Error(`vite build failed:\n${result.stdout}\n${result.stderr}`)
  return dir
}

const HARNESS = `<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;height:100%;overflow:hidden;background:#000}iframe{border:0;width:100vw;height:100vh;display:block}</style></head>
<body><iframe id="f"></iframe><script>
const CHANNEL = ${JSON.stringify(CHANNEL)}
const frame = document.getElementById('f')
window.LINEAR = location.search.includes('space=linear')
frame.src = '/luna-host.html' + (window.LINEAR ? '?space=linear' : '')
window.events = []
addEventListener('message', (e) => { if (e.source === frame.contentWindow && e.data && e.data.channel === CHANNEL) window.events.push(e.data.message) })
window.send = (m) => frame.contentWindow.postMessage({ channel: CHANNEL, message: m }, location.origin)
window.bitmaps = {}
window.bitmapOf = async (n) => {
  if (window.bitmaps[n]) return window.bitmaps[n]
  const c = new OffscreenCanvas(n, n), x = c.getContext('2d')
  const g = x.createRadialGradient(n / 2, n / 2, 0, n / 2, n / 2, n / 2)
  g.addColorStop(0, 'rgba(255,255,255,1)'); g.addColorStop(1, 'rgba(255,255,255,0)')
  x.fillStyle = g; x.fillRect(0, 0, n, n)
  if (n >= 1024) { // noise on top, so the upload is not a trivially compressible image
    const d = x.getImageData(0, 0, n, n)
    for (let i = 0; i < d.data.length; i += 4) { const k = 0.8 + 0.2 * Math.random(); d.data[i] *= k; d.data[i + 1] *= k; d.data[i + 2] *= k }
    x.putImageData(d, 0, 0)
  }
  return (window.bitmaps[n] = await createImageBitmap(c))
}
// A texture is { bitmap } in the protocol. The Node side cannot make a bitmap, so it sends { png: <base64> } and the page decodes it.
window.decodeTextures = async (material) => {
  for (const [slot, t] of Object.entries(material.textures)) {
    if (t && t.png) {
      const blob = await (await fetch('data:image/png;base64,' + t.png)).blob()
      const { png, ...sampler } = t
      material.textures[slot] = { bitmap: await createImageBitmap(blob), ...sampler }
    }
  }
}
window.loadEffect = async (effect) => {
  for (const m of effect.materials) await window.decodeTextures(m)
  window.send({ type: 'load', effect: { linear: window.LINEAR, ...effect } })
}
const curve = (v) => ({ mode: 0, scalar: v, min: v })
window.curve = curve
// One cone emitter that fires up, with every module at its neutral value. \`over\` replaces fields of the emitter.
window.coneEmitter = (over = {}) => ({
  name: 'Emitter', node: 0, material: 0, unsupported: [],
  main: { duration: 5, loop: true, prewarm: false, startDelay: curve(0), startLifetime: curve(2), startSpeed: curve(2.5), startSize: curve(0.8), startRotation: curve(0),
    startColor: { mode: 0, max: [1, 0.6, 0.2, 0.8], min: [1, 0.6, 0.2, 0.8] }, gravity: curve(0), simulationSpace: 0, simulationSpeed: 1, useUnscaledTime: false,
    scalingMode: 1, maxParticles: 1000, emitterVelocityMode: 0, playOnAwake: true },
  emission: { rateOverTime: curve(60), rateOverDistance: curve(0), bursts: [] },
  shape: { type: 4, angle: 25, radius: 0.5, radiusThickness: 1, radiusMode: 0, radiusSpread: 0, length: 5, arc: 360, arcMode: 0, arcSpread: 0, arcSpeed: curve(1),
    donutRadius: 0.2, position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1], randomDirection: 0, sphericalDirection: 0, randomPosition: 0, alignToDirection: false },
  renderer: { renderMode: 0, alignment: 0, sortMode: 0, sortingOrder: 0, lengthScale: 2, velocityScale: 0, minParticleSize: 0, maxParticleSize: 0.5, applyActiveColorSpace: true },
  ...over
})
// Unity cones fire along local +Z, so the node turns -90 degrees around X to fire up.
window.upNode = { name: 'Emitter', parent: -1, position: [0, 0, 0], rotation: [-0.7071068, 0, 0, 0.7071068], scale: [1, 1, 1], active: true }
// A case is { material, startColor?, size?, sampler? }. A texture slot with the value 'MAIN' gets the test texture.
window.loadCase = async (c) => {
  const bitmap = await window.bitmapOf(c.size || 64)
  const sampler = { wrapU: 'clamp', wrapV: 'clamp', filter: 'bilinear', srgb: false, mipmaps: true, ...(c.sampler || {}) }
  const material = JSON.parse(JSON.stringify(c.material))
  for (const k of Object.keys(material.textures)) if (material.textures[k] === 'MAIN') material.textures[k] = { bitmap, ...sampler }
  const color = c.startColor || [1, 0.6, 0.2, 0.8]
  const emitter = window.coneEmitter()
  emitter.main.startColor = { mode: 0, max: color, min: color }
  window.send({ type: 'load', effect: { linear: window.LINEAR, materials: [material], nodes: [window.upNode], emitters: [emitter] } })
}
window.timedLoad = async (c) => {
  await window.bitmapOf(c.size || 64)
  window.events.length = 0
  const t0 = performance.now()
  await window.loadCase(c)
  const loaded = await window.waitFor(() => window.events.find((e) => e.type === 'loaded' || e.type === 'error'), 15000)
  const t1 = performance.now()
  const frame = await window.waitFor(() => window.events.find((e) => e.type === 'time' && e.particles > 0 && e.drawCalls > 0), 15000)
  return { loaded, toLoadedMs: t1 - t0, toFrameMs: performance.now() - t0, frame: Boolean(frame) }
}
window.lastTime = () => { for (let i = window.events.length - 1; i >= 0; i--) if (window.events[i].type === 'time') return window.events[i]; return null }
window.waitFor = async (pred, ms) => { const end = Date.now() + ms; while (Date.now() < end) { const v = pred(); if (v) return v; await new Promise((r) => setTimeout(r, 50)) } return null }
// Control for the CSP check: an inline script in the host page is blocked by its policy and must be logged.
window.injectInlineScript = () => { const d = frame.contentDocument; const s = d.createElement('script'); s.textContent = 'window.__inline = 1'; d.head.appendChild(s) }
window.hostGlobals = () => { try { return typeof frame.contentWindow.pc } catch (e) { return 'blocked' } }
// Tiles of 400x300 with a label, four to a row.
window.sheet = async (tiles) => {
  const w = 400, h = 300, cols = 4, rows = Math.ceil(tiles.length / cols)
  const c = new OffscreenCanvas(w * cols, h * rows), x = c.getContext('2d')
  x.fillStyle = '#000'; x.fillRect(0, 0, c.width, c.height)
  x.font = '16px sans-serif'
  for (const [i, t] of tiles.entries()) {
    const bmp = await createImageBitmap(await (await fetch(t.shot)).blob())
    const px = (i % cols) * w, py = Math.floor(i / cols) * h
    x.drawImage(bmp, px, py, w, h)
    x.fillStyle = '#fff'; x.fillText(t.label, px + 8, py + 20)
  }
  const blob = await c.convertToBlob({ type: 'image/png' })
  return await new Promise((r) => { const f = new FileReader(); f.onload = () => r(f.result); f.readAsDataURL(blob) })
}
// Reads the Luna objects back, so a test sees what the host applied and not only what it drew.
window.systems = () => { const UE = frame.contentWindow.UnityEngine; return Array.from(UE.Object.FindObjectsOfType(UE.ParticleSystem)) }
window.readHierarchy = () => window.systems().map((ps) => {
  const t = ps.transform, p = t.parent
  return { name: ps.gameObject.name, parent: p ? p.gameObject.name : null, active: ps.gameObject.activeInHierarchy, particles: ps.particleCount, playing: ps.isPlaying,
    position: [t.position.x, t.position.y, t.position.z].map((v) => +v.toFixed(3)), localScale: [t.localScale.x, t.localScale.y, t.localScale.z].map((v) => +v.toFixed(3)),
    maxParticles: ps.main.maxParticles, shapeOn: ps.shape.enabled, shapeType: ps.shape.shapeType }
})
window.readFull = () => {
  const ps = window.systems()[0], m = ps.main, e = ps.emission, s = ps.shape
  const n = (v) => +Number(v).toFixed(3)
  const ev = (c, t, r) => n(c.evaluate(t, r || 0))
  const col = (g, t, r) => { const x = g.evaluate(t, r || 0); return [x.r, x.g, x.b, x.a].map(n) }
  const v3 = (v) => [v.x, v.y, v.z].map(n)
  const b = e.m_Bursts.length ? e.getBurst(0) : { time: 0, minCount: 0, maxCount: 0, cycleCount: 0, repeatInterval: 0 }
  return {
    main: { maxParticles: m.maxParticles, loop: m.loop, prewarm: m.prewarm, duration: m.duration, delay: ev(m.startDelay, 0), simulationSpace: m.simulationSpace, simulationSpeed: m.simulationSpeed, scalingMode: m.scalingMode,
      lifetime: [ev(m.startLifetime, 0, 0), ev(m.startLifetime, 0, 1)], speed: ev(m.startSpeed, 0.5), size3D: m.startSize3D, size: [ev(m.startSizeX, 0), ev(m.startSizeY, 0), ev(m.startSizeZ, 0)],
      rotation3D: m.startRotation3D, rotation: [ev(m.startRotationX, 0), ev(m.startRotationY, 0), ev(m.startRotationZ, 0)], color: [[m.startColor._colorMax.r, m.startColor._colorMax.g, m.startColor._colorMax.b, m.startColor._colorMax.a].map(n), [m.startColor._colorMin.r, m.startColor._colorMin.g, m.startColor._colorMin.b, m.startColor._colorMin.a].map(n)], gravity: ev(m.gravityModifier, 0) },
    emission: { rate: ev(e.rateOverTime, 0), distance: ev(e.rateOverDistance, 0), bursts: e.m_Bursts.length, burst: [n(b.time), b.minCount, b.maxCount, b.cycleCount, n(b.repeatInterval)] },
    shape: { enabled: s.enabled, type: s.shapeType, angle: n(s.angle), radius: n(s.radius), thickness: n(s.radiusThickness), arc: n(s.arc), length: n(s.length), position: v3(s.position), rotation: v3(s.rotation), scale: v3(s.scale),
      randomDirection: n(s.randomDirectionAmount), alignToDirection: s.alignToDirection },
    velocity: { enabled: ps.velocityOverLifetime.enabled, xyz: [ev(ps.velocityOverLifetime.x, 0), ev(ps.velocityOverLifetime.y, 0), ev(ps.velocityOverLifetime.z, 0)], space: ps.velocityOverLifetime.space, radial: ev(ps.velocityOverLifetime.radial, 0), speedModifier: ev(ps.velocityOverLifetime.speedModifier, 0) },
    inherit: { enabled: ps.inheritVelocity.enabled, mode: ps.inheritVelocity.mode, curve: ev(ps.inheritVelocity.curve, 0) },
    force: { enabled: ps.forceOverLifetime.enabled, xyz: [ev(ps.forceOverLifetime.x, 0), ev(ps.forceOverLifetime.y, 0), ev(ps.forceOverLifetime.z, 0)], space: ps.forceOverLifetime.space, randomized: ps.forceOverLifetime.randomized },
    limit: { enabled: ps.limitVelocityOverLifetime.enabled, limit: ev(ps.limitVelocityOverLifetime.limit, 0), dampen: n(ps.limitVelocityOverLifetime.dampen), drag: ev(ps.limitVelocityOverLifetime.drag, 0), space: ps.limitVelocityOverLifetime.space },
    colorOverLifetime: { enabled: ps.colorOverLifetime.enabled, mid: col(ps.colorOverLifetime.color, 0.5) },
    colorBySpeed: { enabled: ps.colorBySpeed.enabled, range: [n(ps.colorBySpeed.range.x), n(ps.colorBySpeed.range.y)], end: col(ps.colorBySpeed.color, 1) },
    sizeOverLifetime: { enabled: ps.sizeOverLifetime.enabled, separate: ps.sizeOverLifetime.separateAxes, xyz: [ev(ps.sizeOverLifetime.x, 0.5), ev(ps.sizeOverLifetime.y, 0.5), ev(ps.sizeOverLifetime.z, 0.5)] },
    sizeBySpeed: { enabled: ps.sizeBySpeed.enabled, range: [n(ps.sizeBySpeed.range.x), n(ps.sizeBySpeed.range.y)], size: ev(ps.sizeBySpeed.size, 0.5) },
    rotationOverLifetime: { enabled: ps.rotationOverLifetime.enabled, separate: ps.rotationOverLifetime.separateAxes, z: ev(ps.rotationOverLifetime.z, 0.5) },
    rotationBySpeed: { enabled: ps.rotationBySpeed.enabled, range: [n(ps.rotationBySpeed.range.x), n(ps.rotationBySpeed.range.y)] },
    noise: { enabled: ps.noise.enabled, frequency: n(ps.noise.frequency), damping: ps.noise.damping, strength: ev(ps.noise.strength, 0), position: ev(ps.noise.positionAmount, 0), size: ev(ps.noise.sizeAmount, 0) },
    sheet: { enabled: ps.textureSheetAnimation.enabled, tiles: [ps.textureSheetAnimation.numTilesX, ps.textureSheetAnimation.numTilesY], animation: ps.textureSheetAnimation.animation, row: ps.textureSheetAnimation.rowIndex, randomRow: ps.textureSheetAnimation.useRandomRow, cycles: ps.textureSheetAnimation.cycleCount, startFrame: ev(ps.textureSheetAnimation.startFrame, 0) },
    renderer: (() => { const r = ps.gameObject.GetComponent(frame.contentWindow.UnityEngine.ParticleSystemRenderer), inner = r.zbb$._particleSystemRenderer; return { mode: r.renderMode, alignment: r.alignment, sortMode: inner._sortMode, sortingOrder: r.sortingOrder, lengthScale: n(inner._lengthScale), velocityScale: n(inner._velocityScale), min: n(r.minParticleSize), max: n(r.maxParticleSize), enabled: r.enabled } })()
  }
}
window.pixels = async (dataUrl) => {
  const bmp = await createImageBitmap(await (await fetch(dataUrl)).blob())
  const c = new OffscreenCanvas(bmp.width, bmp.height), x = c.getContext('2d'); x.drawImage(bmp, 0, 0)
  return Array.from(x.getImageData(0, 0, bmp.width, bmp.height).data)
}
</script></body></html>`

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.css': 'text/css', '.woff2': 'font/woff2' }

function serve(dist) {
  const server = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname)
    if (path === '/__harness.html') {
      res.writeHead(200, { 'Content-Type': 'text/html' }).end(HARNESS)
      return
    }
    const file = join(dist, normalize(path === '/' ? '/index.html' : path))
    if (!file.startsWith(dist) || !existsSync(file) || !statSync(file).isFile()) {
      res.writeHead(404).end('not found')
      return
    }
    res.writeHead(200, { 'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream' }).end(readFileSync(file))
  })
  return new Promise((resolveListen, reject) => {
    server.once('error', (e) =>
      reject(new Error(e.code === 'EADDRINUSE' ? `Port ${SERVER_PORT} is busy. Stop the process that holds it (lsof -i :${SERVER_PORT}).` : String(e)))
    )
    server.listen(SERVER_PORT, '127.0.0.1', () => {
      cleanups.push(() => server.close() && server.closeAllConnections())
      resolveListen()
    })
  })
}

// ---- Chrome DevTools -------------------------------------------------------------------------------------------
async function launchChrome() {
  if (!existsSync(chromePath)) throw new Error(`Chrome not found at ${chromePath}. Set CHROME_PATH.`)
  const profile = mkdtempSync(join(tmpdir(), 'asset-hub-luna-chrome-'))
  const flags = ['--headless=new', `--remote-debugging-port=${DEBUG_PORT}`, `--user-data-dir=${profile}`, '--window-size=800,600', '--no-first-run', '--enable-unsafe-swiftshader']
  if (process.platform === 'darwin') flags.push('--use-angle=metal')
  const child = spawn(chromePath, [...flags, 'about:blank'], { stdio: 'ignore' })
  cleanups.push(() => {
    child.kill('SIGTERM')
    rmSync(profile, { recursive: true, force: true })
  })
  let target
  for (let i = 0; i < 100 && !target; i++) {
    await sleep(200)
    try {
      target = (await (await fetch(`http://127.0.0.1:${DEBUG_PORT}/json`)).json()).find((t) => t.type === 'page')
    } catch {
      /* Chrome is not up yet */
    }
  }
  if (!target) throw new Error('Chrome did not start. Is port ' + DEBUG_PORT + ' held by an old Chrome?')
  const ws = new WebSocket(target.webSocketDebuggerUrl)
  await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = () => j(new Error('DevTools socket failed')))))
  cleanups.push(() => ws.close())
  let id = 0
  const pending = new Map()
  const logs = []
  ws.onmessage = (e) => {
    const m = JSON.parse(String(e.data))
    if (m.id && pending.has(m.id)) {
      pending.get(m.id)(m.result ?? { error: m.error })
      pending.delete(m.id)
    } else if (m.method === 'Runtime.exceptionThrown') {
      const d = m.params.exceptionDetails
      logs.push({ level: 'exception', text: String(d.exception?.description ?? d.text).slice(0, 400) })
    } else if (m.method === 'Runtime.consoleAPICalled' && (m.params.type === 'error' || m.params.type === 'warning')) {
      logs.push({ level: m.params.type, text: m.params.args.map((a) => String(a.value ?? a.description ?? '')).join(' ').slice(0, 600) })
    } else if (m.method === 'Log.entryAdded') {
      logs.push({ level: m.params.entry.level, text: `${m.params.entry.text} ${m.params.entry.url ?? ''}`.slice(0, 400) })
    }
  }
  const send = (method, params = {}) => new Promise((r) => ((pending.set(++id, r), ws.send(JSON.stringify({ id, method, params })))))
  const evaluate = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true, timeout: 15000 })
    if (r.exceptionDetails) throw new Error(String(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text))
    return r.result?.value
  }
  await send('Runtime.enable')
  await send('Log.enable')
  await send('Page.enable')
  await send('Emulation.setDeviceMetricsOverride', { width: 800, height: 600, deviceScaleFactor: 1, mobile: false })
  const shot = async () => `data:image/png;base64,${(await send('Page.captureScreenshot', { format: 'png' })).data}`
  const navigate = async (url) => {
    await send('Page.navigate', { url })
    await sleep(300)
  }
  return { evaluate, shot, navigate, logs }
}

/** Counts pixels that differ from the background, and pixels that differ between two frames. */
async function analyse(chrome, before, after) {
  const [a, b] = await Promise.all([chrome.evaluate(`pixels(${JSON.stringify(before)})`), chrome.evaluate(`pixels(${JSON.stringify(after)})`)])
  let lit = 0
  let changed = 0
  for (let i = 0; i < a.length; i += 4) {
    if (Math.abs(a[i] - BACKGROUND[0]) + Math.abs(a[i + 1] - BACKGROUND[1]) + Math.abs(a[i + 2] - BACKGROUND[2]) > 24) lit++
    if (Math.abs(a[i] - b[i]) + Math.abs(a[i + 1] - b[i + 1]) + Math.abs(a[i + 2] - b[i + 2]) > 24) changed++
  }
  return { lit, changed }
}

// ---- shader cases ----------------------------------------------------------------------------------------------
const TINT = [0.5, 0.5, 0.5, 0.5]
const STREAMS = /custom vertex streams/
const legacy = (shader, shaderName, tint) => ({
  shader, shaderName, keywords: [], floats: {}, colors: tint ? { _TintColor: TINT } : {}, vectors: { _MainTex_ST: [1, 1, 0, 0] }, textures: { _MainTex: 'MAIN' }, notes: []
})
const urp = ({ surface = 1, blend = 0, src = 5, dst = 10, zWrite = false, queue = 3000 }) => ({
  shader: 'urp-particles-unlit', shaderName: 'Universal Render Pipeline/Particles/Unlit', keywords: [], state: { src, dst, zWrite, cull: 2, queue },
  floats: { _Surface: surface, _Blend: blend, _ColorMode: 0, _AlphaClip: 0, _Cutoff: 0.5 }, colors: { _BaseColor: [1, 1, 1, 1] },
  vectors: { _BaseMap_ST: [1, 1, 0, 0] }, textures: { _BaseMap: 'MAIN' }, notes: []
})
const klaus = (additive) => ({
  shader: 'klaus', shaderName: `VFX_Klaus/fx_explosion_${additive ? 'add' : 'apb'}`, keywords: [], state: { src: 5, dst: additive ? 1 : 10, zWrite: false, cull: 0, queue: 3000 },
  floats: { _Emission_Power: 1, uAdditive: additive ? 1 : 0 }, colors: {}, vectors: { _Main_Tex_ST: [1, 1, 0, 0] }, textures: { _Main_Tex: 'MAIN' }, notes: ['custom vertex streams']
})
const MULTIPLY_COLOR = [0, 0, 0, 1]

/** One emitter per shader. `unsupported` lists what the host must report for the case (a regex on the detail). */
const CASES = [
  { id: 'legacy-additive', label: 'Legacy Additive', material: legacy('legacy-additive', 'Legacy Shaders/Particles/Additive', true) },
  { id: 'legacy-additive-soft', label: 'Legacy Additive (Soft)', material: legacy('legacy-additive-soft', 'Legacy Shaders/Particles/Additive (Soft)', false) },
  { id: 'legacy-alpha', label: 'Legacy Alpha Blended', material: legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true) },
  { id: 'legacy-premultiply', label: 'Legacy Alpha Premultiply', material: legacy('legacy-alpha-blended-premultiply', 'Legacy Shaders/Particles/Alpha Blended Premultiply', false) },
  { id: 'legacy-multiply', label: 'Legacy Multiply', material: legacy('legacy-multiply', 'Legacy Shaders/Particles/Multiply', false), startColor: MULTIPLY_COLOR },
  { id: 'mobile-additive', label: 'Mobile Additive', material: legacy('mobile-additive', 'Mobile/Particles/Additive', false) },
  { id: 'mobile-alpha', label: 'Mobile Alpha Blended', material: legacy('mobile-alpha-blended', 'Mobile/Particles/Alpha Blended', false) },
  { id: 'urp-alpha', label: 'URP Unlit alpha', material: urp({}) },
  { id: 'urp-premultiply', label: 'URP Unlit premultiply', material: urp({ blend: 1, src: 1, dst: 10 }) },
  { id: 'urp-additive', label: 'URP Unlit additive', material: urp({ blend: 2, src: 5, dst: 1 }) },
  { id: 'urp-multiply', label: 'URP Unlit multiply', material: urp({ blend: 3, src: 2, dst: 0 }), startColor: MULTIPLY_COLOR },
  { id: 'urp-opaque', label: 'URP Unlit opaque', material: urp({ surface: 0, src: 1, dst: 0, zWrite: true, queue: 2000 }) },
  { id: 'default-particle', label: 'Default-Particle', material: { shader: 'default-particle', shaderName: 'Default-Particle', keywords: [], floats: {}, colors: { _TintColor: TINT }, vectors: { _MainTex_ST: [1, 1, 0, 0] }, textures: { _MainTex: 'MAIN' }, notes: [] } },
  { id: 'klaus-add', label: 'Klaus add', material: klaus(true), unsupported: [STREAMS] },
  { id: 'klaus-apb', label: 'Klaus alpha', material: klaus(false), unsupported: [STREAMS] },
  {
    id: 'all-in-1', label: 'AllIn1Vfx',
    material: {
      shader: 'all-in-1-vfx', shaderName: 'AllIn1Vfx/AllIn1Vfx', keywords: ['GLOW_ON'], state: { src: 5, dst: 10, zWrite: false, cull: 0, queue: 3000 },
      floats: { _Alpha: 1, _Glow: 1, _GlowGlobal: 1, _ShapeColorWeight: 1, _ShapeAlphaWeight: 1 }, colors: { _Color: [1, 1, 1, 1], _ShapeColor: [1, 1, 1, 1], _GlowColor: [1, 0.5, 0, 1] }, vectors: { _MainTex_ST: [1, 1, 0, 0] },
      textures: { _MainTex: 'MAIN' }, notes: ['custom vertex streams']
    },
    unsupported: [STREAMS]
  },
  { id: 'sampler-point', label: 'Point, repeat, no mips', material: legacy('legacy-additive', 'Legacy Shaders/Particles/Additive', true), sampler: { filter: 'point', wrapU: 'repeat', wrapV: 'mirror', mipmaps: false } },
  { id: 'sampler-trilinear', label: 'Trilinear, srgb flag', material: legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true), sampler: { filter: 'trilinear', srgb: true } },
  {
    id: 'unknown-shader', label: 'Unknown shader (fallback)',
    material: { shader: null, shaderName: 'Custom/DoesNotExist', keywords: [], floats: {}, colors: { _TintColor: TINT }, vectors: { _MainTex_ST: [1, 1, 0, 0] }, textures: { _MainTex: 'MAIN' }, notes: [],
      state: { src: 5, dst: 10, zWrite: false, cull: 0, queue: 3000 } },
    unsupported: [/Custom\/DoesNotExist/]
  }
]

/** Brightness figures of the pixels that differ from the background. */
async function measure(chrome, shot) {
  const a = await chrome.evaluate(`pixels(${JSON.stringify(shot)})`)
  let lit = 0, sum = 0, peak = 0
  const rgb = [0, 0, 0]
  for (let i = 0; i < a.length; i += 4) {
    if (Math.abs(a[i] - BACKGROUND[0]) + Math.abs(a[i + 1] - BACKGROUND[1]) + Math.abs(a[i + 2] - BACKGROUND[2]) <= 24) continue
    lit++
    const lum = a[i] + a[i + 1] + a[i + 2]
    sum += lum
    peak = Math.max(peak, lum)
    for (let k = 0; k < 3; k++) rgb[k] += a[i + k]
  }
  return { lit, mean: lit ? Math.round(sum / lit) : 0, peak, rgb: rgb.map((v) => (lit ? Math.round(v / lit) : 0)) }
}

/** Draws one case: load, jump to 1.2 s, screenshot. Returns the figures, the screenshot and the events. */
async function runCase(chrome, c) {
  await chrome.evaluate('events.length = 0')
  await chrome.evaluate(`loadCase(${JSON.stringify(c)})`)
  const loaded = await chrome.evaluate('waitFor(() => events.find((e) => e.type === "loaded" || e.type === "error"), 10000)')
  await chrome.evaluate('send({ type: "seek", time: 1.2 })')
  await sleep(700)
  const shot = await chrome.shot()
  const events = await chrome.evaluate('events.filter((e) => e.type === "error" || e.type === "unsupported")')
  return { loaded, shot, events, ...(await measure(chrome, shot)) }
}

// ---- module cases ----------------------------------------------------------------------------------------------
const c = (v) => ({ mode: 0, scalar: v, min: v })
/** A curve that rises from 0 to 1 over the lifetime, times `scalar`. */
const rise = (scalar) => ({ mode: 1, scalar, min: 0, curve: [{ t: 0, v: 0, inTangent: 1, outTangent: 1 }, { t: 1, v: 1, inTangent: 1, outTangent: 1 }] })
const xyz = (a, b, d) => [c(a), c(b), c(d)]
/** An emitter with every module on and a value in every field. The host must apply all of it. */
const FULL = {
  name: 'Emitter', node: 0, material: 0, unsupported: [],
  main: { duration: 7, loop: false, prewarm: false, startDelay: c(0.25), startLifetime: { mode: 3, scalar: 3, min: 1 }, startSpeed: rise(2), startSize: xyz(0.5, 1, 0.25), startRotation: xyz(0.1, 0.2, 0.3),
    startColor: { mode: 2, max: [1, 0, 0, 1], min: [0, 0, 1, 1] }, gravity: c(0.1), simulationSpace: 1, simulationSpeed: 1.5, useUnscaledTime: false, scalingMode: 2, maxParticles: 321, emitterVelocityMode: 0, playOnAwake: true },
  emission: { rateOverTime: c(30), rateOverDistance: c(2), bursts: [{ time: 0.2, min: 5, max: 10, cycles: 2, interval: 0.3 }] },
  shape: { type: 5, angle: 20, radius: 1.5, radiusThickness: 0.5, radiusMode: 0, radiusSpread: 0, length: 4, arc: 270, arcMode: 0, arcSpread: 0, arcSpeed: c(1), donutRadius: 0.2,
    position: [1, 2, 3], rotation: [10, 20, 30], scale: [2, 3, 4], randomDirection: 0.1, sphericalDirection: 0, randomPosition: 0, alignToDirection: true },
  velocity: { x: c(1), y: c(2), z: c(3), world: true, orbital: xyz(0, 0, 0), orbitalOffset: xyz(0, 0, 0), radial: c(0.5), speedModifier: c(1.5) },
  inheritVelocity: { mode: 1, curve: c(0.4) },
  force: { x: c(0.1), y: c(0.2), z: c(0.3), world: true, randomized: true },
  limit: { limit: c(5), dampen: 0.3, drag: c(0.2), dragBySize: false, dragByVelocity: false, world: true },
  colorOverLifetime: { mode: 1, max: [1, 1, 1, 1], min: [1, 1, 1, 1],
    gradient: { fixed: false, colors: [{ t: 0, r: 1, g: 0, b: 0 }, { t: 1, r: 0, g: 0, b: 1 }], alphas: [{ t: 0, a: 1 }, { t: 1, a: 0 }] } },
  colorBySpeed: { color: { mode: 0, max: [0, 1, 0, 1], min: [0, 1, 0, 1] }, min: 1, max: 6 },
  sizeOverLifetime: { separate: true, x: rise(1), y: rise(2), z: rise(3) },
  sizeBySpeed: { separate: false, x: c(1.5), y: c(1.5), z: c(1.5), min: 0, max: 8 },
  rotationOverLifetime: { separate: false, x: c(0), y: c(0), z: c(1.2) },
  rotationBySpeed: { separate: false, x: c(0), y: c(0), z: c(0.5), min: 2, max: 9 },
  sheet: { animation: 1, tilesX: 4, tilesY: 2, frame: c(1), startFrame: c(0.25), cycles: 2, row: 1, randomRow: true },
  noise: { separate: false, strengthX: c(0.7), strengthY: c(0.7), strengthZ: c(0.7), frequency: 0.6, damping: false, scrollSpeed: c(0), positionAmount: c(0.8), sizeAmount: c(0.1) },
  renderer: { renderMode: 1, alignment: 1, sortMode: 2, sortingOrder: 3, lengthScale: 1.5, velocityScale: 0.25, minParticleSize: 0.05, maxParticleSize: 0.75, applyActiveColorSpace: true }
}
/** What the host must read back after `FULL`. A curve that rises from 0 to 1 reads `scalar * t` at `t`. */
const FULL_EXPECT = {
  main: { maxParticles: 321, loop: false, prewarm: false, duration: 7, delay: 0.25, simulationSpace: 1, simulationSpeed: 1.5, scalingMode: 2, lifetime: [1, 3], speed: 1, size3D: true, size: [0.5, 1, 0.25],
    rotation3D: true, rotation: [0.1, 0.2, 0.3], color: [[1, 0, 0, 1], [0, 0, 1, 1]], gravity: 0.1 },
  emission: { rate: 30, distance: 2, bursts: 1, burst: [0.2, 5, 10, 2, 0.3] },
  shape: { enabled: true, type: 5, angle: 20, radius: 1.5, thickness: 0.5, arc: 270, length: 4, position: [1, 2, 3], rotation: [10, 20, 30], scale: [2, 3, 4], randomDirection: 0.1, alignToDirection: true },
  velocity: { enabled: true, xyz: [1, 2, 3], space: 1, radial: 0.5, speedModifier: 1.5 },
  inherit: { enabled: true, mode: 1, curve: 0.4 },
  force: { enabled: true, xyz: [0.1, 0.2, 0.3], space: 1, randomized: true },
  limit: { enabled: true, limit: 5, dampen: 0.3, drag: 0.2, space: 1 },
  colorOverLifetime: { enabled: true, mid: [0.5, 0, 0.5, 0.5] },
  colorBySpeed: { enabled: true, range: [1, 6], end: [0, 1, 0, 1] },
  sizeOverLifetime: { enabled: true, separate: true, xyz: [0.5, 1, 1.5] },
  sizeBySpeed: { enabled: true, range: [0, 8], size: 1.5 },
  rotationOverLifetime: { enabled: true, separate: false, z: 1.2 },
  rotationBySpeed: { enabled: true, range: [2, 9] },
  noise: { enabled: true, frequency: 0.6, damping: false, strength: 0.7, position: 0.8, size: 0.1 },
  sheet: { enabled: true, tiles: [4, 2], animation: 1, row: 1, randomRow: true, cycles: 2, startFrame: 0.25 },
  renderer: { mode: 1, alignment: 1, sortMode: 2, sortingOrder: 3, lengthScale: 1.5, velocityScale: 0.25, min: 0.05, max: 0.75, enabled: true }
}

/** Lists every difference between what was read back and what was expected. Numbers match within 0.01. */
function differences(actual, expected, path = '') {
  if (Array.isArray(expected)) return expected.flatMap((v, i) => differences(actual?.[i], v, `${path}[${i}]`))
  if (expected && typeof expected === 'object') return Object.entries(expected).flatMap(([k, v]) => differences(actual?.[k], v, path ? `${path}.${k}` : k))
  const same = typeof expected === 'number' ? typeof actual === 'number' && Math.abs(actual - expected) <= 0.01 : actual === expected
  return same ? [] : [`${path}: ${JSON.stringify(actual)} instead of ${JSON.stringify(expected)}`]
}

const TEXTURED = legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true)

/** Loads a hand-made effect (a harness expression) and waits for `loaded`. Returns the load event and the events that followed. */
async function loadSpec(chrome, expression) {
  await chrome.evaluate('events.length = 0')
  await chrome.evaluate(`(async () => { const bitmap = await bitmapOf(64); const material = ${JSON.stringify(TEXTURED)}; material.textures._MainTex = { bitmap, wrapU: 'clamp', wrapV: 'clamp', filter: 'bilinear', srgb: false, mipmaps: true }; const effect = ${expression}; effect.materials = [material]; send({ type: 'load', effect: { linear: LINEAR, ...effect } }) })()`)
  const loaded = await chrome.evaluate('waitFor(() => events.find((e) => e.type === "loaded" || e.type === "error"), 10000)')
  return { loaded, events: await chrome.evaluate('events.filter((e) => e.type === "error" || e.type === "unsupported")') }
}

async function moduleTests(chrome) {
  // Every module and field reaches the engine.
  let r = await loadSpec(chrome, `({ nodes: [upNode], emitters: [${JSON.stringify(FULL)}] })`)
  check(r.loaded?.type === 'loaded' && r.events.length === 0, 'modules: a system with every module on loads without events', JSON.stringify(r).slice(0, 300))
  const back = await chrome.evaluate('readFull()')
  const diff = differences(back, FULL_EXPECT)
  check(diff.length === 0, 'modules: the engine reads back every value that was applied', diff.slice(0, 6).join('; '))
  for (const d of diff) console.log(`      ${d}`)

  // A system with no optional module turns them all off, and the engine's defaults for the rest do not leak in.
  r = await loadSpec(chrome, `({ nodes: [upNode], emitters: [coneEmitter({ shape: undefined })] })`)
  const bare = (await chrome.evaluate('readFull()'))
  const off = ['shape', 'velocity', 'inherit', 'force', 'limit', 'colorOverLifetime', 'colorBySpeed', 'sizeOverLifetime', 'sizeBySpeed', 'rotationOverLifetime', 'rotationBySpeed', 'noise', 'sheet'].filter((k) => bare[k].enabled)
  check(off.length === 0, 'modules: a system without optional modules has them all off', `still on: ${off.join(', ')}`)
  check(bare.main.maxParticles === 1000 && bare.main.prewarm === false && bare.main.size[0] === 0.8, 'modules: the main module overrides the engine defaults (cap 10, prewarm on, size 0)', JSON.stringify(bare.main).slice(0, 200))

  // Bursts fire: no continuous emission, one burst at the start.
  const burst = `({ nodes: [upNode], emitters: [coneEmitter({ emission: { rateOverTime: curve(0), rateOverDistance: curve(0), bursts: [{ time: 0, min: 40, max: 40, cycles: 1, interval: 0.01 }] } })] })`
  r = await loadSpec(chrome, burst)
  await chrome.evaluate('send({ type: "seek", time: 0.5 })')
  await sleep(500)
  const burstState = await chrome.evaluate('lastTime()')
  check(burstState.particles >= 30 && burstState.particles <= 40, 'emission: a burst of 40 at the start makes about 40 particles with no rate', JSON.stringify(burstState))
  r = await loadSpec(chrome, `({ nodes: [upNode], emitters: [coneEmitter({ emission: { rateOverTime: curve(0), rateOverDistance: curve(0), bursts: [] } })] })`)
  await chrome.evaluate('send({ type: "seek", time: 0.5 })')
  await sleep(500)
  check((await chrome.evaluate('lastTime()')).particles === 0, 'emission: with no rate and no burst there are no particles')

  // Unsupported modules reach the app per emitter, named by the module.
  r = await loadSpec(chrome, `({ nodes: [upNode], emitters: [coneEmitter({ name: 'Ribbon', unsupported: [{ module: 'Trails', detail: 'Trails are not drawn.' }, { module: 'Collision', detail: 'Collision is not drawn.' }] }), coneEmitter({ name: 'Plain', node: 1 })].slice(0, 1) })`)
  const named = r.events.filter((e) => e.type === 'unsupported')
  check(named.length === 2 && named.every((e) => e.emitter === 'Ribbon') && named.map((e) => e.what).join() === 'Trails,Collision', 'unsupported: each module is reported once, with the system that uses it', JSON.stringify(named))

  // Hierarchy: parent, local transform, activation.
  const tree = `({ nodes: [
    { name: 'Root', parent: -1, position: [0, 0, 0], rotation: [-0.7071068, 0, 0, 0.7071068], scale: [1, 1, 1], active: true },
    { name: 'Child', parent: 0, position: [0, 0, 1], rotation: [0, 0, 0, 1], scale: [2, 2, 2], active: true },
    { name: 'Hidden', parent: 0, position: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], active: false }],
    emitters: [coneEmitter({ name: 'Under child', node: 1 }), coneEmitter({ name: 'Under hidden', node: 2 })] })`
  r = await loadSpec(chrome, tree)
  await sleep(800)
  const parts = await chrome.evaluate('readHierarchy()')
  const child = parts.find((x) => x.name === 'Child')
  const hidden = parts.find((x) => x.name === 'Hidden')
  check(r.loaded?.type === 'loaded' && Boolean(child), 'hierarchy: the system under the child node exists', JSON.stringify(parts).slice(0, 300))
  check(child?.parent === 'Root', 'hierarchy: the child hangs under its parent', JSON.stringify(parts.map((x) => [x.name, x.parent])))
  check(child && child.localScale[0] === 2 && Math.abs(child.position[1] - 1) < 0.01 && Math.abs(child.position[2]) < 0.01, 'hierarchy: local transforms compose into the world position (a child at +Z under a -90 degree X turn is at +Y)', JSON.stringify(child))
  check(child?.active && child.particles > 0, 'hierarchy: the active child plays', JSON.stringify(child))
  check(!hidden, 'hierarchy: a node that the prefab keeps inactive is not active in the scene, so its system does not run', JSON.stringify(hidden))
  // Play on awake off: the system stays stopped until something plays it.
  r = await loadSpec(chrome, `({ nodes: [upNode], emitters: [coneEmitter({ main: { ...coneEmitter().main, playOnAwake: false } })] })`)
  await sleep(600)
  const idle = (await chrome.evaluate('readHierarchy()'))[0]
  check(idle.particles === 0 && !idle.playing, 'main: a system with Play On Awake off does not start by itself', JSON.stringify(idle))

  // The colour space of the effect must match the host.
  r = await chrome.evaluate(`(async () => { events.length = 0; send({ type: 'load', effect: { linear: !LINEAR, materials: [], nodes: [], emitters: [] } }); const e = await waitFor(() => events.find((x) => x.type === 'error' || x.type === 'loaded'), 5000); events.length = 0; return e })()`)
  check(r?.type === 'error' && /colour space/.test(r.message), 'colour space: an effect built for the other space is refused', JSON.stringify(r))
}

// ---- real prefabs ----------------------------------------------------------------------------------------------
const SAMPLES = join(root, 'References/LunaParticlePreview/Assets/LunaParticlePreview/Preview/Samples')
/** Prefabs of the sample set: one emitter with a texture, a hierarchy with a sprite sheet and sub emitter, a project shader, bursts. */
const PREFABS = ['Glow', 'Sprite', 'Glass_crack', 'WaterBurst', 'Chips']

/** The fixture: what the app's parsers read from the sample prefabs (see crates/core/examples/luna_fixture.rs). Null when the samples are absent. */
function loadFixture() {
  if (process.env.LUNA_SMOKE_NO_SAMPLES || !existsSync(SAMPLES)) return null
  const result = spawnSync('cargo', ['run', '-q', '-p', 'asset_hub_core', '--example', 'luna_fixture', '--', SAMPLES, ...PREFABS], { cwd: root, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 })
  if (result.status !== 0) throw new Error(`The fixture dumper failed:\n${result.stderr}`)
  return JSON.parse(result.stdout)
}

/** The effect the app would send for a parsed prefab. Runs the real bridge and material mapping. */
async function effectOf(fx, gamma) {
  const { buildLunaEffect } = await import('../ui/src/luna/bridge.ts')
  const { mapMaterial, samplerFromImport } = await import('../ui/src/luna/materialMap.ts')
  const byId = new Map(fx.scene.nodes)
  const nodes = []
  const indexOf = new Map()
  const add = (id) => {
    if (indexOf.has(id)) return indexOf.get(id)
    const n = byId.get(id)
    const parent = n.parent && byId.has(n.parent) ? add(n.parent) : -1
    nodes.push({ name: n.name, parent, position: n.position, rotation: n.rotation, scale: n.scale, active: n.active })
    indexOf.set(id, nodes.length - 1)
    return nodes.length - 1
  }
  for (const [id] of fx.scene.nodes) add(id)
  const mapped = new Map()
  const materialOf = (guid) => {
    const m = guid ? fx.materials[guid] : undefined
    if (!m) return undefined
    if (!mapped.has(guid)) {
      const textures = {}
      for (const [slot, t] of Object.entries(m.textures)) {
        if (!/\.(png|jpe?g)$/i.test(t.file)) continue
        textures[slot] = { ...samplerFromImport(t.import, gamma), png: readFileSync(t.file).toString('base64') }
      }
      mapped.set(guid, mapMaterial(m.shader ? { shader: { label: m.shader.label, kind: m.shader.kind }, shaderName: m.shader.name ?? undefined, props: m.props, look: m.look, textures } : undefined))
    }
    return mapped.get(guid)
  }
  const emitters = fx.emitters.map((emitter) => {
    if (!indexOf.has(emitter.node)) throw new Error(`The emitter ${emitter.name} has no transform in the scene`)
    return { emitter, node: indexOf.get(emitter.node), material: materialOf(emitter.material) }
  })
  return buildLunaEffect({ nodes, emitters, linear: !gamma })
}

const shotDir = resolve(option('--shots') ?? join(tmpdir(), 'asset-hub-luna-prefabs'))

async function prefabTests(chrome, fixture, mode) {
  mkdirSync(shotDir, { recursive: true })
  for (const name of PREFABS) {
    const fx = fixture[name]
    if (!fx) {
      check(false, `${name} (${mode}): the fixture has the prefab`)
      continue
    }
    const effect = await effectOf(fx, mode === 'gamma')
    const expectedUnsupported = effect.emitters.flatMap((e) => e.unsupported.map((u) => `${e.name}|${u.module}`))
    await chrome.evaluate('events.length = 0')
    const mark = chrome.logs.length
    await chrome.evaluate(`loadEffect(${JSON.stringify(effect)})`)
    const loaded = await chrome.evaluate('waitFor(() => events.find((e) => e.type === "loaded" || e.type === "error"), 15000)')
    // Sample three moments of the first lifetimes, because some effects last a fraction of a second.
    const span = Math.min(3, Math.max(0.15, ...effect.emitters.map((e) => Math.max(e.main.startDelay.scalar, e.main.startDelay.min) + Math.max(e.main.startLifetime.scalar, e.main.startLifetime.min))))
    const frames = []
    for (const fraction of [0.2, 0.4, 0.6]) {
      await chrome.evaluate(`send({ type: "seek", time: ${(span * fraction).toFixed(3)} })`)
      await sleep(600)
      const shot = await chrome.shot()
      frames.push({ shot, ...(await analyse(chrome, shot, shot)) })
    }
    const best = frames.reduce((a, b) => (b.lit > a.lit ? b : a))
    const lit = best.lit
    const changed = Math.max((await analyse(chrome, frames[0].shot, frames[1].shot)).changed, (await analyse(chrome, frames[1].shot, frames[2].shot)).changed)
    writeFileSync(join(shotDir, `${name}-${mode}.png`), Buffer.from(best.shot.split(',')[1], 'base64'))
    const events = await chrome.evaluate('events.filter((e) => e.type === "error" || e.type === "unsupported")')
    const errors = events.filter((e) => e.type === 'error')
    const label = `${name} (${mode})`
    check(loaded?.type === 'loaded' && errors.length === 0, `${label}: loads without error`, JSON.stringify(errors).slice(0, 300))
    for (const l of chrome.logs.slice(mark).filter((x) => x.level === 'error' || x.level === 'exception').slice(0, 4)) console.log(`      log ${l.level}: ${l.text}`)
    check(lit > 300, `${label}: draws non-background pixels`, `${lit} px`)
    check(changed > 100, `${label}: animates between two times`, `${changed} px changed`)
    const parts = await chrome.evaluate('readHierarchy()')
    const nodeNames = new Map(effect.nodes.map((n, i) => [i, n]))
    // The engine finds only systems on nodes that are active in the hierarchy, as Unity does.
    const isActive = (i) => nodeNames.get(i).active && (nodeNames.get(i).parent < 0 || isActive(nodeNames.get(i).parent))
    const visible = effect.emitters.filter((e) => isActive(e.node))
    const hierarchyOk = visible.every((e) => {
      const node = nodeNames.get(e.node)
      const part = parts.find((x) => x.name === node.name && x.maxParticles === e.main.maxParticles)
      return part && part.parent === (node.parent >= 0 ? nodeNames.get(node.parent).name : null)
    })
    check(parts.length === visible.length && hierarchyOk, `${label}: ${effect.emitters.length} system(s), ${visible.length} on active nodes, with the prefab hierarchy`, JSON.stringify(parts.map((x) => [x.name, x.parent])).slice(0, 300))
    const reported = events.filter((e) => e.type === 'unsupported' && e.emitter).map((e) => `${e.emitter}|${e.what}`)
    check(expectedUnsupported.every((u) => reported.includes(u)), `${label}: every unsupported module is reported with its system`, `expected ${JSON.stringify(expectedUnsupported)}, got ${JSON.stringify(reported)}`)
    const other = events.filter((e) => e.type === 'unsupported' && !e.emitter).map((e) => `${e.what}: ${e.detail}`)
    console.log(`      ${label}: ${effect.emitters.length} system(s), ${effect.materials.length} material(s); unsupported modules ${expectedUnsupported.length ? expectedUnsupported.join(', ') : 'none'}; material notes ${other.length ? other.join(' | ') : 'none'}`)
  }
}

/** Opens the harness page, which starts a host in gamma or linear colour space, and waits until the host is ready. */
async function openHarness(chrome, base, linear) {
  await chrome.navigate(`${base}/__harness.html${linear ? '?space=linear' : ''}`)
  return chrome.evaluate('waitFor(() => events.find((e) => e.type === "ready" || e.type === "error" || e.type === "unsupported"), 20000)')
}

/** The colour of the empty frame at one point. It must be the camera background in either colour space. */
async function emptyFramePixel(chrome) {
  await chrome.evaluate('send({ type: "dispose" })')
  await sleep(500)
  const a = await chrome.evaluate(`pixels(${JSON.stringify(await chrome.shot())})`)
  const i = (20 * 800 + 400) * 4
  return [a[i], a[i + 1], a[i + 2]]
}

/** The view commands the app sends to frame the effect like the Unity-style viewer and to match its backdrop and speed. */
async function viewTests(chrome) {
  const near = await runCase(chrome, CASES.find((x) => x.id === 'legacy-alpha'))
  const camera = (position, rotation, fov) => chrome.evaluate(`send({ type: "camera", camera: ${JSON.stringify({ position, rotation, fov, near: 0.1, far: 100 })} })`)
  await camera([0, 0.6, -14], [0, 0, 0, 1], 45)
  await sleep(500)
  const far = await measure(chrome, await chrome.shot())
  check(far.lit > 20 && far.lit < near.lit / 2, 'camera: a camera farther away shows the effect smaller', `near ${near.lit} px, far ${far.lit} px`)
  await camera([0, 0.6, -14], [0, 0, 0, 1], 20)
  await sleep(500)
  const narrow = await measure(chrome, await chrome.shot())
  check(narrow.lit > far.lit * 1.5, 'camera: a narrower field of view shows the effect larger', `fov 45 ${far.lit} px, fov 20 ${narrow.lit} px`)
  // Looks along +X from the -X side: Unity's turn of 90 degrees about Y. The effect stays in view only if the rotation is read as Unity does.
  await camera([-6, 0.6, 0], [0, Math.SQRT1_2, 0, Math.SQRT1_2], 45)
  await sleep(500)
  const turned = await measure(chrome, await chrome.shot())
  check(turned.lit > 300, 'camera: a turned camera still looks at the effect', `${turned.lit} px`)

  await chrome.evaluate('send({ type: "background", color: [0.5, 0.25, 0.75, 1] })')
  const bg = await emptyFramePixel(chrome)
  check(bg.every((v, k) => Math.abs(v - [128, 64, 191][k]) <= 6), 'background: the empty frame takes the colour the app sends', JSON.stringify(bg))
  await chrome.evaluate('send({ type: "background", color: [0.1, 0.1, 0.2, 1] })')

  await chrome.evaluate(`loadCase(${JSON.stringify({ material: legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true) })})`)
  await chrome.evaluate('waitFor(() => events.find((e) => e.type === "loaded"), 10000)')
  await chrome.evaluate('send({ type: "speed", value: 0.25 })')
  await chrome.evaluate('send({ type: "restart" })')
  await sleep(1600)
  const slow = (await chrome.evaluate('lastTime()')).time
  await chrome.evaluate('send({ type: "speed", value: 1 })')
  await chrome.evaluate('send({ type: "restart" })')
  await sleep(1600)
  const normal = (await chrome.evaluate('lastTime()')).time
  check(slow < normal * 0.6 && normal > 1, 'speed: a quarter speed plays the clock slower', `quarter ${slow} s, normal ${normal} s over the same wait`)

  // Loop: a short one-shot effect is gone after a moment. With Loop on it plays again by itself, with Loop off it stays gone.
  const shortCase = { material: legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true) }
  const loadShort = () => chrome.evaluate(`(async () => {
    const c = ${JSON.stringify(shortCase)}
    const bitmap = await window.bitmapOf(64)
    for (const k of Object.keys(c.material.textures)) if (c.material.textures[k] === 'MAIN') c.material.textures[k] = { bitmap, wrapU: 'clamp', wrapV: 'clamp', filter: 'bilinear', srgb: false, mipmaps: true }
    const e = window.coneEmitter()
    e.main.loop = false; e.main.duration = 0.4; e.main.startLifetime = window.curve(0.3)
    window.events.length = 0
    window.send({ type: 'load', effect: { linear: window.LINEAR, materials: [c.material], nodes: [window.upNode], emitters: [e] } })
    await window.waitFor(() => window.events.find((x) => x.type === 'loaded'), 10000)
  })()`)
  const lastParticleTimes = async (ms) => {
    const t0 = Date.now()
    const seen = []
    while (Date.now() - t0 < ms) {
      seen.push(await chrome.evaluate('lastTime()'))
      await sleep(100)
    }
    return seen
  }
  await loadShort()
  await chrome.evaluate('send({ type: "loop", value: true })')
  await sleep(1200)
  const looped = (await lastParticleTimes(1500)).filter((x) => x && x.particles > 0).length
  check(looped > 3, 'loop: with Loop on a finished effect plays again', `${looped} samples with particles over 1.5 s`)
  await loadShort()
  await chrome.evaluate('send({ type: "loop", value: false })')
  await sleep(1200)
  const stayed = (await lastParticleTimes(1500)).filter((x) => x && x.particles > 0).length
  check(stayed === 0, 'loop: with Loop off a finished effect stays finished', `${stayed} samples with particles over 1.5 s`)
  await chrome.evaluate('send({ type: "loop", value: true })')

  // Load ids: two loads one after the other. Every answer of a load carries its id, so the app can tell them apart.
  const loadWith = (id, material, tweak = '') => chrome.evaluate(`(async () => {
    const c = ${JSON.stringify(material)}
    const bitmap = await window.bitmapOf(64)
    for (const k of Object.keys(c.textures)) if (c.textures[k] === 'MAIN') c.textures[k] = { bitmap, wrapU: 'clamp', wrapV: 'clamp', filter: 'bilinear', srgb: false, mipmaps: true }
    const e = window.coneEmitter()
    ${tweak}
    window.send({ type: 'load', id: ${id}, effect: { linear: window.LINEAR, materials: [c], nodes: [window.upNode], emitters: [e] } })
  })()`)
  const unknown = { shader: null, shaderName: 'Custom/DoesNotExist', keywords: [], floats: {}, colors: {}, vectors: { _MainTex_ST: [1, 1, 0, 0] }, textures: { _MainTex: 'MAIN' }, notes: [], state: { src: 5, dst: 10, zWrite: false, cull: 0, queue: 3000 } }
  await chrome.evaluate('events.length = 0')
  await loadWith(21, unknown)
  await loadWith(22, legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true))
  await chrome.evaluate('waitFor(() => events.filter((e) => e.type === "loaded").length >= 2, 15000)')
  const answers = await chrome.evaluate('events.filter((e) => e.type === "loaded" || e.type === "unsupported").map((e) => ({ type: e.type, id: e.id }))')
  check(answers.filter((e) => e.type === 'loaded').map((e) => e.id).join() === '21,22', 'load ids: each loaded event carries the id of its load', JSON.stringify(answers))
  check(answers.some((e) => e.type === 'unsupported' && e.id === 21) && answers.every((e) => e.id === 21 || e.id === 22), 'load ids: unsupported events carry the id of their load', JSON.stringify(answers))

  // A load that fails reports its id, and leaves nothing of the half-built effect in the scene.
  await chrome.evaluate('events.length = 0')
  await loadWith(23, legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true), 'delete e.renderer')
  const failed = await chrome.evaluate('waitFor(() => events.find((e) => e.type === "error" || e.type === "loaded"), 10000)')
  check(failed?.type === 'error' && failed.id === 23, 'failed load: the error carries the id of the load', JSON.stringify(failed))
  const left = await chrome.evaluate('systems().length')
  check(left === 0, 'failed load: no GameObject of the half-built effect stays in the scene', `${left} particle systems`)

  // Dispose removes the effect: the host stops reporting time.
  await loadWith(24, legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true))
  await chrome.evaluate('waitFor(() => events.find((e) => e.type === "loaded" && e.id === 24), 10000)')
  await chrome.evaluate('send({ type: "dispose" })')
  await sleep(300)
  await chrome.evaluate('events.length = 0')
  await sleep(600)
  const after = await chrome.evaluate('events.filter((e) => e.type === "time").length')
  check(after === 0 && (await chrome.evaluate('systems().length')) === 0, 'dispose: the effect is gone and the host stops reporting time', `${after} time events`)
}

// ---- the test --------------------------------------------------------------------------------------------------
async function run() {
  const dist = option('--dist') ? resolve(option('--dist')) : flag('--no-build') ? join(root, 'dist') : buildDist()
  if (!existsSync(join(dist, 'luna-host.html')) || !existsSync(join(dist, 'luna'))) throw new Error(`${dist} has no luna-host.html or luna/ folder. Build first.`)
  await serve(dist)
  const chrome = await launchChrome()
  const base = `http://127.0.0.1:${SERVER_PORT}`

  // Isolation: the app window must not get Luna globals.
  await chrome.navigate(`${base}/`)
  await sleep(1500)
  check((await chrome.evaluate('typeof window.pc')) === 'undefined', 'main app window has no pc global')
  check((await chrome.evaluate('typeof window.UnityEngine')) === 'undefined', 'main app window has no UnityEngine global')

  chrome.logs.length = 0
  const ready = await openHarness(chrome, base, flag('--linear'))
  check(ready?.type === 'ready', 'host sends ready', JSON.stringify(ready))
  if (ready?.type !== 'ready') return
  check((await chrome.evaluate('typeof window.pc')) === 'undefined', 'harness window has no pc global')
  check((await chrome.evaluate('hostGlobals()')) === 'object', 'host iframe has the pc global')

  await chrome.evaluate(`loadCase(${JSON.stringify({ material: legacy('legacy-alpha-blended', 'Legacy Shaders/Particles/Alpha Blended', true), startColor: [1, 0.6, 0.2, 1] })})`)
  await sleep(2500)
  const state = await chrome.evaluate('lastTime()')
  check(Boolean(state) && state.particles > 0, 'particles are alive', JSON.stringify(state))
  check(Boolean(state) && state.drawCalls > 0, 'draw calls happen', JSON.stringify(state))
  const shotA = await chrome.shot()
  await sleep(700)
  const shotB = await chrome.shot()
  const { lit, changed } = await analyse(chrome, shotA, shotB)
  check(lit > 300, 'frame has non-background pixels', `${lit} px`)
  check(changed > 100, 'frame animates between screenshots', `${changed} px changed`)
  writeFileSync(screenshotPath, Buffer.from(shotB.split(',')[1], 'base64'))
  console.log(`Screenshot: ${screenshotPath}`)

  // Commands.
  await chrome.evaluate('send({ type: "pause" })')
  await sleep(400)
  const paused1 = await chrome.evaluate('lastTime().time')
  await sleep(500)
  const paused2 = await chrome.evaluate('lastTime().time')
  check(paused1 === paused2, 'pause holds the time', `${paused1} -> ${paused2}`)
  await chrome.evaluate('send({ type: "play" })')
  await sleep(500)
  check((await chrome.evaluate('lastTime().time')) > paused2, 'play resumes the time')
  await chrome.evaluate('send({ type: "seek", time: 1 })')
  await sleep(400)
  const seeked = await chrome.evaluate('lastTime()')
  check(Math.abs(seeked.time - 1) < 0.15 && seeked.particles > 0, 'seek jumps to 1 s with particles', JSON.stringify(seeked))
  await chrome.evaluate('send({ type: "restart" })')
  await sleep(600)
  const restarted = await chrome.evaluate('lastTime()')
  check(restarted.time < 1.5, 'restart goes back to the start', JSON.stringify(restarted))
  await sleep(2000)
  const before = (await chrome.evaluate('lastTime()')).particles
  await chrome.evaluate('send({ type: "fire" })')
  await sleep(1200)
  const fired = (await chrome.evaluate('lastTime()')).particles
  check(fired > before, 'fire adds particles on top', `${before} -> ${fired}`)
  await chrome.evaluate('send({ type: "dispose" })')
  await viewTests(chrome)


  // One emitter per shader, one after another.
  const results = {}
  const selected = option('--only')?.split(',')
  for (const c of CASES.filter((x) => !selected || selected.includes(x.id))) {
    const mark = chrome.logs.length
    const r = await runCase(chrome, c)
    results[c.id] = r
    const errors = r.events.filter((e) => e.type === 'error')
    if (r.lit <= 300 || errors.length > 0) for (const l of chrome.logs.slice(mark).filter((x) => x.level !== 'info' && !/tracing|404/.test(x.text)).slice(0, 6)) console.log(`      log ${l.level}: ${l.text}`)
    const unsupported = r.events.filter((e) => e.type === 'unsupported')
    const wanted = c.unsupported ?? []
    check(r.loaded?.type === 'loaded' && errors.length === 0, `${c.label}: loads without error`, JSON.stringify(errors).slice(0, 200))
    check(r.lit > 300, `${c.label}: draws non-background pixels`, `${r.lit} px, mean ${r.mean}, peak ${r.peak}, rgb ${r.rgb}`)
    check(
      unsupported.length === wanted.length && wanted.every((re) => unsupported.some((u) => re.test(u.detail))),
      `${c.label}: unsupported events are ${wanted.length === 0 ? 'absent' : 'the expected ones'}`,
      JSON.stringify(unsupported).slice(0, 200)
    )
  }
  const sheet = await chrome.evaluate(`sheet(${JSON.stringify(Object.entries(results).map(([id, r]) => ({ label: CASES.find((c) => c.id === id).label, shot: r.shot })))})`)
  writeFileSync(sheetPath, Buffer.from(sheet.split(',')[1], 'base64'))
  console.log(`Shader sheet: ${sheetPath}`)

  if (!selected) {
    // Additive and alpha blending must look different on the same texture and colour.
    const add = results['legacy-additive']
    const alpha = results['legacy-alpha']
    check(add.peak > alpha.peak + 30 && add.mean > alpha.mean + 15, 'additive is brighter than alpha blended',
      `additive mean ${add.mean} peak ${add.peak} rgb ${add.rgb}; alpha mean ${alpha.mean} peak ${alpha.peak} rgb ${alpha.rgb}`)
    const urpAdd = results['urp-additive']
    const urpAlpha = results['urp-alpha']
    check(urpAdd.peak > urpAlpha.peak + 30, 'URP additive is brighter than URP alpha', `additive peak ${urpAdd.peak}; alpha peak ${urpAlpha.peak}`)
    const multiply = results['legacy-multiply']
    check(multiply.mean < 3 * 26, 'multiply darkens the background', `mean ${multiply.mean}, background sum ${BACKGROUND[0] + BACKGROUND[1] + BACKGROUND[2]}`)

    // Texture upload time: a 1024x1024 texture against a 64x64 one, from the load command to the first drawn frame.
    const small = await chrome.evaluate(`timedLoad(${JSON.stringify(CASES[2])})`)
    const large = await chrome.evaluate(`timedLoad(${JSON.stringify({ ...CASES[2], size: 1024 })})`)
    check(small.frame && large.frame, 'textures of both sizes reach a drawn frame', JSON.stringify({ small, large }).slice(0, 300))
    console.log(
      `Texture timing: 64x64 host ${small.loaded?.textureMs?.toFixed(1)} ms, to loaded ${small.toLoadedMs.toFixed(0)} ms, to first frame ${small.toFrameMs.toFixed(0)} ms; ` +
        `1024x1024 host ${large.loaded?.textureMs?.toFixed(1)} ms, to loaded ${large.toLoadedMs.toFixed(0)} ms, to first frame ${large.toFrameMs.toFixed(0)} ms`
    )
    check(large.toFrameMs < 3000, '1024x1024 texture draws within 3 s of the load command', `${large.toFrameMs.toFixed(0)} ms`)
  }

  if (!selected) {
    const mode = flag('--linear') ? 'linear' : 'gamma'
    await moduleTests(chrome)
    const fixture = loadFixture()
    if (fixture) await prefabTests(chrome, fixture, mode)
    else console.log('SKIP  real prefabs: the sample set is not in References/LunaParticlePreview')
    const bg = await emptyFramePixel(chrome)
    check(bg.every((v, k) => Math.abs(v - BACKGROUND[k]) <= 6), `colour space (${mode}): the empty frame is the camera background`, JSON.stringify(bg))
    if (!flag('--linear')) {
      // The same prefabs in a host that was started in linear colour space, as a linear project draws.
      const linearReady = await openHarness(chrome, base, true)
      check(linearReady?.type === 'ready', 'colour space (linear): the host starts in linear space', JSON.stringify(linearReady))
      if (linearReady?.type === 'ready') {
        const r = await runCase(chrome, CASES.find((x) => x.id === 'legacy-alpha'))
        check(r.loaded?.type === 'loaded' && r.lit > 300, 'colour space (linear): an effect draws', `${r.lit} px, mean ${r.mean}`)
        if (fixture) await prefabTests(chrome, fixture, 'linear')
        const bgLinear = await emptyFramePixel(chrome)
        check(bgLinear.every((v, k) => Math.abs(v - BACKGROUND[k]) <= 6), 'colour space (linear): the empty frame is the camera background', JSON.stringify(bgLinear))
      }
    }
  }

  if (option('--eval')) console.log('eval:', JSON.stringify(await chrome.evaluate(option('--eval'))))

  // Errors and policy.
  const errors = await chrome.evaluate('events.filter((e) => e.type === "error")')
  check(errors.length === 0, 'no error events', JSON.stringify(errors).slice(0, 300))
  const csp = chrome.logs.filter((l) => /content security policy|refused to/i.test(l.text))
  check(csp.length === 0, 'no CSP violation (the host runs without unsafe-eval)', JSON.stringify(csp).slice(0, 300))
  await chrome.evaluate('injectInlineScript()')
  await sleep(300)
  check(
    chrome.logs.some((l) => /content security policy|refused to/i.test(l.text)),
    'CSP check works (an inline script in the host page is reported)'
  )
  const exceptions = chrome.logs.filter((l) => l.level === 'exception')
  check(exceptions.length === 0, 'no uncaught exception', JSON.stringify(exceptions).slice(0, 300))
}

try {
  await run()
} catch (error) {
  console.error(error instanceof Error ? error.message : error)
  failures.push('script error')
} finally {
  cleanup()
}
console.log(failures.length === 0 ? 'Luna smoke test passed.' : `Luna smoke test FAILED: ${failures.join('; ')}`)
process.exit(failures.length === 0 ? 0 : 1)
