import type { ReactNode } from "react"

import { Progress } from "@/components/ui/progress"
import { formatPercent } from "@/lib/format"

function levelClass(percent: number) {
  if (percent > 95) return "*:data-[slot=progress-indicator]:bg-destructive"
  if (percent > 75) return "*:data-[slot=progress-indicator]:bg-warning"
  return "*:data-[slot=progress-indicator]:bg-success"
}

/** The icon and name of a metric, above its value. */
export function MetricLabel({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <span className="flex items-center gap-1 text-sm text-muted-foreground">
      {icon} {children}
    </span>
  )
}

/** A labelled percentage bar that turns amber above 75% and red above 95%. */
export function UsageBar({
  icon,
  label,
  percent,
}: {
  icon: ReactNode
  label: string
  percent: number
}) {
  const value = Math.min(Math.max(percent, 0), 100)
  return (
    <div className="min-w-37.5 flex-1 space-y-2">
      <div className="flex justify-between text-sm">
        <MetricLabel icon={icon}>{label}</MetricLabel>
        <span className="font-medium">{formatPercent(percent)}</span>
      </div>
      <Progress
        value={value}
        // The shadcn/ui Progress does not pass `value` on to Radix, which would set this.
        aria-valuenow={value}
        aria-label={label}
        className={levelClass(percent)}
      />
    </div>
  )
}
