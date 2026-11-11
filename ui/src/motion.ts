/**
 * Small game-like moves done with the Web Animations API. Each one is
 * cosmetic: it is skipped when the OS asks for reduced motion, and the state
 * change it decorates happens either way.
 */

const calm = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches

/** Squash-and-stretch of the chest icon, as if something landed in it. */
export function bumpChest() {
  if (calm()) return
  document
    .querySelector('.pack-chest')
    ?.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.3, 0.75)' }, { transform: 'scale(0.9, 1.15)' }, { transform: 'scale(1)' }], { duration: 400, easing: 'ease-out' })
}

/**
 * Shrinks a copy of the asset's thumbnail (its card, or the details viewer)
 * and hops it along an arc into its chest slot. Call right after adding the
 * asset: the slot is looked up once React has rendered it.
 */
export function flyToPack(assetId: string) {
  if (calm()) return
  const id = CSS.escape(assetId)
  const source = document.querySelector<HTMLElement>(`[data-asset-id="${id}"] .asset-thumb`) ?? document.querySelector<HTMLElement>('.details .viewer')
  if (!source) return
  const from = source.getBoundingClientRect()
  if (!from.width) return
  const img = source.querySelector('img')?.getAttribute('src')
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      // Its place in the chest's list, or the chest itself while the list is folded.
      const item = document.querySelector<HTMLElement>(`[data-pack-id="${id}"]`)
      const slot = item ?? document.querySelector<HTMLElement>('.pack-chest')
      if (!slot) return
      const to = slot.getBoundingClientRect()
      const size = Math.min(from.width, from.height, 140)
      const ghost = document.createElement('div')
      ghost.className = 'pack-ghost'
      Object.assign(ghost.style, {
        left: `${from.left + (from.width - size) / 2}px`,
        top: `${from.top + (from.height - size) / 2}px`,
        width: `${size}px`,
        height: `${size}px`,
        backgroundImage: img ? `url("${img}")` : ''
      })
      document.body.appendChild(ghost)
      if (item) item.style.visibility = 'hidden'
      const start = ghost.getBoundingClientRect()
      const dx = to.left + to.width / 2 - (start.left + size / 2)
      const dy = to.top + to.height / 2 - (start.top + size / 2)
      const end = Math.max(to.height / size, 0.12)
      const flight = ghost.animate(
        [
          { transform: 'translate(0, 0) scale(1)', offset: 0 },
          { transform: 'translate(0, -14px) scale(1.06)', offset: 0.14 },
          { transform: `translate(${dx * 0.45}px, ${Math.min(0, dy) * 0.45 - 140}px) scale(${0.35 + end}) rotate(-10deg)`, offset: 0.55 },
          { transform: `translate(${dx}px, ${dy}px) scale(${end}) rotate(0deg)`, opacity: 0.9, offset: 1 }
        ],
        { duration: 650, easing: 'cubic-bezier(.45,0,.2,1)', fill: 'forwards' }
      )
      const land = () => {
        ghost.remove()
        if (item) item.style.visibility = ''
        if (item) item.animate([{ transform: 'scale(.3)', opacity: 0 }, { transform: 'scale(1.15)', opacity: 1, offset: 0.6 }, { transform: 'scale(1)' }], { duration: 320, easing: 'ease-out' })
        bumpChest()
      }
      flight.onfinish = land
      flight.oncancel = land
    })
  )
}

/** Hops a chest slot up and out, then runs `done` (which removes it). */
export function leaveSlot(slot: Element | null, done: () => void) {
  if (calm() || !slot) return done()
  slot.animate([{ transform: 'none', opacity: 1 }, { transform: 'translateY(-18px) scale(.4) rotate(12deg)', opacity: 0 }], { duration: 220, easing: 'ease-in', fill: 'forwards' }).onfinish = done
}
