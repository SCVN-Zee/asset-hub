import { useSyncExternalStore } from 'react'

export interface ChatMessage {
  id: number
  role: 'user' | 'assistant'
  text: string
  /** An answer still being written, stopped by the user, or failed. */
  state?: 'pending' | 'stopped' | 'failed'
  error?: string
  /** Marked wrong with 👎 (saved for testing). */
  reported?: boolean
  /** Pictures the user attached to this message (data URLs). */
  images?: string[]
  /** Reply buttons the pet offered with this answer; `other` adds one for typing an own answer. */
  choices?: { options: string[]; other: boolean }
}

interface ChatState {
  messages: ChatMessage[]
  busy: boolean
  /** Tools used in this conversation ("search_assets {…}"), sent with a 👎. */
  tools: string[]
  /** The hub tool the pet is using right now (search_assets…), shown while it works. */
  activity?: string
}

/**
 * The AI pet's conversation, kept outside the screens so it survives going to
 * Settings and back. The answer streams in from the main process (Codex).
 */
let state: ChatState = { messages: [], busy: false, tools: [] }
const listeners = new Set<() => void>()
let nextId = 1
/** The answer message being streamed: a new one replaces it (the pet shows the final reply, not "I'll look…" preambles). */
let currentItem: string | undefined

const set = (next: Partial<ChatState>) => {
  state = { ...state, ...next }
  for (const l of listeners) l()
}

const updateAnswer = (change: (m: ChatMessage) => ChatMessage) => {
  const last = state.messages[state.messages.length - 1]
  if (last?.role === 'assistant') set({ messages: [...state.messages.slice(0, -1), change(last)] })
}

let wired = false
function wire() {
  if (wired) return
  wired = true
  window.api.ai.onChat((event) => {
    if (event.type === 'tool') {
      set({ activity: event.tool, tools: [...state.tools, event.tool] })
    } else if (event.type === 'delta') {
      if (state.activity) set({ activity: undefined })
      const fresh = event.itemId !== currentItem
      currentItem = event.itemId
      updateAnswer((m) => ({ ...m, text: fresh ? event.text : m.text + event.text }))
    } else if (event.type === 'done') {
      updateAnswer((m) => ({
        ...m,
        state: event.status === 'interrupted' ? 'stopped' : event.status === 'failed' ? 'failed' : undefined,
        ...(event.error ? { error: event.error } : {})
      }))
      set({ busy: false, activity: undefined })
    } else {
      updateAnswer((m) => ({ ...m, state: 'failed', error: event.message }))
      set({ busy: false, activity: undefined })
    }
  })
}

const subscribe = (listener: () => void) => {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useAiChat(): ChatState {
  wire()
  return useSyncExternalStore(subscribe, () => state)
}

export async function sendChat(text: string, language: string, images: string[] = []): Promise<void> {
  const message = text.trim()
  if ((!message && !images.length) || state.busy) return
  currentItem = undefined
  set({
    busy: true,
    messages: [...state.messages, { id: nextId++, role: 'user', text: message, ...(images.length ? { images } : {}) }, { id: nextId++, role: 'assistant', text: '', state: 'pending' }]
  })
  const result = await window.api.ai.chatSend(message, language, images)
  if (!result.ok) {
    updateAnswer((m) => ({ ...m, state: 'failed', error: result.message }))
    set({ busy: false })
  }
}

/** The pet's offer_choices tool: buttons under the answer being written. */
export function offerChoices(options: unknown, other: unknown): { shown: number } {
  const labels = (Array.isArray(options) ? options : []).map((o) => String(o).trim().slice(0, 40)).filter(Boolean).slice(0, 4)
  if (labels.length < 2) throw new Error('Give 2 to 4 options')
  updateAnswer((m) => ({ ...m, choices: { options: labels, other: other === true } }))
  return { shown: labels.length }
}

export const stopChat = (): void => void window.api.ai.chatStop()

export async function resetChat(): Promise<void> {
  await window.api.ai.chatReset()
  currentItem = undefined
  set({ messages: [], busy: false, tools: [] })
}

/** 👎 on an answer: the conversation so far goes to a feedback file on this Mac. */
export async function reportAnswer(id: number): Promise<void> {
  const upTo = state.messages.findIndex((m) => m.id === id)
  await window.api.ai.feedback({ messages: state.messages.slice(0, upTo + 1).map((m) => ({ role: m.role, text: m.text })), tools: state.tools })
  set({ messages: state.messages.map((m) => (m.id === id ? { ...m, reported: true } : m)) })
}
