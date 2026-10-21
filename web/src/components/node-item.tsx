import { cn } from "cn"
import {
  Activity,
  ArrowDown,
  ArrowUp,
  Cpu,
  HardDrive,
  MemoryStick,
  MonitorCog,
  Network,
} from "lucide-react"
import { memo } from "react"
import { useTranslation } from "react-i18next"

import { CountryFlag } from "@/components/country-flag"
import { NodeCharts } from "@/components/node-charts"
import { NodeDetails } from "@/components/node-details"
import { OsLogo } from "@/components/os-logo"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Separator } from "@/components/ui/separator"
import { MetricLabel, UsageBar } from "@/components/usage-bar"
import { formatBitRate, formatDuration, percentOf } from "@/lib/format"
import { isWindows } from "@/lib/os-icon"
import type { ChartPoint, NodeData } from "@/lib/types"

function OnlineDot({ online }: { online: boolean }) {
  return (
    <span className="relative flex size-1.5" aria-hidden>
      <span
        className={cn(
          "absolute inline-flex h-full w-full rounded-full opacity-75",
          online ? "animate-ping bg-chart-2" : "bg-destructive",
        )}
      />
      <span
        className={cn(
          "relative inline-flex size-1.5 rounded-full",
          online ? "bg-chart-2" : "bg-destructive",
        )}
      />
    </span>
  )
}

interface NodeItemProps {
  node: NodeData
  online: boolean
  history: ChartPoint[]
  open: boolean
  /** Called with the node's name to open or close it. */
  onToggle: (name: string) => void
}

/**
 * A node's summary row, expanding into its details and history charts. Memoized, as the list
 * re-renders on every keystroke in the search box and every toggle of another row.
 */
export const NodeItem = memo(function NodeItem({
  node,
  online,
  history,
  open,
  onToggle,
}: NodeItemProps) {
  const { t } = useTranslation()
  const { host, stats } = node

  return (
    <Collapsible
      open={open}
      onOpenChange={() => onToggle(node.name)}
      className="rounded-xl border shadow-sm"
    >
      <CollapsibleTrigger className="w-full text-left" aria-label={node.name}>
        <Card
          className={cn(
            "flex flex-col gap-1 border-none py-2 shadow-none xl:flex-row",
            open && "rounded-b-none",
          )}
        >
          <CardHeader className="w-auto flex-none px-4 pr-2 xl:w-1/6">
            <div className="flex items-center gap-2">
              <CountryFlag code={node.geoip} className="size-6" />
              <CardTitle>{node.name}</CardTitle>
            </div>
            <CardDescription className="flex items-center gap-1">
              <OnlineDot online={online} />
              <span className="flex gap-3 text-xs font-medium">
                <span>{online ? t("online") : t("offline")}</span>
                {online && <span className="font-mono">{formatDuration(stats.uptime)}</span>}
              </span>
            </CardDescription>
          </CardHeader>
          <div className="hidden items-center xl:flex">
            <Separator orientation="vertical" />
          </div>
          <CardContent className="w-full flex-1 px-4 pl-2">
            <div className="flex flex-wrap gap-4">
              <div className="min-w-[100px] flex-[0.5] space-y-1">
                <MetricLabel icon={<MonitorCog className="size-3.5" aria-hidden />}>
                  {t("os")}
                </MetricLabel>
                <div className="flex items-center gap-1 text-xs">
                  <OsLogo os={host.os} />
                  <span className="truncate">{host.os}</span>
                </div>
              </div>
              <UsageBar
                icon={<Cpu className="size-3.5" aria-hidden />}
                label={t("cpu")}
                percent={stats.cpu_usage}
              />
              <UsageBar
                icon={<MemoryStick className="size-3.5" aria-hidden />}
                label={t("ram")}
                percent={percentOf(stats.used_memory, host.total_memory)}
              />
              <UsageBar
                icon={<HardDrive className="size-3.5" aria-hidden />}
                label={t("disk")}
                percent={percentOf(stats.used_disk, host.total_disk)}
              />
              <div className="min-w-[120px] flex-[0.5] space-y-1">
                <MetricLabel icon={<Activity className="size-3.5" aria-hidden />}>
                  {t("loadavg")}
                </MetricLabel>
                <div className="text-xs">
                  {isWindows(host.os)
                    ? "N/A"
                    : [stats.load1, stats.load5, stats.load15]
                        .map((load) => load.toFixed(2))
                        .join(" | ")}
                </div>
              </div>
              <div className="min-w-[200px] flex-1 space-y-1">
                <MetricLabel icon={<Network className="size-3.5" aria-hidden />}>
                  {t("network")}
                </MetricLabel>
                <div className="flex gap-2 text-xs font-medium">
                  <span className="flex items-center gap-0.5">
                    <ArrowUp className="size-3" aria-label={t("upload")} />
                    {formatBitRate(stats.network_out_speed)}
                  </span>
                  <span className="flex items-center gap-0.5">
                    <ArrowDown className="size-3" aria-label={t("download")} />
                    {formatBitRate(stats.network_in_speed)}
                  </span>
                </div>
              </div>
            </div>
          </CardContent>
        </Card>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="flex flex-col rounded-b-xl border-t bg-muted px-4 py-2 md:flex-row">
          <NodeDetails node={node} />
          <div className="py-2 md:w-2/3 md:px-1 md:py-0">
            <NodeCharts history={history} />
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
})
