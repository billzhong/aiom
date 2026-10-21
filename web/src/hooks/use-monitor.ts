import { useContext } from "react"

import { MonitorContext } from "@/lib/monitor"

export function useMonitor() {
  const context = useContext(MonitorContext)
  if (context === undefined) throw new Error("useMonitor must be used within a MonitorProvider")
  return context
}
