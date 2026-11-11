import { createContext, useContext } from 'react'
import type { PlayableBudget } from '@shared/types'

/**
 * The playable budget chosen in Settings: every ✓ / ⚠, filter and ranking judges by it.
 * Undefined when checks are off (the default): nothing is judged against a playable.
 */
export const BudgetContext = createContext<PlayableBudget | undefined>(undefined)

export const useBudget = () => useContext(BudgetContext)
