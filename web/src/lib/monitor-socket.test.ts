import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import {
  decodeNodes,
  type MonitorError,
  MonitorSocket,
  PING,
  resolveSocketUrl,
} from "@/lib/monitor-socket"
import type { NodeData } from "@/lib/types"
import { encodePayload, FakeWebSocket, makeNode } from "@/test/fixtures"

function setup(options: { maxReconnectAttempts?: number } = {}) {
  const received: NodeData[][] = []
  const errors: MonitorError[] = []
  const monitor = new MonitorSocket({
    url: "ws://monitor.test/ws",
    onNodes: (nodes) => received.push(nodes),
    onError: (error) => errors.push(error),
    ...options,
  })
  return { monitor, received, errors }
}

beforeEach(() => {
  vi.useFakeTimers()
  FakeWebSocket.instances = []
  vi.stubGlobal("WebSocket", FakeWebSocket)
})

afterEach(() => {
  vi.useRealTimers()
})

describe("decodeNodes", () => {
  it("decodes gzip-compressed JSON", () => {
    const nodes = [makeNode()]
    expect(decodeNodes(encodePayload(nodes))).toEqual(nodes)
  })

  it("rejects payloads that are not node lists", () => {
    expect(() => decodeNodes(encodePayload({ not: "a list" }))).toThrow("node list is not an array")
    expect(() => decodeNodes(new TextEncoder().encode("plain").buffer)).toThrow("unexpected EOF")
  })
})

describe("resolveSocketUrl", () => {
  it("keeps absolute URLs", () => {
    expect(resolveSocketUrl("wss://monitor.example/ws", "https://dash.example/")).toBe(
      "wss://monitor.example/ws",
    )
  })

  it("resolves paths against the page", () => {
    expect(resolveSocketUrl(undefined, "https://monitor.example/")).toBe("wss://monitor.example/ws")
    expect(resolveSocketUrl("", "http://localhost:5173/")).toBe("ws://localhost:5173/ws")
    expect(resolveSocketUrl("/live", "https://monitor.example:8443/index.html")).toBe(
      "wss://monitor.example:8443/live",
    )
  })
})

describe("MonitorSocket", () => {
  it("pings on open and again after each answer", () => {
    const { monitor, received } = setup()
    monitor.start()
    const socket = FakeWebSocket.latest()
    expect(socket.url).toBe("ws://monitor.test/ws")
    expect(socket.binaryType).toBe("arraybuffer")
    expect(socket.sent).toEqual([])

    socket.open()
    expect(socket.sent).toEqual([PING])

    const nodes = [makeNode()]
    socket.receiveNodes(nodes)
    expect(received).toEqual([nodes])
    expect(socket.sent).toHaveLength(1)

    vi.advanceTimersByTime(999)
    expect(socket.sent).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(socket.sent).toEqual([PING, PING])
    monitor.stop()
  })

  it("keeps polling after a bad message", () => {
    const { monitor, received, errors } = setup()
    vi.spyOn(console, "error").mockImplementation(() => {})
    monitor.start()
    const socket = FakeWebSocket.latest()
    socket.open()

    socket.receive("not binary")
    expect(errors).toEqual(["bad-data"])
    vi.advanceTimersByTime(1000)
    expect(socket.sent).toEqual([PING, PING])

    socket.receiveNodes([])
    expect(received).toEqual([[]])
    monitor.stop()
  })

  it("reconnects after the connection closes", () => {
    const { monitor, errors } = setup()
    monitor.start()
    const first = FakeWebSocket.latest()
    first.open()
    first.close()
    expect(errors).toEqual(["disconnected"])

    vi.advanceTimersByTime(4999)
    expect(FakeWebSocket.instances).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(FakeWebSocket.instances).toHaveLength(2)

    FakeWebSocket.latest().open()
    expect(FakeWebSocket.latest().sent).toEqual([PING])
    monitor.stop()
  })

  it("gives up after too many failed reconnects in a row", () => {
    const { monitor, errors } = setup({ maxReconnectAttempts: 3 })
    monitor.start()
    for (let attempt = 0; attempt < 3; attempt++) {
      FakeWebSocket.latest().close()
      vi.advanceTimersByTime(5000)
    }
    expect(FakeWebSocket.instances).toHaveLength(3)
    expect(errors).toEqual(["disconnected", "disconnected", "disconnected", "gave-up"])
    vi.advanceTimersByTime(60_000)
    expect(FakeWebSocket.instances).toHaveLength(3)
    monitor.stop()
  })

  it("resets the attempt count after a successful connection", () => {
    const { monitor, errors } = setup({ maxReconnectAttempts: 2 })
    monitor.start()
    for (let i = 0; i < 5; i++) {
      FakeWebSocket.latest().open()
      FakeWebSocket.latest().close()
      vi.advanceTimersByTime(5000)
    }
    expect(errors).not.toContain("gave-up")
    expect(FakeWebSocket.instances).toHaveLength(6)
    monitor.stop()
  })

  it("drops a connection whose server stopped answering", () => {
    const { monitor } = setup()
    monitor.start()
    const socket = FakeWebSocket.latest()
    socket.open()
    vi.advanceTimersByTime(10_000)
    expect(socket.closed).toBe(true)
    vi.advanceTimersByTime(5000)
    expect(FakeWebSocket.instances).toHaveLength(2)
    monitor.stop()
  })

  it("stops without reconnecting or reporting errors", () => {
    const { monitor, errors } = setup()
    monitor.start()
    const socket = FakeWebSocket.latest()
    socket.open()
    socket.receiveNodes([])
    monitor.stop()

    expect(socket.closed).toBe(true)
    vi.advanceTimersByTime(60_000)
    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(socket.sent).toEqual([PING])
    expect(errors).toEqual([])
  })

  it("ignores repeated starts", () => {
    const { monitor } = setup()
    monitor.start()
    monitor.start()
    expect(FakeWebSocket.instances).toHaveLength(1)
    monitor.stop()
  })
})
