#!/usr/bin/env python3
"""Read the same stable GitHub release feed as solidcraft-window.sh, including dates."""
import argparse
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import re
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
FEED = "https://api.github.com/repos/tangency-labs/solidcraft/releases"


def releases(filename=None):
    if filename:
        rows = json.loads(Path(filename).read_text())
        if not isinstance(rows, list):
            raise ValueError("Expected a release list")
        return rows
    rows = []
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "Dutchman-version-policy"}
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    for page in range(1, 21):
        request = urllib.request.Request(f"{FEED}?per_page=100&page={page}", headers=headers)
        with urllib.request.urlopen(request, timeout=30) as response:
            batch = json.load(response)
        if not isinstance(batch, list):
            raise ValueError("Expected a release list")
        rows.extend(batch)
        if len(batch) < 100:
            return rows
    raise ValueError("Release list exceeded 20 pages; refusing an incomplete window")


def window(rows, now=None):
    now = now or datetime.now(timezone.utc)
    stable = {}
    for row in rows:
        match = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)", row.get("tag_name", ""))
        # A plain X.Y.Z tag is a release. SolidCraft publishes some (0.40.0) with GitHub's
        # pre-release flag still set until packaging finishes, so the flag is not trusted;
        # release candidates and nightlies carry a suffix and never match.
        if row.get("draft") or not match:
            continue
        published = datetime.fromisoformat(row["published_at"].replace("Z", "+00:00"))
        if published.tzinfo is None:
            raise ValueError("Release dates must include a timezone")
        if published <= now:
            stable[tuple(map(int, match.groups()))] = published
    if not stable:
        raise ValueError("No published stable SolidCraft release found")
    newest = max(stable)
    selected = {v for v, date in stable.items() if date >= now - timedelta(days=14)} | {newest}
    return newest, min(selected), stable


def version(value):
    return ".".join(map(str, value))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--releases", type=Path, help="saved GitHub release list (with published_at)")
    args = parser.parse_args()
    newest, oldest, dates = window(releases(args.releases))
    for label, value in (("Newest", newest), ("Oldest in window", oldest)):
        print(f"{label}: {version(value)} (published {dates[value].isoformat()})")
    print("Opens-in floor: 0.18.2")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError) as error:
        raise SystemExit(f"SolidCraft window: {error}")
