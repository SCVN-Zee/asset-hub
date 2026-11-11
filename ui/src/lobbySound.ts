/**
 * The lobby's sounds, made with Web Audio (no sound files): a click for START
 * and the chest opening — a wooden knock, the lid's creak and a rising chime.
 * Muting is a per-machine choice, kept in localStorage.
 */
const MUTED_KEY = 'assethub.lobbyMuted'

let ctx: AudioContext | undefined
const audio = () => (ctx ??= new AudioContext())

export function isMuted(): boolean {
  try {
    return localStorage.getItem(MUTED_KEY) === '1'
  } catch {
    return false
  }
}

export function setMuted(muted: boolean): void {
  try {
    localStorage.setItem(MUTED_KEY, muted ? '1' : '0')
  } catch {
    // Not remembered; the choice still holds until the app closes.
  }
}

/** One note: `type` wave from `freq` (gliding to `to`), starting `at` seconds from now, fading out over `length`. */
function tone(type: OscillatorType, freq: number, at: number, length: number, volume: number, to?: number): void {
  const a = audio()
  const start = a.currentTime + at
  const osc = a.createOscillator()
  const gain = a.createGain()
  osc.type = type
  osc.frequency.setValueAtTime(freq, start)
  if (to) osc.frequency.exponentialRampToValueAtTime(to, start + length)
  gain.gain.setValueAtTime(0.0001, start)
  gain.gain.exponentialRampToValueAtTime(volume, start + 0.01)
  gain.gain.exponentialRampToValueAtTime(0.0001, start + length)
  osc.connect(gain).connect(a.destination)
  osc.start(start)
  osc.stop(start + length + 0.05)
}

export function playClick(): void {
  if (isMuted()) return
  tone('triangle', 660, 0, 0.09, 0.18, 990)
}

/** Timed with the scene's opening (lobbyScene T: shake 0, lid 0.6, light 0.75). */
export function playChestOpen(): void {
  if (isMuted()) return
  // Rattle while it shakes, a knock as the lid flies, a creak, then the treasure's chime.
  for (let i = 0; i < 5; i++) tone('square', 140 + i * 6, i * 0.11, 0.05, 0.05)
  tone('sine', 110, 0.6, 0.35, 0.35, 55)
  tone('sawtooth', 220, 0.62, 0.28, 0.04, 330)
  ;[1047, 1319, 1568, 2093].forEach((f, i) => tone('triangle', f, 0.78 + i * 0.09, 0.6, 0.12))
  tone('sine', 3136, 1.15, 0.9, 0.05)
}
