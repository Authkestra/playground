import { cn } from "@/lib/cn"

function Skeleton({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      // `bg-muted`, not the accent at low opacity: a loading screen can show
      // several of these at once (see `Playground`'s loading phase), and a
      // rust-tinted pulse repeated across the page is a warm surface, which
      // the ground is not supposed to be (DESIGN.md §2).
      className={cn("animate-pulse rounded-md bg-muted", className)}
      {...props}
    />
  )
}

export { Skeleton }
