import countries from "i18n-iso-countries"
import en from "i18n-iso-countries/langs/en.json"
import zh from "i18n-iso-countries/langs/zh.json"

countries.registerLocale(en)
countries.registerLocale(zh)

/** Localized name of an ISO 3166-1 alpha-2 country code, if known. */
export function countryName(code: string, language: string): string | undefined {
  if (!code) return undefined
  return countries.getName(code, language, { select: "alias" })
}
