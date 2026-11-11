// `window.api` for the Tauri shell: `AssetHubApi` over the
// bindings that tauri-specta generates (`bindings.ts`). The renderer's 143 call sites do not change.
//
// The bindings answer `{ status, data | error }`. This file turns that back into today's
// `IpcResult<T>`, and maps what the framework itself raises (a plain string for an unknown command,
// bad arguments or a denied permission, or a real `Error`) to an `IpcResult` error.

import { invoke } from '@tauri-apps/api/core'
import type { AssetHubApi, ExportRequest, IpcError, IpcResult, UpdateInfo } from '@shared/ipc'
import { commands, events } from './bindings'
import type * as Wire from './bindings'
import type { DependencyReport, FactKind } from '@shared/types'
import { reviveTangents } from './tangents'

type Outcome = { status: 'ok'; data: unknown } | { status: 'error'; error: unknown }

/** An error value of the shape the page shows (`errors.<code>`); anything else becomes `unknown`. */
function toIpcError(raised: unknown): IpcError {
  if (typeof raised === 'string') return { code: 'unknown', message: raised }
  if (raised instanceof Error) return { code: 'unknown', message: raised.message }
  const e = raised as Partial<IpcError> | null
  if (e && typeof e === 'object' && typeof e.code === 'string' && typeof e.message === 'string') return e as IpcError
  return { code: 'unknown', message: String(raised) }
}

/**
 * The result of a command as an `IpcResult<T>`. `T` is the type `AssetHubApi` states for that call:
 * the wire types are the same shapes except that an absent value may read `null`, and the checks at
 * the end of this file keep the two from drifting apart.
 */
async function result<T>(call: Promise<Outcome>): Promise<IpcResult<T>> {
  try {
    const outcome = await call
    return outcome.status === 'ok' ? { ok: true, data: outcome.data as T } : { ok: false, error: toIpcError(outcome.error) }
  } catch (raised) {
    return { ok: false, error: toIpcError(raised) }
  }
}

/** For the calls that `AssetHubApi` does not wrap in `IpcResult`: a failure rejects. */
async function value<T>(call: Promise<Outcome>): Promise<T> {
  const r = await result<T>(call)
  if (!r.ok) throw new Error(r.error.message)
  return r.data
}

/** A call that answers `null` on success and has no value to return. */
async function done(call: Promise<Outcome>): Promise<void> {
  await value<null>(call)
}

/** `{ ok: true } | { ok: false, message }`: a failed call is the second shape. */
async function okMessage(call: Promise<Outcome>): Promise<{ ok: true } | { ok: false; message: string }> {
  const r = await result<{ ok: boolean; message?: string }>(call)
  if (!r.ok) return { ok: false, message: r.error.message }
  return r.data.ok ? { ok: true } : { ok: false, message: r.data.message ?? '' }
}

/**
 * Listens to an event; the returned function stops listening, also when it is called before the
 * listener has been registered.
 */
function on<P>(event: { listen: (callback: (e: { payload: unknown }) => void) => Promise<() => void> }, listener: (payload: P) => void): () => void {
  // The wire payload reads `number | null` where the contract says `number` (a number that is not
  // finite is the only way to get null), so the payload is taken as the contract's type here.
  const registered = event.listen((e) => listener(e.payload as P))
  return () => void registered.then((stop) => stop(), () => undefined)
}

/** `undefined` does not survive JSON, so a setting the page clears is sent as `null`. */
function withNulls(patch: object): { [key: string]: unknown } {
  return Object.fromEntries(Object.entries(patch).map(([key, v]) => [key, v === undefined ? null : v]))
}

/** Sends raw bytes ahead of a command and answers with the id the command takes them by. */
export const stash = (bytes: Uint8Array): Promise<string> => invoke<string>('stash_bytes', bytes)

/** The request as the backend takes it: previews are bytes sent beforehand, passed by id. */
async function toWireRequest(request: ExportRequest): Promise<Wire.ExportRequest_Deserialize> {
  const entries = await Promise.all(Object.entries(request.previews).map(async ([path, bytes]) => [path, await stash(bytes)] as const))
  return { ...request, previews: Object.fromEntries(entries) }
}

/** Runs `call` with the wire request, and answers a failure to send the previews as an error result. */
async function withRequest<T>(request: ExportRequest, call: (wire: Wire.ExportRequest_Deserialize) => Promise<Outcome>): Promise<IpcResult<T>> {
  try {
    return await result<T>(call(await toWireRequest(request)))
  } catch (raised) {
    return { ok: false, error: toIpcError(raised) }
  }
}

export const api: AssetHubApi = {
  getSettings: () => value(commands.getSettings()),
  setSettings: (patch) => value(commands.setSettings(withNulls(patch))),
  chooseFolder: () => value(commands.chooseFolder()),
  isUnityProject: (path) => value(commands.isUnityProject(path)),
  takeDeepLink: () => value(commands.takeDeepLink()),
  onDeepLink: (listener) => on(events.deepLink, () => listener()),
  openExternal: (url) => done(commands.openExternal(url)),
  auth: {
    status: () => result(commands.authStatus()),
    signInWithToken: (token) => result(commands.authSignInWithToken(token)),
    signInWithGh: () => result(commands.authSignInWithGh()),
    ghWebLogin: () => result(commands.authGhWebLogin()),
    cancelGhWebLogin: () => done(commands.authCancelGhWebLogin()),
    onGhLoginPrompt: (listener) => on(events.ghLoginPrompt, listener),
    signOut: (options) => result(commands.authSignOut(options ? withNulls(options) : null))
  },
  tools: {
    installGh: () => result(commands.toolsInstallGh()),
    onInstallProgress: (listener) => on(events.toolsInstallProgress, listener)
  },
  source: {
    listRepos: () => result(commands.sourceListRepos()),
    listBranches: (repo) => result(commands.sourceListBranches(repo)),
    listTree: (repo, branch) => result(commands.sourceListTree(repo, branch)),
    watchCheck: (repo, branch, seen) => result(commands.sourceWatchCheck(repo, branch, seen))
  },
  assets: {
    resolve: async (repo, branch, path, root, mode, background) => {
      const r = await result<DependencyReport>(commands.assetsResolve(repo, branch, path, root, mode ?? null, background ?? null))
      // The step tangents of the effects' curves arrive as null (JSON has no Infinity).
      return r.ok ? { ok: true, data: reviveTangents(r.data) } : r
    },
    readFile: async (repo, branch, path, background) => {
      const r = await result<ArrayBuffer>(commands.assetsReadFile(repo, branch, path, background ?? null))
      return r.ok ? { ok: true, data: new Uint8Array(r.data) } : r
    },
    saveFile: (repo, branch, path) => result(commands.assetsSaveFile(repo, branch, path)),
    modelBundle: async (repo, branch, path, materials, mode, glb) => {
      try {
        return await result(commands.assetsModelBundle(repo, branch, path, materials, mode, glb ? await stash(glb) : null))
      } catch (raised) {
        return { ok: false, error: toIpcError(raised) }
      }
    },
    blenderInstalled: () => value(commands.assetsBlenderInstalled()),
    findByNames: (repo, branch, names, nearPath, root) => result(commands.assetsFindByNames(repo, branch, names, nearPath, root)),
    onIndexProgress: (listener) => on(events.indexProgress, listener),
    findPrefabMaterials: (repo, branch, path, root, scope, excludedFolders) => result(commands.assetsFindPrefabMaterials(repo, branch, path, root, scope, excludedFolders)),
    onPrefabProgress: (listener) => on(events.prefabProgress, listener),
    knownAssets: (repo, branch, kind) => result(commands.assetsKnownAssets(repo, branch, kind)),
    onKnownProgress: (listener) => on<{ kind: FactKind; done: number; total: number; found?: string[] }>(events.knownProgress, ({ kind, ...progress }) => listener(kind, progress))
  },
  catalog: {
    scan: (repo, current) => result(commands.catalogScan(repo ?? null, current ? { repo: current.repo, branch: current.branch ?? null } : null)),
    pause: () => result(commands.catalogPause()),
    resume: () => result(commands.catalogResume()),
    status: () => result(commands.catalogStatus()),
    list: (repo, excludedFolders, filter) =>
      result(commands.catalogList(repo, excludedFolders, filter ? { repo: filter.repo ?? null, branch: filter.branch ?? null } : null)),
    whereIs: (sha) => result(commands.catalogWhereIs(sha)),
    onStatus: (listener) => on(events.catalogProgress, listener)
  },
  thumbs: {
    get: (keys) => value(commands.thumbsGet(keys)),
    save: (key, dataUrl) => done(commands.thumbsSave(key, dataUrl))
  },
  sharedIndex: {
    status: () => result(commands.sharedIndexStatus()),
    import: () => result(commands.sharedIndexImport()),
    export: () => result(commands.sharedIndexExport()),
    onStatus: (listener) => on(events.sharedIndexProgress, listener)
  },
  cache: {
    usage: () => result(commands.cacheUsage()),
    clear: () => result(commands.cacheClear().then((o) => (o.status === 'ok' ? { status: 'ok' as const, data: undefined } : o)))
  },
  stats: {
    get: (keys) => value(commands.statsGet(keys)),
    save: (key, summary) => value(commands.statsSave(key, summary))
  },
  exporter: {
    unityPackage: (request) => withRequest(request, (wire) => commands.exporterUnityPackage(wire)),
    unityEditors: () => value(commands.exporterUnityEditors()),
    importToUnity: (request, pid) => withRequest(request, (wire) => commands.exporterImportToUnity(wire, pid)),
    onProgress: (listener) => on(events.exportProgress, listener),
    showInFolder: (path) => done(commands.exporterShowInFolder(path))
  },
  update: {
    // `check` answers null when the release repo cannot be reached, so the version badge then shows no update pill.
    check: async () => {
      const r = await result<UpdateInfo | null>(commands.updateCheck())
      return r.ok ? r.data : null
    },
    install: (version) => okMessage(commands.updateInstall(version)),
    onProgress: (listener) => on(events.updateProgress, listener)
  },
  usage: {
    // Fire and forget: counting never gets in the way of what the user does.
    track: (event) => void commands.usageTrack(event).catch(() => undefined)
  },
  feedback: {
    send: async (report) => {
      const r = await result<{ sent: boolean; url?: string }>(commands.feedbackSend({ ...report, context: report.context ?? null }))
      return r.ok && r.data.sent && r.data.url ? { sent: true, url: r.data.url } : { sent: false }
    }
  }
}
