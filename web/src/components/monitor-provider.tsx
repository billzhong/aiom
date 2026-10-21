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
        const key = {
          disconnected: "websocket.error-reconnect",
          "gave-up": "websocket.error-max-attempts",
          "bad-data": "websocket.error-data",
        }[error]
        toast.error(i18n.t(key), { id: TOAST_ID })
        toastShown = true
      },
    })
    socket.start()
    return () => socket.stop()
  }, [i18n])

  const value = useMemo<MonitorState>(() => {
    const { nodes, charts } = snapshot
    const filteredNodes = filterNodes(nodes, { location, query })
    return {
      nodes,
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
  }, [snapshot, now, status, location, query])

  return <MonitorContext.Provider value={value}>{children}</MonitorContext.Provider>
}
