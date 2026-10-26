"use client";

import type { DemoConfig, ScenarioSpec } from "@playground/api-types";
import { isControlValueActive } from "@/components/ScenarioPanel";
import { ArrowRight, Bot, Cpu, KeyRound, Radio, ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/cn";

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  className?: string;
}

interface TopologyNode {
  id: string;
  title: string;
  subtitle: string;
  icon: typeof ShieldCheck;
  isActive: boolean;
  activeFeatures: string[];
}

export function isScenarioActive(config: DemoConfig | null, id: string): boolean {
  return isControlValueActive(config?.scenarios?.[id]);
}

export default function FlowTopology({ scenarios: _scenarios, config, className = "" }: Props) {
  const hasCaptcha = isScenarioActive(config, "captcha");
  const hasPolicy = isScenarioActive(config, "policy");
  const hasOauth = isScenarioActive(config, "oauth");
  const hasPasskeys = isScenarioActive(config, "passkeys");
  const hasTotp = isScenarioActive(config, "totp");
  const hasPasswordless = isScenarioActive(config, "passwordless");
  const hasSsf = isScenarioActive(config, "ssf");
  const hasResource = isScenarioActive(config, "resource");
  const hasMcp = isScenarioActive(config, "mcp");

  const nodes: TopologyNode[] = [
    {
      id: "ingress",
      title: "Ingress",
      subtitle: hasMcp ? "Client & AI Agent" : "Browser Client",
      icon: hasMcp ? Bot : Cpu,
      isActive: true,
      activeFeatures: hasMcp ? ["Agent MCP Client"] : ["Web Browser"],
    },
    {
      id: "policy-gate",
      title: "Policy & Shield",
      subtitle: "Deterministic Gate",
      icon: ShieldCheck,
      isActive: hasPolicy || hasCaptcha,
      activeFeatures: [
        ...(hasPolicy ? ["Cedar ABAC (<100µs)"] : []),
        ...(hasCaptcha ? ["Bot Defense"] : []),
      ],
    },
    {
      id: "auth-core",
      title: "Auth Core",
      subtitle: "Identity Verification",
      icon: KeyRound,
      isActive: hasOauth || hasPasskeys || hasTotp || hasPasswordless,
      activeFeatures: [
        ...(hasPasskeys ? ["Passkeys (FIDO2)"] : []),
        ...(hasPasswordless ? ["Passwordless / OTP"] : []),
        ...(hasTotp ? ["TOTP 2FA"] : []),
        ...(hasOauth ? ["Federated OAuth"] : []),
      ],
    },
    {
      id: "relay-ssf",
      title: "Cloud Relay",
      subtitle: "Continuous Trust",
      icon: Radio,
      isActive: hasSsf,
      activeFeatures: [
        ...(hasSsf ? ["CAEP / SSF Push", "Instant Eviction"] : []),
      ],
    },
    {
      id: "target",
      title: "Target Service",
      subtitle: hasMcp ? "MCP Tools & API" : "Protected API",
      icon: Bot,
      isActive: hasResource || hasMcp,
      activeFeatures: [
        ...(hasMcp ? ["MCP Step-Up Guard"] : []),
        ...(hasResource ? ["Zero-Trust Resource"] : []),
      ],
    },
  ];

  return (
    <div
      role="region"
      aria-label="Authentication Flow Topology"
      className={`rounded-xl border border-border/60 bg-card/60 p-4 shadow-sm ${className}`}
    >
      <div className="mb-3 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="h-2 w-2 rounded-full bg-primary" aria-hidden />
          <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Flow Topology Architecture
          </h3>
        </div>
        <span className="text-xs text-muted-foreground">
          {nodes.filter((n) => n.isActive).length} of {nodes.length} stages active
        </span>
      </div>

      <div className="grid grid-cols-1 gap-2 sm:grid-cols-5">
        {nodes.map((node, index) => {
          const Icon = node.icon;
          const nextActive = index < nodes.length - 1 && node.isActive && nodes[index + 1].isActive;

          return (
            <div
              key={node.id}
              className={cn(
                "relative flex flex-col justify-between rounded-lg border p-3",
                node.isActive ? "border-primary/40 bg-card" : "border-border/30 bg-muted/20",
              )}
            >
              <div className="flex flex-col gap-1.5 text-foreground">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-1.5">
                    <Icon
                      className={`h-4 w-4 ${node.isActive ? "text-primary-accent" : "text-muted-foreground"}`}
                      strokeWidth={1.5}
                      aria-hidden
                    />
                    <span className="text-xs font-medium text-foreground">{node.title}</span>
                  </div>
                  {node.isActive && (
                    <span className="h-1.5 w-1.5 rounded-full bg-primary" aria-hidden />
                  )}
                </div>

                <p className="text-[11px] text-muted-foreground leading-tight">
                  {node.subtitle}
                </p>

                <div className="mt-1 flex flex-wrap gap-1">
                  {node.activeFeatures.length > 0 ? (
                    node.activeFeatures.map((feat) => (
                      <Badge
                        key={feat}
                        variant={node.isActive ? "default" : "outline"}
                        className="px-1.5 py-0 text-[10px] font-normal"
                      >
                        {feat}
                      </Badge>
                    ))
                  ) : (
                    <span className="text-[10px] italic text-muted-foreground">Inert</span>
                  )}
                </div>
              </div>

              {index < nodes.length - 1 && (
                <div
                  className="hidden sm:flex absolute -right-2.5 top-1/2 -translate-y-1/2 z-10 size-5 items-center justify-center rounded-full border border-border/80 bg-background"
                  aria-hidden
                >
                  <ArrowRight
                    className={`h-3 w-3 ${nextActive ? "text-primary-accent" : "text-muted-foreground"}`}
                    strokeWidth={2}
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
