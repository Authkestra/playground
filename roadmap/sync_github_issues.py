#!/usr/bin/env python3
"""
Sync roadmap.json to GitHub: labels, milestones (one per phase), and issues.

Idempotent: existing labels/milestones/issues (matched by name/title) are updated or
skipped rather than duplicated, so it is safe to re-run after editing roadmap.json.

Usage:
    export GITHUB_TOKEN=...            # needs repo scope (issues: write)
    python3 sync_github_issues.py --owner marcjazz --repo authkestra-playground --dry-run
    python3 sync_github_issues.py --owner marcjazz --repo authkestra-playground

Flags:
    --dry-run     print what would happen; make no write calls
    --prefix STR  prepend STR to every issue title (e.g. "[playground] ") — useful if
                  the issues land in the framework repo rather than a dedicated one
"""
import argparse
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.github.com"
HERE = pathlib.Path(__file__).parent


def req(method, path, token, body=None, params=None):
    url = f"{API}{path}"
    if params:
        url += "?" + urllib.parse.urlencode(params)
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method)
    r.add_header("Authorization", f"Bearer {token}")
    r.add_header("Accept", "application/vnd.github+json")
    r.add_header("X-GitHub-Api-Version", "2022-11-28")
    r.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(r) as resp:
            raw = resp.read()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")
        raise SystemExit(
            f"\n!! {method} {path} failed: HTTP {e.code}\n   {detail[:600]}\n"
            "   If this says access is not enabled for the session, the repo has not been\n"
            "   attached to this Claude session — see the note at the top of the handoff.\n"
        )


def paged(path, token, params=None):
    out, page = [], 1
    while True:
        p = dict(params or {})
        p.update({"per_page": 100, "page": page})
        batch = req("GET", path, token, params=p)
        if not batch:
            break
        out.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--owner", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--prefix", default="")
    args = ap.parse_args()

    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if not token:
        raise SystemExit("GITHUB_TOKEN (or GH_TOKEN) must be set")

    data = json.loads((HERE / "roadmap.json").read_text())
    base = f"/repos/{args.owner}/{args.repo}"
    dry = args.dry_run
    tag = "[dry-run] " if dry else ""

    print(f"{tag}Target: {args.owner}/{args.repo}")
    print(f"{tag}Plan: {len(data['labels'])} labels, {len(data['phases'])} milestones, "
          f"{len(data['issues'])} issues\n")

    if not dry:
        repo = req("GET", base, token)
        if not repo.get("has_issues"):
            raise SystemExit(f"!! Issues are disabled on {args.owner}/{args.repo}")

    # ---- labels ----
    existing_labels = {l["name"] for l in paged(f"{base}/labels", token)} if not dry else set()
    for l in data["labels"]:
        if l["name"] in existing_labels:
            print(f"{tag}label   = {l['name']} (exists)")
            continue
        if not dry:
            req("POST", f"{base}/labels", token, body=l)
        print(f"{tag}label   + {l['name']}")

    # ---- milestones ----
    ms_by_title = {}
    if not dry:
        for m in paged(f"{base}/milestones", token, params={"state": "all"}):
            ms_by_title[m["title"]] = m["number"]

    phase_ms = {}
    for ph in data["phases"]:
        title = ph["title"]
        if title in ms_by_title:
            phase_ms[ph["key"]] = ms_by_title[title]
            print(f"{tag}milest. = {title} (exists, #{ms_by_title[title]})")
            continue
        if dry:
            phase_ms[ph["key"]] = None
        else:
            m = req("POST", f"{base}/milestones", token,
                    body={"title": title,
                          "description": f"Goal: {ph['goal']}\n\nExit criteria: {ph['exit']}"})
            phase_ms[ph["key"]] = m["number"]
        print(f"{tag}milest. + {title}")

    # ---- issues ----
    existing_titles = set()
    if not dry:
        for i in paged(f"{base}/issues", token, params={"state": "all"}):
            if "pull_request" not in i:
                existing_titles.add(i["title"])

    created = skipped = 0
    for issue in data["issues"]:
        title = f"{args.prefix}{issue['title']}"
        if title in existing_titles:
            print(f"{tag}issue   = {title[:72]} (exists)")
            skipped += 1
            continue

        phase = next(p for p in data["phases"] if p["key"] == issue["phase"])
        body = (
            f"{issue['body']}\n\n"
            f"---\n"
            f"**Phase:** `{issue['phase']}` — {phase['title'].split('—')[-1].strip()}\n\n"
            f"_Tracked in the playground roadmap (`roadmap.json`). "
            f"Scoped to authkestra capabilities already shipped; the framework's own "
            f"forward roadmap lives in `docs/roadmap.md` upstream._"
        )
        payload = {"title": title, "body": body, "labels": issue["labels"]}
        if phase_ms.get(issue["phase"]):
            payload["milestone"] = phase_ms[issue["phase"]]

        if not dry:
            made = req("POST", f"{base}/issues", token, body=payload)
            print(f"{tag}issue   + #{made['number']} {title[:66]}")
            time.sleep(0.7)  # stay well clear of secondary rate limits
        else:
            print(f"{tag}issue   + {title[:72]}")
        created += 1

    print(f"\n{tag}Done: {created} issue(s) created, {skipped} already present.")
    if dry:
        print("Nothing was written. Re-run without --dry-run to apply.")


if __name__ == "__main__":
    sys.exit(main())
