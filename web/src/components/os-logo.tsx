import { cn } from "cn"

import { osIcon } from "@/lib/os-icon"

function WindowsLogo() {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 4875 4875"
      fill="currentColor"
      className="h-3"
      aria-hidden
    >
      <path d="M0 0h2311v2310H0zm2564 0h2311v2310H2564zM0 2564h2311v2311H0zm2564 0h2311v2311H2564" />
    </svg>
  )
}

/** Logo of the operating system named `os`. */
export function OsLogo({ os }: { os: string }) {
  const icon = osIcon(os)
  if (icon.kind === "windows") return <WindowsLogo />
  return (
    <i
      className={cn(icon.className, icon.className === "fl-apple" && "relative -top-0.5")}
      aria-hidden
    />
  )
}
