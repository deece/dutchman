#!/usr/bin/env python3
"""Builds REPORT.md and results.json from the chunk results of a public corpus sweep.

  corpus_report.py --results DIR --plan plan.json --report REPORT.md --json results.json

DIR holds, anywhere below it, the `<id>.meta.json` files written by corpus_convert.py, each
next to its `<id>.json` (the `dutchman convert --json` report). Standard library only.
"""
import argparse
import collections
import json
import os
import re

MEMORY_LABEL = "needs more memory (runner)"


def load_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def history_class(row):
    """full | partial | flattened | geometry, for a converted file."""
    mode, status = row.get("mode"), row.get("verification")
    if mode == "partial_history" or status == "partial":
        return "partial"
    if status == "flattened":
        return "flattened"
    if mode == "history":
        return "full"
    return "geometry"


def owner_of(error, stage_hint=None):
    error = error or {}
    upstream = (error.get("upstream") or "").lower()
    if upstream == "cadmpeg":
        return "cadmpeg"
    if upstream == "solidcraft" or error.get("stage") == "solidcraft":
        return "SolidCraft"
    return "Dutchman"


def normalize(text):
    """A failure message without the parts that differ between files."""
    text = (text or "").strip().splitlines()[0] if (text or "").strip() else "(no message)"
    text = re.sub(r"/[^\s'\"):,]+", "<path>", text)
    text = re.sub(r"(?<!\w)'[^']*'(?!\w)|\"[^\"]*\"", "<name>", text)
    text = re.sub(r"\b0x[0-9a-fA-F]+\b", "<hex>", text)
    text = re.sub(r"\d+(\.\d+)?", "N", text)
    return text[:200]


def blocker_of(reason):
    """Why history stopped, without the feature's name or the count of features rebuilt."""
    text = re.sub(r"^history stopped at feature [^:]+: ", "", reason or "")
    text = re.sub(r" \(\d+ of \d+ features (?:kept|rebuilt)\).*$", "", text)
    return normalize(text)


def summarize(meta, report, err_tail):
    """One flat row for a file, from its meta and its convert report."""
    report = report or {}
    verification = report.get("verification") or {}
    history = report.get("history") or {}
    completeness = report.get("completeness") or {}
    row = {
        "id": meta["id"], "source": meta["source"], "path": meta["path"], "ext": meta.get("ext", ""),
        "size": meta.get("size", 0), "status": meta["status"], "exit_code": meta.get("exit_code"),
        "wall_s": meta.get("wall_s"), "peak_rss_kb": meta.get("peak_rss_kb"),
        "mode": report.get("mode"), "verification": verification.get("status"),
        "verification_reason": verification.get("reason"),
        "history_rebuilt": history.get("rebuilt"), "history_needing": history.get("needing"),
        "history_mapped": history.get("mapped"), "history_total": history.get("total"),
        "placed": completeness.get("placed"), "placements": completeness.get("placements"),
        "losses": len(report.get("losses") or []),
    }
    if meta["status"] == "converted":
        row["history"] = history_class(row)
        # Only files whose history stopped: the verification reason names where and why.
        blockers = []
        if row["history"] in ("partial", "flattened"):
            if verification.get("reason"):
                blockers.append(verification["reason"])
            else:
                blockers += [f["reason"] for f in history.get("features") or []
                             if f.get("status") == "blocked" and f.get("reason")][:1]
        row["blockers"] = blockers
    elif meta["status"] in ("failed", "timeout"):
        error = report.get("error")
        if error:
            row["error"] = {"message": error.get("message"), "stage": error.get("stage"),
                            "upstream": error.get("upstream")}
            row["owner"] = owner_of(error)
        elif meta["status"] == "timeout":
            row["error"] = {"message": "timed out", "stage": None, "upstream": None}
            row["owner"] = "Dutchman"
        else:
            last = next((l for l in reversed((err_tail or "").splitlines()) if l.strip()), "")
            note = meta.get("note") or (f"crashed or exited {meta.get('exit_code')} with no report: {last}"[:300])
            row["error"] = {"message": note, "stage": None, "upstream": None}
            row["owner"] = "Dutchman"
    return row


def collect(results_dir):
    rows = []
    for dirpath, _, names in os.walk(results_dir):
        for name in sorted(names):
            if not name.endswith(".meta.json"):
                continue
            meta = load_json(os.path.join(dirpath, name))
            if not meta:
                continue
            stem = os.path.join(dirpath, name[: -len(".meta.json")])
            report = load_json(stem + ".json")
            try:
                with open(stem + ".err", errors="replace") as f:
                    tail = f.read()[-2000:]
            except OSError:
                tail = ""
            rows.append(summarize(meta, report, tail))
    rows.sort(key=lambda r: (r["source"], r["path"]))
    return rows


def label(row):
    return MEMORY_LABEL if row["status"] == "needs_memory" else row["status"]


def gb(kb):
    return "-" if not kb else f"{kb / 1048576:.1f} GB"


def duration(seconds):
    if seconds is None:
        return "-"
    if seconds < 90:
        return f"{seconds:.0f} s"
    return f"{seconds / 60:.0f} min"


def table(headers, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"]
    out += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return out


def md_cell(text):
    return str(text).replace("|", "\\|").replace("\n", " ")


def build_report(rows, plan, inputs=None):
    lines = ["# Public corpus sweep", ""]
    if inputs:
        lines += [", ".join(f"{k}: {v}" for k, v in inputs.items()), ""]
    counts = collections.Counter(r["status"] for r in rows)
    lines += [
        f"**{len(rows)} files**: {counts['converted']} converted, {counts['failed'] + counts['timeout']} failed "
        f"({counts['timeout']} timed out), {counts['needs_memory']} {MEMORY_LABEL}, "
        f"{counts['skipped']} not run.",
        "",
    ]
    skipped = [s for s in (plan or {}).get("sources", []) if s.get("status") == "skipped"]
    if skipped:
        lines += ["Sources skipped:", ""] + [f"- {s['repo']}: {s['reason']}" for s in skipped] + [""]

    by_source = collections.defaultdict(list)
    for r in rows:
        by_source[r["source"]].append(r)
    lines += ["## Per source", ""]
    headers = ["Source", "Files", "By type", "Converted", "Failed", MEMORY_LABEL,
               "History full / partial / flattened (geometry only)", "Assemblies placed / total", "Biggest file"]
    table_rows = []
    for source in sorted(by_source):
        rs = by_source[source]
        c = collections.Counter(r["status"] for r in rs)
        types = collections.Counter(r["ext"] for r in rs)
        hist = collections.Counter(r.get("history") for r in rs if r["status"] == "converted")
        placed = sum(r["placed"] or 0 for r in rs if r.get("placements"))
        total = sum(r["placements"] or 0 for r in rs if r.get("placements"))
        biggest = max(rs, key=lambda r: r["wall_s"] or 0)
        table_rows.append([
            source, len(rs), ", ".join(f"{n} {e}" for e, n in sorted(types.items())),
            c["converted"], c["failed"] + c["timeout"], c["needs_memory"],
            f"{hist['full']} / {hist['partial']} / {hist['flattened']} ({hist['geometry']})",
            f"{placed} / {total}" if total else "-",
            f"{duration(biggest['wall_s'])}, {gb(max((r['peak_rss_kb'] or 0) for r in rs))} peak",
        ])
    lines += table(headers, table_rows) + [""]

    failures = [r for r in rows if r["status"] in ("failed", "timeout")]
    groups = collections.defaultdict(list)
    for r in failures:
        groups[(normalize(r["error"]["message"]), r["owner"])].append(r)
    lines += ["## Top failure causes", ""]
    if groups:
        ranked = sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0]))[:15]
        lines += table(["Count", "Owner", "Cause", "Example"], [
            [len(rs), owner, md_cell(cause), md_cell(f"{rs[0]['source']}/{rs[0]['path']}")]
            for (cause, owner), rs in ranked]) + [""]
        by_owner = collections.Counter(owner for (_, owner), rs in groups.items() for _ in rs)
        lines += ["By owner: " + ", ".join(f"{o} {n}" for o, n in by_owner.most_common()), ""]
    else:
        lines += ["No failures.", ""]

    needs = [r for r in rows if r["status"] == "needs_memory"]
    if needs:
        lines += [f"## {MEMORY_LABEL[0].upper() + MEMORY_LABEL[1:]}", "",
                  "These need more memory than the runner has; they are not conversion failures.", ""]
        lines += table(["File", "Size", "Peak memory"], [
            [md_cell(f"{r['source']}/{r['path']}"), f"{r['size'] / 1e6:.0f} MB", gb(r["peak_rss_kb"])]
            for r in sorted(needs, key=lambda r: -r["size"])[:20]]) + [""]

    blockers = collections.defaultdict(list)
    for r in rows:
        for b in dict.fromkeys(blocker_of(b) for b in r.get("blockers", [])):
            blockers[b].append(r)
    lines += ["## Top history blockers", ""]
    if blockers:
        ranked = sorted(blockers.items(), key=lambda kv: (-len(kv[1]), kv[0]))[:15]
        lines += table(["Files", "Reason", "Example"], [
            [len(rs), md_cell(reason), md_cell(f"{rs[0]['source']}/{rs[0]['path']}")]
            for reason, rs in ranked]) + [""]
    else:
        lines += ["No history blockers.", ""]
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--results", required=True)
    ap.add_argument("--plan")
    ap.add_argument("--report", default="REPORT.md")
    ap.add_argument("--json", default="results.json")
    ap.add_argument("--inputs", default="", help="JSON object of workflow inputs, shown in the report")
    args = ap.parse_args()
    rows = collect(args.results)
    plan = load_json(args.plan) if args.plan else None
    inputs = json.loads(args.inputs) if args.inputs else None
    with open(args.report, "w") as f:
        f.write(build_report(rows, plan, inputs) + "\n")
    with open(args.json, "w") as f:
        json.dump({"inputs": inputs, "sources": (plan or {}).get("sources", []), "files": rows}, f, indent=1)
    print(f"{len(rows)} files")


if __name__ == "__main__":
    main()
