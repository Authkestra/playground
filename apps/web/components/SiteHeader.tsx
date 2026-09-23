import Link from "next/link";
import { Button } from "@/components/ui/button";

/** The framework's own site, which links back here. */
const AUTHKESTRA_SITE = "https://authkestra.com";

/**
 * Site-wide chrome, rendered once from `app/layout.tsx` so every route gets
 * it — a server component, since nothing here needs interactivity. Carries
 * the mark, the product name, navigation between the playground and its
 * explainer, and a link out to the framework's own docs. What used to sit
 * here too — the one-line description of what the playground does — is
 * page content, not chrome, and now lives on the pages that actually need it.
 */
export default function SiteHeader() {
  return (
    <header className="border-b border-border">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-3 px-6 py-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          {/*
            A `span`, not an `h1`. This is chrome on every route, and the page
            below it owns its own heading — two `h1`s on the explainer page
            would be a worse outline than one wordmark that is simply not a
            heading. Navigation to the playground is the nav's job, just below.
          */}
          <span className="flex items-center gap-2.5 text-2xl font-semibold tracking-tight text-foreground">
            {/*
              The mark, per the design system's §8. `aria-hidden` because the
              heading already says the name — announcing it twice is worse
              than not labelling it at all. The crossbar carries the rust; the
              strokes take the heading's own ink via `currentColor`.
            */}
            <svg
              viewBox="0 0 32 32"
              fill="none"
              aria-hidden="true"
              className="h-[1.15em] w-[1.15em] shrink-0"
            >
              <path
                d="M5.5 27.5 16 5l10.5 22.5"
                stroke="currentColor"
                strokeWidth="2.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <rect
                x="10"
                y="17.6"
                width="12"
                height="2.8"
                rx="1.4"
                fill="hsl(var(--brand))"
              />
            </svg>
            Authkestra Playground
          </span>
          {/*
            A visitor who likes what they see should not have to go hunting for the
            framework — the playground exists to send people there.
          */}
          <Button asChild variant="outline" size="sm" className="w-fit min-w-28 shrink-0">
            <a href={AUTHKESTRA_SITE} target="_blank" rel="noreferrer">
              authkestra docs
              <span aria-hidden="true">→</span>
              <span className="sr-only">(opens in a new tab)</span>
            </a>
          </Button>
        </div>
        <nav aria-label="Site" className="flex items-center gap-4 text-sm font-medium text-muted-foreground">
          <Link href="/" className="transition-colors hover:text-foreground">
            Playground
          </Link>
          <Link href="/how-it-works" className="transition-colors hover:text-foreground">
            How it works
          </Link>
        </nav>
      </div>
    </header>
  );
}
