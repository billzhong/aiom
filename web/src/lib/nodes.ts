import { isOnline, percentOf } from "@/lib/format"
import type { ChartPoint, Charts, NodeData, Status } from "@/lib/types"

/** Number of samples kept per node for the history charts. */
const MAX_CHART_POINTS = 20

/** The tab value selecting nodes of every location. */
export const ALL_LOCATIONS = "all"

/** Country codes of all nodes, sorted, with unknown ("") last. */
export function locationsOf(nodes: readonly NodeData[]): string[] {
  const codes = [...new Set(nodes.map((node) => node.geoip))].sort()
  if (codes[0] === "") codes.push(codes.shift()!)
  return codes
}

export function filterNodes(
  nodes: readonly NodeData[],
  { location, query }: { location: string; query: string },
): NodeData[] {
  const needle = query.trim().toLowerCase()
  return nodes.filter(
    (node) =>
      (location === ALL_LOCATIONS || node.geoip === location) &&
      (!needle || node.name.toLowerCase().includes(needle)),
  )
}

export function filterByStatus(
  nodes: readonly NodeData[],
  status: Status,
  nowMs: number,
): NodeData[] {
  if (status === "all") return [...nodes]
  const wantOnline = status === "online"
  return nodes.filter((node) => isOnline(node.timestamp, nowMs) === wantOnline)
}

export interface Summary {
  total: number
  online: number
  offline: number
  /** Physical cores of all nodes. */
  cpuCores: number
  /** Average over online nodes, in percent. */
  cpuUsage: number
  /** Bytes, of all nodes. */
  totalMemory: number
  /** Used share of the memory of online nodes, in percent. */
  memoryUsage: number
  /** Bytes per second, over online nodes. */
  downloadSpeed: number
  /** Bytes per second, over online nodes. */
  uploadSpeed: number
  /** Bytes received since boot, of all nodes. */
  downloaded: number
  /** Bytes sent since boot, of all nodes. */
  uploaded: number
}

/**
 * Totals over `nodes`. Rates and usage only count online nodes, since offline nodes keep
 * reporting the stale values of their last sample.
 */
export function summarize(nodes: readonly NodeData[], nowMs: number): Summary {
  const summary: Summary = {
    total: nodes.length,
    online: 0,
    offline: 0,
    cpuCores: 0,
    cpuUsage: 0,
    totalMemory: 0,
    memoryUsage: 0,
    downloadSpeed: 0,
    uploadSpeed: 0,
    downloaded: 0,
    uploaded: 0,
  }
  let cpuUsageSum = 0
  let onlineMemory = 0
  let onlineUsedMemory = 0

  for (const { timestamp, host, stats } of nodes) {
    summary.cpuCores += host.cpu_count
    summary.totalMemory += host.total_memory
    summary.downloaded += stats.network_in
    summary.uploaded += stats.network_out
    if (!isOnline(timestamp, nowMs)) continue

    summary.online += 1
    cpuUsageSum += stats.cpu_usage
    onlineMemory += host.total_memory
    onlineUsedMemory += stats.used_memory
    summary.downloadSpeed += stats.network_in_speed
    summary.uploadSpeed += stats.network_out_speed
  }

  summary.offline = summary.total - summary.online
  summary.cpuUsage = summary.online ? cpuUsageSum / summary.online : 0
  summary.memoryUsage = percentOf(onlineUsedMemory, onlineMemory)
  return summary
}

function chartPointOf(node: NodeData): ChartPoint {
  const { timestamp, host, stats } = node
  return {
    timestamp,
    time: new Date(timestamp * 1000).toLocaleTimeString(),
    cpu: stats.cpu_usage,
    ram: percentOf(stats.used_memory, host.total_memory),
    swap: percentOf(stats.used_swap, host.total_swap),
    download: stats.network_in_speed,
    upload: stats.network_out_speed,
    tcp: stats.tcp_count,
    udp: stats.udp_count,
  }
}

/**
 * Adds the latest sample of every node to its chart history. Repeated samples (a node that
 * has not reported since) are skipped, and the history of nodes that disappeared is dropped.
 */
export function appendChartPoints(
  charts: Charts,
  nodes: readonly NodeData[],
  maxPoints = MAX_CHART_POINTS,
): Charts {
  // Node names can match Object.prototype keys, including __proto__.
  const next: Charts = Object.create(null)
  for (const node of nodes) {
    const history = Object.hasOwn(charts, node.name) ? charts[node.name] : []
    const last = history.at(-1)
    next[node.name] =
      last?.timestamp === node.timestamp
        ? history
        : [...history, chartPointOf(node)].slice(-maxPoints)
  }
  return next
}
