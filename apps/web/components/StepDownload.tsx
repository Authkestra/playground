"use client";

import { useState } from "react";
import { Download, Loader2, Star } from "lucide-react";
import type { DemoConfig, ScenarioSpec } from "@playground/api-types";
import {
  downloadStarterKit,
  type ApiError,
  type StarterKitOptions,
} from "@/lib/api";
import { isControlValueActive } from "@/components/ScenarioPanel";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  onDemoDisabled: () => void;
  onBack: () => void;
}

const STAR_URL = "https://github.com/marcjazz/authkestra";

type State =
  | { kind: "idle" }
  | { kind: "working" }
  | { kind: "done"; filename: string }
  | { kind: "failed"; message: string };

export default function StepDownload({
  scenarios,
  config,
  onDemoDisabled,
  onBack,
}: Props) {
  const [state, setState] = useState<State>({ kind: "idle" });
  // Two independent choices, not a single "extras" toggle: someone on htmx
  // wants the spec and no TypeScript, someone on Next.js may want the client
  // and no utoipa.
  const [options, setOptions] = useState<StarterKitOptions>({
    openapi: false,
    tsClient: false,
  });

  const included = scenarios.filter((s) => {
    const value = config?.scenarios?.[s.id];
    return value ? isControlValueActive(value) : false;
  });

  async function handleDownload() {
    setState({ kind: "working" });
    const result = await downloadStarterKit(options);

    if (!result.ok) {
      if (result.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setState({ kind: "failed", message: describe(result.error) });
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

  const working = state.kind === "working";

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-foreground">Download</h2>
        <p className="text-sm text-muted-foreground">
          Turn what you configured into a real, runnable project.
        </p>
      </div>

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

function describe(error: ApiError): string {
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
  }
}
