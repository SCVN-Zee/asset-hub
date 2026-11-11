import { describe, expect, it } from 'vitest'
import { contextRecovery, type RecoveryPage } from './contextRecovery'

/** A page whose visibility and clock the test moves by hand. */
function fakePage() {
  let hidden = false
  let time = 0
  const visible: Array<() => void> = []
  const queued: Array<() => void> = []
  const page: RecoveryPage = {
    hidden: () => hidden,
    whenVisible: (fn) => void visible.push(fn),
    later: (fn) => void queued.push(fn),
    now: () => time
  }
  return {
    page,
    hide: () => (hidden = true),
    advance: (ms: number) => (time += ms),
    flush: () => queued.splice(0).forEach((fn) => fn()),
    show: () => {
      hidden = false
      visible.splice(0).forEach((fn) => fn())
    }
  }
}

describe('a viewer whose WebGL context was lost', () => {
  it('gets a new renderer once the event has finished', () => {
    const world = fakePage()
    let rebuilt = 0
    const lost = contextRecovery(() => rebuilt++, world.page)
    lost()
    expect(rebuilt).toBe(0)
    world.flush()
    expect(rebuilt).toBe(1)
  })

  it('waits while the window is hidden and rebuilds when it shows again', () => {
    const world = fakePage()
    let rebuilt = 0
    const lost = contextRecovery(() => rebuilt++, world.page)
    world.hide()
    lost()
    world.flush()
    expect(rebuilt).toBe(0)
    world.show()
    expect(rebuilt).toBe(1)
  })

  it('rebuilds once for a burst of loss events', () => {
    const world = fakePage()
    let rebuilt = 0
    const lost = contextRecovery(() => rebuilt++, world.page)
    lost()
    lost()
    lost()
    world.flush()
    expect(rebuilt).toBe(1)
  })

  it('stops after three rebuilds in ten seconds, and starts again later', () => {
    const world = fakePage()
    let rebuilt = 0
    const lost = contextRecovery(() => rebuilt++, world.page)
    for (let i = 0; i < 5; i++) {
      lost()
      world.flush()
      world.advance(1000)
    }
    expect(rebuilt).toBe(3)
    world.advance(11_000)
    lost()
    world.flush()
    expect(rebuilt).toBe(4)
  })
})
