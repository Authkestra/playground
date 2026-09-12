"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type {
  ConfigDiff,
  ControlValue,
  DemoConfig,
  DemoSessionView,
  HealthResponse,
  ScenarioSpec,
} from "@playground/api-types";
import {
  API_BASE,
  configureScenario,
  errorDetail,
  getHealth,
  getScenarios,
  getSession,
  resetSession,
} from "@/lib/api";
import {
  clearGithubPushReturnParams,
  clearOAuthReturnParams,
  readGithubPushReturn,
  readOAuthReturn,
  type GithubPushReturn,
  type OAuthReturn,
} from "@/lib/oauth";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/cn";
import SessionBar from "@/components/SessionBar";
import ScenarioPanel from "@/components/ScenarioPanel";
import StepIndicator from "@/components/StepIndicator";
import StepChooseMethods from "@/components/StepChooseMethods";
import StepSignIn from "@/components/StepSignIn";
import StepDownload from "@/components/StepDownload";

type Phase = "loading" | "unavailable" | "explainer" | "ready";
type Step = 1 | 2 | 3;

export default function Playground() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [, setHealth] = useState<HealthResponse | null>(null);
  const [session, setSession] = useState<DemoSessionView | null>(null);
  const [scenarios, setScenarios] = useState<ScenarioSpec[]>([]);
  const [config, setConfig] = useState<DemoConfig | null>(null);
  const [diff, setDiff] = useState<ConfigDiff | null>(null);
  const [diffScenarioName, setDiffScenarioName] = useState<string | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [pendingIds, setPendingIds] = useState<Set<string>>(new Set());
  const [resetting, setResetting] = useState(false);

  const [step, setStep] = useState<Step>(1);
  const [maxReached, setMaxReached] = useState<Step>(1);
  const [oauthReturn, setOauthReturn] = useState<OAuthReturn | null>(null);
  const [githubPushReturn, setGithubPushReturn] = useState<GithubPushReturn | null>(null);

  // Focus moves into the step that just appeared.
  //
  // The wizard swaps content without a navigation, so a keyboard user who
  // presses "Continue" is left with focus on a button that no longer exists,
  // and a screen reader announces nothing — the page silently became a
  // different page. Focusing the new step's heading puts them at the top of
  // what they just asked for.
  const stepHeadingRef = useRef<HTMLDivElement>(null);
  const [pendingFocus, setPendingFocus] = useState(false);

  const goToStep = useCallback((next: Step) => {
    setStep(next);
    setMaxReached((prev) => (next > prev ? next : prev));
    setPendingFocus(true);
  }, []);

  useEffect(() => {
    if (!pendingFocus) return;
    stepHeadingRef.current?.focus();
    setPendingFocus(false);
  }, [pendingFocus, step]);

  const load = useCallback(async () => {
    setPhase("loading");
    setBanner(null);

    const healthResult = await getHealth();
    if (!healthResult.ok) {
      setPhase("unavailable");
      return;
    }
    setHealth(healthResult.data);

    // Best-effort: fetch the scenario specs even in explainer mode so the
    // disabled controls still render (rather than an empty page).
    const scenariosResult = await getScenarios();
    if (scenariosResult.ok) {
      setScenarios(scenariosResult.data);
    }

    if (!healthResult.data.demo_enabled) {
      setPhase("explainer");
      return;
    }

    if (!scenariosResult.ok) {
      switch (scenariosResult.error.kind) {
        case "demo_disabled":
          setPhase("explainer");
          break;
        case "unavailable":
          setPhase("unavailable");
          break;
        case "rate_limited":
          setBanner(scenariosResult.error.detail);
          setPhase("unavailable");
          break;
        default:
          setBanner(`Could not load scenarios (${errorDetail(scenariosResult.error)}).`);
          setPhase("unavailable");
      }
      return;
    }

    const sessionResult = await getSession();
    if (!sessionResult.ok) {
      switch (sessionResult.error.kind) {
        case "demo_disabled":
          setPhase("explainer");
          break;
        case "unavailable":
          setPhase("unavailable");
          break;
        case "rate_limited":
          setBanner(sessionResult.error.detail);
          setPhase("unavailable");
          break;
        default:
          setBanner(`Could not load session (${errorDetail(sessionResult.error)}).`);
          setPhase("unavailable");
      }
      return;
    }

    setSession(sessionResult.data);
    setConfig(sessionResult.data.config);
    setPhase("ready");

    // The browser may just have navigated back from an OAuth provider — the
    // outcome arrives as query params on this very load. Read them, land the
    // visitor on step 2 to see it, then scrub the URL so a reload or a
    // shared link doesn't replay the same result.
    const parsedOauthReturn = readOAuthReturn(window.location.search);
    if (parsedOauthReturn) {
      setOauthReturn(parsedOauthReturn);
      clearOAuthReturnParams();
      goToStep(2);
    }

    // Same idea for the push-to-GitHub connect round trip (#40), which lands
    // back on step 3 rather than step 2 — that's where the GitHub card lives.
    const parsedGithubPushReturn = readGithubPushReturn(window.location.search);
    if (parsedGithubPushReturn) {
      setGithubPushReturn(parsedGithubPushReturn);
      clearGithubPushReturnParams();
      goToStep(3);
    }
  }, [goToStep]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleReset = useCallback(async () => {
    setResetting(true);
    setBanner(null);
    const result = await resetSession();
    setResetting(false);

    if (!result.ok) {
      switch (result.error.kind) {
        case "demo_disabled":
          setPhase("explainer");
          break;
        case "unavailable":
          setPhase("unavailable");
          break;
        case "rate_limited":
          setBanner(result.error.detail);
          break;
        default:
          setBanner("Could not reset the session. Please try again.");
      }
      return;
    }

    setSession(result.data);
    setConfig(result.data.config);
    setDiff(null);
    setDiffScenarioName(null);
    setOauthReturn(null);
    setGithubPushReturn(null);
    setStep(1);
    setMaxReached(1);
  }, []);

  const handleChange = useCallback(
    async (id: string, value: ControlValue) => {
      setPendingIds((prev) => new Set(prev).add(id));
      setBanner(null);

      // Move the control immediately and roll back if the server disagrees.
      // Waiting for the round trip made the switch feel dead — and on a
      // free-tier host that has spun down, the first interaction can take
      // tens of seconds, which reads as nothing happening at all.
      const previousConfig = config;
      setConfig((prev) =>
        prev
          ? { ...prev, scenarios: { ...prev.scenarios, [id]: value } }
          : prev,
      );

      const result = await configureScenario(id, { value });

      setPendingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });

      if (!result.ok) {
        // Put the control back where it was, so the UI never claims a change
        // the server did not accept.
        setConfig(previousConfig);
        switch (result.error.kind) {
          case "demo_disabled":
            setPhase("explainer");
            break;
          case "unavailable":
            setPhase("unavailable");
            break;
          case "rate_limited":
            setBanner(result.error.detail);
            break;
          default:
            setBanner(`Could not update "${id}": ${errorDetail(result.error)}`);
        }
        return;
      }

      // The server's copy is authoritative — it may normalise the value.
      setConfig(result.data.config);
      setDiff(result.data.diff);
      setDiffScenarioName(scenarios.find((s) => s.id === id)?.name ?? id);
    },
    [scenarios, config],
  );

  if (phase === "loading") {
    return (
      <Shell busy>
        <div className="flex flex-col gap-4">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-6 w-3/4" />
        </div>
        <div className="flex flex-col gap-2">
          <Skeleton className="h-5 w-1/2" />
          <Skeleton className="h-5 w-2/3" />
        </div>
        <p className="text-sm text-muted-foreground">
          Loading playground… The API runs on a free tier and can take up to a minute to wake
          up on its first request.
        </p>
      </Shell>
    );
  }

  if (phase === "unavailable") {
    return (
      <Shell>
        <Alert variant="destructive">
          <AlertTitle>API unavailable</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-3">
            <p>
              The playground couldn&apos;t reach the API at{" "}
              <code className="font-mono">{API_BASE}</code>. Start the backend and
              reload this page.
            </p>
            {banner && <p className="font-medium text-warning-foreground">{banner}</p>}
            <Button type="button" variant="outline" size="sm" onClick={() => void load()}>
              Retry
            </Button>
          </AlertDescription>
        </Alert>
      </Shell>
    );
  }

  if (phase === "explainer") {
    return (
      <Shell>
        <Alert>
          <AlertTitle>Demo currently disabled</AlertTitle>
          <AlertDescription>
            The live playground is switched off right now, so controls below are
            shown for reference only. This is expected behaviour, not an error —
            check back later to try the flows interactively.
          </AlertDescription>
        </Alert>
        <section className="flex flex-col gap-3">
          <h2 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
            Scenarios
          </h2>
          <ScenarioPanel
            scenarios={scenarios}
            config={config}
            pendingIds={pendingIds}
            disabled
            disabledReason="Controls are disabled while the demo is switched off."
            onChange={() => {}}
          />
        </section>
      </Shell>
    );
  }

  return (
    <Shell wide>
      <div aria-live="polite" className="min-h-10">
        {banner && (
          <Alert className="border-warning/40 bg-warning/10 text-warning-foreground [&>svg]:text-warning-foreground">
            <AlertDescription>{banner}</AlertDescription>
          </Alert>
        )}
      </div>

      <SessionBar
        session={session}
        onReset={() => void handleReset()}
        resetting={resetting}
      />

      <StepIndicator current={step} maxReached={maxReached} onNavigate={goToStep} />

      <Separator />

      {/* The focus target for a step change. `tabIndex={-1}` makes it
          programmatically focusable without adding a tab stop of its own. */}
      <div ref={stepHeadingRef} tabIndex={-1} className="focus-visible:outline-none">
        {step === 1 && (
          <StepChooseMethods
            scenarios={scenarios}
            config={config}
            pendingIds={pendingIds}
            onChange={(id, value) => void handleChange(id, value)}
            diff={diff}
            diffScenarioName={diffScenarioName}
            onContinue={() => goToStep(2)}
          />
        )}

        {step === 2 && (
          <StepSignIn
            scenarios={scenarios}
            config={config}
            oauthReturn={oauthReturn}
            onDismissOauthReturn={() => setOauthReturn(null)}
            onDemoDisabled={() => setPhase("explainer")}
            onBack={() => goToStep(1)}
            onContinue={() => goToStep(3)}
          />
        )}

        {step === 3 && (
          <StepDownload
            scenarios={scenarios}
            config={config}
            onDemoDisabled={() => setPhase("explainer")}
            onBack={() => goToStep(2)}
            githubPushReturn={githubPushReturn}
            onDismissGithubPushReturn={() => setGithubPushReturn(null)}
          />
        )}
      </div>
    </Shell>
  );
}

/** The framework's own site, which links back here. */
const AUTHKESTRA_SITE = "https://authkestra.com";

/**
 * The page shell every phase renders into: a header bar with the product
 * name and a link back to the framework's site, and a max-width content
 * column below it with consistent vertical rhythm. `wide` widens the column
 * for the ready phase, which carries the session bar, step nav and a step's
 * own panels; the loading/unavailable/explainer phases stay narrower since
 * they hold a single message.
 */
function Shell({
  children,
  wide = false,
  busy = false,
}: {
  children: ReactNode;
  wide?: boolean;
  busy?: boolean;
}) {
  return (
    <div className="flex min-h-screen flex-col">
      <Header />
      <main
        aria-busy={busy || undefined}
        aria-label={busy ? "Loading playground" : undefined}
        className={cn(
          "mx-auto flex w-full flex-1 flex-col gap-8 px-6 py-10 sm:py-14",
          wide ? "max-w-5xl" : "max-w-3xl",
        )}
      >
        {children}
      </main>
    </div>
  );
}

function Header() {
  return (
    <header className="border-b border-border">
      <div className="mx-auto flex w-full max-w-5xl flex-wrap items-start justify-between gap-3 px-6 py-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-foreground">
            Authkestra Playground
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Choose your sign-in methods, see the config diff, and try the flows live.
          </p>
        </div>
        {/*
          A visitor who likes what they see should not have to go hunting for the
          framework — the playground exists to send people there.
        */}
        <Button asChild variant="outline" size="sm" className="shrink-0">
          <a href={AUTHKESTRA_SITE} target="_blank" rel="noreferrer">
            authkestra docs
            <span aria-hidden="true">→</span>
            <span className="sr-only">(opens in a new tab)</span>
          </a>
        </Button>
      </div>
    </header>
  );
}
