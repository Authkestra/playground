#!/usr/bin/env node
// Colour-contrast audit for the playground UI.
//
// Auth flows are where an unreadable error message stops being cosmetic: the
// text that fails contrast here is usually the text explaining why someone
// could not sign in.
//
// This reads the Tailwind classes actually used in the components rather than
// a hand-kept list of pairs, so it cannot quietly drift out of date. It checks
// every `text-*` against the nearest background in the same `className`, and
// falls back to the page background when an element declares none.
//
// WCAG AA: 4.5:1 for normal text, 3:1 for large text (>=18.66px bold or
// >=24px). Sizes are not knowable from a class string alone, so anything at
// `text-xs`/`text-sm` is judged as normal text, which is the conservative
// reading and matches how this UI is written.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// Tailwind's default palette, only the families this UI uses.
const PALETTE = {
  "slate-950": "#020617", "slate-900": "#0f172a", "slate-800": "#1e293b",
  "slate-700": "#334155", "slate-600": "#475569", "slate-500": "#64748b",
  "slate-400": "#94a3b8", "slate-300": "#cbd5e1", "slate-200": "#e2e8f0",
  "slate-100": "#f1f5f9", "slate-50": "#f8fafc",
  "emerald-500": "#10b981", "emerald-400": "#34d399", "emerald-300": "#6ee7b7",
  "amber-500": "#f59e0b", "amber-400": "#fbbf24", "amber-300": "#fcd34d",
  "red-500": "#ef4444", "red-400": "#f87171", "red-300": "#fca5a5",
  "rose-500": "#f43f5e", "rose-400": "#fb7185", "rose-300": "#fda4af",
  "indigo-500": "#6366f1", "indigo-400": "#818cf8",
  white: "#ffffff",
};

/** The page background, from `app/layout.tsx`. */
const PAGE_BG = "slate-950";

/** Minimum ratio for normal-size text. */
const AA_NORMAL = 4.5;

function srgbToLinear(c) {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);
}

/** Flatten a translucent colour (`bg-x/10`) over what sits behind it. */
function blend(hex, alpha, behindHex) {
  const mix = (i) => {
    const top = parseInt(hex.slice(i, i + 2), 16);
    const bottom = parseInt(behindHex.slice(i, i + 2), 16);
    return Math.round(top * alpha + bottom * (1 - alpha));
  };
  return "#" + [1, 3, 5].map((i) => mix(i).toString(16).padStart(2, "0")).join("");
}

function contrast(fgHex, bgHex) {
  const [a, b] = [luminance(fgHex), luminance(bgHex)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
}

/** `emerald-500/10` -> { name, alpha } */
function parseColour(token) {
  const [name, opacity] = token.split("/");
  if (!PALETTE[name]) return null;
  return { name, alpha: opacity ? Number(opacity) / 100 : 1 };
}

function classNames(source) {
  // Both `className="..."` and `className={`...`}` forms.
  return [...source.matchAll(/className=(?:"([^"]*)"|\{`([^`]*)`\})/g)].map(
    (m) => m[1] ?? m[2],
  );
}

const files = [];
for (const dir of ["apps/web/components", "apps/web/app"]) {
  for (const entry of readdirSync(dir)) {
    if (entry.endsWith(".tsx")) files.push(join(dir, entry));
  }
}

const failures = [];
let checked = 0;

for (const file of files) {
  const source = readFileSync(file, "utf8");
  for (const cls of classNames(source)) {
    const tokens = cls.split(/\s+/);

    // Ignore state variants: they describe hover/focus, where the paired
    // colour is not knowable from this string alone.
    const plain = tokens.filter((t) => !t.includes(":"));

    // The *colour* among the text utilities, not merely the first one: a
    // class list almost always carries `text-xs` too, and taking that would
    // skip the whole element.
    const fg = plain
      .filter((t) => t.startsWith("text-"))
      .map((t) => t.slice(5))
      .find((t) => parseColour(t));
    const bgToken = plain
      .filter((t) => t.startsWith("bg-"))
      .map((t) => t.slice(3))
      .find((t) => parseColour(t));
    if (!fg) continue;

    const fgColour = parseColour(fg);

    const bgColour = bgToken ? parseColour(bgToken) : { name: PAGE_BG, alpha: 1 };
    if (!bgColour) continue;

    const behind = PALETTE[PAGE_BG];
    const bgHex =
      bgColour.alpha < 1
        ? blend(PALETTE[bgColour.name], bgColour.alpha, behind)
        : PALETTE[bgColour.name];
    const fgHex =
      fgColour.alpha < 1
        ? blend(PALETTE[fgColour.name], fgColour.alpha, bgHex)
        : PALETTE[fgColour.name];

    checked += 1;
    const ratio = contrast(fgHex, bgHex);
    if (ratio < AA_NORMAL) {
      failures.push({
        file,
        fg: fg,
        bg: bgToken ?? `${PAGE_BG} (page)`,
        ratio: ratio.toFixed(2),
      });
    }
  }
}

console.log(`checked ${checked} colour pair(s) across ${files.length} file(s)`);

if (failures.length > 0) {
  console.error(`\n${failures.length} pair(s) below WCAG AA (${AA_NORMAL}:1):\n`);
  const seen = new Set();
  for (const f of failures) {
    const key = `${f.fg}|${f.bg}`;
    if (seen.has(key)) continue;
    seen.add(key);
    console.error(`  ${f.ratio}:1  text-${f.fg} on bg-${f.bg}`);
    console.error(`           ${f.file}`);
  }
  process.exit(1);
}

console.log("all pairs meet WCAG AA for normal text");
