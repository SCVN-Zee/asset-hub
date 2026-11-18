import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { displayName } from '@shared/displayName'
import type { AuthStatus, CacheUsage } from '@shared/ipc'
import { DEFAULT_EXPORT_LAYOUT, EXPORT_LAYOUTS, type Settings as AppSettings, type Language } from '@shared/settings'
import type { BudgetPreset, PlayableBudget } from '@shared/types'
import { formatBytes } from '../format'
import { BUDGET_KEYS, BUDGET_PRESETS, DEFAULT_BUDGET_PRESET, budgetFor } from '../playable'
import { commands, type CliInstall, type McpClient, type McpStatus } from '../bindings'
import { Dropdown } from '../components/Dropdown'
import { ExcludedFoldersDialog } from '../components/ExcludedFoldersDialog'
import { CacheScope } from '../components/CacheScope'
import { CloseIcon } from '../components/Icons'
import { LangToggle } from '../components/LangToggle'
import { useStoredFlag } from '../useStoredState'

interface Props {
  settings: AppSettings
  auth: AuthStatus | undefined
  /** Settles once the app has saved the patch. */
  onChange(patch: Partial<AppSettings>): void | Promise<unknown>
  /** `gh`: also log the account out of GitHub CLI on this Mac. */
  onSignOut(gh: boolean): void
  onBack(): void
}

/** The patterns in a short line: the first few. */
const SUMMARY_SHOWN = 3
const summarize = (patterns: string[]) => {
  const shown = patterns.slice(0, SUMMARY_SHOWN).join(', ')
  return patterns.length > SUMMARY_SHOWN ? `${shown}, …` : shown
}

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
        <input id="target-project" className="input mono prose-empty" value={settings.targetProject ?? ''} placeholder={t('target.none')} readOnly />
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

export function Settings({ settings, auth, onChange, onSignOut, onBack }: Props) {
  const { t } = useTranslation()
  const [editingFolders, setEditingFolders] = useState(false)
  // Kept between launches, like the other ticks.
  const [signOutGh, setSignOutGh] = useStoredFlag('assethub.signOut.gh', true)

  // A panel over the hub: focus moves into it, Escape closes it, and focus goes back to where it came from.
  const sheet = useRef<HTMLDivElement>(null)
  const back = useRef(onBack)
  back.current = onBack
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : undefined
    sheet.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      // A dropdown inside the panel uses Escape to close itself first.
      if (e.key === 'Escape' && !e.defaultPrevented) back.current()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
      if (before?.isConnected) before.focus()
    }
  }, [])

  return (
    <div
      className="settings-layer"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onBack()
      }}
    >
      <div className="settings-sheet" role="dialog" aria-modal="true" aria-labelledby="settings-title" tabIndex={-1} ref={sheet}>
        <div className="settings-head">
          <h1 id="settings-title">{t('settings.title')}</h1>
          <button type="button" className="btn icon-btn" aria-label={t('common.close')} title={t('common.close')} onClick={onBack}>
            <CloseIcon />
          </button>
        </div>
        <div className="settings">
          <section className="setting">
            <h2>{t('settings.account')}</h2>
            {auth?.signedIn ? (
              <>
                <ul className="account-list" aria-label={t('settings.account')}>
                  {auth.accounts.map((login) => (
                    <li key={login}>
                      <span className="account-login">{login}</span>
                      {displayName(login) !== login && <span className="muted">{displayName(login)}</span>}
                      {login === auth.login && <span className="badge">{t('settings.accountActive')}</span>}
                    </li>
                  ))}
                </ul>
                <span className="muted small">{t(`settings.method.${auth.method}`)}</span>
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

          <section className="setting">
            <h2>{t('common.language')}</h2>
            <div>
              <LangToggle value={settings.language} onChange={(language: Language) => onChange({ language })} />
            </div>
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
            <h2>{t('settings.excluded')}</h2>
            <div className="excluded-row">
              <span className="muted mono excluded-summary">
                {settings.excludedFolders.length ? t('settings.excludedSummary', { count: settings.excludedFolders.length, list: summarize(settings.excludedFolders) }) : t('settings.excludedNone')}
              </span>
              <button type="button" className="btn" onClick={() => setEditingFolders(true)}>
                {t('settings.excludedEdit')}
              </button>
            </div>
            {editingFolders && (
              <ExcludedFoldersDialog
                patterns={settings.excludedFolders}
                onSave={(excludedFolders) => {
                  onChange({ excludedFolders })
                  setEditingFolders(false)
                }}
                onClose={() => setEditingFolders(false)}
              />
            )}
          </section>

          <TargetProjectSetting settings={settings} onChange={onChange} />

          <CacheSetting settings={settings} onChange={onChange} />

          <CliSetting />

          <McpSetting />
        </div>
      </div>
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
        <span className="small warn-text">
          {t('settings.cliFailed', { error: failed })}
        </span>
      )}
    </section>
  )
}

/** Registers the asset-hub MCP server with Claude Code or Codex, and shows whether each already runs it. */
function McpSetting() {
  const { t } = useTranslation()
  const [busy, setBusy] = useState<McpClient>()
  const [connected, setConnected] = useState<McpClient>()
  const [failed, setFailed] = useState<{ client: McpClient; error: string }>()
  const [status, setStatus] = useState<McpStatus[]>([])
  const name = (client: McpClient) => (client === 'claude' ? 'Claude Code' : 'Codex')
  const refresh = async () => {
    try {
      const outcome = await commands.agentMcpStatus()
      if (outcome.status === 'ok') setStatus(outcome.data)
    } catch {
      // The status line is a convenience; the buttons still work without it.
    }
  }
  useEffect(() => {
    void refresh()
  }, [])
  const stateOf = (client: McpClient) => status.find((s) => s.client === client)
  const connect = async (client: McpClient) => {
    setBusy(client)
    setFailed(undefined)
    setConnected(undefined)
    try {
      const outcome = await commands.agentMcpConnect(client)
      if (outcome.status === 'ok') setConnected(outcome.data.client)
      else setFailed({ client, error: outcome.error.message })
    } catch (raised) {
      setFailed({ client, error: raised instanceof Error ? raised.message : String(raised) })
    } finally {
      setBusy(undefined)
      await refresh()
    }
  }
  return (
    <section className="setting">
      <h2>{t('settings.mcpTitle')}</h2>
      <span className="muted small">{t('settings.mcpNote')}</span>
      <div className="field-row">
        <button type="button" className="btn" disabled={busy !== undefined} onClick={() => void connect('claude')}>
          {t(stateOf('claude')?.state === 'connected' ? 'settings.mcpReconnectClaude' : 'settings.mcpConnectClaude')}
        </button>
        <button type="button" className="btn" disabled={busy !== undefined} onClick={() => void connect('codex')}>
          {t(stateOf('codex')?.state === 'connected' ? 'settings.mcpReconnectCodex' : 'settings.mcpConnectCodex')}
        </button>
      </div>
      {status.map((entry) => (
        <span key={entry.client} className={entry.state === 'otherCopy' ? 'small warn-text' : 'muted small'}>
          {entry.state === 'connected' && t('settings.mcpStateConnected', { client: name(entry.client) })}
          {entry.state === 'otherCopy' && t('settings.mcpStateOtherCopy', { client: name(entry.client), command: entry.command })}
          {entry.state === 'missing' && t('settings.mcpStateMissing', { client: name(entry.client) })}
        </span>
      ))}
      {connected && (
        <span className="muted small" role="status">
          {t('settings.mcpConnected', { client: name(connected) })}
        </span>
      )}
      {failed && (
        <span className="small warn-text">
          {t('settings.mcpFailed', { client: name(failed.client), error: failed.error })}
        </span>
      )}
    </section>
  )
}

/** How much the cache holds, a way to empty the downloads, and which repos and branches the index caches. The app keeps the folder and the size itself. */
function CacheSetting({ settings, onChange }: { settings: AppSettings; onChange: Props['onChange'] }) {
  const { t } = useTranslation()
  const [usage, setUsage] = useState<CacheUsage>()
  const [clearing, setClearing] = useState(false)
  const measure = () => void window.api.cache.usage().then((r) => r.ok && setUsage(r.data))
  useEffect(measure, [])
  const clear = async () => {
    setClearing(true)
    await window.api.cache.clear()
    setClearing(false)
    measure()
  }
  return (
    <section className="setting">
      <h2>{t('settings.cache')}</h2>
      <div className="field-row">
        <button type="button" className="btn" onClick={() => void clear()} disabled={clearing}>
          {t('settings.cacheClear')}
        </button>
      </div>
      {usage && (
        <span className="muted small" role="status">
          {t('settings.cacheUsage', { downloads: formatBytes(usage.downloads), thumbs: formatBytes(usage.thumbs), other: formatBytes(usage.other) })}
        </span>
      )}
      <CacheScope exclusions={settings.cacheExclusions} onSave={(cacheExclusions) => onChange({ cacheExclusions })} onSaved={measure} />
    </section>
  )
}
