import { type ReactNode, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { MonitorContext, type MonitorState } from "@/lib/monitor"
import { MonitorSocket, resolveSocketUrl } from "@/lib/monitor-socket"
import {
  ALL_LOCATIONS,
  appendChartPoints,
  filterByStatus,
  filterNodes,
  locationsOf,
  summarize,
} from "@/lib/nodes"
import type { Charts, NodeData, Status } from "@/lib/types"

const DEFAULT_URL = resolveSocketUrl(import.meta.env.VITE_WS_URL, window.location.href)

/** Toasts share an id, so a flapping connection shows one toast instead of a pile. */
const TOAST_ID = "monitor-connection"

interface Snapshot {
  nodes: NodeData[]
  charts: Charts
}

export function MonitorProvider({ children }: { children: ReactNode }) {
  const { i18n } = useTranslation()
  const [snapshot, setSnapshot] = useState<Snapshot>({ nodes: [], charts: Object.create(null) })
  const [status, setStatus] = useState<Status>("all")
  const [location, setLocation] = useState(ALL_LOCATIONS)
  const [query, setQuery] = useState("")
  // Ticks on its own, so nodes still go offline while no node lists arrive.
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])

  useEffect(() => {
    // Dismissing re-renders the toaster even without a toast, so only do it when one is shown.
    let toastShown = false
    const socket = new MonitorSocket({
      url: DEFAULT_URL,
      onNodes: (nodes) => {
        setSnapshot((previous) => ({ nodes, charts: appendChartPoints(previous.charts, nodes) }))
        if (toastShown) {
          toast.dismiss(TOAST_ID)
          toastShown = false
        }
      },
      onError: (error) => {
        const badData = error === "bad-data"
        toast.error(i18n.t(badData ? "websocket.error-data" : "websocket.error-reconnect"), {
          id: TOAST_ID,
          // Stays up until the next node list arrives, however long reconnecting takes.
          duration: Infinity,
          // Set either way: updating the shown toast keeps any option left out, such as this one.
          action: badData
            ? { label: i18n.t("refresh"), onClick: () => window.location.reload() }
            : undefined,
        })
        toastShown = true
      },
    })
    socket.start()
    return () => socket.stop()
  }, [i18n])

  const locations = useMemo(() => locationsOf(snapshot.nodes), [snapshot.nodes])
  // Once the selected location has no nodes left, it has no tab left to show it is selected.
  if (location !== ALL_LOCATIONS && !locations.has(location)) setLocation(ALL_LOCATIONS)

  const value = useMemo<MonitorState>(() => {
    const { nodes, charts } = snapshot
    const filteredNodes = filterNodes(nodes, { location, query })
    return {
      nodes,
      locations,
      visibleNodes: filterByStatus(filteredNodes, status, now),
      summary: summarize(filteredNodes, now),
      charts,
      now,
      status,
      setStatus,
      location,
      setLocation,
      query,
      setQuery,
    }
  }, [snapshot, locations, now, status, location, query])

  return <MonitorContext.Provider value={value}>{children}</MonitorContext.Provider>
}
