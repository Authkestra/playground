#!/usr/bin/env node
// JavaScript budget for the playground routes.
//
// "Optimal performance from day one" degrades silently unless something
// measures it. This gates the thing that actually regresses — how much
// JavaScript a visitor downloads before the page is usable — by reading
// Next.js's own build manifest rather than parsing its printed table.
//
// Sizes are gzipped, because that is what crosses the network. Numbers are
// deliberately close to current: a budget with generous headroom is a budget
// nobody notices breaking.
//
// What this does NOT measure, stated so it is a known gap rather than a
// discovery: third-party scripts a page fetches at runtime. The captcha
// widgets load from Cloudflare, hCaptcha and Google, and none of them appears
// in the build manifest — they are not bundled, so nothing here can see them.
// Two things keep that honest rather than convenient. Our own code for
// mounting them *is* counted, since it ships in the route chunk. And the
// scripts are loaded lazily, per provider, only once a visitor has switched
// bot protection on and reached the sign-in step — so the budgeted number is
// what a visitor who never turns it on actually downloads, which is the
// number this file exists to defend. Measuring the vendors properly would
// mean fetching them at build time and gating on somebody else's release
// schedule; if that becomes worth doing it wants its own check, not a bigger
// number here.
//
//   node scripts/bundle-budget.mjs           # check
//   node scripts/bundle-budget.mjs --report  # print, do not fail

import { gzipSync } from "node:zlib";
import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const APP = "apps/web";
const NEXT = join(APP, ".next");

/**
 * First Load JS per route, gzipped, in kilobytes.
 *
 * `/` is the playground itself: a client-rendered island with three steps and
 * every ceremony panel. `/_not-found` is the floor — what Next.js costs before
 * any of our code — and is budgeted so a framework upgrade that doubles the
 * baseline is visible rather than absorbed. `/how-it-works` is the explainer
 * route from issue #21: a pure server component, no `"use client"` anywhere
 * in its tree, so its budget exists to defend one specific thing — that the
 * playground's client island (the ~50 kB of the switches, steps and ceremony
 * panels that make up `/page`) cannot leak into it. It measures a few
 * kilobytes above the `/_not-found` floor rather than sitting on it exactly,
 * because the shared header's navigation and the page's own link back into
 * the playground both go through `next/link`, which costs a little over the
 * bare 404 page. If this number ever climbs anywhere near `/page`'s, that is
 * the client bundle leaking and is a bug, not a budget to raise.
 *
 * ## Why `/how-it-works/page` is 93
 *
 * Measured at 92.1 kB gzipped against a 86.2 kB `/_not-found` floor — the
 * `next/link` cost above, nothing else. 93 leaves headroom of less than a
 * kilobyte on purpose: generous headroom here is exactly what would let a
 * stray client import go unnoticed.
 *
 * ## Why `/page` went from 115 to 136
 *
 * The shadcn/ui pass. What that bought, since a raised budget is worthless
 * without the reason attached:
 *
 * The controls in this UI are the UI — it is a panel of switches, radios and
 * checkboxes — and they were hand-rolled. The switch was a `<button>` with an
 * absolutely-positioned span whose travel had been hand-measured to
 * `translate-x-[22px]`, and the radios and checkboxes were bare native inputs
 * carrying no styling at all. Radix's primitives replace that with the
 * keyboard handling, focus management, form association and ARIA wiring that
 * a correct switch/radio/checkbox actually needs, none of which the hand-
 * rolled versions had in full. That is roughly 20 kB gzipped and it is the
 * whole of the increase.
 *
 * It is not more than that because the increase was audited rather than
 * accepted. Radix's Tooltip — the single heaviest primitive, since it pulls
 * in the popper/floating-ui machinery — came to about 14 kB on its own and
 * was removed outright: its only use was explaining why a control was
 * disabled, and a tooltip on a disabled control is unreachable by keyboard
 * and unreliable by mouse, so the explanation is plain visible text tied to
 * the control with `aria-describedby` instead. Cheaper and more accessible.
 * `lucide-react` adds nothing measurable; Next already rewrites its barrel
 * imports to deep paths.
 *
 * ## And why it then went 136 -> 138
 *
 * The second delivery path: pushing a generated project straight to a GitHub
 * repository instead of downloading a zip, plus the deploy-target picker that
 * chooses which manifests come with it. That is a connect/authorise/name/push
 * flow and seven distinct error states, each with its own message, because the
 * API deliberately distinguishes a taken repo name from a dead token from a
 * missing scope and flattening them back into "something went wrong" in the UI
 * would waste the whole point.
 *
 * It measured 136.8 kB — 0.8 over — and the alternative was stripping
 * decorative icons to buy the difference. Two kilobytes for a second way to
 * get your project out of the playground is a better trade than a page that
 * fits by being slightly worse to look at.
 *
 * The number stays deliberately close to actual (136.8 kB at the time of
 * writing). If a future change needs more, it needs a paragraph here too.
 *
 * ## And why it then went 138 -> 140
 *
 * Verifying the resource scenario's tokens in the visitor's own browser
 * (#76). Measured at 139.7 kB against 136.7 kB before it, so three
 * kilobytes, and no dependency: the signature check is `crypto.subtle` with
 * Ed25519, which every supported browser already ships. What the three
 * kilobytes buy is the decoder for the token's claims, a two-step
 * fetch-then-verify flow, and the prose that tells a visitor how to catch us
 * out — watch the network panel stay silent, or go offline and verify anyway.
 *
 * Most of it is that prose, and that is the right trade rather than an
 * embarrassing one. A verdict computed locally is worth no more than a verdict
 * computed on our server unless the visitor is told how to tell the two apart,
 * so the explanation is not decoration on the feature — it is the feature. The
 * cheap way to buy the kilobytes back would be to cut it, which would leave a
 * badge nobody has reason to believe.
 *
 * ## And why it then went 140 -> 141, net, despite deleting most of the panel
 *
 * Collapsing the resource panel's eleven buttons into one `Select` and one
 * "Run it" (the "thousand of buttons" feedback after #82/#83). Measured at
 * 140.7 kB against 139.7 kB before it — essentially flat, which is the
 * interesting result given how much moved. The panel deleted three top
 * buttons, six forgery buttons, two verify-step buttons, and three-plus
 * standing paragraphs of prose, and added a live three-item checklist, a
 * two-card result layout, and a collapsible disclosure for everything that
 * used to sit in the open. Those roughly cancel out in bytes because none of
 * it is a new dependency — same `crypto.subtle` verification, same
 * `fetchJwks`/`verifyTokenSignature` from `lib/jwt.ts`, just chained behind
 * one button instead of split across nine.
 *
 * The one genuine addition is `components/ui/select.tsx` — and it is a
 * **native** `<select>` with a `ChevronDown` from `lucide-react` overlaid on
 * it, not `@radix-ui/react-select`. That was tried first and reverted: Radix's
 * Select depends on the same popper/portal/dismissable-layer/focus-scope/
 * scroll-lock stack Tooltip was carrying when it was removed above for ~14 kB
 * — except Select's is bigger, since it also needs a scrollable, virtualized
 * listbox. Measured, it alone cost over 20 kB gzipped, which would have made
 * this "delete most of the panel" change a net *increase* to the budget. A
 * `<select>` with an `<optgroup>` for "intentionally broken" already does
 * everything the approved mockup's dropdown needs — grouped options, keyboard
 * operation, screen-reader semantics — for free, so that is what shipped.
 *
 * ## And then back down a hair, in review
 *
 * The collapsed disclosure still read like documentation once someone
 * actually clicked it open: three paragraphs restating, in prose, a
 * distinction the panel already states once, inline, exactly when it
 * applies (the "agreement, not conflict" line beside the result cards). Cut
 * to one paragraph, plus a "Decode on jwt.io ↗" link beside the raw header
 * for whoever wants the familiar debugger view — it decodes client-side off
 * a URL fragment, which browsers never put on the wire, so nothing is
 * handed to a third party. Net effect on the number below is noise: 140.6 kB.
 *
 * ## And then 141 -> 142, for a fourth "Try:" option
 *
 * jwt.io was asked to do double duty — decode a token *and* verify it — and
 * cannot: its own documented algorithm list for signature verification is
 * HS384/512, RS384/512, PS256/384 and ES256/384, no EdDSA, so no public key
 * pasted into it will ever check a token this scenario issues. Verifying an
 * arbitrary token — one built by hand, or one of ours edited — was only ever
 * going to happen here, because `lib/jwt.ts` is the only thing on either side
 * of this feature that speaks Ed25519 at all.
 *
 * "Your own token" is a fourth `<option>` in the same `<select>`, wired
 * through the same mint-call-fetch-verify chain `runIt` already ran for the
 * other three — the one new dependency-free thing is a fallback for
 * `jwks_url` (`/.well-known/jwks.json` off this API's own base URL) for the
 * case a pasted token is the very first thing a visitor tries, before any
 * mint has ever supplied one. Measured at 140.9 kB against 140.6 kB before it
 * — three tenths of a kilobyte for a fourth option and one constant. The
 * budget below moves to 142 not because this needed it, but because 141
 * left it none: a change that costs nothing should not be the one blamed for
 * tripping the budget on ordinary output drift.
 *
 * ## And then 142 -> 138, for deleting most of the panel a second time
 *
 * Every earlier entry above the last two was in the spirit of "keep the
 * capability, shrink the chrome around it." This one drops a capability: the
 * six named forgeries, the "Try:" dropdown, the call to this deployment's own
 * protected route, and the two-card "Our API vs your browser" comparison are
 * all gone, not collapsed. What is left answers one question — does this
 * token verify against this key set — with two always-visible fields and one
 * button, because that turned out to be the actual ask underneath four
 * rounds of "still too many controls."
 *
 * The forgeries are not lost, only no longer curated: typing a wrong `kid`,
 * pointing the JWKS field at the wrong issuer, or editing a byte of the
 * signature reaches every one of them by hand instead of by preset — which
 * is a fair trade for a visitor who came here to check a real token, and a
 * worse one for a visitor who has never seen a broken JWT and would not know
 * what to type. That trade was made deliberately, not as a side effect of
 * cutting bytes.
 *
 * Measured at 137.5 kB — 3.4 kB less than the entry above it, most of it the
 * disclosure, the checklist widget and the second result card leaving
 * entirely rather than shrinking. The budget follows the actual number down
 * rather than keeping the old headroom, for the same reason it has followed
 * every number up: slack nobody is using is slack nobody notices disappear.
 *
 * ## And then 138 -> 139, for bringing the forgeries back as a convenience
 *
 * The entry above called dropping the six presets a deliberate trade,
 * "worse... for a visitor who has never seen a broken JWT and would not know
 * what to type." That half of the trade did not hold up — a "Try one of
 * ours:" select is back, offering a valid token and the six named forgeries
 * by name again. It is not the old dropdown: nothing else in the panel is
 * gated behind it, it only ever fills the two fields that were already there,
 * and picking one resets the JWKS field to this deployment's own on purpose
 * — a forgery is signed by our key under our `kid`, so checking it against
 * whatever a visitor had typed into that field would show `unknown_kid` for
 * nearly all six regardless of which was picked, burying the specific thing
 * each is named for under a more basic "wrong key set" answer.
 *
 * Measured at 138.0 kB against 137.5 kB before it — half a kilobyte for the
 * select, the six-entry array behind it, and the mint-then-verify wiring,
 * all of which already existed in some form two entries back and is being
 * reused, not rebuilt.
 */
const BUDGETS_KB = {
  "/page": 139,
  "/_not-found/page": 92,
  "/how-it-works/page": 93,
};

const report = process.argv.includes("--report");

let manifest;
try {
  manifest = JSON.parse(readFileSync(join(NEXT, "app-build-manifest.json"), "utf8"));
} catch {
  console.error(
    "no build found. Run `pnpm --filter ./apps/web run build` first — this\n" +
      "measures the built output, not the source.",
  );
  process.exit(1);
}

const failures = [];
const rows = [];

for (const [route, files] of Object.entries(manifest.pages)) {
  const budget = BUDGETS_KB[route];
  // Layout chunks are shared rather than a route a visitor loads.
  if (budget === undefined) continue;

  let raw = 0;
  let gzipped = 0;
  for (const file of files) {
    const path = join(NEXT, file);
    try {
      const bytes = readFileSync(path);
      raw += statSync(path).size;
      gzipped += gzipSync(bytes).length;
    } catch {
      console.error(`missing chunk ${file} — is the build complete?`);
      process.exit(1);
    }
  }

  const kb = gzipped / 1024;
  rows.push({ route, kb, budget, raw: raw / 1024 });
  if (kb > budget) failures.push({ route, kb, budget });
}

if (rows.length === 0) {
  console.error("no budgeted routes found in the manifest — has routing changed?");
  process.exit(1);
}

for (const r of rows) {
  const pct = ((r.kb / r.budget) * 100).toFixed(0);
  console.log(
    `  ${r.route.padEnd(20)} ${r.kb.toFixed(1).padStart(6)} kB gz` +
      `  (${pct}% of ${r.budget} kB budget, ${r.raw.toFixed(0)} kB raw)`,
  );
}

if (report) process.exit(0);

if (failures.length > 0) {
  console.error("\nover budget:\n");
  for (const f of failures) {
    console.error(
      `  ${f.route} is ${f.kb.toFixed(1)} kB gzipped, over its ${f.budget} kB budget.`,
    );
  }
  console.error(
    "\nEither trim the route, or raise the budget in scripts/bundle-budget.mjs\n" +
      "with a note saying what it bought. Raising it silently is how a budget\n" +
      "stops meaning anything.",
  );
  process.exit(1);
}

console.log("\nevery route within budget");
