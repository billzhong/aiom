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

function setup() {
  const received: NodeData[][] = []
  const errors: MonitorError[] = []
  const monitor = new MonitorSocket({
    url: "ws://monitor.test/ws",
    onNodes: (nodes) => received.push(nodes),
    onError: (error) => errors.push(error),
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
  it("decodes gzip-compressed JSON", async () => {
    const nodes = [makeNode({ name: "東京-α" })]
    expect(await decodeNodes(await encodePayload(nodes))).toEqual(nodes)
  })

  it("rejects payloads that are not node lists", async () => {
    await expect(decodeNodes(await encodePayload({ not: "a list" }))).rejects.toThrow(
      "node list is not an array",
    )
  })

  it("rejects payloads that are not gzip, or cut short", async () => {
    // Engines word decompression errors differently; all throw a TypeError.
    const plain = await new Response("plain").arrayBuffer()
    await expect(decodeNodes(plain)).rejects.toBeInstanceOf(TypeError)
    const truncated = (await encodePayload([makeNode()])).slice(0, -4)
    await expect(decodeNodes(truncated)).rejects.toBeInstanceOf(TypeError)
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
  it("pings on open and again after each answer", async () => {
    const { monitor, received } = setup()
    monitor.start()
    const socket = FakeWebSocket.latest()
    expect(socket.url).toBe("ws://monitor.test/ws")
    expect(socket.binaryType).toBe("arraybuffer")
    expect(socket.sent).toEqual([])

    socket.open()
    expect(socket.sent).toEqual([PING])

    const nodes = [makeNode()]
    await socket.receiveNodes(nodes)
    expect(received).toEqual([nodes])
    expect(socket.sent).toHaveLength(1)

    vi.advanceTimersByTime(999)
    expect(socket.sent).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(socket.sent).toEqual([PING, PING])
    monitor.stop()
  })

  it("keeps polling after a bad message", async () => {
    const { monitor, received, errors } = setup()
    vi.spyOn(console, "error").mockImplementation(() => {})
    monitor.start()
    const socket = FakeWebSocket.latest()
    socket.open()

    await socket.receive("not binary")
    expect(errors).toEqual(["bad-data"])
    vi.advanceTimersByTime(1000)
    expect(socket.sent).toEqual([PING, PING])

    await socket.receiveNodes([])
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

  it("backs off while reconnects keep failing, but never gives up", () => {
    const { monitor, errors } = setup()
    monitor.start()
    for (const delay of [5000, 10_000, 20_000, 40_000, 60_000, 60_000]) {
      const count = FakeWebSocket.instances.length
      FakeWebSocket.latest().close()
      vi.advanceTimersByTime(delay - 1)
      expect(FakeWebSocket.instances).toHaveLength(count)
      vi.advanceTimersByTime(1)
      expect(FakeWebSocket.instances).toHaveLength(count + 1)
    }
    expect(errors).toEqual(Array(6).fill("disconnected"))
    monitor.stop()
  })

  it("resets the backoff after a successful connection", () => {
    const { monitor } = setup()
    monitor.start()
    for (let i = 0; i < 5; i++) {
      FakeWebSocket.latest().open()
      FakeWebSocket.latest().close()
      vi.advanceTimersByTime(5000)
    }
    expect(FakeWebSocket.instances).toHaveLength(6)
    monitor.stop()
  })

  it("reconnects at once when the browser is back online or the page is shown", () => {
    const { monitor } = setup()
    monitor.start()
    FakeWebSocket.latest().close()
    window.dispatchEvent(new Event("online"))
    expect(FakeWebSocket.instances).toHaveLength(2)

    FakeWebSocket.latest().close()
    document.dispatchEvent(new Event("visibilitychange"))
    expect(FakeWebSocket.instances).toHaveLength(3)

    // The skipped wait does not connect a second time, and a live connection is left alone.
    vi.advanceTimersByTime(60_000)
    window.dispatchEvent(new Event("online"))
    expect(FakeWebSocket.instances).toHaveLength(3)
    monitor.stop()
  })

  it("keeps waiting while the page is hidden", () => {
    const { monitor } = setup()
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden")
    monitor.start()
    FakeWebSocket.latest().close()
    document.dispatchEvent(new Event("visibilitychange"))
    expect(FakeWebSocket.instances).toHaveLength(1)
    vi.advanceTimersByTime(5000)
    expect(FakeWebSocket.instances).toHaveLength(2)
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

  it("stops without reconnecting or reporting errors", async () => {
    const { monitor, errors } = setup()
    monitor.start()
    const socket = FakeWebSocket.latest()
    socket.open()
    await socket.receiveNodes([])
    monitor.stop()

    expect(socket.closed).toBe(true)
    vi.advanceTimersByTime(60_000)
    window.dispatchEvent(new Event("online"))
    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(socket.sent).toEqual([PING])
    expect(errors).toEqual([])
  })

  it("drops an answer still decoding when the connection closes", async () => {
    const { monitor, received, errors } = setup()
    monitor.start()
    const first = FakeWebSocket.latest()
    first.open()
    const answer = first.receive(await encodePayload([makeNode()]))
    first.close()
    await answer
    expect(received).toEqual([])
    expect(errors).toEqual(["disconnected"])

    // No poll is left behind to ping the next connection early.
    vi.advanceTimersByTime(5000)
    const second = FakeWebSocket.latest()
    second.open()
    vi.advanceTimersByTime(1000)
    expect(second.sent).toEqual([PING])
    monitor.stop()
  })

  it("drops an answer still decoding when stopped", async () => {
    const { monitor, received, errors } = setup()
    vi.spyOn(console, "error").mockImplementation(() => {})
    monitor.start()
    const socket = FakeWebSocket.latest()
    socket.open()
    const good = await encodePayload([makeNode()])
    const bad = await new Response("not gzip").arrayBuffer()
    const answers = [socket.receive(good), socket.receive(bad)]
    monitor.stop()
    await Promise.all(answers)
    expect(received).toEqual([])
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
