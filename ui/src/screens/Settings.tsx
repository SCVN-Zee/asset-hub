import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { displayName } from '@shared/displayName'
import type { AiStatus, AuthStatus, CacheUsage, InstallProgress } from '@shared/ipc'
import { CACHE_LIMITS_GB, DEFAULT_CACHE_LIMIT_GB, DEFAULT_EXPORT_LAYOUT, EXPORT_LAYOUTS, type Settings as AppSettings, type Language } from '@shared/settings'
import type { BudgetPreset, PlayableBudget } from '@shared/types'
import { formatBytes } from '../format'
import { BUDGET_KEYS, BUDGET_PRESETS, DEFAULT_BUDGET_PRESET, budgetFor } from '../playable'
import { commands, type CliInstall } from '../bindings'
import { Dropdown } from '../components/Dropdown'
import { ChevronLeftIcon } from '../components/Icons'
import { LangToggle } from '../components/LangToggle'
import { useSharedIndex } from '../useSharedIndex'
import { useStoredFlag } from '../useStoredState'

interface Props {
  settings: AppSettings
  auth: AuthStatus | undefined
  onChange(patch: Partial<AppSettings>): void
  /** `gh`: also log the account out of GitHub CLI on this Mac. */
  onSignOut(gh: boolean): void
  onBack(): void
}

const parseFolders = (text: string) =>
  text
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)

/** How each limit is shown and typed: bytes as MB or KB. */
const UNITS: Record<keyof PlayableBudget, { unit: string; scale: number }> = {
  triangles: { unit: 'tris', scale: 1 },
  textureSize: { unit: 'px', scale: 1 },
  textureBytes: { unit: 'MB', scale: 1024 * 1024 },
  fileBytes: { unit: 'MB', scale: 1024 * 1024 },
  bones: { unit: '', scale: 1 },
  audioBytes: { unit: 'KB', scale: 1024 },
  audioSeconds: { unit: 's', scale: 1 },
  fontBytes: { unit: 'KB', scale: 1024 },
  particles: { unit: '', scale: 1 }
}

/** The Unity project packages are compared with before export (read only). */
function TargetProjectSetting({ settings, onChange }: Pick<Props, 'settings' | 'onChange'>) {
  const { t } = useTranslation()
  const [error, setError] = useState(false)
  const choose = async () => {
    const folder = await window.api.chooseFolder()
    if (!folder) return
    const ok = await window.api.isUnityProject(folder)
    setError(!ok)
    if (ok) onChange({ targetProject: folder })
  }
  return (
    <section className="setting">
      <h2>
        <label htmlFor="target-project">{t('target.setting')}</label>
      </h2>
      <div className="field-row">
        <input id="target-project" className="input mono" value={settings.targetProject ?? ''} placeholder={t('target.none')} readOnly />
        <button type="button" className="btn" onClick={() => void choose()}>
          {t('settings.choose')}
        </button>
        {settings.targetProject && (
          <button type="button" className="btn" onClick={() => onChange({ targetProject: '' })}>
            {t('target.clear')}
          </button>
        )}
      </div>
      {error && <p className="warn-note">{t('target.notProject')}</p>}
    </section>
  )
}

/** Preset choice, and the limits it stands for (editable for the custom preset). */
function BudgetSetting({ settings, onChange }: Pick<Props, 'settings' | 'onChange'>) {
  const { t } = useTranslation()
  const preset = settings.budgetPreset ?? DEFAULT_BUDGET_PRESET
  const budget = budgetFor(preset, settings.customBudget)
  const setLimit = (key: keyof PlayableBudget, text: string) => {
    const value = Number(text) * UNITS[key].scale
    if (Number.isFinite(value) && value > 0 && value !== budget[key]) onChange({ customBudget: { ...settings.customBudget, [key]: value } })
  }
  return (
    <section className="setting">
      <h2>
        <label htmlFor="budget">{t('budget.preset')}</label>
      </h2>
      <div>
        <Dropdown
          id="budget"
          value={preset}
          options={BUDGET_PRESETS.map((p) => ({ value: p, label: t(`budget.presets.${p}`) }))}
          onChange={(budgetPreset: BudgetPreset) => onChange({ budgetPreset })}
          label={t('budget.preset')}
        />
      </div>
      {preset !== 'off' && (
      <dl className="stat-grid">
        {BUDGET_KEYS.map((key) => (
          <div key={`${preset}:${key}`}>
            <dt>{t(`budget.limits.${key}`)}</dt>
            <dd>
              {preset === 'custom' ? (
                <input
                  className="input budget-input"
                  type="number"
                  min={0}
                  step="any"
                  defaultValue={+(budget[key] / UNITS[key].scale).toFixed(2)}
                  onBlur={(e) => setLimit(key, e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && setLimit(key, e.currentTarget.value)}
                />
              ) : (
                +(budget[key] / UNITS[key].scale).toFixed(2)
              )}{' '}
              {UNITS[key].unit}
            </dd>
          </div>
        ))}
      </dl>
      )}
    </section>
  )
}

/**
 * The AI assistant: ChatGPT through Codex, which the app downloads once. The
 * user signs in with their own ChatGPT account in the browser; no API key.
 */
function AiSetting({ settings, onChange }: { settings: AppSettings; onChange(patch: Partial<AppSettings>): void }) {
  const { t } = useTranslation()
  const [status, setStatus] = useState<AiStatus>()
  const [progress, setProgress] = useState<InstallProgress>()
  useEffect(() => {
    void window.api.ai.status().then(setStatus)
    return window.api.ai.onStatus(setStatus)
  }, [])
  useEffect(() => window.api.ai.onInstallProgress(setProgress), [])

  const installThenConnect = async () => {
    const after = await window.api.ai.install()
    setProgress(undefined)
    if (after.state === 'signedOut') setStatus(await window.api.ai.connect())
    else setStatus(after)
  }
  const percent = progress?.stage === 'download' && progress.total ? Math.round((progress.received / progress.total) * 100) : undefined
  const state = status?.state

  return (
    <section className="setting">
      <h2>{t('ai.title')}</h2>
      {state === 'unsupported' && <span className="small">{t('ai.unsupported')}</span>}
      {state === 'connected' && <span>{t('ai.connected', { email: status?.email ?? '—', plan: status?.plan ?? '—' })}</span>}
      {state === 'connected' && (
        <label className="check">
          <input type="checkbox" checked={settings.aiPet !== false} onChange={(e) => onChange({ aiPet: e.target.checked })} />
          {t('ai.pet')}
        </label>
      )}
      {state === 'connecting' && <span className="small">{t('ai.connecting')}</span>}
      {state === 'installing' && (
        <div className="ai-install">
          <span className="small">{progress?.stage === 'verify' ? t('ai.verifying') : progress?.stage === 'extract' ? t('ai.extracting') : t('ai.downloading', { percent: percent ?? 0 })}</span>
          <progress className="progress" max={100} value={percent ?? undefined} />
        </div>
      )}
      {status?.error && state !== 'connected' && <span className="warn-text small">{t('ai.failed', { message: status.error })}</span>}
      <div className="setting-actions">
        {state === 'notInstalled' && (
          <button type="button" className="btn btn-primary" onClick={() => void installThenConnect()}>
            {t('ai.install')}
          </button>
        )}
        {(state === 'signedOut' || state === 'error') && (
          <button type="button" className="btn btn-primary" onClick={() => void window.api.ai.connect().then(setStatus)}>
            {t('ai.connect')}
          </button>
        )}
        {state === 'connecting' && (
          <>
            <button type="button" className="btn" onClick={() => void window.api.ai.connect().then(setStatus)}>
              {t('ai.reopen')}
            </button>
            <button type="button" className="btn" onClick={() => void window.api.ai.cancelConnect().then(setStatus)}>
              {t('ai.cancel')}
            </button>
          </>
        )}
        {state === 'connected' && (
          <button type="button" className="btn btn-signout" onClick={() => void window.api.ai.disconnect().then(setStatus)}>
            {t('ai.disconnect')}
          </button>
        )}
      </div>
    </section>
  )
}

export function Settings({ settings, auth, onChange, onSignOut, onBack }: Props) {
  const { t } = useTranslation()
  const [folders, setFolders] = useState(settings.excludedFolders.join(', '))
  // Kept between launches, like the other ticks.
  const [signOutGh, setSignOutGh] = useStoredFlag('assethub.signOut.gh', true)
  useEffect(() => setFolders(settings.excludedFolders.join(', ')), [settings.excludedFolders])

  const chooseCache = async () => {
    const folder = await window.api.chooseFolder()
    if (folder) onChange({ cacheDir: folder })
  }

  return (
    <div className="settings">
      <div className="settings-head">
        <button type="button" className="btn icon-btn" aria-label={t('common.back')} onClick={onBack}>
          <ChevronLeftIcon />
        </button>
        <h1>{t('settings.title')}</h1>
      </div>

      <section className="setting">
        <h2>{t('settings.account')}</h2>
        {auth?.signedIn ? (
          <>
            <span>
              {t('settings.signedInAs', { login: displayName(auth.login ?? '') })} · <span className="muted">{t(`settings.method.${auth.method}`)}</span>
            </span>
            {auth.persisted === false && <span className="muted small">{t('settings.notPersisted')}</span>}
          </>
        ) : null}
        {auth?.signedIn && auth.method === 'gh' && (
          <label className="check">
            <input type="checkbox" checked={signOutGh} onChange={(e) => setSignOutGh(e.target.checked)} />
            {t('settings.signOutGh')}
          </label>
        )}
        <div>
          <button type="button" className={auth?.signedIn ? 'btn btn-signout' : 'btn'} onClick={() => onSignOut(auth?.signedIn === true && auth.method === 'gh' && signOutGh)}>
            {auth?.signedIn ? t('settings.signOut') : t('settings.signIn')}
          </button>
        </div>
      </section>

      <AiSetting settings={settings} onChange={onChange} />

      <section className="setting">
        <h2>{t('common.language')}</h2>
        <div>
          <LangToggle value={settings.language} onChange={(language: Language) => onChange({ language })} />
        </div>
      </section>

      <section className="setting">
        <h2>{t('settings.lobby')}</h2>
        <label className="check">
          <input type="checkbox" checked={settings.skipLobby === true} onChange={(e) => onChange({ skipLobby: e.target.checked })} />
          {t('settings.skipLobby')}
        </label>
        <span className="muted small">{t('settings.skipLobbyNote')}</span>
      </section>

      <BudgetSetting settings={settings} onChange={onChange} />

      <section className="setting">
        <h2>
          <label htmlFor="export-layout">{t('export.layout')}</label>
        </h2>
        <div>
          <Dropdown
            id="export-layout"
            value={settings.exportLayout ?? DEFAULT_EXPORT_LAYOUT}
            options={EXPORT_LAYOUTS.map((l) => ({ value: l, label: t(`export.layout${l[0].toUpperCase()}${l.slice(1)}`) }))}
            onChange={(exportLayout: (typeof EXPORT_LAYOUTS)[number]) => onChange({ exportLayout })}
            label={t('export.layout')}
          />
        </div>
      </section>

      <section className="setting">
        <h2>
          <label htmlFor="excluded">{t('settings.excluded')}</label>
        </h2>
        <input
          id="excluded"
          className="input mono"
          value={folders}
          placeholder="_ThirdParty, Plugins"
          onChange={(e) => setFolders(e.target.value)}
          onBlur={() => onChange({ excludedFolders: parseFolders(folders) })}
          onKeyDown={(e) => e.key === 'Enter' && onChange({ excludedFolders: parseFolders(folders) })}
        />
      </section>

      <TargetProjectSetting settings={settings} onChange={onChange} />

      <section className="setting">
        <h2>
          <label htmlFor="cache">{t('settings.cacheDir')}</label>
        </h2>
        <div className="field-row">
          <input id="cache" className="input mono" value={settings.cacheDir} readOnly />
          <button type="button" className="btn" onClick={chooseCache}>
            {t('settings.choose')}
          </button>
        </div>
      </section>

      <CacheSetting settings={settings} onChange={onChange} />

      {auth?.signedIn && <SharedIndexSetting settings={settings} onChange={onChange} />}

      <CliSetting />

      <section className="setting">
        <h2>{t('settings.usage')}</h2>
        <label className="check">
          <input type="checkbox" checked={settings.usageStats !== false} onChange={(e) => onChange({ usageStats: e.target.checked })} />
          {t('settings.usageStats')}
        </label>
        <span className="muted small">{t('settings.usageNote')}</span>
      </section>
    </div>
  )
}

/** Links `asset-hub-cli` into `~/.local/bin` so coding agents can run it. */
function CliSetting() {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const [installed, setInstalled] = useState<CliInstall>()
  const [failed, setFailed] = useState<string>()
  const install = async () => {
    setBusy(true)
    setFailed(undefined)
    setInstalled(undefined)
    try {
      const outcome = await commands.agentCliInstall()
      if (outcome.status === 'ok') setInstalled(outcome.data)
      else setFailed(outcome.error.message)
    } catch (raised) {
      setFailed(raised instanceof Error ? raised.message : String(raised))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="setting">
      <h2>{t('settings.cliTitle')}</h2>
      <span className="muted small">{t('settings.cliNote')}</span>
      <div>
        <button type="button" className="btn" disabled={busy} onClick={() => void install()}>
          {t('settings.cliInstall')}
        </button>
      </div>
      {installed && (
        <span className="muted small mono" role="status">
          {t('settings.cliInstalled', { link: installed.link })}
        </span>
      )}
      {installed && !installed.onPath && <span className="muted small">{t('settings.cliNotOnPath')}</span>}
      {failed && (
        <span className="small" style={{ color: 'var(--warn-text)' }}>
          {t('settings.cliFailed', { error: failed })}
        </span>
      )}
    </section>
  )
}

/** How much the cache holds, its size limit, and a way to empty it. */
function CacheSetting({ settings, onChange }: Pick<Props, 'settings' | 'onChange'>) {
  const { t } = useTranslation()
  const [usage, setUsage] = useState<CacheUsage>()
  const [clearing, setClearing] = useState(false)
  const measure = () => void window.api.cache.usage().then((r) => r.ok && setUsage(r.data))
  useEffect(measure, [settings.cacheDir, settings.cacheLimitGB])
  const limit = settings.cacheLimitGB ?? DEFAULT_CACHE_LIMIT_GB
  const clear = async () => {
    setClearing(true)
    await window.api.cache.clear()
    setClearing(false)
    measure()
  }
  return (
    <section className="setting">
      <h2>
        <label htmlFor="cache-limit">{t('settings.cacheLimit')}</label>
      </h2>
      <div className="field-row">
        <Dropdown
          id="cache-limit"
          value={String(limit)}
          options={CACHE_LIMITS_GB.map((gb) => ({ value: String(gb), label: gb ? `${gb} GB` : t('settings.cacheNoLimit') }))}
          onChange={(v: string) => onChange({ cacheLimitGB: Number(v) })}
          label={t('settings.cacheLimit')}
        />
        <button type="button" className="btn" onClick={() => void clear()} disabled={clearing}>
          {t('settings.cacheClear')}
        </button>
      </div>
      {usage && (
        <span className="muted small" role="status">
          {t('settings.cacheUsage', { downloads: formatBytes(usage.downloads), thumbs: formatBytes(usage.thumbs), other: formatBytes(usage.other) })}
        </span>
      )}
    </section>
  )
}

/** The team's shared index (GitHub repo, or a folder): what one app has listed and drawn, everyone takes in, so cross-branch search starts full. */
function SharedIndexSetting({ settings, onChange }: Pick<Props, 'settings' | 'onChange'>) {
  const { t, i18n } = useTranslation()
  const status = useSharedIndex()
  const [failed, setFailed] = useState<string>()
  const busy = status?.state === 'importing' || status?.state === 'exporting'
  const choose = async () => {
    const folder = await window.api.chooseFolder()
    if (folder) onChange({ sharedIndexDir: folder })
  }
  const act = async (fn: () => ReturnType<typeof window.api.sharedIndex.import>) => {
    setFailed(undefined)
    const r = await fn()
    if (!r.ok) setFailed(r.error.message)
  }
  const when = (iso: string) => new Date(iso).toLocaleString(i18n.language)
  const m = status?.manifest
  const error = failed ?? (status?.state === 'error' ? status.error : undefined)
  return (
    <section className="setting">
      <h2>
        <label htmlFor="shared-index">{t('settings.sharedIndex')}</label>
      </h2>
      <div className="field-row">
        <input
          id="shared-index"
          className="input mono"
          value={settings.sharedIndexDir ?? ''}
          placeholder={status?.via === 'github' ? t('settings.sharedIndexNone', { repo: status.repo }) : t('settings.sharedIndexNoGit')}
          readOnly
        />
        <button type="button" className="btn" onClick={() => void choose()}>
          {t('settings.choose')}
        </button>
        {settings.sharedIndexDir && (
          <button type="button" className="btn" onClick={() => onChange({ sharedIndexDir: '' })}>
            {t('settings.sharedIndexStop')}
          </button>
        )}
      </div>
      {(settings.sharedIndexDir || status?.via === 'github') && (
        <>
          {status?.via === 'github' && <span className="muted small">{t(status.readOnly ? 'settings.sharedIndexReadOnly' : 'settings.sharedIndexAuto', { repo: status.repo })}</span>}
          <span className="muted small" role="status">
            {m
              ? t('settings.sharedIndexThere', { when: when(m.exportedAt), who: m.exportedBy ?? '?', repos: m.repos.length, thumbs: m.thumbs })
              : t('settings.sharedIndexEmpty')}
            {status?.importedAt && ` · ${t('settings.sharedIndexTaken', { when: when(status.importedAt) })}`}
          </span>
          <div className="field-row">
            <button type="button" className="btn" disabled={busy || !m} onClick={() => void act(window.api.sharedIndex.import)}>
              {status?.state === 'importing' ? t('settings.sharedIndexImporting') : t('settings.sharedIndexImport')}
            </button>
            <button type="button" className="btn" disabled={busy || !!status?.readOnly} onClick={() => void act(window.api.sharedIndex.export)}>
              {status?.state === 'exporting' ? t('settings.sharedIndexExporting') : t('settings.sharedIndexExport')}
            </button>
          </div>
          {status?.state === 'idle' && status.last && (
            <span className="muted small">
              {t(status.lastAction === 'exported' ? 'settings.sharedIndexExported' : 'settings.sharedIndexImported', { ...status.last })}
            </span>
          )}
          {error && <span className="small" style={{ color: 'var(--warn-text)' }}>{t('settings.sharedIndexFailed', { error })}</span>}
        </>
      )}
    </section>
  )
}
