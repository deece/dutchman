#!/usr/bin/env python3
"""Reads docs/candidates.toml for the candidates workflow.

  scripts/candidates.py matrix  [manifest]   one JSON build matrix (candidate x target)
  scripts/candidates.py list    [manifest]   candidates.json, as published beside the builds

Refuses a malformed manifest with a message naming the entry, so the workflow fails early.
"""
import json
import re
import sys
import tomllib

DEFAULT_TARGETS = ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]
RUNNERS = {
    "x86_64-pc-windows-msvc": {"os": "windows-latest", "container": ""},
    "x86_64-unknown-linux-gnu": {"os": "ubuntu-latest", "container": "debian:bookworm"},
    "aarch64-unknown-linux-gnu": {"os": "ubuntu-24.04-arm", "container": "debian:bookworm"},
    "aarch64-apple-darwin": {"os": "macos-latest", "container": ""},
    "x86_64-apple-darwin": {"os": "macos-latest", "container": ""},
}
NAME = re.compile(r"^[a-z0-9-]{1,40}$")
SHA = re.compile(r"^[0-9a-f]{40}$")
REPO = re.compile(r"^https://github\.com/[A-Za-z0-9_.-]+/cadmpeg$")
NOTE = re.compile(r"^[A-Za-z0-9 .,:()-]{0,80}$")


def load(path):
    with open(path, "rb") as f:
        data = tomllib.load(f)
    unknown = set(data) - {"candidate"}
    if unknown:
        sys.exit(f"{path}: unknown keys {sorted(unknown)}")
    seen = set()
    out = []
    for i, entry in enumerate(data.get("candidate", [])):
        where = f"{path}: candidate {i + 1}"
        extra = set(entry) - {"name", "note", "dutchman", "cadmpeg", "cadmpeg_repo", "targets"}
        if extra:
            sys.exit(f"{where}: unknown keys {sorted(extra)}")
        name = entry.get("name", "")
        if not NAME.match(name) or name in seen:
            sys.exit(f"{where}: name must be unique lower-case letters, digits and dashes")
        seen.add(name)
        for key in ("dutchman", "cadmpeg"):
            if not SHA.match(entry.get(key, "")):
                sys.exit(f"{where} ({name}): {key} must be a full 40-hex commit")
        repo = entry.get("cadmpeg_repo", "https://github.com/cadmpeg/cadmpeg")
        if not REPO.match(repo):
            sys.exit(f"{where} ({name}): cadmpeg_repo must be a GitHub cadmpeg repository")
        note = entry.get("note", name)
        if not NOTE.match(note):
            sys.exit(f"{where} ({name}): note must be one plain line of at most 80 characters")
        targets = entry.get("targets", DEFAULT_TARGETS)
        if not targets or any(t not in RUNNERS for t in targets):
            sys.exit(f"{where} ({name}): targets must be from {sorted(RUNNERS)}")
        out.append({"name": name, "note": note, "dutchman": entry["dutchman"],
                    "cadmpeg": entry["cadmpeg"], "cadmpeg_repo": repo, "targets": targets})
    return out


def main():
    if len(sys.argv) not in (2, 3) or sys.argv[1] not in ("matrix", "list"):
        sys.exit(__doc__)
    candidates = load(sys.argv[2] if len(sys.argv) == 3 else "docs/candidates.toml")
    if sys.argv[1] == "matrix":
        include = [dict(c, target=t, **RUNNERS[t]) for c in candidates for t in c["targets"]]
        for row in include:
            del row["targets"]
        print(json.dumps({"include": include}))
    else:
        print(json.dumps({"schema": 1, "candidates": candidates}, indent=2))


if __name__ == "__main__":
    main()
