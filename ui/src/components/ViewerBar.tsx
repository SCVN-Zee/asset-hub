import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { FbxViewer } from '../viewer/FbxViewer'
import { AutoRotateIcon, ResetViewIcon, WireframeIcon } from './Icons'
import { MoreMenu } from './InspectorParts'

/** What the stage under the asset is: the calibrated grey, or the dark backdrop an effect can be judged on. */
export type ViewerStage = 'stage' | 'dark'

/**
 * The stage and its bar as one box. The box owns the viewer height, so the
 * resize bar (which measures what is above it) drags the same thing it reads.
 */
export function ViewerFrame({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={`viewer-frame${className ? ` ${className}` : ''}`}>{children}</div>
}

interface BarProps {
  /** The main controls, left to right. */
  children?: ReactNode
  /** The rows of the "…" menu: everything that does not need a place in the bar. */
  menu?: ReactNode
  /** The viewing condition, stated at the right end; absent when the stage is not calibrated (audio, images, fonts). */
  stage?: ViewerStage
}

/** One 32px bar under the stage: the controls, a "…" menu and the calibration reading. */
export function ViewerBar({ children, menu, stage }: BarProps) {
  const { t } = useTranslation()
  return (
    <div className="viewer-bar" role="toolbar" aria-label={t('viewer.tools')}>
      {children}
      {menu && <MoreMenu label={t('viewer.options')} align="start" iconSize={14}>{menu}</MoreMenu>}
      {stage && (
        <span className="viewer-cal" title={t(stage === 'dark' ? 'inspector.calibrationDark' : 'inspector.calibration')}>
          <i className="cal-ball" data-dark={stage === 'dark'} aria-hidden="true" />
          <span className="num">{stage === 'dark' ? t('texture.backdrop.dark') : '18%'}</span>
        </span>
      )}
    </div>
  )
}

interface ButtonProps {
  icon: ReactNode
  label: string
  /** Set for a toggle; a plain action leaves it out. */
  pressed?: boolean
  onClick(): void
  title?: string
}

/** A 24px icon button of the bar. The label is its name and its tooltip. */
export function ViewerButton({ icon, label, pressed, onClick, title }: ButtonProps) {
  return (
    <button type="button" className="viewer-btn" aria-label={label} title={title ?? label} aria-pressed={pressed} onClick={onClick}>
      {icon}
    </button>
  )
}

interface ViewToolsProps {
  view: FbxViewer | undefined
  autoRotate: boolean
  wireframe: boolean
  onAutoRotate(on: boolean): void
  onWireframe(on: boolean): void
}

/** What a 3D stage offers in the bar: back to the default camera, spin, wireframe. */
export function ViewTools({ view, autoRotate, wireframe, onAutoRotate, onWireframe }: ViewToolsProps) {
  const { t } = useTranslation()
  return (
    <>
      <ViewerButton icon={<ResetViewIcon />} label={t('viewer.view.reset')} onClick={() => view?.setView('reset')} />
      <ViewerButton
        icon={<AutoRotateIcon />}
        label={t('viewer.autoRotate')}
        pressed={autoRotate}
        onClick={() => {
          view?.setAutoRotate(!autoRotate)
          onAutoRotate(!autoRotate)
        }}
      />
      <ViewerButton
        icon={<WireframeIcon />}
        label={t('viewer.wireframe')}
        pressed={wireframe}
        onClick={() => {
          view?.setWireframe(!wireframe)
          onWireframe(!wireframe)
        }}
      />
    </>
  )
}
