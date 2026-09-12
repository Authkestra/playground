"use client";

import { useEffect, useRef } from "react";
import { AlertTriangle, CheckCircle2, XCircle, type LucideIcon } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

export type OutcomeTone = "success" | "warning" | "error";

const TONE_STYLE: Record<
  OutcomeTone,
  { variant: "default" | "destructive"; className: string; Icon: LucideIcon }
> = {
  success: {
    variant: "default",
    className: "border-success/40 bg-success/10 text-success-foreground [&>svg]:text-success-foreground",
    Icon: CheckCircle2,
  },
  // A cancelled round trip is an ordinary outcome — calm warning tone, not
  // an error.
  warning: {
    variant: "default",
    className: "border-warning/40 bg-warning/10 text-warning-foreground [&>svg]:text-warning-foreground",
    Icon: AlertTriangle,
  },
  error: {
    variant: "destructive",
    className: "bg-destructive/10",
    Icon: XCircle,
  },
};

/**
 * The banner a visitor sees right after a browser round trip to a third
 * party and back — signing in with an OAuth provider, or connecting GitHub
 * to push (#40). Shared by `StepSignIn`'s and `StepDownload`'s own return
 * banners because the shape is identical: one outcome message, a tone, and a
 * way to dismiss it.
 *
 * Focuses itself on mount: the browser drops focus at the top of a freshly
 * loaded document, so without this the outcome of the thing a visitor just
 * did is somewhere below, unannounced — and for a screen-reader user, the
 * round trip appears to have done nothing. `role="status"` announces it;
 * the ref focuses it, so keyboard users continue from the result rather than
 * tabbing back to it.
 */
export function OutcomeBanner({
  tone,
  message,
  onDismiss,
}: {
  tone: OutcomeTone;
  message: string;
  onDismiss: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);

  const { variant, className, Icon } = TONE_STYLE[tone];

  return (
    <Alert ref={ref} tabIndex={-1} role="status" variant={variant} className={className}>
      <Icon className="h-4 w-4" aria-hidden />
      <AlertDescription className="flex items-start justify-between gap-3">
        <p>{message}</p>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-auto shrink-0 px-2 py-1 text-xs underline underline-offset-2"
          onClick={onDismiss}
        >
          Dismiss
        </Button>
      </AlertDescription>
    </Alert>
  );
}
