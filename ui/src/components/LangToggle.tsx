import { useTranslation } from 'react-i18next'
import { LANGUAGES, type Language } from '@shared/settings'
import { Dropdown } from './Dropdown'

interface Props {
  value: Language
  onChange(language: Language): void
}

/** Each language named in itself, so it can be found whatever the current one is. */
const OPTIONS = LANGUAGES.map((value) => ({ value, label: { en: 'English', vi: 'Tiếng Việt', ko: '한국어' }[value] }))

export function LangToggle({ value, onChange }: Props) {
  const { t } = useTranslation()
  return <Dropdown value={value} options={OPTIONS} onChange={onChange} label={t('common.language')} title={t('common.language')} className="lang-trigger" />
}
