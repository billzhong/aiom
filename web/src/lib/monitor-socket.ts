import type { NodeData } from "@/lib/types"

/** Text frame asking the server for the node list. */
export const PING = "ping"

/** Dashboard route of the server, used when no URL is configured. */
const DEFAULT_PATH = "/ws"

/** Delay between receiving a node list and asking for the next, in ms. */
const POLL_INTERVAL = 1000

/** Delay before the first reconnect, in ms; it doubles with every failed attempt in a row. */
const RECONNECT_DELAY = 5000

/** Longest delay between reconnects, in ms. */
const MAX_RECONNECT_DELAY = 60_000

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
  /** The server sent something that is not a node list. */
  | "bad-data"

interface MonitorSocketOptions {
  url: string
  onNodes: (nodes: NodeData[]) => void
  onError: (error: MonitorError) => void
}

/** Decodes a gzip-compressed JSON node list. */
export async function decodeNodes(data: ArrayBuffer): Promise<NodeData[]> {
  const json = new Response(data).body!.pipeThrough(new DecompressionStream("gzip"))
  const nodes: unknown = await new Response(json).json()
  if (!Array.isArray(nodes)) throw new TypeError("node list is not an array")
  return nodes as NodeData[]
}

/**
 * Polls the server for the node list over a WebSocket: sends a ping, waits for the answer,
 * then pings again after `POLL_INTERVAL`. Reconnects when the connection drops or the server
 * stops answering, backing off while attempts keep failing, but never giving up: a dashboard can
 * stay open for days. Waiting is cut short when the browser goes back online or the page is shown.
 */
export class MonitorSocket {
  private readonly options: MonitorSocketOptions
  private socket: WebSocket | null = null
  private failedAttempts = 0
  private stopped = true
  private pollTimer: ReturnType<typeof setTimeout> | undefined
  private responseTimer: ReturnType<typeof setTimeout> | undefined
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined

  constructor(options: MonitorSocketOptions) {
    this.options = options
  }

  start(): void {
    if (!this.stopped) return
    this.stopped = false
    this.failedAttempts = 0
    window.addEventListener("online", this.reconnectNow)
    document.addEventListener("visibilitychange", this.reconnectNow)
    this.connect()
  }

  stop(): void {
    this.stopped = true
    window.removeEventListener("online", this.reconnectNow)
    document.removeEventListener("visibilitychange", this.reconnectNow)
    this.clearTimers()
    const socket = this.socket
    this.socket = null
    if (socket) {
      socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null
      socket.close()
    }
  }

  private connect(): void {
    const socket = new WebSocket(this.options.url)
    socket.binaryType = "arraybuffer"
    socket.onopen = () => {
      this.failedAttempts = 0
      this.ping()
    }
    // Browsers ignore the returned promise; tests wait on it.
    socket.onmessage = (event: MessageEvent) => this.receive(socket, event.data)
    socket.onclose = () => this.reconnect()
    this.socket = socket
  }

  private async receive(socket: WebSocket, data: unknown): Promise<void> {
    clearTimeout(this.responseTimer)
    try {
      if (!(data instanceof ArrayBuffer)) throw new TypeError("expected a binary message")
      const nodes = await decodeNodes(data)
      // The connection may have been stopped or replaced while decoding.
      if (socket !== this.socket) return
      this.options.onNodes(nodes)
    } catch (error) {
      if (socket !== this.socket) return
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
    const delay = Math.min(RECONNECT_DELAY * 2 ** this.failedAttempts, MAX_RECONNECT_DELAY)
    this.failedAttempts += 1
    this.reconnectTimer = setTimeout(() => this.connect(), delay)
  }

  /** Skips the rest of the wait for a scheduled reconnect, unless the page is hidden. */
  private readonly reconnectNow = (): void => {
    // Listening only while started, a reconnect is pending exactly when there is no socket.
    if (this.socket || document.visibilityState === "hidden") return
    clearTimeout(this.reconnectTimer)
    this.connect()
  }

  private clearTimers(): void {
    clearTimeout(this.pollTimer)
    clearTimeout(this.responseTimer)
    clearTimeout(this.reconnectTimer)
  }
}
