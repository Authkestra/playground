"use client";

import * as React from "react";

import { cn } from "@/lib/cn";
import { Button, type ButtonProps } from "@/components/ui/button";

/**
 * `Button`, with this project's one sizing convention baked in: a minimum
 * width matching the sign-in step's Continue button (`min-w-28`), free to
 * grow for a longer label (`w-fit`) rather than clipping or wrapping it.
 *
 * Positioning is deliberately left to the caller rather than folded in here
 * — a lone action sits in a `flex justify-end` row, a pair like Back/Continue
 * shares one `flex justify-end gap-2` row, and a button that is the second
 * child of an existing `justify-between` row (SessionBar's Reset,
 * OutcomeBanner's Dismiss) needs no wrapper at all. Baking in the wrapper
 * would have fought that last case instead of covering it.
 */
export const ActionButton = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, ...props }, ref) => (
    <Button ref={ref} className={cn("w-fit min-w-28", className)} {...props} />
  ),
);
ActionButton.displayName = "ActionButton";
