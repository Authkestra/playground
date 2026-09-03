#!/usr/bin/env python3
"""Generate ROADMAP.md from roadmap.json so the doc and the GitHub sync can't drift."""
import json
import pathlib
import re

HERE = pathlib.Path(__file__).parent
data = json.loads((HERE / "roadmap.json").read_text())

out = []


def w(line=""):
    out.append(line)


w(f"# {data['project']} — Roadmap")
w()
w(f"> **Scope.** {data['scope_note']}")
w()

w("## Phases at a glance")
w()
w("| Phase | Title | Issues | Goal |")
w("|---|---|---|---|")
for ph in data["phases"]:
    n = sum(1 for i in data["issues"] if i["phase"] == ph["key"])
    goal = ph["goal"].replace("|", "\\|")
    short = ph["title"].split("—")[-1].strip()
    w(f"| `{ph['key']}` | {short} | {n} | {goal} |")
w()

total = len(data["issues"])
w(f"**{total} issues across {len(data['phases'])} phases.** P0–P5 is v0; P6 is backlog.")
w()

for ph in data["phases"]:
    issues = [i for i in data["issues"] if i["phase"] == ph["key"]]
    w(f"## {ph['title']}")
    w()
    w(f"**Goal.** {ph['goal']}")
    w()
    w(f"**Exit criteria.** {ph['exit']}")
    w()
    if issues:
        w("### Issues")
        w()
        for i in issues:
            labels = " ".join(f"`{l}`" for l in i["labels"])
            w(f"#### {i['title']}")
            w()
            w(labels)
            w()
            w(i["body"])
            w()
    w("---")
    w()

w("## Labels")
w()
w("| Label | Meaning |")
w("|---|---|")
for l in data["labels"]:
    w(f"| `{l['name']}` | {l['description']} |")
w()

w("## How this syncs to GitHub")
w()
w("`roadmap.json` is the source of truth. `sync_github_issues.py` reads it and creates the "
  "labels, milestones (one per phase) and issues, idempotently — re-running it will not "
  "create duplicates. Regenerate this document with `python3 gen_roadmap_md.py` after "
  "editing the JSON.")
w()

text = "\n".join(out)
text = re.sub(r"\n{3,}", "\n\n", text)
(HERE / "ROADMAP.md").write_text(text)
print(f"wrote ROADMAP.md ({len(text)} bytes, {total} issues)")
