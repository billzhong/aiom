import { createContext } from "react"

import type { Summary } from "@/lib/nodes"
import type { Charts, NodeData, Status } from "@/lib/types"

export interface MonitorState {
  /** All nodes, in the server's (natural name) order. */
  nodes: NodeData[]
  /** Nodes matching the location, name and status filters. */
  visibleNodes: NodeData[]
  /** Totals over the nodes matching the location and name filters. */
  summary: Summary
  charts: Charts
  /** Current time in ms, updated every second; online status is relative to it. */
  now: number
  status: Status
  setStatus: (status: Status) => void
  location: string
  setLocation: (location: string) => void
  query: string
  setQuery: (query: string) => void
}

export const MonitorContext = createContext<MonitorState | undefined>(undefined)
