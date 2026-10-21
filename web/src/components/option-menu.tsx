import { cn } from "cn"
import type { ReactNode } from "react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"

interface Option<T extends string> {
  value: T
  label: string
  icon: ReactNode
}

/** An icon button opening a menu of options, the selected one highlighted. */
export function OptionMenu<T extends string>({
  label,
  trigger,
  options,
  selected,
  onSelect,
}: {
  /** Accessible name of the button. */
  label: string
  /** Content of the button. */
  trigger: ReactNode
  options: readonly Option<T>[]
  selected: string | undefined
  onSelect: (value: T) => void
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="icon" aria-label={label}>
          {trigger}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {options.map((option) => (
          <DropdownMenuItem
            key={option.value}
            className={cn("cursor-pointer", selected === option.value && "bg-accent")}
            onClick={() => onSelect(option.value)}
          >
            {option.icon} {option.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
