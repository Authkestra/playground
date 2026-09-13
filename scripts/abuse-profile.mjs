#!/usr/bin/env node
// Load and abuse test harness for the playground API (#35).
//
// This drives real traffic at a running deployment and reports whether the
// cost guardrails held. It exists because "the rate limiter is configured"
// and "the rate limiter works under concurrent load against a cold-started
// free-tier instance" are different claims, and only the second one is worth
// anything the day a scraper or a script kiddie finds the playground.
//
// ## The two buckets this is built around
//
// `apps/api/src/lib.rs` wires two `tower_governor` buckets:
//
//   - standard:  burst 30, replenishing 1 token / 2s  (~0.5 req/s steady)
//   - sensitive: burst 10, replenishing 1 token / 5s  (~12 req/min steady)
//
// The sensitive bucket covers `POST /api/scenarios/:id/action/:action`,
// `GET /api/starter-kit`, the OAuth navigation routes and the GitHub push
// routes — everything that creates credentials or calls a third party. That
// is deliberate (see lib.rs's comment on `sensitive`): those are the requests
// that cost real money or burn someone else's rate limit, so they get the
// tight bucket while a visitor clicking through every scenario in the UI
// never notices it.
//
// Bucketing is by client IP (`ClientIpKeyExtractor`), not by session or
// visitor. Every request this script sends comes from one machine, so an
// "abuse" run here is exactly the case the sensitive bucket exists for: one
// IP going far past its burst. It cannot exercise fairness *across* IPs —
// that would need real distribution, which is out of scope for a script run
// from a laptop.
//
// ## Two profiles
//
// `interactive` — several concurrent, human-paced visitors, each making one
// pass through the site: list scenarios, fetch/create a session, toggle a
// control, read the diff the API returns with it, try one ceremony step, and
// — for one in five — download the starter kit. This should never see a 429
// in a healthy deployment: the standard and sensitive bursts are sized
// precisely so a visitor clicking through the site isn't throttled (see
// lib.rs: "Sized so a visitor clicking through every scenario is never
// throttled"), and each visitor here is capped to at most 3 standard-bucket
// and 2 sensitive-bucket requests — comfortably under both bursts even
// though every visitor this script runs shares one real IP (this machine's),
// and so one rate-limit bucket. If it 429s anyway, the burst is mis-sized,
// not the visitor.
//
// `abuse` — scripted concurrent hammering, concentrated on the sensitive
// bucket, at a rate deliberately well past its documented burst of 10. The
// point is to *cause* 429s: a run that never sees one means the limiter
// isn't holding, which is worse than a slow response.
//
// ## Safety rails (non-negotiable, do not relax these)
//
// 1. `--target` is required. There is no default, because a load generator
//    with a default target is one `node scripts/abuse-profile.mjs` away from
//    hitting production by accident.
//
// 2. The abuse profile refuses to run against anything that looks like this
//    project's production surface (`play.authkestra.com`, or any
//    `*.onrender.com` host) unless `--yes-really` is also passed. A
//    concentrated abuse run against the real deployment is not a test, it's
//    an incident: real visitors get throttled, and if it lands on an
//    endpoint that reaches a provider, it burns that provider's quota too.
//
// 3. Every run is bounded — a duration ceiling, a request-count ceiling, and
//    a hard ceiling neither flag can raise — so a mistyped `--duration 999999`
//    cannot run all night against someone's dashboard.
//
// 4. Neither profile ever drives an OAuth or captcha-verification endpoint.
//    `POST /api/scenarios/oauth/action/*`, `POST /api/scenarios/captcha/action/*`,
//    `GET /auth/login/:provider`, `GET /auth/callback/:provider` and every
//    `/api/github/*` route are excluded outright, in both profiles, even
//    though `oauth` and `captcha` share the same sensitive bucket this script
//    is trying to trip. Those endpoints call a real third party — GitHub,
//    Google, Turnstile/hCaptcha/reCAPTCHA — and every request this script
//    sends would come out of *their* rate limit, not just ours. Enough of
//    that from an automated abuse run is how a provider suspends the
//    deployment's OAuth app. Configuring those scenarios (`POST
//    /api/scenarios/:id/configure`) is fine and used freely — that endpoint
//    only writes session state, it never reaches a provider — only the
//    *action* step is excluded. In their place this hammers the ceremony
//    steps that are local-only (TOTP provisioning, a WebAuthn challenge, JWT
//    issuance) and the starter-kit download, which is the single most
//    expensive request the service serves and shares the same bucket without
//    touching anyone else's quota.
//
// ## What a passing run does NOT prove
//
// See docs/load-testing.md for the full list. In short: that billing alerts
// fire, that the kill switch actually degrades traffic under concurrent load,
// and third-party quota consumption are none of them things this script can
// verify by itself — they need a maintainer with access to the real provider
// dashboards and the real admin token. This script only verifies the one
// thing observable from the outside: does the HTTP surface throttle or fall
// over.
//
// Usage:
//   node scripts/abuse-profile.mjs --target <url> [--profile interactive|abuse] [--report]
//
// Flags:
//   --target <url>       Required. Base URL of the API to test.
//   --profile <name>     "interactive" (default) or "abuse".
//   --report             Print the same report but always exit 0.
//   --yes-really         Required to run --profile abuse against a host that
//                         looks like production. Says nothing on its own —
//                         still has to be paired with --profile abuse.
//   --visitors <n>        Interactive: concurrent visitors, each making one
//                         pass through the site (default 5). Not a loop —
//                         see visitorPass's doc comment for why.
//   --workers <n>         Abuse: concurrent hammering workers (default 20).
//   --duration <secs>     Abuse only: soft ceiling on wall-clock run time
//                         (default 8s); the hard request ceiling can still
//                         end the run first. Ignored by the interactive
//                         profile, which has no loop to bound.
//   --max-requests <n>    Ceiling on total requests sent, clamped to
//                         HARD_REQUEST_CEILING regardless of what is asked for.

const SAFE_ACTIONS = {
  totp: "provision",
  passkeys: "register_start",
  resource: "issue",
};

const NEVER_TOUCH_SCENARIOS = new Set(["oauth", "captcha"]);

const DEFAULTS = {
  interactive: { visitors: 5, duration: 15, maxRequests: 300 },
  abuse: { workers: 20, duration: 8, maxRequests: 400 },
};

// No flag combination can push a run past this many requests. Chosen well
// above any sane default so a deliberately larger `--max-requests` still
// works, and well below "all night" at any concurrency this script uses.
const HARD_REQUEST_CEILING = 5000;

// Hard wall-clock ceiling, for the same reason.
const HARD_DURATION_SECS = 120;

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function looksLikeProduction(hostname) {
  const h = hostname.toLowerCase();
  return h === "play.authkestra.com" || h === "onrender.com" || h.endsWith(".onrender.com");
}

function parseArgs(argv) {
  const out = { profile: "interactive", report: false, yesReally: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case "--target":
        out.target = argv[++i];
        break;
      case "--profile":
        out.profile = argv[++i];
        break;
      case "--report":
        out.report = true;
        break;
      case "--yes-really":
        out.yesReally = true;
        break;
      case "--visitors":
        out.visitors = Number(argv[++i]);
        break;
      case "--workers":
        out.workers = Number(argv[++i]);
        break;
      case "--duration":
        out.duration = Number(argv[++i]);
        break;
      case "--max-requests":
        out.maxRequests = Number(argv[++i]);
        break;
      case "--help":
      case "-h":
        out.help = true;
        break;
      default:
        console.error(`unrecognised argument: ${a}`);
        process.exit(1);
    }
  }
  return out;
}

// ------------------------------------------------------------- HTTP helpers

/** Tiny per-visitor cookie jar. Node's fetch does not keep one for us. */
class CookieJar {
  constructor() {
    this.jar = new Map();
  }
  header() {
    if (this.jar.size === 0) return undefined;
    return [...this.jar.entries()].map(([k, v]) => `${k}=${v}`).join("; ");
  }
  capture(res) {
    const setCookies =
      typeof res.headers.getSetCookie === "function"
        ? res.headers.getSetCookie()
        : res.headers.get("set-cookie")
          ? [res.headers.get("set-cookie")]
          : [];
    for (const sc of setCookies) {
      const pair = sc.split(";", 1)[0];
      const idx = pair.indexOf("=");
      if (idx > 0) this.jar.set(pair.slice(0, idx).trim(), pair.slice(idx + 1).trim());
    }
  }
}

/**
 * Fire one request and time it. Never throws — a connection failure comes
 * back as status 0 with `error` set, so a bad `--target` shows up in the
 * report as a wall of status-0s rather than an unhandled rejection.
 */
async function request(baseUrl, { method = "GET", path, body, jar, parseJson = false }) {
  const start = performance.now();
  try {
    const headers = {};
    if (body !== undefined) headers["content-type"] = "application/json";
    if (jar) {
      const c = jar.header();
      if (c) headers.cookie = c;
    }
    const res = await fetch(new URL(path, baseUrl), {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (jar) jar.capture(res);
    let json;
    if (parseJson) {
      json = await res.json().catch(() => undefined);
    } else {
      await res.arrayBuffer().catch(() => {});
    }
    return { status: res.status, ms: performance.now() - start, json };
  } catch (err) {
    return { status: 0, ms: performance.now() - start, error: String(err?.message ?? err) };
  }
}

function humanPause(minMs = 150, maxMs = 450) {
  const ms = minMs + Math.random() * (maxMs - minMs);
  return new Promise((r) => setTimeout(r, ms));
}

/** A shared, monotonically-decreasing request budget across every worker/visitor. */
class Budget {
  constructor(max) {
    this.max = max;
    this.used = 0;
  }
  take() {
    if (this.used >= this.max) return false;
    this.used++;
    return true;
  }
}

function sampleControlValue(spec) {
  switch (spec.control?.kind) {
    case "select_one": {
      const opt = spec.control.options?.[0];
      return { kind: "select_one", selected: opt ? opt.id : null };
    }
    case "select_many": {
      const opt = spec.control.options?.[0];
      return { kind: "select_many", selected: opt ? [opt.id] : [] };
    }
    case "toggle":
    default:
      return { kind: "toggle", enabled: true };
  }
}

/**
 * Every scenario this deployment offers whose one ceremony step this script
 * is willing to drive is both present and local-only. Used by both profiles
 * to find endpoints in the sensitive bucket that never reach a third party.
 */
async function discoverSafeScenarios(baseUrl) {
  const res = await request(baseUrl, { path: "/api/scenarios", parseJson: true });
  if (res.status !== 200 || !Array.isArray(res.json)) {
    throw new Error(`unexpected response from GET /api/scenarios (status ${res.status})`);
  }
  return res.json.filter((s) => {
    if (NEVER_TOUCH_SCENARIOS.has(s.id)) return false;
    const action = SAFE_ACTIONS[s.id];
    return action && Array.isArray(s.actions) && s.actions.includes(action);
  });
}

// ------------------------------------------------------------- interactive

/**
 * One visitor's single pass through the site: list scenarios, get/create a
 * session, toggle a control, read the diff that comes back with it, try one
 * ceremony step, and — for a fixed one-in-five of visitors, not a per-visitor
 * coin flip — download the starter kit.
 *
 * Deliberately one pass, not a loop until some duration elapses. Every
 * visitor this script runs shares one client IP (this machine's), so it
 * shares one rate-limit bucket too — see `ClientIpKeyExtractor` in the header
 * comment. A loop that keeps re-visiting would eventually exhaust that shared
 * bucket no matter how few visitors there are, which would just be measuring
 * this script's own concurrency rather than answering "does a real visitor's
 * session get throttled". One pass per visitor keeps the *worst case* load
 * this profile can generate bounded by construction: at most 3 standard-
 * bucket requests and at most 2 sensitive-bucket requests per visitor,
 * against bursts of 30 and 10 respectively — so `--visitors` up to a dozen or
 * so comfortably fits under both bursts with no dependence on timing luck.
 */
async function visitorPass(baseUrl, records, budget, downloadsStarterKit) {
  const jar = new CookieJar();
  const push = (rec) => {
    if (rec) records.push(rec);
  };
  const spend = async (opts) => {
    if (!budget.take()) return null;
    const rec = await request(baseUrl, { ...opts, jar });
    push(rec);
    return rec;
  };

  const list = await spend({ path: "/api/scenarios", parseJson: true });
  if (!list) return;
  await humanPause();

  if (!(await spend({ path: "/api/session" }))) return;
  await humanPause();

  const specs = Array.isArray(list.json) ? list.json : [];

  // What gets configured (and its diff read back) can be anything on offer —
  // configuring never reaches a third party, only the ceremony step does.
  const configureTarget = specs.find((s) => s.available) ?? specs[0];
  if (configureTarget) {
    const value = sampleControlValue(configureTarget);
    if (
      !(await spend({
        method: "POST",
        path: `/api/scenarios/${configureTarget.id}/configure`,
        body: { value },
      }))
    )
      return;
    await humanPause();
  }

  // The ceremony step is chosen independently, and only from the scenarios
  // this script is safe to drive — never OAuth or captcha, see the header.
  const actionTarget = specs.find(
    (s) =>
      s.available &&
      !NEVER_TOUCH_SCENARIOS.has(s.id) &&
      SAFE_ACTIONS[s.id] &&
      s.actions?.includes(SAFE_ACTIONS[s.id]),
  );
  if (actionTarget) {
    if (
      !(await spend({
        method: "POST",
        path: `/api/scenarios/${actionTarget.id}/action/${SAFE_ACTIONS[actionTarget.id]}`,
        body: {},
      }))
    )
      return;
    await humanPause();
  }

  if (downloadsStarterKit) {
    await spend({ path: "/api/starter-kit" });
  }
}

async function runInteractive(baseUrl, args) {
  const visitors = clamp(args.visitors ?? DEFAULTS.interactive.visitors, 1, 50);
  const maxRequests = Math.min(
    args.maxRequests ?? DEFAULTS.interactive.maxRequests,
    HARD_REQUEST_CEILING,
  );

  const budget = new Budget(maxRequests);
  const records = [];

  await Promise.all(
    Array.from({ length: visitors }, (_, i) =>
      visitorPass(baseUrl, records, budget, i % 5 === 0),
    ),
  );
  return records;
}

// ------------------------------------------------------------------ abuse

async function runAbuse(baseUrl, args) {
  const workers = clamp(args.workers ?? DEFAULTS.abuse.workers, 1, 200);
  const durationSecs = clamp(args.duration ?? DEFAULTS.abuse.duration, 1, HARD_DURATION_SECS);
  const maxRequests = Math.min(
    args.maxRequests ?? DEFAULTS.abuse.maxRequests,
    HARD_REQUEST_CEILING,
  );

  const stopAt = Date.now() + durationSecs * 1000;
  const budget = new Budget(maxRequests);

  const endpoints = [];
  try {
    const safe = await discoverSafeScenarios(baseUrl);
    for (const s of safe) {
      endpoints.push({
        method: "POST",
        path: `/api/scenarios/${s.id}/action/${SAFE_ACTIONS[s.id]}`,
        body: {},
      });
    }
  } catch (err) {
    console.error(
      `warning: could not discover scenarios (${err.message}); ` +
        `hammering only GET /api/starter-kit`,
    );
  }
  // The most expensive request the service serves, and never third-party.
  // Always included, discovery or not.
  endpoints.push({ method: "GET", path: "/api/starter-kit" });

  const records = [];
  async function worker() {
    while (Date.now() < stopAt && budget.take()) {
      const ep = endpoints[Math.floor(Math.random() * endpoints.length)];
      records.push(await request(baseUrl, ep));
    }
  }
  await Promise.all(Array.from({ length: workers }, worker));
  return records;
}

// --------------------------------------------------------- report + verdict

function percentile(sortedMs, p) {
  if (sortedMs.length === 0) return 0;
  const idx = clamp(Math.ceil((p / 100) * sortedMs.length) - 1, 0, sortedMs.length - 1);
  return sortedMs[idx];
}

function summarize(records) {
  const statusCounts = {};
  for (const r of records) {
    statusCounts[r.status] = (statusCounts[r.status] ?? 0) + 1;
  }
  const sortedMs = records.map((r) => r.ms).sort((a, b) => a - b);
  const count5xx = records.filter((r) => r.status >= 500 && r.status < 600).length;
  const count429 = statusCounts[429] ?? 0;
  const networkErrors = statusCounts[0] ?? 0;
  return {
    total: records.length,
    statusCounts,
    p50: percentile(sortedMs, 50),
    p95: percentile(sortedMs, 95),
    p99: percentile(sortedMs, 99),
    count5xx,
    count429,
    networkErrors,
  };
}

function guardrailFailed(profile, s) {
  if (s.count5xx > 0) {
    return {
      failed: true,
      reason:
        `${s.count5xx} request(s) returned a 5xx status. The guardrail is that ` +
        `load gets throttled (429), not that the service falls over.`,
    };
  }
  if (s.networkErrors > 0) {
    return {
      failed: true,
      reason: `${s.networkErrors} request(s) never got a response at all — check --target and that the service is up.`,
    };
  }
  if (profile === "abuse" && s.count429 === 0) {
    return {
      failed: true,
      reason:
        "the abuse run produced zero 429s. The sensitive bucket's burst is 10 " +
        "requests; this run sent far more than that from one IP and none were " +
        "throttled, which means the rate limit did not hold.",
    };
  }
  return { failed: false };
}

function printReport({ profile, target, wallMs, summary }) {
  const rps = summary.total / Math.max(wallMs / 1000, 0.001);
  console.log(`profile: ${profile}`);
  console.log(`target: ${target}`);
  console.log(`requests sent: ${summary.total}`);
  console.log(`wall time: ${(wallMs / 1000).toFixed(2)}s (achieved ~${rps.toFixed(1)} req/s)`);
  console.log("status codes:");
  for (const code of Object.keys(summary.statusCounts).sort()) {
    const label = code === "0" ? "0 (no response / network error)" : code;
    console.log(`  ${label}: ${summary.statusCounts[code]}`);
  }
  console.log(
    `latency: p50 ${summary.p50.toFixed(0)}ms  p95 ${summary.p95.toFixed(0)}ms  p99 ${summary.p99.toFixed(0)}ms`,
  );
  console.log(`429s: ${summary.count429}   5xx: ${summary.count5xx}`);
}

// ---------------------------------------------------------------------- main

async function main() {
  const args = parseArgs(process.argv.slice(2));

  if (args.help) {
    console.log(
      "node scripts/abuse-profile.mjs --target <url> [--profile interactive|abuse] [--report]",
    );
    process.exit(0);
  }

  if (!args.target) {
    console.error(
      "usage: node scripts/abuse-profile.mjs --target <url> [--profile interactive|abuse] [--report]\n\n" +
        "--target is required; there is no default.",
    );
    process.exit(1);
  }

  let url;
  try {
    url = new URL(args.target);
  } catch {
    console.error(`--target is not a valid URL: ${args.target}`);
    process.exit(1);
  }

  if (args.profile !== "interactive" && args.profile !== "abuse") {
    console.error(`--profile must be "interactive" or "abuse", got "${args.profile}"`);
    process.exit(1);
  }

  if (args.profile === "abuse" && looksLikeProduction(url.hostname) && !args.yesReally) {
    console.error(
      `refusing: "${url.hostname}" looks like a production host (play.authkestra.com, or\n` +
        `anything on onrender.com).\n\n` +
        `An abuse run concentrates requests well past the documented burst on the\n` +
        `endpoints that create credentials and call third-party providers. Pointed\n` +
        `at the real deployment that is not a test, it is an incident: real visitors\n` +
        `get throttled behind it, and if it lands on a provider-facing endpoint it\n` +
        `burns that provider's quota too.\n\n` +
        `Pass --yes-really if this is genuinely intended.`,
    );
    process.exit(1);
  }

  const wallStart = Date.now();
  let records;
  try {
    records = args.profile === "abuse" ? await runAbuse(url, args) : await runInteractive(url, args);
  } catch (err) {
    console.error(`run failed: ${err.message}`);
    process.exit(1);
  }
  const wallMs = Date.now() - wallStart;

  const summary = summarize(records);
  printReport({ profile: args.profile, target: url.toString(), wallMs, summary });

  if (args.report) {
    process.exit(0);
  }

  const verdict = guardrailFailed(args.profile, summary);
  if (verdict.failed) {
    console.error(`\nFAILED: ${verdict.reason}`);
    process.exit(1);
  }
  console.log("\nguardrail held.");
  process.exit(0);
}

main();
