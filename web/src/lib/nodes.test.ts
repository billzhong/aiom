import { describe, expect, it } from "vitest"

import {
  ALL_LOCATIONS,
  appendChartPoints,
  filterByStatus,
  filterNodes,
  locationsOf,
  summarize,
} from "@/lib/nodes"
import { makeNode, type NodeOverrides } from "@/test/fixtures"

const NOW = 1_700_000_000_000
const online = (name: string, overrides: NodeOverrides = {}) =>
  makeNode({ name, timestamp: NOW / 1000, ...overrides })
const offline = (name: string, overrides: NodeOverrides = {}) =>
  makeNode({ name, timestamp: NOW / 1000 - 3600, ...overrides })

describe("locations", () => {
  it("counts the nodes of each location, with unknown last", () => {
    const nodes = [
      makeNode({ geoip: "US" }),
      makeNode({ geoip: "" }),
      makeNode({ geoip: "DE" }),
      makeNode({ geoip: "US" }),
    ]
    expect([...locationsOf(nodes)]).toEqual([
      ["DE", 1],
      ["US", 2],
      ["", 1],
    ])
    expect(locationsOf([]).size).toBe(0)
  })
})

describe("filterNodes", () => {
  const nodes = [
    makeNode({ name: "Tokyo-1", geoip: "JP" }),
    makeNode({ name: "tokyo-2", geoip: "JP" }),
    makeNode({ name: "Frankfurt", geoip: "DE" }),
    makeNode({ name: "mystery", geoip: "" }),
  ]
  const names = (location: string, query: string) =>
    filterNodes(nodes, { location, query }).map((n) => n.name)

  it("filters by location", () => {
    expect(names(ALL_LOCATIONS, "")).toHaveLength(4)
    expect(names("JP", "")).toEqual(["Tokyo-1", "tokyo-2"])
    expect(names("", "")).toEqual(["mystery"])
  })

  it("filters by name, ignoring case and surrounding spaces", () => {
    expect(names(ALL_LOCATIONS, " TOKYO ")).toEqual(["Tokyo-1", "tokyo-2"])
    expect(names("DE", "tokyo")).toEqual([])
  })
})

describe("filterByStatus", () => {
  const nodes = [online("a"), offline("b"), online("c")]

  it("splits nodes by online status", () => {
    expect(filterByStatus(nodes, "all", NOW).map((n) => n.name)).toEqual(["a", "b", "c"])
    expect(filterByStatus(nodes, "online", NOW).map((n) => n.name)).toEqual(["a", "c"])
    expect(filterByStatus(nodes, "offline", NOW).map((n) => n.name)).toEqual(["b"])
  })
})

describe("summarize", () => {
  it("counts and totals nodes", () => {
    const summary = summarize(
      [
        online("a", {
          host: { cpu_count: 2, total_memory: 1000 },
          stats: {
            cpu_usage: 10,
            used_memory: 250,
            network_in_speed: 5,
            network_out_speed: 7,
            network_in: 100,
            network_out: 200,
          },
        }),
        online("b", {
          host: { cpu_count: 4, total_memory: 3000 },
          stats: {
            cpu_usage: 30,
            used_memory: 750,
            network_in_speed: 0,
            network_out_speed: 0,
            network_in: 1,
            network_out: 2,
          },
        }),
      ],
      NOW,
    )
    expect(summary).toEqual({
      total: 2,
      online: 2,
      offline: 0,
      cpuCores: 6,
      cpuUsage: 20,
      totalMemory: 4000,
      memoryUsage: 25,
      downloadSpeed: 5,
      uploadSpeed: 7,
      downloaded: 101,
      uploaded: 202,
    })
  })

  it("leaves stale values of offline nodes out of rates and usage", () => {
    const summary = summarize(
      [
        online("up", {
          host: { cpu_count: 1, total_memory: 1000 },
          stats: {
            cpu_usage: 40,
            used_memory: 500,
            network_in_speed: 10,
            network_out_speed: 0,
            network_in: 0,
            network_out: 0,
          },
        }),
        offline("down", {
          host: { cpu_count: 1, total_memory: 1000 },
          stats: {
            cpu_usage: 100,
            used_memory: 1000,
            network_in_speed: 999,
            network_out_speed: 999,
            network_in: 50,
            network_out: 0,
          },
        }),
      ],
      NOW,
    )
    expect(summary.online).toBe(1)
    expect(summary.offline).toBe(1)
    expect(summary.cpuUsage).toBe(40)
    expect(summary.memoryUsage).toBe(50)
    expect(summary.downloadSpeed).toBe(10)
    // Capacity and transfer totals still include offline nodes.
    expect(summary.cpuCores).toBe(2)
    expect(summary.totalMemory).toBe(2000)
    expect(summary.downloaded).toBe(50)
  })

  it("handles an empty list", () => {
    const summary = summarize([], NOW)
    expect(summary.total).toBe(0)
    expect(summary.cpuUsage).toBe(0)
    expect(summary.memoryUsage).toBe(0)
  })
})

describe("appendChartPoints", () => {
  it.each(["constructor", "__proto__", "toString"])(
    "handles history updates for a node named %s",
    (name) => {
      const node = makeNode({ name, timestamp: 1 })
      let charts = appendChartPoints({}, [node])
      expect(Object.hasOwn(charts, name)).toBe(true)
      expect(charts[name].map((point) => point.timestamp)).toEqual([1])

      const history = charts[name]
      charts = appendChartPoints(charts, [node])
      expect(charts[name]).toBe(history)

      charts = appendChartPoints(charts, [{ ...node, timestamp: 2 }])
      expect(charts[name].map((point) => point.timestamp)).toEqual([1, 2])

      charts = appendChartPoints(charts, [])
      expect(Object.hasOwn(charts, name)).toBe(false)
      expect(charts[name]).toBeUndefined()

      charts = appendChartPoints(charts, [{ ...node, timestamp: 3 }])
      expect(charts[name].map((point) => point.timestamp)).toEqual([3])
    },
  )

  it("records a point per node", () => {
    const node = makeNode({
      host: { total_memory: 200, total_swap: 0 },
      stats: {
        cpu_usage: 12,
        used_memory: 50,
        used_swap: 5,
        network_in_speed: 1,
        network_out_speed: 2,
        tcp_count: 3,
        udp_count: 4,
      },
    })
    const charts = appendChartPoints({}, [node])
    expect(charts[node.name]).toEqual([
      expect.objectContaining({
        timestamp: node.timestamp,
        cpu: 12,
        ram: 25,
        swap: 0,
        download: 1,
        upload: 2,
        tcp: 3,
        udp: 4,
      }),
    ])
  })

  it("skips repeated samples and keeps the last points", () => {
    let charts = {}
    for (let t = 1; t <= 5; t++) {
      charts = appendChartPoints(charts, [makeNode({ timestamp: t })], 3)
      charts = appendChartPoints(charts, [makeNode({ timestamp: t })], 3)
    }
    const timestamps = (charts as ReturnType<typeof appendChartPoints>)["node-1"]?.map(
      (p) => p.timestamp,
    )
    expect(timestamps).toEqual([3, 4, 5])
  })

  it("keeps the same history object when nothing changed", () => {
    const node = makeNode()
    const charts = appendChartPoints({}, [node])
    expect(appendChartPoints(charts, [node])[node.name]).toBe(charts[node.name])
  })

  it("drops nodes that disappeared", () => {
    const charts = appendChartPoints({}, [makeNode({ name: "a" }), makeNode({ name: "b" })])
    expect(Object.keys(appendChartPoints(charts, [makeNode({ name: "b" })]))).toEqual(["b"])
  })
})
