import { Globe } from "lucide-react"
import { useTranslation } from "react-i18next"

import { CountryFlag } from "@/components/country-flag"
import { Input } from "@/components/ui/input"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useMonitor } from "@/hooks/use-monitor"
import { ALL_LOCATIONS, locationsOf } from "@/lib/nodes"

/** Location tabs and the node name filter. */
export function LocationFilter() {
  const { t } = useTranslation()
  const { nodes, location, setLocation, query, setQuery } = useMonitor()

  return (
    <div className="flex justify-between px-2">
      <Tabs value={location} onValueChange={setLocation} className="overflow-x-auto">
        <TabsList className="flex">
          <TabsTrigger value={ALL_LOCATIONS}>
            <Globe className="size-5" aria-hidden />
            <span>{t("all")}</span>
          </TabsTrigger>
          {locationsOf(nodes).map((code) => (
            <TabsTrigger key={code} value={code}>
              <CountryFlag code={code} className="size-5" />
              <span>{code || t("unknown")}</span>
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <div className="pl-2">
        <Input
          className="w-24"
          type="search"
          placeholder={t("filter")}
          aria-label={t("filter")}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
    </div>
  )
}
