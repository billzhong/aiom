import { describe, expect, it } from "vitest"

import { osIcon } from "@/lib/os-icon"

const fontIcon = (os: string) => {
  const icon = osIcon(os)
  return icon.kind === "font" ? icon.className : icon.kind
}

describe("osIcon", () => {
  it.each([
    ["Ubuntu", "fl-ubuntu"],
    ["Debian GNU/Linux", "fl-debian"],
    ["CentOS Stream", "fl-centos"],
    ["Rocky Linux", "fl-rocky-linux"],
    ["AlmaLinux", "fl-almalinux"],
    ["Alpine Linux", "fl-alpine"],
    ["Arch Linux", "fl-archlinux"],
    ["Fedora Linux", "fl-fedora"],
    ["openSUSE Leap", "fl-leap"],
    ["Amazon Linux", "fl-redhat"],
    ["Linux Mint", "fl-linuxmint"],
    ["Darwin", "fl-apple"],
    ["FreeBSD", "fl-freebsd"],
  ])("maps %s to %s", (os, className) => {
    expect(fontIcon(os)).toBe(className)
  })

  it("falls back to Tux", () => {
    expect(fontIcon("OpenWrt")).toBe("fl-tux")
    expect(fontIcon("Android")).toBe("fl-tux")
    expect(fontIcon("")).toBe("fl-tux")
  })

  it("draws Windows separately", () => {
    expect(osIcon("Windows")).toEqual({ kind: "windows" })
  })
})
