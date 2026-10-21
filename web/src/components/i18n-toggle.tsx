import { CN, US } from "country-flag-icons/react/3x2"
import { Languages } from "lucide-react"
import { useTranslation } from "react-i18next"

import { OptionMenu } from "@/components/option-menu"
import { LANGUAGES } from "@/i18n"

const FLAGS = { en: US, zh: CN } satisfies Record<(typeof LANGUAGES)[number], unknown>

export function I18nToggle() {
  const { t, i18n } = useTranslation()

  return (
    <OptionMenu
      label={t("language.toggle")}
      trigger={<Languages />}
      options={LANGUAGES.map((language) => {
        const Flag = FLAGS[language]
        return { value: language, label: t(`language.${language}`), icon: <Flag aria-hidden /> }
      })}
      selected={i18n.resolvedLanguage}
      onSelect={(language) => void i18n.changeLanguage(language)}
    />
  )
}
