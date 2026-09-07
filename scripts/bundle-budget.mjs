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
 * baseline is visible rather than absorbed.
 */
const BUDGETS_KB = {
  "/page": 115,
  "/_not-found/page": 92,
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
