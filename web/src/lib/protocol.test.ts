import { describe, expect, it } from "vitest"

import { decodeNodes } from "@/lib/monitor-socket"
import type { Host, NodeData, Stats } from "@/lib/types"
import { encodePayload } from "@/test/fixtures"

import nodeListJson from "../../../crates/protocol/fixtures/node-list.json?raw"

/** The JSON type of every field of `T`; the compiler checks the keys and types against `T`. */
type Fields<T> = {
  [K in keyof Required<T>]: Required<T>[K] extends string
    ? "string"
    : Required<T>[K] extends number
      ? "number"
      : "object"
}

const NODE_FIELDS: Fields<NodeData> = {
  timestamp: "number",
  name: "string",
  host: "object",
  stats: "object",
  geoip: "string",
}

const HOST_FIELDS: Fields<Host> = {
  agent: "string",
  os: "string",
  os_version: "string",
  cpu: "string",
  cpu_count: "number",
  cpu_arch: "string",
  total_memory: "number",
  total_swap: "number",
  total_disk: "number",
  v12n: "string",
}

const STATS_FIELDS: Fields<Stats> = {
  cpu_usage: "number",
  used_memory: "number",
  used_swap: "number",
  used_disk: "number",
  network_in: "number",
  network_out: "number",
  network_in_speed: "number",
  network_out_speed: "number",
  uptime: "number",
  boot_time: "number",
  load1: "number",
  load5: "number",
  load15: "number",
  tcp_count: "number",
  udp_count: "number",
  process_count: "number",
}

/** A node list as the server sends it, checked against the Rust types by the protocol crate. */
const fixture = JSON.parse(nodeListJson) as Record<string, unknown>[]

function typesOf(value: unknown): Record<string, string> {
  return Object.fromEntries(
    Object.entries(value as object).map(([key, field]) => [key, typeof field]),
  )
}

describe("protocol", () => {
  it("knows every field of the node list, with its type", () => {
    expect(fixture).not.toHaveLength(0)
    for (const node of fixture) {
      expect(typesOf(node)).toEqual(NODE_FIELDS)
      expect(typesOf(node.host)).toEqual(HOST_FIELDS)
      expect(typesOf(node.stats)).toEqual(STATS_FIELDS)
    }
  })

  it("decodes the node list as the server encodes it", async () => {
    const nodes = await decodeNodes(await encodePayload(fixture))
    expect(nodes.map((node) => node.name)).toEqual(["tokyo-1", "unknown-location"])
    expect(nodes[0].host.cpu_count).toBe(2)
    expect(nodes[1].geoip).toBe("")
  })
})
