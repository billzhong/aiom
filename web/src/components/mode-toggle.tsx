import { Moon, Sun, SunMoon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { OptionMenu } from "@/components/option-menu"
import { useTheme } from "@/hooks/use-theme"
import type { Theme } from "@/lib/theme"

const THEMES = [
  { theme: "light", Icon: Sun },
  { theme: "dark", Icon: Moon },
  { theme: "system", Icon: SunMoon },
] as const satisfies readonly { theme: Theme; Icon: unknown }[]

export function ModeToggle() {
  const { theme, setTheme } = useTheme()
  const { t } = useTranslation()

  return (
    <OptionMenu
      label={t("theme.toggle")}
      trigger={
        <>
          <Sun className="scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90" />
          <Moon className="absolute scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0" />
        </>
      }
      options={THEMES.map(({ theme: option, Icon }) => ({
        value: option,
        label: t(`theme.${option}`),
        icon: <Icon />,
      }))}
      selected={theme}
      onSelect={setTheme}
    />
  )
}
