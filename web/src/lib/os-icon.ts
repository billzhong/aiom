/** How to draw the logo of an operating system. */
type OsIcon = { kind: "windows" } | { kind: "font"; className: string }

/** Font Logos icons of operating systems, see https://github.com/lukas-w/font-logos. */
const FONT_LOGOS = new Set([
  "almalinux",
  "alpine",
  "aosc",
  "apple",
  "archlinux",
  "artix",
  "centos",
  "coreos",
  "debian",
  "deepin",
  "devuan",
  "elementary",
  "endeavour",
  "fedora",
  "freebsd",
  "garuda",
  "gentoo",
  "kali-linux",
  "leap",
  "linuxmint",
  "mageia",
  "manjaro",
  "mxlinux",
  "nixos",
  "openbsd",
  "opensuse",
  "parrot",
  "pop-os",
  "raspberry-pi",
  "redhat",
  "rocky-linux",
  "slackware",
  "solus",
  "tumbleweed",
  "ubuntu",
  "void",
  "zorin",
])

/** OS names (lower case) whose logo has a different name. */
const ALIASES: Record<string, string> = {
  darwin: "apple",
  macos: "apple",
  arch: "archlinux",
  "arch linux": "archlinux",
  "rocky linux": "rocky-linux",
  "kali gnu/linux": "kali-linux",
  "linux mint": "linuxmint",
  "pop!_os": "pop-os",
  "opensuse leap": "leap",
  "opensuse tumbleweed": "tumbleweed",
  "red hat enterprise linux": "redhat",
  "amazon linux": "redhat",
  amazon: "redhat",
  raspbian: "raspberry-pi",
}

const normalize = (os: string) => os.trim().toLowerCase()

/** Whether an OS name as reported by the client is a Windows version. */
export function isWindows(os: string): boolean {
  return normalize(os).startsWith("windows")
}

/** Picks the logo for an OS name as reported by the client, falling back to Tux. */
export function osIcon(os: string): OsIcon {
  if (isWindows(os)) return { kind: "windows" }

  const name = normalize(os)

  const firstWord = name.split(/[\s/]/)[0] ?? ""
  const logo = [ALIASES[name], name, ALIASES[firstWord], firstWord].find(
    (candidate) => candidate !== undefined && FONT_LOGOS.has(candidate),
  )
  return { kind: "font", className: `fl-${logo ?? "tux"}` }
}
