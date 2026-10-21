import i18n from "i18next"
import LanguageDetector from "i18next-browser-languagedetector"
import { initReactI18next } from "react-i18next"

import en from "@/locales/en.json"
import zh from "@/locales/zh.json"

export const LANGUAGES = ["en", "zh"] as const

i18n.on("languageChanged", (language) => {
  document.documentElement.lang = language
})

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    fallbackLng: "en",
    supportedLngs: LANGUAGES,
    nonExplicitSupportedLngs: true,
    interpolation: {
      // React escapes already.
      escapeValue: false,
    },
    resources: {
      en: { translation: en },
      zh: { translation: zh },
    },
  })
