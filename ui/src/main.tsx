import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './i18n'
// Bundled (not from Google) so the app looks the same offline; all three cover Vietnamese.
import '@fontsource/be-vietnam-pro/400.css'
import '@fontsource/be-vietnam-pro/500.css'
import '@fontsource/be-vietnam-pro/600.css'
import '@fontsource/be-vietnam-pro/700.css'
import '@fontsource/chakra-petch/600.css'
import '@fontsource/chakra-petch/700.css'
import '@fontsource/jetbrains-mono/400.css'
import '@fontsource/jetbrains-mono/500.css'
import './styles/app.css'
import { App } from './App'
import { api } from './api'

// Before the first render: every screen reaches the backend through `window.api`.
window.api = api

// For scripts that check how many effects render at once (a contact sheet): one card, drawn as the grid draws it.
;(window as unknown as { assetHubRender: unknown }).assetHubRender = (repo: string, branch: string, root: string, path: string) =>
  import('./viewer/prefabThumb').then((m) => m.renderPrefabThumbnail({ repo, branch, root }, path))
;(window as unknown as { assetHubFilm: unknown }).assetHubFilm = (repo: string, branch: string, root: string, path: string, times: number[], only?: string[], width?: number, height?: number, zoom?: number) =>
  import('./viewer/prefabThumb').then((m) => m.filmEffectAt({ repo, branch, root }, path, times, only, width, height, zoom))
;(window as unknown as { assetHubInspect: unknown }).assetHubInspect = (repo: string, branch: string, root: string, path: string) =>
  import('./viewer/prefabThumb').then((m) => m.inspectEffect({ repo, branch, root }, path))

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>
)
