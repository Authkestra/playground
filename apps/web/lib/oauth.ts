import type { OAuthMode } from "@playground/api-types";
import { API_BASE } from "@/lib/api";

/**
 * Outcome the OAuth callback hands back as query parameters on the
 * frontend's URL. See docs/api-contract.md#oauth-navigation-routes.
 */
export type OAuthReturn =
  | { status: "success"; provider: string; mode: OAuthMode | null }
  | { status: "denied"; provider: string; reason: string | null }
  | { status: "error"; provider: string; reason: string | null };

/**
 * Starts the OAuth round trip with a top-level navigation (never a fetch —
 * the browser has to actually leave for the provider and come back).
 */
export function loginUrl(provider: string, mode?: OAuthMode): string {
  const params = new URLSearchParams();
  if (mode) params.set("mode", mode);
  const query = params.toString();
  return `${API_BASE}/auth/login/${encodeURIComponent(provider)}${query ? `?${query}` : ""}`;
}

/**
 * Reads the outcome of a completed OAuth round trip from the current URL's
 * query string, if present. Returns null when this load isn't a return from
 * the provider at all (the common case).
 */
export function readOAuthReturn(search: string): OAuthReturn | null {
  const params = new URLSearchParams(search);
  const outcome = params.get("oauth");
  const provider = params.get("provider");
  if (!outcome || !provider) return null;

  if (outcome === "success") {
    const modeParam = params.get("mode");
    // Accept only the exact strings the API sends back.
    const mode: OAuthMode | null = modeParam === "session" || modeParam === "jwt" ? modeParam : null;
    return { status: "success", provider, mode };
  }
  if (outcome === "denied") return { status: "denied", provider, reason: params.get("reason") };
  if (outcome === "error") return { status: "error", provider, reason: params.get("reason") };
  return null;
}

/**
 * Strips the `oauth`/`provider`/`reason`/`mode` query params from the current URL
 * without a navigation, so reloading or sharing the link doesn't replay the
 * same result.
 */
export function clearOAuthReturnParams(): void {
  if (typeof window === "undefined") return;
  const url = new URL(window.location.href);
  url.searchParams.delete("oauth");
  url.searchParams.delete("provider");
  url.searchParams.delete("reason");
  url.searchParams.delete("mode");
  window.history.replaceState(null, "", url.pathname + url.search + url.hash);
}

/**
 * Outcome `GET /api/github/callback` hands back on the frontend's URL after
 * the push-to-GitHub connect round trip (#40). A separate shape from
 * `OAuthReturn` above rather than a reuse of it: this is a different OAuth
 * app, with its own query vocabulary (`github_push=...` rather than
 * `oauth=...`) and no `mode` at all.
 */
export type GithubPushReturn =
  | { status: "connected" }
  | { status: "denied"; reason: string | null }
  | { status: "error"; reason: string | null };

/**
 * Reads the outcome of a completed GitHub-push connect round trip from the
 * current URL's query string, if present. Returns null when this load isn't
 * a return from that flow at all (the common case).
 */
export function readGithubPushReturn(search: string): GithubPushReturn | null {
  const params = new URLSearchParams(search);
  const outcome = params.get("github_push");
  if (!outcome) return null;

  if (outcome === "connected") return { status: "connected" };
  if (outcome === "denied") return { status: "denied", reason: params.get("reason") };
  if (outcome === "error") return { status: "error", reason: params.get("reason") };
  return null;
}

/**
 * Strips the `github_push`/`reason` query params from the current URL
 * without a navigation, so reloading or sharing the link doesn't replay the
 * same result.
 */
export function clearGithubPushReturnParams(): void {
  if (typeof window === "undefined") return;
  const url = new URL(window.location.href);
  url.searchParams.delete("github_push");
  url.searchParams.delete("reason");
  window.history.replaceState(null, "", url.pathname + url.search + url.hash);
}
