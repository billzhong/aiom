import { cn } from "cn"
import {
  Activity,
  ArrowDown,
  ArrowUp,
  ChevronDown,
  Cpu,
  HardDrive,
  MemoryStick,
  MonitorCog,
  Network,
} from "lucide-react"
import { lazy, memo, Suspense } from "react"
import { useTranslation } from "react-i18next"

import { CountryFlag } from "@/components/country-flag"
import { NodeDetails } from "@/components/node-details"
import { OsLogo } from "@/components/os-logo"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Separator } from "@/components/ui/separator"
import { MetricLabel, UsageBar } from "@/components/usage-bar"
import { useMonitor } from "@/hooks/use-monitor"
import { formatBitRate, formatDuration, formatTimestamp, percentOf } from "@/lib/format"
import { isWindows } from "@/lib/os-icon"
import type { ChartPoint, NodeData } from "@/lib/types"

// Recharts is by far the largest dependency, and only needed once a node is expanded.
const NodeCharts = lazy(async () => ({
  default: (await import("@/components/node-charts")).NodeCharts,
}))

/** Holds the charts' place while their code loads, so the opening row does not jump. */
function ChartsFallback() {
  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      {[0, 1, 2, 3].map((panel) => (
        <div key={panel} className="aspect-video" />
      ))}
    </div>
  )
}

function OnlineDot({ online }: { online: boolean }) {
  return (
    <span className="relative flex size-1.5" aria-hidden>
      <span
        className={cn(
          "absolute inline-flex h-full w-full rounded-full opacity-75",
          online ? "bg-success motion-safe:animate-ping" : "bg-destructive",
        )}
      />
      <span
        className={cn(
          "relative inline-flex size-1.5 rounded-full",
          online ? "bg-success" : "bg-destructive",
        )}
      />
    </span>
  )
}

/**
 * How long an offline node has not reported, formatted like the uptime of online ones. Reads the
 * clock itself, so that only this span re-renders as it ticks, not the memoized row.
 */
function OfflineFor({ timestamp }: { timestamp: number }) {
  const { now } = useMonitor()
  return (
    // Above the overlay, so its title shows on hover.
    <span className="relative z-20 font-mono" title={formatTimestamp(timestamp)}>
      {formatDuration(now / 1000 - timestamp)}
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
    <Collapsible asChild open={open} onOpenChange={() => onToggle(node.name)}>
      <li className="rounded-xl border shadow-sm">
        <Card
          className={cn(
            "relative flex flex-col gap-1 border-none py-2 shadow-none transition-colors hover:bg-accent/50 xl:flex-row",
            open && "rounded-b-none",
          )}
        >
          <CardHeader className="w-auto flex-none px-4 pr-2 xl:w-1/6">
            <div className="flex items-center gap-2">
              <CountryFlag code={node.geoip} className="size-6" />
              <CardTitle>{node.name}</CardTitle>
              {/*
               * Its overlay makes the whole row the button, while the metrics stay outside of it:
               * a button's content is hidden from screen readers.
               */}
              <CollapsibleTrigger
                className={cn(
                  "ml-auto text-muted-foreground outline-none after:absolute after:inset-0 after:z-10 focus-visible:after:ring-[3px] focus-visible:after:ring-ring/50",
                  open ? "after:rounded-t-xl" : "after:rounded-xl",
                )}
                aria-label={node.name}
              >
                <ChevronDown
                  className={cn(
                    "size-4 transition-transform motion-reduce:transition-none",
                    open && "rotate-180",
                  )}
                  aria-hidden
                />
              </CollapsibleTrigger>
            </div>
            <CardDescription className="flex items-center gap-1">
              <OnlineDot online={online} />
              <span className="flex gap-3 text-xs font-medium">
                <span>{online ? t("online") : t("offline")}</span>
                {online ? (
                  <span className="font-mono">{formatDuration(stats.uptime)}</span>
                ) : (
                  <OfflineFor timestamp={node.timestamp} />
                )}
              </span>
            </CardDescription>
          </CardHeader>
          <div className="hidden items-center xl:flex">
            <Separator orientation="vertical" />
          </div>
          {/* Offline nodes keep showing their last sample, which is no longer live. */}
          <CardContent className={cn("w-full flex-1 px-4 pl-2", !online && "opacity-50")}>
            <div className="flex flex-wrap gap-4">
              <div className="min-w-25 flex-[0.5] space-y-1">
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
              <div className="min-w-30 flex-[0.5] space-y-1">
                <MetricLabel icon={<Activity className="size-3.5" aria-hidden />}>
                  {t("loadavg")}
                </MetricLabel>
                <div className="text-xs">
                  {isWindows(host.os)
                    ? t("not-available")
                    : [stats.load1, stats.load5, stats.load15]
                        .map((load) => load.toFixed(2))
                        .join(" | ")}
                </div>
              </div>
              <div className="min-w-50 flex-1 space-y-1">
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
        <CollapsibleContent className="overflow-hidden data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down motion-reduce:animate-none">
          <div className="flex flex-col rounded-b-xl border-t bg-muted px-4 py-2 md:flex-row">
            <NodeDetails node={node} />
            <div className="py-2 md:w-2/3 md:px-1 md:py-0">
              <Suspense fallback={<ChartsFallback />}>
                <NodeCharts history={history} />
              </Suspense>
            </div>
          </div>
        </CollapsibleContent>
      </li>
    </Collapsible>
  )
})
