import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { BundleMaterial, DependencyReport, MaterialRef } from '@shared/types'
import { formatBytes } from '../format'
import { parse } from '../parse'
import { unwrap } from '../remoteSource'
import { WarnIcon } from './Icons'
import type { LiveContext } from './ModelViewer'

type Mode = 'zip' | 'blender'
export type BundleState = { status: 'idle' } | { status: 'busy'; mode: Mode } | { status: 'done'; mode: Mode; path: string; bytes?: number; textures: number } | { status: 'error'; mode: Mode; message: string }

/** An FBX: its materials and the textures it names itself, found as the preview finds them. */
async function fbxMaterials(live: LiveContext, path: string, report?: DependencyReport, prefabMaterial?: MaterialRef): Promise<BundleMaterial[]> {
  const names = await parse.fbxTextureNames(live, path)
  const found = names.length ? unwrap(await window.api.assets.findByNames(live.repo, live.branch, names, path, live.root)) : {}
  return parse.modelBundleMaterials(report, prefabMaterial, Object.values(found).filter((p): p is string => !!p))
}

/** A prefab put together now (models placed, materials named by .mat GUID) and exported as one GLB, with the GUIDs its meshes use. */
async function prefabGlb(live: LiveContext, path: string, report: DependencyReport): Promise<{ glb: Uint8Array; used: Set<string> }> {
  const [{ buildPrefab }, { GLTFExporter }] = await Promise.all([import('../viewer/prefabBuild'), import('three/examples/jsm/exporters/GLTFExporter.js')])
  const build = await buildPrefab(live, path, report, false, false)
  try {
    if (!build.meshes || build.missing >= build.meshes) throw new Error('No models found in this prefab')
    const used = new Set<string>()
    build.group.traverse((o) => {
      const m = (o as { material?: { name: string } | Array<{ name: string }> }).material
      for (const x of Array.isArray(m) ? m : m ? [m] : []) used.add(x.name)
    })
    const glb = (await new GLTFExporter().parseAsync(build.group, { binary: true })) as ArrayBuffer
    return { glb: new Uint8Array(glb), used }
  } finally {
    build.dispose()
  }
}

/** One action of the bundle, ready to be a menu row. */
export interface BundleAction {
  label: string
  /** Why it waits, when it does. */
  hint?: string
  disabled: boolean
  onSelect(): void
}

/**
 * A model out of Unity's world: one .zip with its textures and a script that
 * rebuilds its materials in Blender, or straight into Blender (when it is
 * installed). A prefab is put together and exported only when asked.
 */
export function useModelBundle(source: { kind: 'fbx' | 'prefab'; live: LiveContext; path: string; report?: DependencyReport; prefabMaterial?: MaterialRef } | undefined): { zip: BundleAction; blender?: BundleAction; state: BundleState } | undefined {
  const { t } = useTranslation()
  const [state, setState] = useState<BundleState>({ status: 'idle' })
  const [blender, setBlender] = useState(false)
  const enabled = !!source
  useEffect(() => setState({ status: 'idle' }), [source?.live.repo, source?.live.branch, source?.path])
  useEffect(() => {
    if (enabled) void window.api.assets.blenderInstalled().then(setBlender, () => setBlender(false))
  }, [enabled])
  // "Opened in Blender" says so for a moment, then the action is ready again.
  useEffect(() => {
    if (state.status !== 'done' || state.mode !== 'blender') return
    const id = setTimeout(() => setState({ status: 'idle' }), 4000)
    return () => clearTimeout(id)
  }, [state])

  // A prefab needs its dependencies (where its models and materials are) before it can be put together.
  const waiting = source?.kind === 'prefab' && (!source.report?.guidPaths || !!source.report.partial)
  const run = async (mode: Mode) => {
    if (!source) return
    const { kind, live, path, report, prefabMaterial } = source
    setState({ status: 'busy', mode })
    try {
      // A prefab's particle materials are on no mesh: their textures stay out.
      const prefab = kind === 'prefab' ? await prefabGlb(live, path, report!) : undefined
      const materials = prefab ? (await parse.prefabBundleMaterials(report!)).filter((m) => prefab.used.has(m.target)) : await fbxMaterials(live, path, report, prefabMaterial)
      const result = unwrap(await window.api.assets.modelBundle(live.repo, live.branch, path, materials, mode, prefab?.glb))
      setState(result.canceled || !result.path ? { status: 'idle' } : { status: 'done', mode, path: result.path, bytes: result.bytes, textures: result.textures ?? 0 })
    } catch (error) {
      setState({ status: 'error', mode, message: error instanceof Error ? error.message : String(error) })
    }
  }

  const busy = state.status === 'busy' || waiting
  const zipSaved = state.status === 'done' && state.mode === 'zip' ? state : undefined
  const stateOf = (mode: Mode) => (state.status !== 'idle' && state.mode === mode ? state : undefined)
  const zip = stateOf('zip')
  const bl = stateOf('blender')
  const hint = waiting ? t('bundle.waiting') : undefined
  if (!source) return undefined
  return {
    state,
    zip: {
      label: zip?.status === 'busy' ? t('download.saving') : zipSaved ? t('export.showInFolder') : t('bundle.zip'),
      hint,
      // Once saved, the row shows the file in its folder: that needs no waiting.
      disabled: zipSaved ? false : busy,
      onSelect: () => (zipSaved ? void window.api.exporter.showInFolder(zipSaved.path) : void run('zip'))
    },
    ...(blender ? { blender: { label: bl?.status === 'busy' ? t('bundle.opening') : bl?.status === 'done' ? t('bundle.opened') : t('bundle.blender'), hint, disabled: busy, onSelect: () => void run('blender') } } : {})
  }
}

/** What the last bundle did, in words: working, saved (and where), or why it failed. */
export function BundleNotice({ state }: { state: BundleState }) {
  const { t } = useTranslation()
  if (state.status === 'idle') return null
  if (state.status === 'error')
    return (
      <p className="inspector-note" data-state="warn" role="alert">
        <WarnIcon size={14} />
        <span>{state.message}</span>
      </p>
    )
  const text =
    state.status === 'busy'
      ? t(state.mode === 'zip' ? 'download.saving' : 'bundle.opening')
      : state.mode === 'zip'
        ? t('bundle.saved', { name: state.path.split('/').pop(), size: formatBytes(state.bytes), count: state.textures })
        : t('bundle.opened')
  return (
    <p className="inspector-note" role="status">
      {text}
    </p>
  )
}
