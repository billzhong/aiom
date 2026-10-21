import { describe, expect, it } from "vitest"

import {
  formatBitRate,
  formatBytes,
  formatDuration,
  formatTimestamp,
  isOnline,
  ONLINE_THRESHOLD_SECONDS,
  percentOf,
} from "@/lib/format"

describe("percentOf", () => {
  it("computes percentages", () => {
    expect(percentOf(1, 4)).toBe(25)
    expect(percentOf(3, 3)).toBe(100)
  })

  it("treats zero values as 0%", () => {
    expect(percentOf(0, 10)).toBe(0)
    expect(percentOf(5, 0)).toBe(0)
  })
})

describe("isOnline", () => {
  const now = 1_700_000_000_000

  it("accepts reports up to the threshold old", () => {
    expect(isOnline(now / 1000, now)).toBe(true)
    expect(isOnline(now / 1000 - ONLINE_THRESHOLD_SECONDS, now)).toBe(true)
    expect(isOnline(now / 1000 - ONLINE_THRESHOLD_SECONDS - 1, now)).toBe(false)
  })

  it("tolerates clocks that run slightly ahead", () => {
    expect(isOnline(now / 1000 + 2, now)).toBe(true)
  })
})

describe("formatTimestamp", () => {
  it("formats local date and time", () => {
    const timestamp = new Date(2025, 0, 2, 3, 4, 5).getTime() / 1000
    expect(formatTimestamp(timestamp)).toBe("2025-01-02 03:04:05")
  })

  it("shows a dash when unknown", () => {
    expect(formatTimestamp(0)).toBe("-")
  })
})

describe("formatDuration", () => {
  it("splits seconds into days, hours, minutes and seconds", () => {
    expect(formatDuration(90061)).toBe("1d 1h 1m 1s")
    expect(formatDuration(59)).toBe("0d 0h 0m 59s")
    expect(formatDuration(3600.9)).toBe("0d 1h 0m 0s")
  })

  it("clamps negative durations to zero", () => {
    expect(formatDuration(-5)).toBe("0d 0h 0m 0s")
  })
})

describe("formatBytes", () => {
  it("uses binary multiples", () => {
    expect(formatBytes(0)).toBe("0 B")
    expect(formatBytes(512)).toBe("512.00 B")
    expect(formatBytes(1536)).toBe("1.50 KB")
    expect(formatBytes(4 * 1024 ** 3)).toBe("4.00 GB")
  })

  it("treats invalid values as zero", () => {
    expect(formatBytes(-1)).toBe("0 B")
    expect(formatBytes(Number.NaN)).toBe("0 B")
  })
})

describe("formatBitRate", () => {
  it("converts bytes per second to bits per second with SI multiples", () => {
    expect(formatBitRate(0)).toBe("0 bps")
    expect(formatBitRate(100)).toBe("800.00 bps")
    expect(formatBitRate(125_000)).toBe("1.00 Mbps")
    expect(formatBitRate(1_250_000_000)).toBe("10.00 Gbps")
  })
})
