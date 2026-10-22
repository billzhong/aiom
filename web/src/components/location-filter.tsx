import { Globe } from "lucide-react"
import { useTranslation } from "react-i18next"

import { CountryFlag } from "@/components/country-flag"
import { Input } from "@/components/ui/input"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useMonitor } from "@/hooks/use-monitor"
import { countryName } from "@/lib/countries"
import { ALL_LOCATIONS } from "@/lib/nodes"

function NodeCount({ count }: { count: number }) {
  return <span className="text-xs text-muted-foreground tabular-nums">{count}</span>
}

/** Location tabs, with the number of nodes in each, and the node name filter. */
export function LocationFilter() {
  const { t, i18n } = useTranslation()
  const { nodes, locations, location, setLocation, query, setQuery } = useMonitor()

  return (
    <div className="flex justify-between px-2">
      <Tabs value={location} onValueChange={setLocation} className="overflow-x-auto">
        <TabsList className="flex">
          <TabsTrigger value={ALL_LOCATIONS}>
            <Globe className="size-5" aria-hidden />
            <span>{t("all")}</span> <NodeCount count={nodes.length} />
          </TabsTrigger>
          {[...locations].map(([code, count]) => (
            <TabsTrigger key={code} value={code} title={countryName(code, i18n.resolvedLanguage)}>
              <CountryFlag code={code} className="size-5" />
              <span>{code || t("unknown")}</span> <NodeCount count={count} />
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <div className="pl-2">
        <Input
          className="w-32 md:w-48"
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
