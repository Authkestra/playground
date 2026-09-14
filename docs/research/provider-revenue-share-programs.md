# Provider revenue-share and kickback programmes

**Status:** research complete (tracked as GitHub issue #53)
**Date:** 2026-09-14
**Roadmap:** P6 — post-launch / wizard path

Every percentage, threshold and application process below was checked on
2026-09-14 against the provider's own docs, legal terms or blog wherever one
exists, with the exact URL cited next to the claim. Treat all of it as
perishable: Railway itself changed its cash-out rail between the two Railway
posts cited below (BuyMeACoffee/GitHub Sponsors, then Stripe Connect), and
partner-programme terms are the kind of page a provider edits without a
changelog. Re-check the source link before relying on a number here for an
actual integration decision.

## Correcting the issue's premise: Fly.io is not the P0 host

Issue #53 says to check Fly.io first "since it's now the actual P0 host per
this roadmap." That is stale. `docs/decisions/0003-hosting-platform.md`
records the *decision* to move off Shuttle onto Fly.io, dated 2026-09-04, but
the README's current **Status** and **Deployment** sections — which describe
what is actually live, not what was decided three weeks ago — say plainly:

> Backend on Render (`authkestra-playground-api` service), frontend on
> Vercel.

and, further down:

> Backend to **Render** (from `render.yaml`), frontend to **Vercel**
> (`.github/workflows/deploy-web.yml`), both on merge to `main`.

Fly.io is now one of two **dormant** targets, kept only as a
`workflow_dispatch`-only GitHub Actions workflow: "Its trial ended, and a
push-triggered deploy that always fails leaves `main` red for reasons
unrelated to the commit, so the trigger was removed rather than the
workflow." Cloud Run is the other dormant target, also `workflow_dispatch`
only. Neither is what a visitor's request actually hits today.

This document therefore checks Fly.io as one entry among many rather than as
a privileged "same-provider" candidate, and the recommendation at the end is
built around the real hosting picture — Render for the API, Vercel for the
frontend — not the one in the issue text.

## The two mechanisms, restated

- **Mechanism 1 (kickback):** the provider pays the template/integration
  creator a cut of what the provider itself bills the end user for hosting.
  No pricing, no billing relationship, no customer data ever touches the
  creator. This is the only mechanism that fits authkestra's "no data
  custody" constraint for free.
- **Mechanism 2 (marketplace fee):** the creator lists and sells something —
  a SaaS product, an AMI, a paid add-on — sets their own price, and the
  platform takes a percentage of the creator's own revenue. This requires
  the creator to already be a merchant of record: a priced product, usually
  a registered business entity, tax forms and a payout/bank account. It is
  the AWS/Azure/GCP Marketplace shape, and per the issue's own scope note,
  it is "only relevant once there's something to actually sell" — the
  managed-hosting side, not the free playground or starter-kit generator.

## Comparison table

| Provider | Programme exists | Mechanism | Rate / terms | Fit for authkestra |
| --- | --- | --- | --- | --- |
| Railway | Yes | 1 (+ Tech Partner extension) | 15% of hosting spend, 25% with active Template Queue support; cash via Stripe Connect or credits | Good — real compute host, no data custody, self-serve |
| Render | No | — | Only an unpublished one-off customer-referral affiliate deal, not tied to templates | N/A — already our API host, but pays nothing for a template |
| Fly.io | No | — | Staff confirmed "We do not" have an affiliate/referral programme | N/A — dormant host anyway |
| DigitalOcean | Yes, Add-Ons only | 2 | 75% of net collected Add-On revenue; free 1-Click Apps pay nothing | Gated behind a paid SaaS Add-On we don't have |
| Vercel | Yes, paid integrations only | 2 | Undisclosed %; free Template Gallery pays nothing | Wrong layer anyway — only fronts `apps/web` |
| Cloudflare | No | — | PowerUP Partner Network is a reseller/MSP programme, no percentages | N/A — Workers is WASM/edge, ruled out regardless |
| Netlify | Yes, but wrong shape | Customer-referral commission, not template kickback | 20% of referred customer's spend for 12–24 months | Wrong mechanism; also no long-running-process hosting |
| Coolify | No | — | Sustained by donations and paid Coolify Cloud only | N/A — self-hosted software, not itself a biller |
| Northflank | No | — | Template sharing is a link, no monetary layer | Good compute fit, nothing to collect |
| Zeabur | Yes, capped and credit-only | 1 (bounty-shaped) + referral | $10–$300 in credits per usage milestone; referral commission on one-time triggers only | Credits only, not cash; programme only 3 months old |
| Koyeb | No usable programme | — | Only a B2B "reseller margin," no public percentage | Opaque, contact-only |
| Porter (porter.run) | No | — | No partner/affiliate page exists at all | Good K8s compute fit, nothing to collect |
| Qovery | Yes, terms undisclosed | Referral/reseller commission | "Uncapped commission," no published percentage | Opaque, requires direct negotiation |
| Heroku | Yes, Add-ons only | 2 | 70% of net add-on subscription revenue; deploy Button pays nothing | Gated behind building and shipping a real SaaS add-on |
| AWS Marketplace | Yes | 2 | 3% SaaS / 20% AMI public-offer seller fee | Gated behind a legal entity, tax forms, priced product |
| Azure Marketplace | Yes | 2 | 3% transact fee (1.5% on renewals) | Same gating as AWS |
| Google Cloud Marketplace | Yes | 2 | 3% public offer, tiered down to 1.5% at scale | Same gating as AWS |
| Supabase | No confirmed creator payout | — | Partner terms exist only in private per-partner addenda | Data service only — can't host the Rust binary anyway |
| Neon | No confirmed creator payout | — | B2B reseller discount tier; OSS programme gives credits, not % | Data service only |
| Upstash | No programme found | — | No partner/affiliate page exists | Data service only |
| Replit | No | — | Bounties (gig-payment, mechanism 2-ish) appears discontinued since ~Sept 2025 | Compute fit for a Rust binary itself unverified |
| NodeOps CreateOS | Yes | 1 (+ Builder Partner tiers) | Creators keep 70% of usage fees; partner tiers add 10–20% platform rev-share | Supports Rust; young crypto-VC-backed platform, real platform risk |
| Elestio | Yes | 1 | 10% of all revenue to upstream OSS projects in its catalog | Curated catalog only — no self-serve listing path found |
| PikaPods | Yes | 1 | 20% revenue share "where possible," negotiated per project | Same curated-catalog limitation as Elestio |
| opensource.hosting | Yes | 1 | 10% of invoiced hosting fee, paid monthly via the project's own donation channel | Same curated-catalog limitation; also a very small, single-operator business |

## Detail: mechanism 1 — real kickbacks of hosting spend

### Railway

Confirmed, and the strongest programme found anywhere in this research.
Template creators earn 15% of the hosting costs a deployer incurs, rising to
25% when the creator actively answers questions in the Template Queue inside
Central Station. A separate **Technology Partner** tier pays out on *any*
community template using a qualifying open-source or open-core technology —
not only templates the partner wrote — stacked on top of, not instead of,
the original template creator's cut. Both are documented at
[docs.railway.com/templates/kickbacks](https://docs.railway.com/templates/kickbacks)
and
[docs.railway.com/templates/partners](https://docs.railway.com/templates/partners),
with the programme's history covered in
[Railway's "GIT PUSH && GET PAID" post](https://blog.railway.com/p/template-kickback-program-cash)
and the ["~$1M Paid to Template Developers" post](https://blog.railway.com/p/1M-paid-to-developers-who-built-railway-templates).

Payout defaults to Railway credits, or cash out via Stripe Connect in
$100–$10,000 increments (minimum processed kickback is $0.01), usually
landing within 10 business days, in 130+ countries (Brazil, China and Russia
excluded). Application is simply publishing a template to the public
Marketplace under Railway's Fair Use Policy; the Technology Partner tier is
an application at railway.com/partners, reviewed on an ongoing basis with no
published SLA, followed by a co-marketing page once accepted.

Railway can run a compiled Rust binary as an ordinary long-running container,
so nothing about authkestra's compute model rules it out, and the mechanism
never asks authkestra to store or process a visitor's payment details —
Railway is the merchant of record, we are only a template author. That
combination — real cash, self-serve, no data custody, no compute-model
mismatch — is why it anchors the recommendation below.

### NodeOps CreateOS

A newer entrant with a genuinely comparable model:
[nodeops.network/createos/templates/free](https://nodeops.network/createos/templates/free)
states creators "earn 70% of usage fees every time someone forks and deploys"
a template (full publisher access needs their $149/month Pro plan), and a
separate tiered **Builder Partner Program**
([nodeops.network/createos/partners](https://nodeops.network/createos/partners))
pays 10–20% of platform revenue driven by a partner's referred and activated
users, plus a flat monthly credit stipend, paid monthly by ACH or
international wire. CreateOS explicitly lists Rust as a supported deploy
language alongside custom containers, so the compute-model fit is real, not
theoretical.

The caveat is platform risk rather than mechanism risk: NodeOps is a
crypto/DePIN project (Arbitrum Foundation grant, Borderless Capital and
Wormhole among its seed investors, a live $NODE token), and its "protocol
revenue" framing means the payout pool may be more entangled with token
economics than a plain fiat arrangement. Worth a listing — it costs nothing
to publish a free-tier template — but not something to depend on the way the
Railway relationship could become.

### Zeabur

Has a real, documented programme, but not one that behaves like a kickback
in the way Railway's does.
[zeabur.com/docs/en-US/rewards/contribution](https://zeabur.com/docs/en-US/rewards/contribution)
pays a fixed, one-off credit award per unique-deployer milestone a template
crosses (50 users = $10, up to 10,000 users = $300 cumulative, in Zeabur
credits, not cash), and only for templates created on or after 1 June 2026.
A separate
[referral programme](https://zeabur.com/docs/en-US/rewards/referral) pays a
percentage commission, but only on specific one-time triggers (first server
rental purchase, AI hub top-up) — not an ongoing share of a deployer's
hosting bill. Credits are redeemable only against future Zeabur spend, so
there is no path to extractable cash. Worth doing once a template exists —
the milestone thresholds are low — but it's a bounty, not a revenue stream.

### Elestio, PikaPods, opensource.hosting

These three share the same shape and the same limitation, so they're grouped
here rather than repeated three times. Each pays a flat 10–20% of hosting
revenue to the maintainers of open-source projects it hosts as managed
instances:

- Elestio: "10% of all revenue is distributed to the open-source projects
  whose software we manage" —
  [elest.io/about](https://elest.io/about),
  [blog.elest.io/why-open-source](https://blog.elest.io/why-open-source/).
- PikaPods: "20% revenue share with project authors where possible" —
  [pikapods.com](https://www.pikapods.com/), corroborated by a
  [maintainer discussion thread](https://community.vikunja.io/t/consider-pikapods-revenue-sharing/4629).
- opensource.hosting: "10% of your invoiced hosting fee," distributed
  monthly through whatever donation channel (GitHub Sponsors, Open
  Collective) the project already uses —
  [opensource.hosting](https://opensource.hosting/).

All three run real, long-running managed instances — genuine compute, not
serverless or WASM — so there's no technical objection. The limitation is
structural: each operates a **fixed, curated catalogue** of pre-selected
open-source applications (Nextcloud, Vaultwarden, Gitea, Matomo and similar),
not a self-serve marketplace a new Rust starter-kit template could join by
publishing to it. None publishes a "submit your project" form; getting
listed means emailing the operator and pitching authkestra as a project
worth adding to the catalogue, with no guaranteed acceptance. opensource.hosting
in particular is a very small, apparently single-operator business, which is
worth weighing against the other two on longevity grounds even though its
programme mechanics are the most transparent of the three (a public monthly
ledger of which projects were funded).

## Detail: mechanism 2 — marketplace fees (creator sells, platform takes a cut)

These only matter once authkestra has an actual priced product to sell — a
managed-hosting offering, not the free playground or starter-kit generator —
per the issue's own scope note. Recorded here for completeness and because
the issue's acceptance criteria require every provider to be accounted for.

**DigitalOcean** pays 75% of "Net Collected Add-On Revenues" to the vendor
of a paid SaaS Add-On, by wire or ACH within 60 days of the month the
revenue was collected
([digitalocean.com/legal/marketplace-vendor-terms](https://www.digitalocean.com/legal/marketplace-vendor-terms)).
This is a materially different thing from DigitalOcean's free 1-Click Apps
listings, which the same vendor terms and DigitalOcean's own
[Add-Ons announcement](https://www.digitalocean.com/blog/announcing-add-ons)
confirm pay nothing at all — a free 1-Click listing is pure distribution, no
money changes hands. Applying means submitting through the
[vendor form](https://marketplace.digitalocean.com/vendors) and meeting the
[publishing guidelines](https://marketplace.digitalocean.com/vendors/guidelines-resources);
no turnaround time is published.

**Vercel** runs the same split: its Integrations Marketplace Agreement
states plainly that "Developer Applications distributed for free" carry no
fee in either direction
([vercel.com/legal/integrations-marketplace-agreement](https://vercel.com/legal/integrations-marketplace-agreement)),
so the free Template Gallery — the thing that actually resembles "someone
deploys our template" — pays nothing. A separate paid "native integration
product" track exists for storage/observability-style vendors, with
undisclosed commission terms, gated behind a Pro-plan team and a review by
integrations@vercel.com
([vercel.com/marketplace/program#become-a-provider](https://vercel.com/marketplace/program#become-a-provider)).
Moot for authkestra either way: Vercel only ever fronts the Next.js web app,
never the Rust API, so even a generous undisclosed split wouldn't apply to
the component that actually incurs meaningful compute cost.

**Heroku** pays add-on partners 70% of net subscription revenue through the
Elements marketplace
([heroku.com/policy/heroku-elements-licensing](https://www.heroku.com/policy/heroku-elements-licensing/)),
but reaching GA requires operating a real provisioning API and clearing a
minimum of 100 unique tested installs before launch
([devcenter.heroku.com/articles/becoming-an-add-on-partner](https://devcenter.heroku.com/articles/becoming-an-add-on-partner)) —
categorically a different undertaking from shipping a template. The
"Deploy to Heroku" button itself, still functional per
[devcenter.heroku.com/articles/heroku-button](https://devcenter.heroku.com/articles/heroku-button)
(last updated March 2026, though unsupported on the newer "Fir" generation),
carries no payout mechanism of any kind.

**AWS, Azure and Google Cloud Marketplace** all converge on roughly the same
number: AWS takes 3% of a SaaS public offer (20% for AMI/container offers),
tiering down to 1.5% at high contract value
([docs.aws.amazon.com/marketplace/latest/userguide/listing-fees.html](https://docs.aws.amazon.com/marketplace/latest/userguide/listing-fees.html));
Azure takes 3% of a transact offer, halved to 1.5% on renewals
([learn.microsoft.com — marketplace commercial transaction fee](https://learn.microsoft.com/en-us/partner-center/marketplace-offers/marketplace-commercial-transaction-capabilities-and-considerations));
Google mirrors the same 3%-tiering-to-1.5% shape since an April 2025 policy
change
([cloud.google.com/terms/marketplace-revenue-share-schedule](https://cloud.google.com/terms/marketplace-revenue-share-schedule),
worked examples at
[docs.cloud.google.com/marketplace/docs/partners/revenue-share-scenarios](https://docs.cloud.google.com/marketplace/docs/partners/revenue-share-scenarios)).
All three require a registered legal entity in an eligible jurisdiction, tax
forms, and a compliant payout/bank account before a paid listing is even
possible — real overhead disproportionate to an unincorporated open-source
project with nothing priced to sell. AWS Activate (startup credits) and AWS
ISV Accelerate (a co-sell programme gated behind already having Marketplace
revenue) are sometimes mistaken for kickback programmes; neither pays a
percentage of anything — confirmed via
[aws.amazon.com/activate](https://aws.amazon.com/activate/) and
[aws.amazon.com/partners/programs/isv-accelerate](https://aws.amazon.com/partners/programs/isv-accelerate/).

**Netlify** and **Qovery** both run real partner-commission schemes, but
neither is a template kickback and neither is fully disclosed. Netlify pays
20% of a *referred customer's* Netlify spend for 12–24 months
([netlify.com/partners/program-agreement](https://www.netlify.com/partners/program-agreement/))
— a customer-acquisition commission, not a per-template payout, and Netlify
is a static/Jamstack/functions platform with no native long-running-process
hosting anyway. Qovery advertises "uncapped commissions on every deal" with
no published percentage, reviewed within 48 hours of applying
([qovery.com/partners](https://www.qovery.com/partners)) — a real
application path, but the actual number only appears after direct
negotiation, so it can't be compared against anything else in this table.

## Detail: confirmed absent

**Render** — our own API host — has nothing beyond an unpublished, one-off
customer-referral affiliate arrangement (per a third-party tracker,
[openaffiliate.dev/programs/render](https://openaffiliate.dev/programs/render);
not itself an official Render page). The "Deploy to Render" button
([render.com/docs/deploy-to-render](https://render.com/docs/deploy-to-render))
carries no compensation terms. Searched: Render's docs site search for
"template" and "revenue share," the `render-examples` GitHub org, and a
direct email-contact path via devrel@render.com with no submission-for-payout
process found.

**Fly.io** — a Fly.io staff member answered a direct community question
("Do you have an affiliate programme like DigitalOcean?") with "We do not"
([community.fly.io/t/fly-io-affiliate-program/18859](https://community.fly.io/t/fly-io-affiliate-program/18859)).
Billing docs and the Extensions programme docs
([fly.io/docs/about/billing](https://fly.io/docs/about/billing/),
[fly.io/docs/about/extensions](https://fly.io/docs/about/extensions/))
confirm no referral or creator-payout language anywhere. Moot in any case —
Fly.io is a dormant `workflow_dispatch`-only target, not a live host.

**Cloudflare** — the PowerUP Partner Network
([cloudflare.com/partners/power-up-program](https://www.cloudflare.com/partners/power-up-program/))
is a reseller/MSP/consultant channel with no published percentages and no
template-creator angle; a 2026 Cloudflare community thread explicitly
complains that "Cloudflare has no way for users to earn credit or
recognition for referring others" unlike AWS, DigitalOcean or Vercel
([community.cloudflare.com](https://community.cloudflare.com/t/the-open-source-opportunity-for-cloudflare-part-1-affiliate-referral-program/925541)).
Workers Launchpad is startup funding, not revenue share. None of this
matters for the Rust backend regardless: Workers/Pages is an edge/WASM
runtime, independently disqualified by the long-running-compiled-binary
requirement.

**Coolify** is self-hosted, open-source software — you run it on your own
server — so the entire "provider bills the end user" premise doesn't map
onto it; its own philosophy page frames sustainability purely through
donations and a paid managed-dashboard tier, with no creator payment
anywhere ([coolify.io/philosophy](https://coolify.io/philosophy)).

**Northflank** has a template-sharing *link*, not a monetary layer — its own
docs describe it purely as "anyone with a Northflank account... can add the
template to their own account"
([northflank.com/docs/v1/application/infrastructure-as-code/share-a-template](https://northflank.com/docs/v1/application/infrastructure-as-code/share-a-template)).
No partners, affiliates or marketplace page exists on the site at all —
good container-compute fit, nothing to collect.

**Koyeb**'s only partner-facing page
([koyeb.com/partners](https://www.koyeb.com/partners)) offers resellers "a
competitive margin" with zero published terms and a contact form as the only
next step — not a usable, comparable programme, and not template-specific.

**Porter** (porter.run) has no partner, affiliate or marketplace page
anywhere on its site or in its docs
([porter.run](https://www.porter.run/),
[docs.porter.run/introduction](https://docs.porter.run/introduction)).
Good Kubernetes-based long-running-compute fit; genuinely nothing to apply
to. (Search results for "Porter" are heavily polluted by an unrelated Porter
Metrics affiliate programme and Porter the logistics company — neither is
porter.run.)

**Supabase** and **Neon** are Postgres data services, not compute hosts —
neither could ever run our compiled Rust binary, only back a database
connection it makes. Neither has a confirmed per-template creator payout:
Supabase's Master Partner Program Agreement defers all money terms to
private, per-partner addenda
([supabase.com/legal/partner-resources/master-partner-program-agreement](https://supabase.com/legal/partner-resources/master-partner-program-agreement)),
and Neon's Partner Program is a wholesale-discount tier for platforms
embedding Neon, not a payout to us, with its Open Source Program granting up
to $5,000/year in platform *credits* rather than a percentage
([neon.com/partners](https://neon.com/partners),
[neon.com/programs/open-source](https://neon.com/programs/open-source)).

**Upstash** — also a data service (serverless Redis/Kafka), same host-fit
caveat as Supabase/Neon. No programme found at all: no partners/affiliate
link in its site navigation, no dedicated blog post, and searches for
"Upstash affiliate," "Upstash partner programme" and "Upstash referral
programme" surfaced only unrelated similarly-named companies (UpStack,
Upstox, UpPromote) or unofficial directory listings.

**Replit**'s Bounties programme — a fixed-price, completed-gig payment
(Cycles per hour of estimated work,
[replit.com/blog/bounties](https://replit.com/blog/bounties)) rather than an
ongoing hosting kickback — appears to have been quietly discontinued around
September 2025, per a contemporaneous Hacker News thread describing an email
to users
([news.ycombinator.com/item?id=44643875](https://news.ycombinator.com/item?id=44643875));
no official Replit statement confirming the shutdown was found, so treat
that as the best available evidence rather than a certainty. Replit's 2026
monetization push (the "Agent Market," reported by
[The New Stack](https://thenewstack.io/replit-revenuecat-help-vibe-coders-monetize/))
is a direct-sale storefront for AI agents, not a template-deploy kickback.
Separately, whether Replit's "Reserved VM" deployment type
([docs.replit.com/features/publishing/deployment-types](https://docs.replit.com/features/publishing/deployment-types))
cleanly supports an arbitrary compiled Rust binary was not confirmed either
way in the docs fetched — moot here since there is no payout mechanism to
pursue regardless.

## Recommendation

Ranked by real money on the table, weighed against authkestra's actual
constraints — a real compiled Rust binary that needs a long-running process
(no WASM/edge/serverless-only host), no interest in owning a customer
payment or data relationship (mechanism 1 only, for anything pursued
without first building a paid product), and a live deployment reality of
Render (API) and Vercel (frontend), neither of which pays a template
kickback today.

1. **Railway.** The only mechanism-1 programme found anywhere in this
   research that pays real cash, is fully self-serve, publishes its exact
   terms, and can run our compiled binary as an ordinary container. Since
   the deploy-to-cloud manifest feature (P4) is still unbuilt, adding a
   Railway target to the starter-kit generator is new work either way —
   doing it for a provider that pays 15–25% of a deployer's hosting spend is
   strictly better than doing the same work for Render or Fly.io, both of
   which pay nothing. This is the one worth actually building toward.

2. **NodeOps CreateOS**, as a low-cost second listing rather than a plan to
   depend on. It genuinely supports Rust, its 70% creator cut is more
   generous than Railway's, and publishing a free-tier template costs
   nothing — but it's an early crypto-VC-backed platform with real
   longevity and payout-mechanism risk that Railway doesn't carry. Worth
   doing opportunistically once the Railway manifest exists, since most of
   the packaging work would already be done.

3. **Elestio, PikaPods and opensource.hosting, as a single outreach pass.**
   None is self-serve — each requires pitching authkestra into a curated
   catalogue rather than publishing to an open marketplace — so this is
   "send three emails once a Railway template exists," not a build task.
   Each pays 10–20% of hosting revenue if accepted, which is free money for
   the cost of an email, but none should be prioritised over actually
   shipping the Railway integration.

Everything under mechanism 2 (DigitalOcean Add-Ons, Vercel paid
integrations, Heroku Elements, AWS/Azure/GCP Marketplace) is correctly out
of scope for now, exactly as the issue frames it: none of it pays anything
until authkestra is selling a priced managed-hosting product, which it
isn't. Revisit that list only if and when such a product exists — at that
point AWS's 3% is the cheapest of the three clouds, and DigitalOcean's 75%
Add-On split is the best rate found anywhere in this research, full stop.
