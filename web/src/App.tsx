import { useTranslation } from "react-i18next"

import { I18nToggle } from "@/components/i18n-toggle"
import { LocationFilter } from "@/components/location-filter"
import { ModeToggle } from "@/components/mode-toggle"
import { MonitorProvider } from "@/components/monitor-provider"
import { NodeList } from "@/components/node-list"
import { StatusCards } from "@/components/status-cards"
import { Separator } from "@/components/ui/separator"
import { Toaster } from "@/components/ui/sonner"

// Read once: rendering has to stay pure.
const year = new Date().getFullYear()

export default function App() {
  const { t } = useTranslation()

  return (
    <div className="mx-3 max-w-7xl xl:mx-auto">
      <header className="m-1 mt-2 flex items-center justify-between">
        <div className="flex h-5 items-center gap-2">
          <h1 className="text-3xl font-bold text-primary">AIOM</h1>
          <Separator orientation="vertical" />
          <span className="text-xl font-light text-muted-foreground">{t("subtitle")}</span>
        </div>
        <div className="flex items-center gap-2">
          <I18nToggle />
          <ModeToggle />
        </div>
      </header>
      <main className="@container/main flex flex-col gap-4 py-4 md:gap-6">
        <MonitorProvider>
          <LocationFilter />
          <StatusCards />
          <NodeList />
        </MonitorProvider>
      </main>
      <footer className="px-2 py-4 text-center">
        <p className="text-sm text-muted-foreground">© {year} AIOM</p>
      </footer>
      <Toaster position="top-center" />
    </div>
  )
}
