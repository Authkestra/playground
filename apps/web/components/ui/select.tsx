"use client"

import * as React from "react"
import { ChevronDown } from "lucide-react"

import { cn } from "@/lib/cn"

/**
 * A styled native `<select>` — deliberately not a Radix listbox.
 *
 * Every other control in this UI (`Switch`, `RadioGroup`, `Checkbox`) reaches
 * for Radix because the native HTML element cannot be restyled into what was
 * asked for: a native checkbox cannot look like a filled square, a native
 * radio cannot lose its OS-drawn dot. `<select>` has no such gap — it already
 * opens a listbox, groups options under a heading via `<optgroup>`, and is
 * keyboard-operable and screen-reader friendly without a line of our
 * JavaScript. Reaching for `@radix-ui/react-select` here bought the same
 * behaviour for a real cost: its dependency tree pulls in the
 * popper/portal/dismissable-layer/focus-scope/scroll-lock stack, the same
 * machinery Tooltip was carrying when it was removed from this codebase for
 * costing ~14 kB gzipped it wasn't worth (see the "why 136 kB" note in
 * `scripts/bundle-budget.mjs`). Measured, the Select primitive alone cost
 * over 20 kB more — for a dropdown a plain element already renders. This
 * component exists so that cost is paid again only if something a native
 * `<select>` truly cannot do shows up, not by default.
 */
const Select = React.forwardRef<
  HTMLSelectElement,
  React.SelectHTMLAttributes<HTMLSelectElement>
>(({ className, children, ...props }, ref) => (
  <div className="relative inline-block">
    <select
      ref={ref}
      className={cn(
        // `appearance-none` drops the OS-drawn arrow so the chevron below is
        // the only one, and `pr-8` keeps text from running under it.
        // Focus is the global rule in app/globals.css, not a ring here.
        "h-9 w-full appearance-none rounded-md border border-input bg-transparent py-2 pl-3 pr-8 text-sm shadow-sm disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
      {...props}
    >
      {children}
    </select>
    <ChevronDown
      aria-hidden
      strokeWidth={1.5}
      className="pointer-events-none absolute right-2 top-1/2 h-4 w-4 -translate-y-1/2 opacity-50"
    />
  </div>
))
Select.displayName = "Select"

export { Select }
