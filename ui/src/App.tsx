import { useCallback, useEffect, useState } from 'react'
import type { AssetSource } from '@shared/types'
import type { AuthStatus } from '@shared/ipc'
import { DEFAULT_EXPORT_LAYOUT, type Settings as AppSettings } from '@shared/settings'
import { HandlerRegistry } from './handlers'
import { parse } from './parse'
import { budgetFor, DEFAULT_BUDGET_PRESET } from './playable'
import { BudgetContext } from './budget'
import i18n from './i18n'
import { RemoteAssetSource, RemoteError, unwrap } from './remoteSource'
import { Browse } from './screens/Browse'
import { Settings } from './screens/Settings'
import { ErrorNote } from './components/ErrorNote'
import { SignIn } from './screens/SignIn'

type Screen = 'loading' | 'signin' | 'browse'

export function App() {
  const [settings, setSettings] = useState<AppSettings>()
  const [auth, setAuth] = useState<AuthStatus>()
  const [source, setSource] = useState<AssetSource>()
  const [screen, setScreen] = useState<Screen>('loading')
  /** Settings is a panel over the hub, so the hub (search, selection, picks, export list) stays mounted behind it. */
  const [settingsOpen, setSettingsOpen] = useState(false)
  /** The asset types, as the Rust side declares them. */
  const [registry, setRegistry] = useState<HandlerRegistry>()
  const [registryError, setRegistryError] = useState<unknown>()
  useEffect(() => {
    parse.handlerDescriptors().then(
      (descriptors) => setRegistry(new HandlerRegistry(descriptors)),
      (error: unknown) => setRegistryError(error)
    )
  }, [])

  const enterGitHub = useCallback((status: AuthStatus) => {
    setAuth(status)
    setSource(new RemoteAssetSource())
    setScreen('browse')
  }, [])

  useEffect(() => {
    void window.api.getSettings().then(setSettings)
    // Restores the previous sign-in straight into the hub; any failure falls back to the sign-in screen.
    window.api.auth.status().then(
      (result) => (result.ok && result.data.signedIn ? enterGitHub(result.data) : setScreen('signin')),
      () => setScreen('signin')
    )
  }, [enterGitHub])

  useEffect(() => {
    if (!settings) return
    void i18n.changeLanguage(settings.language)
    document.documentElement.lang = settings.language
  }, [settings])

  const update = useCallback((patch: Partial<AppSettings>) => {
    void window.api.setSettings(patch).then(setSettings)
  }, [])

  if (!settings || screen === 'loading') return null

  const setLanguage = (language: AppSettings['language']) => update({ language })

  const signOut = async (gh = false) => {
    await window.api.auth.signOut({ gh })
    setAuth(undefined)
    setSource(undefined)
    setSettingsOpen(false)
    setScreen('signin')
  }

  switch (screen) {
    case 'signin':
      return (
        <SignIn
          language={settings.language}
          onLanguage={setLanguage}
          onToken={async (token) => unwrap(await window.api.auth.signInWithToken(token))}
          onGh={async () => {
            const existing = await window.api.auth.signInWithGh()
            if (existing.ok) return existing.data
            if (existing.error.code !== 'ghNotLoggedIn') throw new RemoteError(existing.error)
            // gh is installed but logged out: log it in through the browser right here.
            return unwrap(await window.api.auth.ghWebLogin())
          }}
          onEnter={enterGitHub}
        />
      )
    case 'browse':
      return source && registry ? (
        <BudgetContext.Provider value={(settings.budgetPreset ?? DEFAULT_BUDGET_PRESET) === 'off' ? undefined : budgetFor(settings.budgetPreset, settings.customBudget)}>
        {/* While the panel is open the hub stays mounted but cannot take focus or clicks. */}
        <div className="hub" {...(settingsOpen ? ({ inert: '' } as object) : {})}>
        <Browse
          source={source}
          registry={registry}
          excludedFolders={settings.excludedFolders}
          login={auth?.login}
          initialRepo={settings.lastRepo}
          initialBranch={settings.lastBranch}
          onOpenBranch={(lastRepo, lastBranch) => {
            if (lastRepo !== settings.lastRepo || lastBranch !== settings.lastBranch) update({ lastRepo, lastBranch })
          }}
          onSettings={() => setSettingsOpen(true)}
          paused={settingsOpen}
          budgetPreset={settings.budgetPreset ?? DEFAULT_BUDGET_PRESET}
          onBudgetPreset={(budgetPreset) => update({ budgetPreset })}
          exportLayout={settings.exportLayout ?? DEFAULT_EXPORT_LAYOUT}
          onExportLayout={(exportLayout) => update({ exportLayout })}
          targetProject={settings.targetProject}
          collections={settings.collections}
          onCollections={(collections) => update({ collections })}
          recent={settings.recent}
          onRecent={(recent) => update({ recent })}
          watched={settings.watched ?? []}
          onWatched={(watched) => update({ watched })}
        />
        </div>
        {settingsOpen && <Settings settings={settings} auth={auth} onChange={update} onSignOut={(gh) => void signOut(gh)} onBack={() => setSettingsOpen(false)} />}
        </BudgetContext.Provider>
      ) : registryError ? (
        <ErrorNote error={registryError} />
      ) : null
  }
}
