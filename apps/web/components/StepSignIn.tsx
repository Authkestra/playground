"use client";

import { Fragment, useState } from "react";
import type { ReactNode } from "react";
import { Fingerprint, Globe, LogIn, ShieldQuestion, Smartphone, type LucideIcon } from "lucide-react";
import type { DemoConfig, OAuthMode, ScenarioSpec } from "@playground/api-types";
import { loginUrl, type OAuthReturn } from "@/lib/oauth";
import { isControlValueActive } from "@/components/ScenarioPanel";
import { OutcomeBanner } from "@/components/OutcomeBanner";
import TotpPanel from "@/components/TotpPanel";
import PasskeysPanel from "@/components/PasskeysPanel";
import ResourcePanel from "@/components/ResourcePanel";
import CaptchaPanel from "@/components/CaptchaPanel";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  oauthReturn: OAuthReturn | null;
  onDismissOauthReturn: () => void;
  onDemoDisabled: () => void;
  onBack: () => void;
  onContinue: () => void;
}

/**
 * Scenarios with a panel in the sign-in step, in the order they appear.
 *
 * Bot protection leads: a captcha guards the form, so it belongs in front of
 * the methods rather than after them.
 */
export const PANEL_ORDER = ["captcha", "oauth", "passkeys", "totp", "resource"] as const;

/** Switched on by the visitor *and* usable on this deployment. */
export function isScenarioLive(
  scenarios: ScenarioSpec[],
  config: DemoConfig | null,
  id: string,
): boolean {
  const spec = scenarios.find((s) => s.id === id);
  // Both halves matter. A kill-switched scenario used to render its panel
  // anyway, and then every button inside it answered 503.
  return !!spec && spec.available !== false && isControlValueActive(config?.scenarios?.[id]);
}

/**
 * Which panels to show, in order.
 *
 * Pure and exported because this is where the bug was: the "is anything on?"
 * check listed every panel except the resource server, so turning on only the
 * protected route showed "no sign-in method is turned on yet" while its panel
 * sat one branch away, never rendered. Deriving the list from `PANEL_ORDER`
 * makes that unrepresentable — a panel cannot render without counting towards
 * the empty state, and the dividers between them cannot disagree with what is
 * actually on either side.
 */
export function visiblePanels(
  scenarios: ScenarioSpec[],
  config: DemoConfig | null,
  oauthOptionCount: number,
): string[] {
  return PANEL_ORDER.filter((id) => {
    if (!isScenarioLive(scenarios, config, id)) return false;
    // A provider can be selected and no longer offered — credentials pulled
    // from the deployment — so OAuth needs a button to show, not a selection.
    if (id === "oauth") return oauthOptionCount > 0;
    return true;
  });
}

/** Whether any of the live panels is actually a way to sign in. */
export function hasSignInMethod(visible: string[]): boolean {
  return visible.some((id) => id === "oauth" || id === "passkeys" || id === "totp");
}

const PANEL_ICON: Record<string, LucideIcon> = {
  captcha: ShieldQuestion,
  passkeys: Fingerprint,
  totp: Smartphone,
  resource: Globe,
};

const PANEL_TITLE: Record<string, string> = {
  captcha: "Bot protection",
  passkeys: "Passkey",
  totp: "Authenticator app",
  resource: "Protected API route",
};

export default function StepSignIn({
  scenarios,
  config,
  oauthReturn,
  onDismissOauthReturn,
  onDemoDisabled,
  onBack,
  onContinue,
}: Props) {
  const [oauthMode, setOauthMode] = useState<OAuthMode>("session");

  const oauthScenario = scenarios.find((s) => s.id === "oauth");
  const oauthValue = config?.scenarios.oauth;
  const selectedProviders = oauthValue?.kind === "select_many" ? oauthValue.selected : [];
  const oauthOptions =
    oauthScenario?.control.kind === "select_many" ? oauthScenario.control.options : [];
  const activeOauthOptions = oauthOptions.filter((o) => selectedProviders.includes(o.id));

  const visible = visiblePanels(scenarios, config, activeOauthOptions.length);
  const signInMethodOn = hasSignInMethod(visible);

  const renderPanel = (id: string): ReactNode => {
    switch (id) {
      case "captcha":
        return (
          <MethodPanel id="captcha">
            <CaptchaPanel scenarioId="captcha" onDemoDisabled={onDemoDisabled} />
          </MethodPanel>
        );
      case "oauth":
        return (
          <div className="flex flex-col gap-2">
            {activeOauthOptions.map((option) => (
              <Button
                key={option.id}
                type="button"
                variant="outline"
                className="w-full justify-center gap-2"
                onClick={() => {
                  window.location.href = loginUrl(option.id, oauthMode);
                }}
              >
                <LogIn className="h-4 w-4" aria-hidden />
                Continue with {option.label}
              </Button>
            ))}
            <div className="mt-2 flex flex-col items-center gap-1.5">
              <span className="text-xs text-muted-foreground">Identity mode</span>
              <Tabs
                value={oauthMode}
                onValueChange={(value) => setOauthMode(value as OAuthMode)}
              >
                <TabsList>
                  <TabsTrigger value="session">Session</TabsTrigger>
                  <TabsTrigger value="jwt">Stateless (JWT)</TabsTrigger>
                </TabsList>
              </Tabs>
            </div>
          </div>
        );
      case "passkeys":
        return (
          <MethodPanel id="passkeys">
            <PasskeysPanel scenarioId="passkeys" onDemoDisabled={onDemoDisabled} />
          </MethodPanel>
        );
      case "totp":
        return (
          <MethodPanel id="totp">
            <TotpPanel scenarioId="totp" onDemoDisabled={onDemoDisabled} />
          </MethodPanel>
        );
      case "resource":
        return (
          <MethodPanel id="resource">
            <ResourcePanel scenarioId="resource" onDemoDisabled={onDemoDisabled} />
          </MethodPanel>
        );
      default:
        return null;
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-foreground">Sign in</h2>
        <p className="text-sm text-muted-foreground">
          A real sign-in screen assembled from what you chose in step 1. Each panel shows
          the request it made and what came back, so nothing here has to be taken on
          trust.
        </p>
      </div>

      <Card className="mx-auto w-full max-w-2xl">
        <CardContent className="flex flex-col gap-5 pt-6">
          {oauthReturn && (
            <OAuthReturnBanner result={oauthReturn} onDismiss={onDismissOauthReturn} />
          )}

          {visible.length === 0 ? (
            <p className="text-center text-sm text-muted-foreground">
              Nothing is turned on yet.{" "}
              <Button
                type="button"
                variant="link"
                className="h-auto p-0 align-baseline text-sm"
                onClick={onBack}
              >
                Go back to step 1
              </Button>{" "}
              to choose something.
            </p>
          ) : (
            <>
              <div className="text-center">
                <h3 className="text-base font-semibold text-foreground">
                  Sign in to Authkestra
                </h3>
                <p className="text-sm text-muted-foreground">
                  {signInMethodOn
                    ? "Choose how you'd like to continue."
                    : "No sign-in method is on — try what you did turn on below."}
                </p>
              </div>

              <div className="mx-auto flex w-full max-w-sm flex-col gap-5">
                {visible.map((id, i) => (
                  <Fragment key={id}>
                    {i > 0 && <Divider />}
                    {renderPanel(id)}
                  </Fragment>
                ))}
              </div>
            </>
          )}
        </CardContent>
      </Card>

      <div className="flex items-center justify-between">
        <Button type="button" variant="secondary" onClick={onBack}>
          Back
        </Button>
        <Button type="button" variant="default" onClick={onContinue}>
          Continue
        </Button>
      </div>
    </div>
  );
}

/** A bordered sub-panel for one ceremony (captcha, passkeys, TOTP, protected route). */
function MethodPanel({ id, children }: { id: string; children: ReactNode }) {
  const Icon = PANEL_ICON[id];
  return (
    <Card>
      <CardHeader className="flex-row items-center gap-2 space-y-0 p-4 pb-2">
        {Icon && <Icon className="h-4 w-4 text-muted-foreground" aria-hidden />}
        <CardTitle className="text-sm font-medium">{PANEL_TITLE[id]}</CardTitle>
      </CardHeader>
      <CardContent className="p-4 pt-0">{children}</CardContent>
    </Card>
  );
}

function Divider() {
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <Separator className="flex-1" />
      or
      <Separator className="flex-1" />
    </div>
  );
}

function describeOauthErrorReason(reason: string): string {
  switch (reason) {
    case "missing_code":
      return "the provider didn't send back a code";
    case "unknown_provider":
      return "that provider isn't configured on this deployment";
    case "exchange_failed":
      return "the code exchange with the provider failed";
    case "state_missing":
      return "the browser didn't send back the flow's state cookie — this usually means more than 15 minutes passed, cookies were blocked, or the flow was started in a different browser";
    case "state_invalid":
      return "the flow's state cookie was invalid or tampered with";
    case "callback_failed":
      return "the callback from the provider failed unexpectedly";
    case "demo_disabled":
      return "OAuth is temporarily switched off";
    default:
      return reason;
  }
}

function OAuthReturnBanner({
  result,
  onDismiss,
}: {
  result: OAuthReturn;
  onDismiss: () => void;
}) {
  let message: string;
  if (result.status === "success") {
    let modeDescription = "";
    if (result.mode === "session") {
      modeDescription = " A server-side session was established with its ID in a cookie.";
    } else if (result.mode === "jwt") {
      modeDescription = " A signed token (JWT) was issued with no server-side session.";
    }
    message = `Signed in with ${result.provider}.${modeDescription}`;
  } else if (result.status === "denied") {
    message = `You cancelled signing in with ${result.provider}. No harm done — try again whenever you're ready.`;
  } else {
    message = `Couldn't complete sign-in with ${result.provider}${
      result.reason ? ` — ${describeOauthErrorReason(result.reason)}` : ""
    }.`;
  }

  const tone = result.status === "success" ? "success" : result.status === "denied" ? "warning" : "error";
  return <OutcomeBanner tone={tone} message={message} onDismiss={onDismiss} />;
}
