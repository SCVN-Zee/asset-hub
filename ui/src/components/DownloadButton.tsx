import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { formatBytes } from '../format'
import { unwrap } from '../remoteSource'
import { ErrorNote } from './ErrorNote'

type State = { status: 'idle' } | { status: 'saving' } | { status: 'saved'; path: string; bytes?: number } | { status: 'error'; error: unknown }

/** Saves one file from the repo to disk (asks where), then offers to show it. */
export function DownloadButton({ repo, branch, path }: { repo: string; branch: string; path: string }) {
  const { t } = useTranslation()
  const [state, setState] = useState<State>({ status: 'idle' })
  useEffect(() => setState({ status: 'idle' }), [repo, branch, path])

  const save = async () => {
    setState({ status: 'saving' })
    try {
      const result = unwrap(await window.api.assets.saveFile(repo, branch, path))
      setState(result.canceled || !result.path ? { status: 'idle' } : { status: 'saved', path: result.path, bytes: result.bytes })
    } catch (error) {
      setState({ status: 'error', error })
    }
  }

  return (
    <>
      <div className="field-row">
        <button type="button" className="btn btn-primary" onClick={() => void save()} disabled={state.status === 'saving'}>
          {state.status === 'saving' ? t('download.saving') : t('download.button')}
        </button>
        {state.status === 'saved' && (
          <button type="button" className="btn" onClick={() => void window.api.exporter.showInFolder(state.path)}>
            {t('export.showInFolder')}
          </button>
        )}
      </div>
      {state.status === 'saved' && (
        <p className="muted small" style={{ margin: 0 }}>
          {t('download.saved', { name: state.path.split('/').pop(), size: formatBytes(state.bytes) })}
        </p>
      )}
      {state.status === 'error' && <ErrorNote error={state.error} />}
    </>
  )
}
