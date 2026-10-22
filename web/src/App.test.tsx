import { act, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import App from "@/App"
import { ThemeProvider } from "@/components/theme-provider"
import { PING } from "@/lib/monitor-socket"
import { THEME_STORAGE_KEY } from "@/lib/theme"
import type { NodeData } from "@/lib/types"
import { FakeWebSocket, makeNode, nowSeconds } from "@/test/fixtures"

const tokyo = () => makeNode({ name: "tokyo", geoip: "JP" })
const frankfurt = () =>
  makeNode({
    name: "frankfurt",
    geoip: "DE",
    timestamp: nowSeconds() - 3600,
    host: { os: "Debian GNU/Linux", v12n: "none" },
  })
const mystery = () => makeNode({ name: "mystery", geoip: "" })

function renderApp() {
  const user = userEvent.setup()
  render(
    <ThemeProvider>
      <App />
    </ThemeProvider>,
  )
  return { user }
}

/** Sends `nodes` as the server would, opening the connection first if needed. */
async function serve(nodes: NodeData[]) {
  const socket = FakeWebSocket.latest()
  await act(async () => {
    if (socket.readyState !== FakeWebSocket.OPEN) socket.open()
    await socket.receiveNodes(nodes)
  })
  return socket
}

const nodeRow = (name: string) => screen.getByRole("button", { name })
/** The list item of a node, holding its toggle button and metrics. */
const nodeCard = (name: string) => nodeRow(name).closest("li")!
const visibleNodes = () =>
  ["tokyo", "frankfurt", "mystery"].filter((name) => screen.queryByRole("button", { name }))

beforeEach(() => {
  FakeWebSocket.instances = []
  vi.stubGlobal("WebSocket", FakeWebSocket)
})

describe("App", () => {
  it("connects and lists the nodes the server sends", async () => {
    renderApp()
    expect(screen.getByRole("heading", { level: 1, name: "AIO Monitor" })).toBeInTheDocument()
    expect(screen.getByText("Waiting for nodes...")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "AIO Monitor" })).toHaveAttribute(
      "href",
      "https://github.com/billzhong/aiom",
    )

    const socket = FakeWebSocket.latest()
    expect(socket.url).toBe("ws://monitor.test/ws")
    act(() => socket.open())
    expect(socket.sent).toEqual([PING])

    await serve([tokyo(), frankfurt()])
    expect(visibleNodes()).toEqual(["tokyo", "frankfurt"])
    expect(screen.getByRole("button", { name: "Total: 2" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Online: 1" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Offline: 1" })).toBeInTheDocument()
  })

  it("shows the status and live metrics of each node", async () => {
    renderApp()
    await serve([tokyo(), frankfurt()])

    const online = within(nodeCard("tokyo"))
    // Inside the button, the metrics would be hidden from screen readers.
    expect(within(nodeRow("tokyo")).queryByRole("progressbar")).not.toBeInTheDocument()
    expect(online.getByText("Online")).toBeInTheDocument()
    expect(online.getByText("1d 1h 1m 1s")).toBeInTheDocument()
    // CPU and RAM are both at 25%, the disk at 50%.
    expect(online.getAllByText("25.00%")).toHaveLength(2)
    expect(online.getByText("50.00%")).toBeInTheDocument()
    expect(online.getByRole("progressbar", { name: "Disk" })).toHaveAttribute("aria-valuenow", "50")
    expect(online.getByText("2.00 Mbps")).toBeInTheDocument()
    expect(online.getByText("0.50 | 0.25 | 0.13")).toBeInTheDocument()

    const offline = within(nodeCard("frankfurt"))
    expect(offline.getByText("Offline")).toBeInTheDocument()
    expect(offline.queryByText("1d 1h 1m 1s")).not.toBeInTheDocument()
    // Reported an hour ago; rendering may take place a second later.
    expect(offline.getByText(/^0d 1h 0m [01]s$/)).toBeInTheDocument()

    // Fleet-wide totals over the online node.
    expect(screen.getByTitle("CPU usage")).toHaveTextContent("25.00%")
    expect(screen.getByTitle("Total memory")).toHaveTextContent("8.00 GB")
  })

  it("filters by status when a status card is clicked", async () => {
    const { user } = renderApp()
    await serve([tokyo(), frankfurt()])

    await user.click(screen.getByRole("button", { name: "Online: 1" }))
    expect(visibleNodes()).toEqual(["tokyo"])
    expect(screen.getByRole("button", { name: "Online: 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )

    await user.click(screen.getByRole("button", { name: "Offline: 1" }))
    expect(visibleNodes()).toEqual(["frankfurt"])

    await user.click(screen.getByRole("button", { name: "Total: 2" }))
    expect(visibleNodes()).toEqual(["tokyo", "frankfurt"])
  })

  it("filters by location and name", async () => {
    const { user } = renderApp()
    await serve([tokyo(), frankfurt(), mystery()])

    const tabs = screen.getAllByRole("tab").map((tab) => tab.textContent)
    expect(tabs).toEqual(["All 3", "DE 1", "JP 1", "Unknown 1"])
    expect(screen.getByRole("tab", { name: "DE 1" })).toHaveAttribute("title", "Germany")

    await user.click(screen.getByRole("tab", { name: "DE 1" }))
    expect(visibleNodes()).toEqual(["frankfurt"])
    expect(screen.getByRole("button", { name: "Total: 1" })).toBeInTheDocument()

    await user.click(screen.getByRole("tab", { name: "Unknown 1" }))
    expect(visibleNodes()).toEqual(["mystery"])

    await user.click(screen.getByRole("tab", { name: "All 3" }))
    await user.type(screen.getByRole("searchbox", { name: "Filter" }), "TOK")
    expect(visibleNodes()).toEqual(["tokyo"])

    await user.type(screen.getByRole("searchbox", { name: "Filter" }), "xyz")
    expect(visibleNodes()).toEqual([])
    expect(screen.getByText("No nodes match the filters.")).toBeInTheDocument()
  })

  it("falls back to all locations once the selected one has no nodes left", async () => {
    const { user } = renderApp()
    await serve([tokyo(), frankfurt()])
    await user.click(screen.getByRole("tab", { name: "DE 1" }))
    expect(visibleNodes()).toEqual(["frankfurt"])

    await serve([tokyo()])
    expect(screen.getByRole("tab", { name: "All 1" })).toHaveAttribute("aria-selected", "true")
    expect(visibleNodes()).toEqual(["tokyo"])
  })

  it("expands one node at a time to show its details", async () => {
    const { user } = renderApp()
    await serve([tokyo(), frankfurt()])
    expect(screen.queryByText("AMD EPYC 7B13")).not.toBeInTheDocument()

    await user.click(nodeRow("tokyo"))
    expect(nodeRow("tokyo")).toHaveAttribute("aria-expanded", "true")
    expect(screen.getByText("AMD EPYC 7B13")).toBeInTheDocument()
    expect(screen.getByText("2 Cores")).toBeInTheDocument()
    expect(screen.getByText("Japan")).toBeInTheDocument()
    expect(screen.getByText("x86_64 / kvm")).toBeInTheDocument()
    expect(screen.getByText("v0.1.0")).toBeInTheDocument()
    expect(screen.getByText("4.00 GB")).toBeInTheDocument()

    await user.click(nodeRow("frankfurt"))
    expect(nodeRow("tokyo")).toHaveAttribute("aria-expanded", "false")
    expect(screen.getByText("Germany")).toBeInTheDocument()
    // "none" means not virtualized.
    expect(screen.getByText("x86_64")).toBeInTheDocument()

    await user.click(nodeRow("frankfurt"))
    expect(screen.queryByText("Germany")).not.toBeInTheDocument()
  })

  it("updates as new node lists arrive", async () => {
    renderApp()
    await serve([tokyo()])
    await serve([tokyo(), mystery()])
    expect(visibleNodes()).toEqual(["tokyo", "mystery"])
    await serve([])
    expect(visibleNodes()).toEqual([])
  })

  it("warns when the connection drops", async () => {
    renderApp()
    const socket = await serve([tokyo()])
    act(() => socket.close())
    expect(
      await screen.findByText("WebSocket is disconnected, trying to reconnect..."),
    ).toBeInTheDocument()
  })

  it("offers a refresh for bad data only, not once the connection drops after it", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {})
    renderApp()
    const socket = await serve([tokyo()])
    await act(() => socket.receive("not a node list"))
    expect(await screen.findByRole("button", { name: "Refresh" })).toBeInTheDocument()

    act(() => socket.close())
    expect(
      await screen.findByText("WebSocket is disconnected, trying to reconnect..."),
    ).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Refresh" })).not.toBeInTheDocument()
  })

  it("takes nodes offline once their reports are stale, even while disconnected", async () => {
    vi.useFakeTimers()
    try {
      renderApp()
      const socket = await serve([tokyo()])
      act(() => socket.close())

      // Timestamps are whole seconds, so the report is up to a second older than it looks.
      act(() => vi.advanceTimersByTime(29_000))
      expect(screen.getByRole("button", { name: "Online: 1" })).toBeInTheDocument()

      act(() => vi.advanceTimersByTime(2000))
      expect(screen.getByRole("button", { name: "Online: 0" })).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Offline: 1" })).toBeInTheDocument()
      expect(within(nodeCard("tokyo")).getByText("Offline")).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it("switches the language", async () => {
    const { user } = renderApp()
    await serve([tokyo()])

    await user.click(screen.getByRole("button", { name: "Change language" }))
    await user.click(await screen.findByRole("menuitem", { name: "Chinese" }))
    expect(screen.getByRole("tab", { name: "全部 1" })).toBeInTheDocument()
    // The name is not translated.
    expect(screen.getByRole("heading", { level: 1, name: "AIO Monitor" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "在线: 1" })).toBeInTheDocument()
    expect(document.documentElement.lang).toBe("zh")
  })

  it("switches and remembers the theme", async () => {
    const { user } = renderApp()

    await user.click(screen.getByRole("button", { name: "Toggle theme" }))
    await user.click(await screen.findByRole("menuitem", { name: "Dark" }))
    expect(document.documentElement).toHaveClass("dark")
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark")

    await user.click(screen.getByRole("button", { name: "Toggle theme" }))
    await user.click(await screen.findByRole("menuitem", { name: "Light" }))
    expect(document.documentElement).toHaveClass("light")
    expect(document.documentElement).not.toHaveClass("dark")
  })

  it("closes the connection when unmounted", () => {
    const { unmount } = render(
      <ThemeProvider>
        <App />
      </ThemeProvider>,
    )
    const socket = FakeWebSocket.latest()
    act(() => socket.open())
    unmount()
    expect(socket.closed).toBe(true)
  })
})
