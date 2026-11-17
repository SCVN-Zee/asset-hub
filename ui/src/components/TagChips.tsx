import { useTranslation } from 'react-i18next'
import type { AssetTag } from '../tags'

/** The tags of an asset as small chips: on the card's stage (`stage`) or in the inspector. */
export function TagChips({ tags, stage }: { tags: readonly AssetTag[]; stage?: boolean }) {
  const { t } = useTranslation()
  if (!tags.length) return null
  return (
    <>
      {tags.map((tag) => (
        <span key={tag} className={stage ? 'cell-tag' : 'tag-chip'} title={t(`tags.${tag}Hint`)}>
          {t(`tags.${tag}`)}
        </span>
      ))}
    </>
  )
}
