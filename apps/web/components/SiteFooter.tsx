/**
 * Site-wide chrome, rendered once from `app/layout.tsx` so every route gets
 * it — a server component, quiet and small on purpose: it is a footer, not a
 * second header. Three links, each labelled so a visitor cannot confuse the
 * framework's repository with this playground's own, which is the same
 * distinction the README opens with.
 */
export default function SiteFooter() {
  return (
    <footer className="border-t border-border">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-2 px-6 py-8 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
        <a
          href="https://authkestra.com"
          target="_blank"
          rel="noreferrer"
          className="transition-colors hover:text-foreground"
        >
          authkestra docs
        </a>
        <nav aria-label="Repositories" className="flex flex-wrap items-center gap-x-4 gap-y-1">
          <a
            href="https://github.com/marcjazz/authkestra"
            target="_blank"
            rel="noreferrer"
            className="transition-colors hover:text-foreground"
          >
            authkestra on GitHub (the framework)
          </a>
          <a
            href="https://github.com/Authkestra/playground"
            target="_blank"
            rel="noreferrer"
            className="transition-colors hover:text-foreground"
          >
            this playground on GitHub
          </a>
        </nav>
      </div>
    </footer>
  );
}
