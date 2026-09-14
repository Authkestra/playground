# 0009 — Usage metrics: counters we hold, on a page that ships no analytics

**Status:** accepted
**Date:** 2026-09-14
**Roadmap:** P5, #36 "Instrument conversion and usage"

## Context

#36 opens with the reason it exists: "without this there's no way to know
whether the playground actually drives adoption, which is the entire point."
That is true, and it sits directly against its own second task — "privacy-
respecting analytics (no third-party surveillance on an auth demo — it
undercuts the message)". Both have to be satisfied by the same design or one of
them is decoration.

The tension is smaller than it looks, because of what is actually being asked.
Every question #36 poses — which scenarios get toggled, which flows complete,
where visitors drop, what gets downloaded in which configuration — is a
question about **totals**, not about people. None of them needs to know that
the visitor who enabled passkeys is the same one who downloaded twenty minutes
later; they need to know how many did each. A design that answers the questions
asked, and is incapable of answering "what did this person do", is not a
compromise between the two tasks. It is what both of them describe.

## The metric that means success

**Starter-kit downloads per week, and the session-to-download rate.**

#36 lists four candidates — repo stars, starter-kit downloads, docs
click-through, crates.io pulls — and only one of them is a decision a visitor
makes.

- **Stars** measure that somebody approved of a thing for four seconds. They
  cost nothing, they never decay, and they are the metric most improved by
  being posted about rather than by being good.
- **Docs click-through** measures curiosity, and it is upstream of every
  outcome including "read two paragraphs and left".
- **crates.io pulls** are the closest thing to real adoption, and they are
  unattributable: a pull tells you nothing about whether this playground caused
  it, and CI pulls dwarf human ones.
- **A download** is the only action here that costs the visitor something —
  they configured a project, waited for an archive, and now have a directory on
  their machine they will either build or delete. It is the first point at
  which somebody has decided to *try* rather than to *look*.

The **rate** matters more than the count, and that is why sessions are counted
at all. A download total goes up when a post does well. `downloaded / sessions`
goes up only when the playground gets better at convincing the people who
already arrived, which is the thing this repository can actually change.

The secondary metric is the download **breakdown by configuration**, which #36
correctly identifies as doubling as product research: which combinations people
actually want is the question the wizard idea in P6 is built on, and guessing at
it is how that phase goes wrong.

## Decision

Server-side, first-party, aggregate-only counters in the store the service
already runs on. No third-party analytics, no client-side script, no added
JavaScript at all. The implementation is `apps/api/src/metrics.rs`.

Everything #36 asks for is inferable from requests the API already serves, so
the frontend needed no change whatsoever to produce any of it. That is worth
stating plainly because it is the whole argument: this playground's claim is
that authentication is something you should own and be able to see working, and
the network tab of a page making that claim should not contain somebody else's
tracker. Here it contains nothing extra at all.

### What is counted

Per ISO week, each under a TTL so the bucket ages out on its own:

- sessions handed out — the denominator
- per scenario: how often it was switched **on**, ceremony actions
  **attempted**, and ceremony actions **completed**
- the funnel: sessions that configured something, that attempted a ceremony,
  that downloaded
- downloads, keyed by a canonical name for the configuration that produced
  them (sorted and joined, so one choice is one row however it was reached)

Attempts and completions are counted separately and deliberately. A scenario
people reach for and cannot finish is the most useful thing this view can show,
and it is invisible in a design that only records success.

### What is not collected

No identifier of any kind, no IP address, no user agent, no referrer, no
timestamps finer than the week, no per-visitor record, and no sequence of one
visitor's actions. There is no cookie beyond the session cookie the playground
already needed, and nothing is written that outlives the session except a
number that has been added to.

The one place this was genuinely hard is the funnel. "Where visitors drop"
needs each session counted once per stage rather than once per request — a
visitor who toggles eight scenarios must not read as eight visitors — and the
obvious implementation of that is a per-visitor record, which is the thing this
decision exists to avoid. What is used instead is a write-once marker keyed by
the session id, carrying the session's own TTL: an atomic `set_if_absent` says
whether this session has been here before, the counter moves only when the
answer is no, and the marker expires with the session that owns it. The marker
holds the literal value `1`. It records that a stage was reached, not what was
done, and it is gone within hours.

When the store cannot be reached, that check answers "no", so an outage
under-counts rather than double-counts. Of the two ways to be wrong, a funnel
that quietly inflates is the one that would be believed.

### Retention

Twelve weeks — a quarter — after the last write to a bucket. Long enough to see
whether a change moved anything, short enough that the store is never holding a
year of data nobody has looked at. There is no argument for keeping it longer,
because aggregate data cannot be asked a new question later: it is not a
dataset that becomes more valuable with age, it is a number.

### Reading it

`GET /admin/metrics`, behind the same `ADMIN_TOKEN` gate as the kill switch —
which means that when no token is configured the route is not mounted at all,
rather than being mounted and open. It takes an optional `weeks` parameter,
clamped to the retention window, and returns the weekly view newest first.

The gate is not modesty about the numbers. A public endpoint reporting what
people download in which configuration is a free competitive-intelligence feed,
and unlike the rest of the playground this is our operational data rather than
the visitor's.

### Failure

Counting never breaks a request. Every write is infallible from the caller's
point of view and a store error is logged and dropped, for the same reason the
flow log works that way: a visitor's sign-in must not fail because our
bookkeeping could not be written. There is a test that asserts this against a
store where every operation fails.

## Consequences

- The acceptance criterion is met: a weekly view of what visitors try, what
  they complete, and what they download, readable in one request.
- **This cannot answer retention or cohort questions**, and never will without
  a different decision. "Do people who download come back" is unanswerable
  here, by construction. That is accepted: the question this playground has to
  answer first is whether visitors convert at all, and a design that could
  answer the cohort question would be the design this record rejects.
- It cannot attribute anything to a source either — no referrer is kept — so
  "did the HN post work" has to be answered from the traffic shape in the
  week's session count, not from the data.
- There is no dashboard. The weekly view is JSON behind an admin token, which
  is the right amount of surface for a number one person reads once a week. If
  it is ever read more often than that, a dashboard is a small thing to add on
  top of an endpoint that already exists.
- Three primitives were added to `KeyValue` (`increment`, `set_if_absent`,
  `entries_with_prefix`), each atomic for the same reason `take` is: the
  read-modify-write version of any of them loses exactly the concurrent traffic
  that is worth counting.
- If a third-party analytics product is ever adopted anyway, this record should
  be superseded explicitly rather than quietly kept alongside it. The argument
  above is not about tooling preference; it is that the page makes a claim, and
  the page should not contradict it.
