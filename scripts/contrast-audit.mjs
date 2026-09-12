#!/usr/bin/env node
// Colour-contrast audit for the playground UI.
//
// Auth flows are where an unreadable error message stops being cosmetic: the
// text that fails contrast here is usually the text explaining why someone
// could not sign in.
//
// This reads the Tailwind classes actually used in the components rather than
// a hand-kept list of pairs, so it cannot quietly drift out of date.
//
// Since the shadcn pass, components address colour only through the semantic
// tokens in `app/globals.css` (`text-muted-foreground`, `bg-card`), so this
// resolves those tokens from that file — one source of truth for the palette,
// read by both the browser and this audit. Three consequences worth stating:
//
//   1. A raw palette class (`text-slate-400`) is now a *failure*, not something
//      to measure. The convention in docs/ui-conventions.md is that colour comes
//      from tokens; a shade that slipped through is caught here rather than
//      silently honoured.
//   2. An unknown token is a failure too. The previous version skipped anything
//      it could not resolve, which meant the migration to token classes would
//      have left it reporting "checked 0 pairs" and passing green.
//   3. Text that declares no background of its own is checked against *every*
//      surface it could plausibly sit on, and judged by the worst. The old
//      version assumed the page background, which is the most forgiving
//      surface in a dark UI — that is how `text-slate-400` captions sat at
//      4.3:1 on a raised card while the audit called them fine.
//
// WCAG AA: 4.5:1 for normal text, 3:1 for large text (>=18.66px bold or
// >=24px). Sizes are not knowable from a class string alone, so anything at
// `text-xs`/`text-sm` is judged as normal text, which is the conservative
// reading and matches how this UI is written.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const CSS = "apps/web/app/globals.css";

/** Minimum ratio for normal-size text. */
const AA_NORMAL = 4.5;

/*
  A floor on coverage. This audit's failure mode is not a wrong answer, it is
  going quiet: a refactor renames the classes, nothing resolves, every pair is
  skipped and CI reports green on an unchecked UI. If a legitimate change drops
  the count below this, raise it deliberately in the same commit.
*/
const MIN_CHECKED = 60;

/*
  Surfaces a bare piece of text might be sitting on. `background` is the page,
  `card` every panel, `popover` anything floating, `muted` inert blocks and
  code plates. All are dark, so the lightest of them is the binding constraint
  and checking all four costs nothing.
*/
const SURFACES = ["background", "card", "popover", "muted"];

/** Tailwind's built-in palette families — using one of these is the error. */
const PALETTE_FAMILIES =
  /^(slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-(50|\d{3})$/;

/** `text-*` and `bg-*` utilities that set something other than a colour. */
const NOT_A_COLOUR =
  /^(xs|sm|base|lg|xl|\d?xl|left|center|right|justify|start|end|balance|pretty|nowrap|wrap|clip|ellipsis|top|bottom|middle|gradient-to-\w+|clip-\w+|cover|contain|repeat|no-repeat|fixed|local|scroll|origin-\w+|auto|none|opacity-\d+|\[.*\])$/;

/** Colours that need no lookup. */
const LITERAL = { white: "#ffffff", black: "#000000" };

function hslToHex(h, s, l) {
  s /= 100;
  l /= 100;
  const k = (n) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  const hex = (n) =>
    Math.round(255 * f(n))
      .toString(16)
      .padStart(2, "0");
  return `#${hex(0)}${hex(8)}${hex(4)}`;
}

/**
 * Pull `--name: H S% L%` out of globals.css. The values are stored bare rather
 * than wrapped in `hsl()` so Tailwind can compose them with an opacity
 * modifier, which is also what makes them straightforward to parse here.
 */
function readTokens(path) {
  const css = readFileSync(path, "utf8");
  const tokens = {};
  for (const [, name, h, s, l] of css.matchAll(
    /--([a-z-]+):\s*([\d.]+)\s+([\d.]+)%\s+([\d.]+)%\s*;/g,
  )) {
    tokens[name] = hslToHex(Number(h), Number(s), Number(l));
  }
  return tokens;
}

const TOKENS = readTokens(CSS);

function srgbToLinear(c) {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);
}

/** Flatten a translucent colour (`bg-primary/10`) over what sits behind it. */
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

/**
 * `muted-foreground/70` -> { name, alpha } | { palette } | { unknown } | null
 *
 * `null` means "this utility is not a colour at all" (`text-sm`). The other
 * shapes are all reportable problems, which is the point: the only silent path
 * out of here is a utility that was never about colour.
 */
function parseColour(token) {
  const [name, opacity] = token.split("/");
  const alpha = opacity ? Number(opacity) / 100 : 1;
  if (NOT_A_COLOUR.test(name)) return null;
  if (name === "transparent" || name === "current" || name === "inherit") return null;
  if (LITERAL[name]) return { hex: LITERAL[name], alpha, name };
  if (PALETTE_FAMILIES.test(name)) return { palette: name };
  if (TOKENS[name]) return { hex: TOKENS[name], alpha, name };
  return { unknown: name };
}

function classNames(source) {
  // `className="..."`, `className={`...`}`, and the cva/cn string arguments
  // that shadcn components are built from.
  const attrs = [...source.matchAll(/className=(?:"([^"]*)"|\{`([^`]*)`\})/g)].map(
    (m) => m[1] ?? m[2],
  );
  const strings = [...source.matchAll(/"((?:[a-z0-9-]+:)?[a-z][a-z0-9-]*(?:\/\d+)?(?:\s+[^"]*)?)"/g)]
    .map((m) => m[1])
    .filter((s) => /(?:^|\s)(?:text|bg)-/.test(s));
  return [...attrs, ...strings];
}

const files = [];
for (const dir of ["apps/web/components", "apps/web/components/ui", "apps/web/app"]) {
  for (const entry of readdirSync(dir)) {
    if (entry.endsWith(".tsx")) files.push(join(dir, entry));
  }
}

const failures = [];
const misuse = [];
let checked = 0;

for (const file of files) {
  const source = readFileSync(file, "utf8");
  for (const cls of classNames(source)) {
    const tokens = cls.split(/\s+/);

    // Ignore state variants: they describe hover/focus, where the paired
    // colour is not knowable from this string alone.
    const plain = tokens.filter((t) => !t.includes(":"));

    // The *colour* among the text utilities, not merely the first one: a class
    // list almost always carries `text-xs` too, and taking that would skip the
    // whole element.
    const textTokens = plain.filter((t) => t.startsWith("text-")).map((t) => t.slice(5));
    const bgTokens = plain.filter((t) => t.startsWith("bg-")).map((t) => t.slice(3));

    for (const t of [...textTokens, ...bgTokens]) {
      const parsed = parseColour(t);
      if (parsed?.palette) {
        misuse.push({ file, token: parsed.palette, why: "raw palette class" });
      } else if (parsed?.unknown) {
        misuse.push({ file, token: parsed.unknown, why: "unknown token" });
      }
    }

    const fg = textTokens.map(parseColour).find((c) => c?.hex);
    if (!fg) continue;
    const bg = bgTokens.map(parseColour).find((c) => c?.hex);

    // A declared background is the answer. Without one, the text could be on
    // any surface, so judge it by the worst of them.
    const candidates = bg ? [bg] : SURFACES.map((s) => ({ hex: TOKENS[s], alpha: 1, name: s }));

    let worst = null;
    for (const surface of candidates) {
      // A translucent surface sits over the page; a translucent foreground
      // sits over whatever surface we just resolved.
      const behind = TOKENS.background;
      const bgHex =
        surface.alpha < 1 ? blend(surface.hex, surface.alpha, behind) : surface.hex;
      const fgHex = fg.alpha < 1 ? blend(fg.hex, fg.alpha, bgHex) : fg.hex;
      const ratio = contrast(fgHex, bgHex);
      if (!worst || ratio < worst.ratio) worst = { ratio, on: surface.name };
    }

    checked += 1;
    if (worst.ratio < AA_NORMAL) {
      failures.push({
        file,
        fg: fg.name,
        bg: worst.on,
        ratio: worst.ratio.toFixed(2),
      });
    }
  }
}

console.log(
  `checked ${checked} colour pair(s) across ${files.length} file(s), ` +
    `${Object.keys(TOKENS).length} token(s) from ${CSS}`,
);

let failed = false;

if (misuse.length > 0) {
  const seen = new Set();
  console.error(`\n${misuse.length} colour class(es) outside the token set:\n`);
  for (const m of misuse) {
    if (seen.has(m.token)) continue;
    seen.add(m.token);
    console.error(`  ${m.token.padEnd(28)} ${m.why}`);
    console.error(`  ${"".padEnd(28)} ${m.file}`);
  }
  console.error(`\nColour comes from the tokens in ${CSS}. See docs/ui-conventions.md.`);
  failed = true;
}

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
  failed = true;
}

if (checked < MIN_CHECKED) {
  console.error(
    `\nOnly ${checked} pair(s) checked, below the floor of ${MIN_CHECKED}.\n` +
      `An audit that stops recognising the classes in use reports green on an\n` +
      `unchecked UI. Either the classes moved, or the floor needs lowering on\n` +
      `purpose — decide which, in this commit.`,
  );
  failed = true;
}

if (failed) process.exit(1);

console.log("all pairs meet WCAG AA for normal text");
