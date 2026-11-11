import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import * as THREE from 'three'
import type { FbxViewer } from '../viewer/FbxViewer'

/**
 * Unity's axes in the viewer's (three.js) space: models and prefabs are shown
 * mirrored along X, as Unity imports them, so Unity's +X is three.js's -X.
 */
const AXES = [
  { name: 'X', color: '#e0485a', dir: new THREE.Vector3(-1, 0, 0) },
  { name: 'Y', color: '#7fb236', dir: new THREE.Vector3(0, 1, 0) },
  { name: 'Z', color: '#4b8fe8', dir: new THREE.Vector3(0, 0, 1) }
] as const

const SIZE = 84
const ARM = 28
const BALL = 9.5

interface Ball {
  key: string
  label: string
  color: string
  positive: boolean
  dir: THREE.Vector3
  x: number
  y: number
  depth: number
}

/** Where each axis end shows from the camera's orientation now, the farthest first. */
function project(q: THREE.Quaternion): Ball[] {
  const inv = q.clone().invert()
  const balls: Ball[] = []
  for (const a of AXES)
    for (const positive of [true, false]) {
      const dir = a.dir.clone().multiplyScalar(positive ? 1 : -1)
      const v = dir.clone().applyQuaternion(inv)
      balls.push({ key: (positive ? '+' : '-') + a.name, label: a.name, color: a.color, positive, dir, x: SIZE / 2 + v.x * ARM, y: SIZE / 2 - v.y * ARM, depth: v.z })
    }
  return balls.sort((a, b) => a.depth - b.depth)
}

/** Axis gizmo in the viewer's corner, turning with the camera: click an axis end to look from that side. */
export function ViewGizmo({ viewer }: { viewer?: FbxViewer }) {
  const { t } = useTranslation()
  const [balls, setBalls] = useState<Ball[]>([])
  const [hover, setHover] = useState<string>()

  useEffect(() => {
    if (!viewer) return
    let last = ''
    const follow = () => {
      const q = viewer.cameraQuaternion
      const key = [q.x, q.y, q.z, q.w].map((n) => n.toFixed(4)).join()
      if (key === last) return
      last = key
      setBalls(project(q))
    }
    follow()
    return viewer.addUpdater(follow)
  }, [viewer])

  if (!viewer) return null
  const look = (b: Ball) => {
    // Already looking from there: the other side, like Blender.
    const from = viewer.viewDirection.dot(b.dir) > 0.999 ? b.dir.clone().negate() : b.dir
    viewer.lookFrom(from)
  }

  return (
    <svg className="view-gizmo" width={SIZE} height={SIZE} viewBox={`0 0 ${SIZE} ${SIZE}`} role="group" aria-label={t('viewer.gizmo')}>
      {balls.map((b) => {
        const on = hover === b.key
        const label = t('viewer.lookFrom', { axis: b.key })
        return (
          <g
            key={b.key}
            role="button"
            tabIndex={0}
            aria-label={label}
            className="view-gizmo-ball"
            onClick={() => look(b)}
            onKeyDown={(e) => {
              if (e.key !== 'Enter' && e.key !== ' ') return
              e.preventDefault()
              look(b)
            }}
            onPointerEnter={() => setHover(b.key)}
            onPointerLeave={() => setHover(undefined)}
          >
            <title>{label}</title>
            {b.positive && <line x1={SIZE / 2} y1={SIZE / 2} x2={b.x} y2={b.y} stroke={b.color} strokeWidth={2.5} strokeLinecap="round" />}
            <circle
              cx={b.x}
              cy={b.y}
              r={b.positive ? BALL : BALL - 1}
              fill={b.color}
              fillOpacity={b.positive ? 1 : 0.28}
              stroke={on ? '#fff' : b.color}
              strokeWidth={b.positive ? (on ? 1.5 : 0) : 1.5}
            />
            {(b.positive || on) && (
              <text x={b.x} y={b.y} dy="0.35em" textAnchor="middle" className="view-gizmo-label" fill={b.positive ? '#0e131b' : '#fff'}>
                {b.positive ? b.label : `-${b.label}`}
              </text>
            )}
          </g>
        )
      })}
    </svg>
  )
}
