import { gzipSync, strToU8 } from "fflate"

import type { Host, NodeData, Stats } from "@/lib/types"

export const nowSeconds = () => Math.floor(Date.now() / 1000)

export type NodeOverrides = Partial<Omit<NodeData, "host" | "stats">> & {
  host?: Partial<Host>
  stats?: Partial<Stats>
}

/** A node that reported just now, with every field filled in unless overridden. */
export function makeNode({ host, stats, ...overrides }: NodeOverrides = {}): NodeData {
  return {
    timestamp: nowSeconds(),
    name: "node-1",
    geoip: "JP",
    host: {
      agent: "0.1.0",
      os: "Ubuntu",
      os_version: "24.04",
      cpu: "AMD EPYC 7B13",
      cpu_count: 2,
      cpu_arch: "x86_64",
      total_memory: 4 * 1024 ** 3,
      total_swap: 1024 ** 3,
      total_disk: 100 * 1024 ** 3,
      v12n: "kvm",
      ...host,
    },
    stats: {
      cpu_usage: 25,
      used_memory: 1024 ** 3,
      used_swap: 0,
      used_disk: 50 * 1024 ** 3,
      network_in: 10 * 1024 ** 3,
      network_out: 5 * 1024 ** 3,
      network_in_speed: 125_000,
      network_out_speed: 250_000,
      uptime: 90061,
      boot_time: nowSeconds() - 90061,
      load1: 0.5,
      load5: 0.25,
      load15: 0.125,
      tcp_count: 12,
      udp_count: 3,
      process_count: 120,
      ...stats,
    },
    ...overrides,
  }
}

/** Encodes a value like the server does: gzip-compressed JSON. */
export function encodePayload(value: unknown): ArrayBuffer {
  const bytes = gzipSync(strToU8(JSON.stringify(value)))
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
}

/** Stands in for the browser WebSocket; tests drive the server side. */
export class FakeWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3

  static instances: FakeWebSocket[] = []

  static latest(): FakeWebSocket {
    const socket = FakeWebSocket.instances.at(-1)
    if (!socket) throw new Error("no socket was created")
    return socket
  }

  readyState: number = FakeWebSocket.CONNECTING
  binaryType = "blob"
  sent: string[] = []
  closed = false
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null

  readonly url: string

  constructor(url: string) {
    this.url = url
    FakeWebSocket.instances.push(this)
  }

  send(data: string) {
    this.sent.push(data)
  }

  close() {
    if (this.closed) return
    this.closed = true
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.(new CloseEvent("close"))
  }

  // Server side.

  open() {
    this.readyState = FakeWebSocket.OPEN
    this.onopen?.(new Event("open"))
  }

  receive(data: unknown) {
    this.onmessage?.(new MessageEvent("message", { data }))
  }

  receiveNodes(nodes: NodeData[]) {
    this.receive(encodePayload(nodes))
  }
}
