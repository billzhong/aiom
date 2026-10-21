import { useCallback, useState } from "react"
import { useTranslation } from "react-i18next"

import { NodeItem } from "@/components/node-item"
import { useMonitor } from "@/hooks/use-monitor"
import { isOnline } from "@/lib/format"
import type { ChartPoint } from "@/lib/types"

/** Shared by nodes without history, so their memoized rows see the same prop. */
const NO_HISTORY: ChartPoint[] = []

/** The nodes matching all filters. */
export function NodeList() {
  const { t } = useTranslation()
  const { nodes, visibleNodes, charts, now } = useMonitor()
  /** Name of the node whose details are open, or "". */
  const [expanded, setExpanded] = useState("")
  const toggle = useCallback(
    (name: string) => setExpanded((current) => (current === name ? "" : name)),
    [],
  )

  if (visibleNodes.length === 0) {
    return (
      <p className="px-2 py-8 text-center text-sm text-muted-foreground">
        {nodes.length === 0 ? t("waiting") : t("no-match")}
      </p>
    )
  }

  return (
    <div className="flex flex-col gap-4 px-2">
      {visibleNodes.map((node) => (
        <NodeItem
          key={node.name}
          node={node}
          online={isOnline(node.timestamp, now)}
          history={charts[node.name] ?? NO_HISTORY}
          open={expanded === node.name}
          onToggle={toggle}
        />
      ))}
    </div>
  )
}
