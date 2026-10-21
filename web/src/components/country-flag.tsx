import * as Flags from "country-flag-icons/react/3x2"
import { SquareX } from "lucide-react"

const FLAGS: Record<string, (typeof Flags)["US"] | undefined> = Flags

/** Flag of an ISO 3166-1 alpha-2 country code, or a placeholder when unknown. */
export function CountryFlag({ code, className }: { code: string; className?: string }) {
  const Flag = FLAGS[code]
  return Flag ? (
    <Flag className={className} aria-hidden />
  ) : (
    <SquareX className={className} aria-hidden />
  )
}
