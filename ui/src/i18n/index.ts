import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from './locales/en.json'
import vi from './locales/vi.json'
import ko from './locales/ko.json'

// English is the default and the fallback for any missing translated key.
void i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, vi: { translation: vi }, ko: { translation: ko } },
  lng: 'en',
  fallbackLng: 'en',
  interpolation: { escapeValue: false }
})

export default i18n
