import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Compose class names, letting a later class win over an earlier one that sets
 * the same property. Plain template-literal concatenation — what this UI used
 * before — cannot do that: `"px-4" + " px-2"` leaves both in the string and the
 * winner is whichever Tailwind emitted last, not whichever the caller passed
 * last. That makes a component's `className` prop unable to override its own
 * defaults, which is the whole contract shadcn components are written against.
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
