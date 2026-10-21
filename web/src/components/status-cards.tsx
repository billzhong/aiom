import { cn } from "cn"
import {
  ArrowDown,
  ArrowUp,
  Cpu,
  HardDriveDownload,
  HardDriveUpload,
  MemoryStick,
  Percent,
  Power,
  PowerOff,
  Server,
} from "lucide-react"
import type { KeyboardEvent, ReactNode } from "react"
import { useTranslation } from "react-i18next"

import { Badge } from "@/components/ui/badge"
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useMonitor } from "@/hooks/use-monitor"
import { formatBitRate, formatBytes } from "@/lib/format"
import type { Status } from "@/lib/types"

function StatusCard({
  status,
  label,
  count,
  icon,
  className,
  selectedClassName,
}: {
  status: Status
  label: string
  count: number
  icon: ReactNode
  className?: string
  /** Applied while this status is the selected filter. */
  selectedClassName?: string
}) {
  const { status: selected, setStatus } = useMonitor()
  const select = () => setStatus(status)
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault()
      select()
    }
  }

  return (
    <Card
      // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- a <button> cannot hold <div>s
      role="button"
      tabIndex={0}
      aria-pressed={selected === status}
      aria-label={`${label}: ${count}`}
      onClick={select}
      onKeyDown={onKeyDown}
      className={cn(
        "@container/card py-2 transition-shadow hover:ring-2",
        className,
        selected === status && selectedClassName,
      )}
    >
      <CardHeader className="my-auto px-4">
        <CardDescription>{label}</CardDescription>
        <CardTitle className="mt-1 text-2xl tabular-nums">
          <div className="flex items-center gap-1">
            {icon} {count}
          </div>
        </CardTitle>
      </CardHeader>
    </Card>
  )
}

interface Stat {
  icon: ReactNode
  value: string
}

function StatBadge({ icon, value, variant }: Stat & { variant: "outline" | "secondary" }) {
  return (
    <Badge variant={variant} className="flex gap-1 rounded-lg text-xs">
      {icon} {value}
    </Badge>
  )
}

/** A card of fleet-wide totals: two rows, each of a figure and a related second figure. */
function TotalsCard({ title, rows }: { title: string; rows: [Stat, Stat][] }) {
  return (
    <Card className="@container/card py-2">
      <CardHeader className="my-auto px-4">
        <CardDescription>{title}</CardDescription>
        <CardTitle className="flex flex-col gap-1 font-light tabular-nums">
          {rows.map(([primary, secondary], row) => (
            <div key={row} className="flex items-center gap-1">
              <StatBadge variant="outline" {...primary} />
              <StatBadge variant="secondary" {...secondary} />
            </div>
          ))}
        </CardTitle>
      </CardHeader>
    </Card>
  )
}

/** Node counts by status, which double as the status filter, plus fleet-wide totals. */
export function StatusCards() {
  const { t } = useTranslation()
  const { summary } = useMonitor()

  return (
    <div className="grid grid-cols-1 gap-4 px-2 *:data-[slot=card]:bg-linear-to-t *:data-[slot=card]:from-primary/5 *:data-[slot=card]:to-card @2xl/main:grid-cols-2 @4xl/main:grid-cols-3 @5xl/main:grid-cols-5 dark:*:data-[slot=card]:bg-card">
      <StatusCard
        status="all"
        label={t("total")}
        count={summary.total}
        icon={<Server className="size-5 text-primary" aria-hidden />}
      />
      <StatusCard
        status="online"
        label={t("online")}
        count={summary.online}
        icon={<Power className="size-5 animate-pulse text-chart-2" aria-hidden />}
        className="hover:ring-chart-2"
        selectedClassName="ring-2 ring-chart-2"
      />
      <StatusCard
        status="offline"
        label={t("offline")}
        count={summary.offline}
        icon={<PowerOff className="size-5 animate-pulse text-destructive" aria-hidden />}
        className="hover:ring-destructive"
        selectedClassName="ring-2 ring-destructive"
      />
      <TotalsCard
        title={t("hardware")}
        rows={[
          [
            { icon: <Cpu className="size-3" />, value: String(summary.cpuCores) },
            { icon: <Percent className="size-3" />, value: summary.cpuUsage.toFixed(2) },
          ],
          [
            { icon: <MemoryStick className="size-3" />, value: formatBytes(summary.totalMemory) },
            { icon: <Percent className="size-3" />, value: summary.memoryUsage.toFixed(2) },
          ],
        ]}
      />
      <TotalsCard
        title={t("network")}
        rows={[
          [
            { icon: <ArrowUp className="size-3" />, value: formatBitRate(summary.uploadSpeed) },
            { icon: <HardDriveUpload className="size-3" />, value: formatBytes(summary.uploaded) },
          ],
          [
            { icon: <ArrowDown className="size-3" />, value: formatBitRate(summary.downloadSpeed) },
            {
              icon: <HardDriveDownload className="size-3" />,
              value: formatBytes(summary.downloaded),
            },
          ],
        ]}
      />
    </div>
  )
}
