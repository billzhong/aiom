/**
 * Nodes count as online while their last report is at most this old. Leaves room for missed
 * reports (clients report at least every 5 s), reconnects, and clock differences between the
 * server and the browser.
 */
export const ONLINE_THRESHOLD_SECONDS = 30

export function percentOf(value: number, total: number): number {
  if (!value || !total) return 0
  return (value / total) * 100
}

export function isOnline(timestamp: number, nowMs: number): boolean {
  return Math.abs(timestamp - nowMs / 1000) <= ONLINE_THRESHOLD_SECONDS
}

/** Formats a percentage with two decimals, e.g. `12.50%`. */
export function formatPercent(percent: number): string {
  return `${percent.toFixed(2)}%`
}

const pad = (value: number) => value.toString().padStart(2, "0")

/** Formats a Unix timestamp in seconds as local `YYYY-MM-DD HH:mm:ss`. */
export function formatTimestamp(timestamp: number): string {
  if (!timestamp) return "-"
  const date = new Date(timestamp * 1000)
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ` +
    `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  )
}

/** Formats a duration in seconds as `1d 2h 3m 4s`. */
export function formatDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds))
  const days = Math.floor(total / 86400)
  const hours = Math.floor((total % 86400) / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  return `${days}d ${hours}h ${minutes}m ${total % 60}s`
}

function formatWithUnits(value: number, base: number, units: readonly string[]): string {
  if (!(value > 0)) return `0 ${units[0]}`
  const exponent = Math.min(Math.floor(Math.log(value) / Math.log(base)), units.length - 1)
  return `${(value / base ** exponent).toFixed(2)} ${units[exponent]}`
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB", "EB"] as const
const BIT_RATE_UNITS = ["bps", "Kbps", "Mbps", "Gbps", "Tbps", "Pbps", "Ebps"] as const

/** Formats a byte count with binary multiples, e.g. `1.50 KB` for 1536. */
export function formatBytes(bytes: number): string {
  return formatWithUnits(bytes, 1024, BYTE_UNITS)
}

/** Formats a speed given in bytes per second as bits per second with SI multiples. */
export function formatBitRate(bytesPerSecond: number): string {
  return formatWithUnits(bytesPerSecond * 8, 1000, BIT_RATE_UNITS)
}
