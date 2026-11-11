import type { BundleMaterial, CatalogListing, CatalogLocation, DependencyReport, FactKind, PrefabMaterialsResult, PrefabScope, RepoRef, SourceErrorCode, StatsSummary, TreeEntry } from './types'
import type { Settings } from './settings'
import type { AiToolResult } from './aiTools'
import type { FeedbackReport, FeedbackResult, UsageEvent } from './usage'

/** Error codes that can cross IPC; the UI shows `errors.<code>`. */
export type AppErrorCode = SourceErrorCode | 'ghNotFound' | 'ghNotLoggedIn' | 'emptyToken' | 'ghInstallFailed'

export type InstallProgress =
  | { stage: 'download'; received: number; total: number }
  | { stage: 'verify' }
  | { stage: 'extract' }
  /** An update the app started by itself failed; the user can try again from the button. */
  | { stage: 'failed'; message: string }

/** The AI assistant's connection to ChatGPT (through Codex, installed by the app). */
export interface AiStatus {
  state: 'unsupported' | 'notInstalled' | 'installing' | 'signedOut' | 'connecting' | 'connected' | 'error'
  /** The ChatGPT account, when connected. */
  email?: string
  /** Its plan, e.g. "team". */
  plan?: string
  /** Why the last step failed. */
  error?: string
}

/** What the assistant's thread streams to the chat: answer text as it is written, then the end of the turn. */
export type AiChatEvent =
  | { type: 'delta'; itemId: string; text: string }
  /** It is using one of the hub's tools. */
  | { type: 'tool'; tool: string }
  | { type: 'done'; status: 'completed' | 'interrupted' | 'failed'; error?: string }
  | { type: 'error'; message: string }

/** This version and, when the release repo has a later one, that one. */
export interface UpdateInfo {
  current: string
  latest?: string
  /** The app can replace itself (a packaged app in a folder this account may write to). */
  canInstall: boolean
  /** The release page, for installing by hand. */
  page: string
}

export interface ExportRequest {
  repo: string
  branch: string
  root: string
  /** Repo paths of the models to export; their dependencies are added automatically. */
  paths: string[]
  layout: 'source' | 'grouped' | 'byType'
  /** Leave out textures that were only matched by file name. */
  skipByName: boolean
  /** More repo paths to include, e.g. materials found on prefabs (with .meta). */
  extraFiles: string[]
  /** PNG previews keyed by repo path. */
  previews: Record<string, Uint8Array>
  /** Default file name in the save dialog. */
  suggestedName: string
  /** Unity project the package is meant for (read only): files it already has are left out or reported. */
  targetProject?: string
  /** Leave out files the project has under another GUID, pointing references at its copy. */
  reuseExisting?: boolean
  /** Remove the project's own scripts (.cs, .dll in the repo) from prefabs and leave those files out. */
  stripScripts?: boolean
}

/** What the package does to the target project. Paths of the project are "Assets/…". */
export interface TargetReport {
  project: string
  /** Already there with the same GUID and content: left out. */
  same: number
  /** Same content under another GUID: left out, references point at the project's file. */
  reused: Array<{ path: string; existing: string }>
  /** Same GUID, other content: Unity will replace the project's file. */
  overwrite: Array<{ path: string; existing: string }>
  /** Path taken by another asset: Unity will import it as "Name 1". */
  renamed: Array<{ path: string; existing: string }>
  /** Shaders the project has under the same name (other GUID): left out, materials use the project's. */
  shaderByName: Array<{ path: string; existing: string }>
  /** Shaders the project has under the same GUID with other content: left out, the project's version is kept. */
  shaderKept: Array<{ path: string; existing: string }>
  /** Shaders imported although the project has one by the same name. */
  shaderClash: Array<{ path: string; existing: string }>
  /** Materials switched to a project shader by `.assethub/shader-mapping.json` ("existing" = "from → to"). */
  shaderMapped: Array<{ path: string; existing: string }>
  /** URP materials switched to built-in shaders, the project having no URP ("existing" = "from → to"). */
  urpConverted: Array<{ path: string; existing: string }>
  /** Exported assets using package scripts the project lacks ("existing" = their GUIDs, shortened): "Missing Script" after import. */
  missingScripts: Array<{ path: string; existing: string }>
  /** Project shaders packed materials use that neither the package nor the project has (path: shader name, existing: its materials): they would turn magenta. */
  missingShaders: Array<{ path: string; existing: string }>
  /** Problems with the mapping file (rules skipped). */
  mappingErrors: string[]
  /** URP versions of the source and target projects, when they differ. */
  urp?: { source: string; target?: string; /** Materials that still need URP after conversion. */ needed?: number }
}

export interface ExportResult {
  canceled: boolean
  path?: string
  assets?: number
  bytes?: number
  skipped?: Array<{ path: string; reason: 'noMeta' | 'noGuid' | 'duplicateGuid' }>
  target?: TargetReport
  /** Opened in this Unity project (Import into Unity) instead of saved. */
  unity?: string
  /** Script components removed from prefabs (`stripScripts`). */
  scriptsRemoved?: number
}

/** A Unity editor open on this Mac, with its project. */
export interface RunningUnity {
  pid: number
  projectPath: string
  /** The project folder's name. */
  project: string
  /** Unity's productName, when it differs from the folder. */
  product?: string
  version?: string
}

export type ExportProgress = { stage: 'resolve' | 'download' | 'write'; done: number; total: number }

/** Background indexing of every branch (Phase 4.5). */
export interface CatalogStatus {
  state: 'idle' | 'running' | 'paused' | 'error'
  /** Branches listed in this run, out of `total`. */
  done: number
  total: number
  /** repo@branch being listed. */
  current?: string
  error?: IpcError
}

/** Bytes the cache folder holds, by kind. */
export interface CacheUsage {
  /** Downloaded files, listings and indexes (count toward the size limit). */
  downloads: number
  thumbs: number
  /** Git clones, measurements and the rest. */
  other: number
}

/** What a shared index folder holds (`manifest.json`). */
export interface SharedIndexManifest {
  format: 1
  /** ISO time it was written; a newer one is taken in. */
  exportedAt: string
  /** GitHub login of who wrote it. */
  exportedBy?: string
  /** Asset types indexed (handler ids); an index made for other types is left alone. */
  types: string
  repos: Array<{ repo: string; branches: number; commits: number }>
  thumbs: number
  stats: number
}

/** Counts of what an import took in, or an export wrote. */
export interface SharedIndexResult {
  commits: number
  facts: number
  stats: number
  thumbs: number
}

export interface SharedIndexStatus {
  state: 'idle' | 'importing' | 'exporting' | 'error'
  error?: string
  /** Shared through the GitHub repo (no folder picked) or a folder. */
  via?: 'github' | 'folder'
  /** The GitHub repo, `via: 'github'`. */
  repo?: string
  /** This account can read the GitHub repo but not write to it. */
  readOnly?: boolean
  /** In the shared folder now. */
  manifest?: SharedIndexManifest
  /** `exportedAt` of the index taken in last. */
  importedAt?: string
  last?: SharedIndexResult
  lastAction?: 'imported' | 'exported'
}

export interface IpcError {
  code: AppErrorCode
  message: string
  resetAt?: number
  ssoUrl?: string
}

/** Errors are returned as values: a thrown error would reach the page as a plain string. */
export type IpcResult<T> = { ok: true; data: T } | { ok: false; error: IpcError }

export type AuthMethod = 'gh' | 'pat'

export interface AuthStatus {
  signedIn: boolean
  login?: string
  method?: AuthMethod
  /** False when the OS keychain was unavailable and the token lives only in memory. */
  persisted?: boolean
}

/** API exposed to the renderer as `window.api` by the preload script. */
export interface AssetHubApi {
  getSettings(): Promise<Settings>
  setSettings(patch: Partial<Settings>): Promise<Settings>
  /** Opens a folder picker; resolves to null when cancelled. */
  chooseFolder(): Promise<string | null>
  /** The folder has Assets and ProjectSettings (read only). */
  isUnityProject(path: string): Promise<boolean>
  /** An assethub:// link that opened the app, if any (asked once the UI can open it). */
  takeDeepLink(): Promise<string | null>
  /** A link arrived while the app runs: take it with takeDeepLink. */
  onDeepLink(listener: () => void): () => void
  /** Opens an https URL in the system browser. */
  openExternal(url: string): Promise<void>
  auth: {
    status(): Promise<IpcResult<AuthStatus>>
    signInWithToken(token: string): Promise<IpcResult<AuthStatus>>
    signInWithGh(): Promise<IpcResult<AuthStatus>>
    /** Logs gh in through the browser (device code), then signs in with it. */
    ghWebLogin(): Promise<IpcResult<AuthStatus>>
    cancelGhWebLogin(): Promise<void>
    /** Fires with the one-time code while ghWebLogin waits; returns an unsubscribe function. */
    onGhLoginPrompt(listener: (prompt: { code: string; url: string }) => void): () => void
    /** Forgets the app's sign-in; `gh` also logs the account out of GitHub CLI on this Mac. */
    signOut(options?: { gh?: boolean }): Promise<IpcResult<AuthStatus>>
  }
  tools: {
    /** Downloads and verifies GitHub CLI into the app's own folder. */
    installGh(): Promise<IpcResult<{ version: string }>>
    onInstallProgress(listener: (progress: InstallProgress) => void): () => void
  }
  source: {
    listRepos(): Promise<IpcResult<RepoRef[]>>
    listBranches(repo: string): Promise<IpcResult<string[]>>
    listTree(repo: string, branch: string): Promise<IpcResult<TreeEntry[]>>
    /** A watched branch: its tip commit, and the asset files added since `seen` (repo paths). */
    watchCheck(repo: string, branch: string, seen: string): Promise<IpcResult<{ head: string; added: string[]; more: boolean }>>
  }
  assets: {
    /** Materials, textures and shaders of one asset. Repo paths throughout. */
    /**
     * `instant` answers with what is known already, `quick` also searches nearby
     * files (both may be `partial`); `full` waits for the branch's GUID index.
     */
    /** `background` (grid thumbnails): not counted as what the user waits on, so other background reads are not held up by it. */
    resolve(repo: string, branch: string, path: string, root: string, mode?: 'instant' | 'quick' | 'full', background?: boolean): Promise<IpcResult<DependencyReport>>
    /** `background` reads (grid thumbnails) wait while the user is loading something. */
    readFile(repo: string, branch: string, path: string, background?: boolean): Promise<IpcResult<Uint8Array>>
    findByNames(repo: string, branch: string, names: string[], nearPath: string, root: string): Promise<IpcResult<Record<string, string | null>>>
    /** Progress of the one-off GUID index build for a commit. */
    onIndexProgress(listener: (progress: { done: number; total: number }) => void): () => void
    /** Materials that prefabs put on this model; `near` reads only related prefabs. */
    findPrefabMaterials(repo: string, branch: string, path: string, root: string, scope: PrefabScope, excludedFolders: string[]): Promise<IpcResult<PrefabMaterialsResult>>
    /** Asks where to save, then writes the file itself (real contents for LFS files). */
    saveFile(repo: string, branch: string, path: string): Promise<IpcResult<{ canceled: boolean; path?: string; bytes?: number }>>
    /**
     * A model with its textures and a script that rebuilds its materials in Blender: `zip` asks where to save
     * one .zip; `blender` writes them to a temporary folder and opens them in Blender. A prefab passes the
     * GLB it was exported as (`glb`, materials named by .mat GUID); an FBX is read from the repo.
     */
    modelBundle(repo: string, branch: string, path: string, materials: BundleMaterial[], mode: 'zip' | 'blender', glb?: Uint8Array): Promise<IpcResult<{ canceled: boolean; path?: string; bytes?: number; textures?: number }>>
    /** Blender is installed (where the model bundle can open it). */
    blenderInstalled(): Promise<boolean>
    /** Bytes read so far by findPrefabMaterials. */
    onPrefabProgress(listener: (progress: { done: number; total: number }) => void): () => void
    /** Blob SHAs of the branch's assets of a kind known by reading them (effects, sprites, TextMeshPro fonts); reads files not seen before. */
    knownAssets(repo: string, branch: string, kind: FactKind): Promise<IpcResult<string[]>>
    /** Bytes read so far by knownAssets, with what was just found. */
    onKnownProgress(listener: (kind: FactKind, progress: { done: number; total: number; found?: string[] }) => void): () => void
  }
  catalog: {
    /** Indexes every branch of `repo`, or of every repo when it is omitted; returns at once and runs in the background. */
    /** `current` (the open repo / branch) is indexed first. */
    scan(repo?: string, current?: { repo: string; branch?: string }): Promise<IpcResult<CatalogStatus>>
    pause(): Promise<IpcResult<CatalogStatus>>
    resume(): Promise<IpcResult<CatalogStatus>>
    status(): Promise<IpcResult<CatalogStatus>>
    /**
     * Indexed assets of `repo` (every indexed repo when null), identical files grouped, on `filter`'s
     * repo and branch, with the filters' choices. Each location carries its first few branches only.
     */
    list(repo: string | null, excludedFolders: string[], filter?: { repo?: string; branch?: string }): Promise<IpcResult<CatalogListing>>
    /** Every repo, branch and path holding a file with this blob SHA. */
    whereIs(sha: string): Promise<IpcResult<CatalogLocation[]>>
    onStatus(listener: (status: CatalogStatus) => void): () => void
  }
  thumbs: {
    /** URLs of the thumbnails rendered earlier, for those of `keys` that have one. */
    get(keys: string[]): Promise<Record<string, string>>
    save(key: string, dataUrl: string): Promise<void>
  }
  /** A catalog, facts, measurements and thumbnails shared through a folder (`Settings.sharedIndexDir`). */
  sharedIndex: {
    status(): Promise<IpcResult<SharedIndexStatus>>
    /** Takes in the folder's index now, even one taken in before. */
    import(): Promise<IpcResult<SharedIndexStatus>>
    /** Writes this app's index there (after taking in what is already there). */
    export(): Promise<IpcResult<SharedIndexStatus>>
    onStatus(listener: (status: SharedIndexStatus) => void): () => void
  }
  cache: {
    /** Bytes the cache folder holds, by kind. */
    usage(): Promise<IpcResult<CacheUsage>>
    /** Empties downloaded files (thumbnails and measurements stay). */
    clear(): Promise<IpcResult<void>>
  }
  /** Model measurements by blob SHA, for ranking results without opening them. */
  stats: {
    get(keys: string[]): Promise<Record<string, StatsSummary>>
    /** Answers what is kept for the key now (a thumbnail's reading never replaces a viewer's); null for an invalid one. */
    save(key: string, summary: StatsSummary): Promise<StatsSummary | null>
  }
  exporter: {
    /** Asks where to save, then builds the .unitypackage. */
    unityPackage(request: ExportRequest): Promise<IpcResult<ExportResult>>
    /** The Unity editors open right now (several projects can be open at once). */
    unityEditors(): Promise<RunningUnity[]>
    /** Builds the package (compared with that editor's project) and opens it in that editor, which shows its import dialog. */
    importToUnity(request: ExportRequest, pid: number): Promise<IpcResult<ExportResult & { unity: string }>>
    onProgress(listener: (progress: ExportProgress) => void): () => void
    showInFolder(path: string): Promise<void>
  }
  ai: {
    status(): Promise<AiStatus>
    /** Downloads the pinned Codex (about 125 MB, checked against its hashes). */
    install(): Promise<AiStatus>
    onInstallProgress(listener: (progress: InstallProgress) => void): () => void
    /** Opens ChatGPT's sign-in page; the status turns `connected` once the user has signed in. */
    connect(): Promise<AiStatus>
    cancelConnect(): Promise<AiStatus>
    disconnect(): Promise<AiStatus>
    onStatus(listener: (status: AiStatus) => void): () => void
    /**
     * Sends a message in the current conversation (`language`: the app's, for the answer), with up to 4 attached
     * images as data URLs (PNG, JPEG, WebP, GIF); the answer streams through onChat.
     */
    chatSend(text: string, language: string, images?: string[]): Promise<{ ok: true } | { ok: false; message: string }>
    chatStop(): Promise<void>
    /** Starts a new conversation. */
    chatReset(): Promise<void>
    onChat(listener: (event: AiChatEvent) => void): () => void
    /** The hub runs the pet's tools: a call to act on, answered with toolResult. */
    onToolCall(listener: (call: { callId: string; tool: string; args: unknown }) => void): () => void
    toolResult(callId: string, result: AiToolResult): Promise<void>
    /** A wrong answer, kept on this Mac (userData/ai-feedback) to test the pet against later. */
    feedback(report: { messages: Array<{ role: string; text: string }>; tools: string[] }): Promise<void>
  }
  update: {
    /** Asks the release repo for its latest version; null when it cannot be reached. */
    check(): Promise<UpdateInfo | null>
    /** Downloads, verifies and installs `version`, then restarts the app (resolves only on failure). */
    install(version: string): Promise<{ ok: false; message: string } | { ok: true }>
    onProgress(listener: (progress: InstallProgress) => void): () => void
  }
  usage: {
    /** Counted in this session's usage notes (fire and forget). */
    track(event: UsageEvent): void
  }
  feedback: {
    /** Opens an issue in the feedback repo; offline or without access it is kept and sent later. */
    send(report: FeedbackReport): Promise<FeedbackResult>
  }
}
