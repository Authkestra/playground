"use client";

import { useEffect, useState } from "react";
import { Download, GitBranch, Loader2, Star } from "lucide-react";
import type {
  DemoConfig,
  GitHubPushRequest,
  GitHubPushResponse,
  ScenarioSpec,
} from "@playground/api-types";
import {
  downloadStarterKit,
  githubConnectUrl,
  pushToGithub,
  type ApiError,
  type DeployTarget,
  type StarterKitOptions,
} from "@/lib/api";
import { type GithubPushReturn } from "@/lib/oauth";
import { isControlValueActive } from "@/components/ScenarioPanel";
import { OutcomeBanner } from "@/components/OutcomeBanner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  onDemoDisabled: () => void;
  onBack: () => void;
  /** The outcome of a just-completed GitHub push connect round trip (#40). */
  githubPushReturn: GithubPushReturn | null;
  onDismissGithubPushReturn: () => void;
}

const STAR_URL = "https://github.com/marcjazz/authkestra";

type State =
  | { kind: "idle" }
  | { kind: "working" }
  | { kind: "done"; filename: string }
  | { kind: "failed"; message: string };

/** One row of the "Deploy manifests" group: which flag it sets, and its pitch. */
const DEPLOY_TARGET_COPY: Array<{ id: DeployTarget; label: string; blurb: string }> = [
  { id: "docker", label: "Docker", blurb: "A Dockerfile that works anywhere." },
  { id: "render", label: "Render", blurb: "A blueprint plus a deploy button." },
  { id: "railway", label: "Railway", blurb: "A blueprint plus a deploy button." },
  { id: "fly", label: "Fly.io", blurb: "A fly.toml, ready for fly launch." },
];

type PushState =
  | { kind: "idle" }
  | { kind: "working" }
  | { kind: "done"; result: GitHubPushResponse }
  | { kind: "failed"; message: string };

/**
 * GitHub's own rule for a repository name, mirrored client-side so a visitor
 * finds out before a round trip rather than after: letters, digits, hyphens,
 * underscores and periods, 1-100 characters, and not literally `.` or `..`.
 * The server (`is_plausible_repo_name` in `github_routes.rs`) is still the
 * final authority — this only saves the common case a request.
 */
export function isValidRepoName(name: string): boolean {
  return (
    name.length > 0 &&
    name.length <= 100 &&
    name !== "." &&
    name !== ".." &&
    /^[A-Za-z0-9._-]+$/.test(name)
  );
}

/**
 * A starting point for the repository-name field, derived from what the
 * visitor actually turned on so two different configurations don't default
 * to the same name.
 */
export function defaultRepoName(includedScenarioIds: string[]): string {
  if (includedScenarioIds.length === 0) return "authkestra-starter";
  return `authkestra-starter-${includedScenarioIds.join("-")}`;
}

export default function StepDownload({
  scenarios,
  config,
  onDemoDisabled,
  onBack,
  githubPushReturn,
  onDismissGithubPushReturn,
}: Props) {
  const [state, setState] = useState<State>({ kind: "idle" });

  const included = scenarios.filter((s) => {
    const value = config?.scenarios?.[s.id];
    return value ? isControlValueActive(value) : false;
  });

  // Two independent checkboxes, not a single "extras" toggle: someone on htmx
  // wants the spec and no TypeScript, someone on Next.js may want the client
  // and no utoipa. Docker defaults on because it works everywhere; the other
  // three default off since they commit to a specific host.
  const [options, setOptions] = useState<StarterKitOptions>({
    openapi: false,
    tsClient: false,
    deploy: { docker: true, render: false, fly: false, railway: false },
  });

  function setDeploy(target: DeployTarget, checked: boolean) {
    setOptions((o) => ({ ...o, deploy: { ...o.deploy, [target]: checked } }));
  }

  // GitHub push (#40). `connected` mirrors whether the backend is currently
  // holding a usable token for this session — it starts false, flips true
  // once a `connect` round trip reports success, and flips back to false
  // after any push attempt, because the backend's token is single-use: it
  // clears it once the attempt is resolved, win or lose, except when the
  // request never got that far (an invalid name, or the feature not being
  // configured at all).
  const [githubConnected, setGithubConnected] = useState(false);
  const [repoName, setRepoName] = useState(() => defaultRepoName(included.map((s) => s.id)));
  const [push, setPush] = useState<PushState>({ kind: "idle" });

  useEffect(() => {
    if (githubPushReturn?.status === "connected") {
      setGithubConnected(true);
    }
  }, [githubPushReturn]);

  async function handleDownload() {
    setState({ kind: "working" });
    const result = await downloadStarterKit(options);

    if (!result.ok) {
      if (result.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setState({ kind: "failed", message: describeDownloadError(result.error) });
      return;
    }

    const { blob, filename } = result.data;
    // Hand the bytes to the browser's own save flow. The object URL is revoked
    // straight after: it pins the blob in memory until it is.
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);

    setState({ kind: "done", filename });
  }

  function handleConnectGithub() {
    // A navigation, not a fetch: the browser has to actually leave for
    // GitHub's consent screen and come back through /api/github/callback.
    window.location.href = githubConnectUrl();
  }

  async function handlePush() {
    const trimmed = repoName.trim();
    if (!isValidRepoName(trimmed)) return;

    setPush({ kind: "working" });
    const body: GitHubPushRequest = {
      repo_name: trimmed,
      description: null,
      openapi: options.openapi,
      ts_client: options.tsClient,
      deploy: options.deploy
        ? {
            docker: options.deploy.docker ?? null,
            render: options.deploy.render ?? null,
            fly: options.deploy.fly ?? null,
            railway: options.deploy.railway ?? null,
          }
        : null,
    };

    const result = await pushToGithub(body);

    if (!result.ok) {
      const error = result.error;
      if (error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      // See the `githubConnected` comment: only these two error kinds are
      // returned before the token is ever touched, so every other outcome —
      // including a plain server-side repeat of the same invalid name — ends
      // the connection and requires reconnecting.
      if (error.kind !== "github_invalid_repo_name" && error.kind !== "github_push_not_configured") {
        setGithubConnected(false);
      }
      setPush({ kind: "failed", message: describeGithubPushError(error) });
      return;
    }

    setGithubConnected(false);
    setPush({ kind: "done", result: result.data });
  }

  const working = state.kind === "working";
  const pushing = push.kind === "working";
  const repoNameTrimmed = repoName.trim();
  const repoNameInvalid = repoNameTrimmed.length > 0 && !isValidRepoName(repoNameTrimmed);

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-foreground">Download</h2>
        <p className="text-sm text-muted-foreground">
          Turn what you configured into a real, runnable project.
        </p>
      </div>

      <div className="grid gap-6 md:grid-cols-[3fr_2fr] md:items-start">
        <Card>
          <CardHeader>
            <CardTitle className="text-sm font-medium">What you&apos;ll get</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-4 pt-0">
            {included.length > 0 ? (
              <div className="flex flex-wrap gap-2">
                {included.map((s) => (
                  <Badge key={s.id} variant="secondary">
                    {s.name}
                  </Badge>
                ))}
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">
                You haven&apos;t turned anything on, so this is the smallest project
                that still runs: sessions and the framework&apos;s{" "}
                <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs">
                  /auth
                </code>{" "}
                routes, ready for you to add a method to.
              </p>
            )}

            <p className="text-sm text-muted-foreground">
              A Cargo project pinned to the same authkestra version this playground
              runs, with a README that names every value you need to fill in and
              where to get it. No sign-up, no gate.
            </p>

            <Separator />

            <fieldset className="flex flex-col gap-3">
              <legend className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Optional
              </legend>

              <div className="flex items-start gap-2.5">
                <Checkbox
                  id="option-openapi"
                  checked={options.openapi}
                  onCheckedChange={(checked) =>
                    setOptions((o) => ({ ...o, openapi: checked === true }))
                  }
                  className="mt-0.5"
                />
                <div className="grid gap-1 leading-none">
                  <Label htmlFor="option-openapi" className="cursor-pointer">
                    OpenAPI document
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    Annotates the handlers and serves the spec at{" "}
                    <code className="font-mono">/openapi.json</code>. Adds{" "}
                    <code className="font-mono">utoipa</code>.
                  </p>
                </div>
              </div>

              <div className="flex items-start gap-2.5">
                <Checkbox
                  id="option-ts-client"
                  checked={options.tsClient}
                  onCheckedChange={(checked) =>
                    setOptions((o) => ({ ...o, tsClient: checked === true }))
                  }
                  className="mt-0.5"
                />
                <div className="grid gap-1 leading-none">
                  <Label htmlFor="option-ts-client" className="cursor-pointer">
                    TypeScript client
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    A dependency-free client that handles the base64url conversion{" "}
                    <code className="font-mono">navigator.credentials</code> needs.
                    No Rust dependency.
                  </p>
                </div>
              </div>
            </fieldset>

            <Separator />

            <fieldset className="flex flex-col gap-3">
              <legend className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Deploy manifests
              </legend>

              {DEPLOY_TARGET_COPY.map(({ id, label, blurb }) => (
                <div key={id} className="flex items-start gap-2.5">
                  <Checkbox
                    id={`deploy-${id}`}
                    checked={options.deploy?.[id] ?? false}
                    onCheckedChange={(checked) => setDeploy(id, checked === true)}
                    className="mt-0.5"
                  />
                  <div className="grid gap-1 leading-none">
                    <Label htmlFor={`deploy-${id}`} className="cursor-pointer">
                      {label}
                    </Label>
                    <p className="text-xs text-muted-foreground">{blurb}</p>
                  </div>
                </div>
              ))}
            </fieldset>

            <Button
              type="button"
              onClick={() => void handleDownload()}
              disabled={working}
              className="gap-2"
            >
              {working ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
                  Preparing…
                </>
              ) : (
                <>
                  <Download className="h-4 w-4" aria-hidden />
                  Download the project
                </>
              )}
            </Button>

            <div aria-live="polite" className="min-h-[1.25rem]">
              {state.kind === "done" && (
                <p className="text-sm text-success-foreground">
                  Saved{" "}
                  <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs">
                    {state.filename}
                  </code>
                  . Unzip it, then follow the README.
                </p>
              )}
              {state.kind === "failed" && (
                <p className="text-sm text-warning-foreground">{state.message}</p>
              )}
            </div>
          </CardContent>
        </Card>

        <Card className="border-border/60 bg-card/50">
          <CardHeader>
            <CardTitle className="text-sm font-medium">Push to GitHub</CardTitle>
            <CardDescription>
              Optional — the zip needs no account. Use this if you&apos;d rather
              start from a repository.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4 pt-0">
            {githubPushReturn && (
              <GithubPushReturnBanner
                result={githubPushReturn}
                onDismiss={onDismissGithubPushReturn}
              />
            )}

            {!githubConnected ? (
              <>
                <p className="text-sm text-muted-foreground">
                  Creates one new public repository on your GitHub account for
                  this configuration.
                </p>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="gap-2"
                  onClick={handleConnectGithub}
                >
                  <GitBranch className="h-4 w-4" aria-hidden />
                  Connect GitHub
                </Button>
              </>
            ) : (
              <>
                <div className="grid gap-1.5">
                  <Label htmlFor="github-repo-name">Repository name</Label>
                  <Input
                    id="github-repo-name"
                    value={repoName}
                    onChange={(e) => setRepoName(e.target.value)}
                    aria-describedby="github-repo-name-hint"
                    aria-invalid={repoNameInvalid ? true : undefined}
                  />
                  <p id="github-repo-name-hint" className="text-xs text-muted-foreground">
                    Letters, digits, periods, hyphens and underscores only.
                  </p>
                </div>
                <Button
                  type="button"
                  size="sm"
                  className="gap-2"
                  disabled={pushing || !isValidRepoName(repoNameTrimmed)}
                  onClick={() => void handlePush()}
                >
                  {pushing ? (
                    <>
                      <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
                      Pushing…
                    </>
                  ) : (
                    <>
                      <GitBranch className="h-4 w-4" aria-hidden />
                      Push to GitHub
                    </>
                  )}
                </Button>
              </>
            )}

            <div aria-live="polite" className="min-h-[1.25rem]">
              {push.kind === "done" && (
                <p className="text-sm text-success-foreground">
                  Pushed to{" "}
                  <a
                    href={push.result.html_url}
                    target="_blank"
                    rel="noreferrer noopener"
                    className="font-medium underline underline-offset-2 hover:text-primary"
                  >
                    {push.result.owner}/{push.result.repo}
                  </a>
                  . Connect again to push another.
                </p>
              )}
              {push.kind === "failed" && (
                <p className="text-sm text-warning-foreground">{push.message}</p>
              )}
            </div>
          </CardContent>
        </Card>
      </div>

      <Card className="bg-card/50">
        <CardContent className="flex items-start gap-2.5 p-4">
          <Star className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" aria-hidden />
          <p className="text-sm text-muted-foreground">
            If this saved you time, a star on{" "}
            <a
              href={STAR_URL}
              target="_blank"
              rel="noreferrer noopener"
              className="font-medium text-foreground underline underline-offset-2 hover:text-primary"
            >
              marcjazz/authkestra
            </a>{" "}
            helps other people find it. Entirely optional, and never a condition
            of the download.
          </p>
        </CardContent>
      </Card>

      <div>
        <Button type="button" variant="secondary" onClick={onBack}>
          Back
        </Button>
      </div>
    </div>
  );
}

function GithubPushReturnBanner({
  result,
  onDismiss,
}: {
  result: GithubPushReturn;
  onDismiss: () => void;
}) {
  let message: string;
  if (result.status === "connected") {
    message = "GitHub connected — pick a name and push below.";
  } else if (result.status === "denied") {
    message = "GitHub connection cancelled. Try again anytime.";
  } else {
    message = `Couldn't connect GitHub${
      result.reason ? ` — ${describeGithubConnectReason(result.reason)}` : ""
    }.`;
  }

  const tone = result.status === "connected" ? "success" : result.status === "denied" ? "warning" : "error";
  return <OutcomeBanner tone={tone} message={message} onDismiss={onDismiss} />;
}

/**
 * The prose for a `github_push=error&reason=...` connect-round-trip failure.
 * Only `not_configured` gets its own line — the same deployment limitation
 * `describeGithubPushError` names for the push endpoint itself, and the one
 * most likely to be seen (#40). Everything else falls back to the code
 * GitHub/the backend actually reported, same as `describeOauthErrorReason`
 * does for the unrelated sign-in flow.
 */
export function describeGithubConnectReason(reason: string): string {
  return reason === "not_configured" ? "this deployment hasn't set up GitHub push yet" : reason;
}

function describeDownloadError(error: ApiError): string {
  switch (error.kind) {
    case "unavailable":
      return "Couldn't reach the API. Check your connection and try again.";
    case "rate_limited":
      return error.detail;
    case "demo_disabled":
      // Handled by the caller, which switches the whole page into explainer
      // mode rather than reporting it here.
      return "Live flows are switched off right now.";
    case "state_unavailable":
      // Distinct from `demo_disabled` on purpose: the demo is not switched
      // off, the store behind it is unreachable. Saying "temporarily" is the
      // honest difference — this one is worth retrying, and it is an outage
      // rather than an intentional state.
      return "The playground's state store is temporarily unreachable, so the project could not be generated. Try again in a moment.";
    case "http_error":
      return `The download failed (${error.status}). ${error.detail}`;
    default:
      // The zip download never produces any of `POST /api/github/push`'s own
      // error kinds — they exist on the shared `ApiError` type because that
      // endpoint does — so there is nothing more specific to say here.
      return "Something went wrong preparing the download. Please try again.";
  }
}

/**
 * Maps `POST /api/github/push`'s error vocabulary onto one actionable
 * message apiece (#40) — never flattened into "something went wrong", since
 * each of these has an unrelated fix and only the visitor who hit it can
 * tell which applies.
 */
export function describeGithubPushError(error: ApiError): string {
  switch (error.kind) {
    case "unavailable":
      return "Couldn't reach the API. Check your connection and try again.";
    case "rate_limited":
      return error.detail;
    case "demo_disabled":
      // Handled by the caller, which switches the whole page into explainer
      // mode rather than reporting it here.
      return "Live flows are switched off right now.";
    case "state_unavailable":
      return "The playground's state store is temporarily unreachable, so nothing could be pushed. Try again in a moment.";
    case "http_error":
      return `The push failed (${error.status}). ${error.detail}`;
    case "github_push_not_configured":
      // The current state of the live deployment: the OAuth app isn't
      // registered yet. Read as a limitation of this deployment, not
      // something the visitor did — and point at the path that still works.
      return "This deployment hasn't set up GitHub push yet. Use the zip download instead.";
    case "github_not_connected":
      return "Your GitHub connection is gone. Connect again.";
    case "github_repo_name_taken":
      return "That name already exists on your GitHub account. Choose another and push again.";
    case "github_invalid_repo_name":
      // GitHub's own message, verbatim, as the task calls for.
      return error.detail;
    case "github_token_rejected":
      return "Your GitHub connection expired (15 min limit). Reconnect and try again.";
    case "github_scope_missing":
      return "GitHub didn't grant the needed permission. Reconnect and accept it.";
    case "github_rate_limited":
      return error.detail;
    case "github_network_error":
      return error.detail;
  }
}
