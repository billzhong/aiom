/** Static facts about a host, gathered once when its client starts. */
export interface Host {
  agent: string
  os: string
  os_version: string
  cpu: string
  cpu_count: number
  cpu_arch: string
  /** Bytes. */
  total_memory: number
  /** Bytes. */
  total_swap: number
  /** Bytes. */
  total_disk: number
  /** Virtualization technology as reported by `systemd-detect-virt`, or "none". */
  v12n: string
}

/** Metrics sampled by the client on every tick. */
export interface Stats {
  /** Percent. */
  cpu_usage: number
  used_memory: number
  used_swap: number
  used_disk: number
  /** Bytes received since boot. */
  network_in: number
  /** Bytes sent since boot. */
  network_out: number
  /** Bytes per second. */
  network_in_speed: number
  /** Bytes per second. */
  network_out_speed: number
  /** Seconds. */
  uptime: number
  /** Unix timestamp in seconds. */
  boot_time: number
  load1: number
  load5: number
  load15: number
  tcp_count: number
  udp_count: number
  process_count: number
}

/** One node as sent by the server. */
export interface NodeData {
  /** Unix timestamp in seconds of the last report the server received. */
  timestamp: number
  name: string
  host: Host
  stats: Stats
  /** ISO 3166-1 alpha-2 country code, or empty when unknown. */
  geoip: string
}

export type Status = "all" | "online" | "offline"

/** One sample of a node's history charts. */
export interface ChartPoint {
  /** Unix timestamp in seconds. */
  timestamp: number
  /** Percent. */
  cpu: number
  /** Percent. */
  ram: number
  /** Percent. */
  swap: number
  /** Bytes per second. */
  download: number
  /** Bytes per second. */
  upload: number
  tcp: number
  udp: number
}

export type Charts = Record<string, ChartPoint[]>
