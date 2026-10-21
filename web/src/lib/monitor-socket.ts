import { decompressSync, strFromU8 } from "fflate"

import type { NodeData } from "@/lib/types"

/** Text frame asking the server for the node list. */
export const PING = "ping"

/** Dashboard route of the server, used when no URL is configured. */
const DEFAULT_PATH = "/ws"

/** Delay between receiving a node list and asking for the next, in ms. */
const POLL_INTERVAL = 1000

/** Delay before reconnecting, in ms. */
const RECONNECT_DELAY = 5000

/** Reconnect when a ping is not answered within this many ms. */
const RESPONSE_TIMEOUT = 10_000

/**
 * Resolves the dashboard URL of the server against the page at `base`: absolute URLs stay as
 * they are, while a path, `DEFAULT_PATH` by default, points to the server serving the page.
 */
export function resolveSocketUrl(url: string | undefined, base: string): string {
  const resolved = new URL(url || DEFAULT_PATH, base)
  if (resolved.protocol === "http:") resolved.protocol = "ws:"
  else if (resolved.protocol === "https:") resolved.protocol = "wss:"
  return resolved.href
}

export type MonitorError =
  /** The connection closed; a reconnect is scheduled. */
  | "disconnected"
  /** Too many reconnects failed in a row; no further attempts are made. */
  | "gave-up"
  /** The server sent something that is not a node list. */
  | "bad-data"

interface MonitorSocketOptions {
  url: string
  onNodes: (nodes: NodeData[]) => void
  onError: (error: MonitorError) => void
  /** Reconnects to attempt after a disconnect before giving up. */
  maxReconnectAttempts?: number
}

/** Decodes a gzip-compressed JSON node list. */
export function decodeNodes(data: ArrayBuffer): NodeData[] {
  const nodes: unknown = JSON.parse(strFromU8(decompressSync(new Uint8Array(data))))
  if (!Array.isArray(nodes)) throw new TypeError("node list is not an array")
  return nodes as NodeData[]
}

/**
 * Polls the server for the node list over a WebSocket: sends a ping, waits for the answer,
 * then pings again after `POLL_INTERVAL`. Reconnects when the connection drops or the server
 * stops answering.
 */
export class MonitorSocket {
  private readonly options: Required<MonitorSocketOptions>
  private socket: WebSocket | null = null
  private failedAttempts = 0
  private stopped = true
  private pollTimer: ReturnType<typeof setTimeout> | undefined
  private responseTimer: ReturnType<typeof setTimeout> | undefined
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined

  constructor(options: MonitorSocketOptions) {
    this.options = { maxReconnectAttempts: 10, ...options }
  }

  start(): void {
    if (!this.stopped) return
    this.stopped = false
    this.failedAttempts = 0
    this.connect()
  }

  stop(): void {
    this.stopped = true
    this.clearTimers()
    const socket = this.socket
    this.socket = null
    if (socket) {
      socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null
      socket.close()
    }
  }

  private connect(): void {
    if (this.failedAttempts >= this.options.maxReconnectAttempts) {
      this.options.onError("gave-up")
      return
    }

    const socket = new WebSocket(this.options.url)
    socket.binaryType = "arraybuffer"
    socket.onopen = () => {
      this.failedAttempts = 0
      this.ping()
    }
    socket.onmessage = (event: MessageEvent) => this.receive(event.data)
    socket.onclose = () => this.reconnect()
    this.socket = socket
  }

  private receive(data: unknown): void {
    clearTimeout(this.responseTimer)
    try {
      if (!(data instanceof ArrayBuffer)) throw new TypeError("expected a binary message")
      this.options.onNodes(decodeNodes(data))
    } catch (error) {
      console.error("Invalid node list:", error)
      this.options.onError("bad-data")
    }
    clearTimeout(this.pollTimer)
    this.pollTimer = setTimeout(() => this.ping(), POLL_INTERVAL)
  }

  private ping(): void {
    const socket = this.socket
    if (socket?.readyState !== WebSocket.OPEN) return
    socket.send(PING)
    clearTimeout(this.responseTimer)
    // A half-open connection never closes by itself; give up on it after a while.
    this.responseTimer = setTimeout(() => socket.close(), RESPONSE_TIMEOUT)
  }

  private reconnect(): void {
    this.clearTimers()
    this.socket = null
    if (this.stopped) return
    this.options.onError("disconnected")
    this.failedAttempts += 1
    this.reconnectTimer = setTimeout(() => this.connect(), RECONNECT_DELAY)
  }

  private clearTimers(): void {
    clearTimeout(this.pollTimer)
    clearTimeout(this.responseTimer)
    clearTimeout(this.reconnectTimer)
  }
}
