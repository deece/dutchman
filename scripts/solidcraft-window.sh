#!/usr/bin/env bash
# Print window constants/matrix entries, or fetch a verified CI release.
# Usage: solidcraft-window.sh <oldest-x.y.z> <newest-x.y>
#        solidcraft-window.sh --fetch <x.y|x.y.z> [pinned-sha256]
set -euo pipefail
helper="$(dirname "$0")/solidcraft-release.py"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

latest_patch() {
    local minor=$1 page count
    : "${GITHUB_TOKEN:?Set GITHUB_TOKEN to resolve the newest SolidCraft patch}"
    for page in {1..20}; do
        curl -fsSL --retry 5 --retry-all-errors --connect-timeout 15 --max-time 120 \
            -H "Authorization: Bearer $GITHUB_TOKEN" -H 'Accept: application/vnd.github+json' \
            "https://api.github.com/repos/tangency-labs/solidcraft/releases?per_page=100&page=$page" \
            -o "$work/page-$page.json"
        count=$(python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); assert isinstance(p,list), "Expected releases list"; print(len(p))' "$work/page-$page.json")
        if (( count < 100 )); then
            python3 "$helper" resolve "$minor" "$work"/page-*.json
            return
        fi
    done
    echo 'SolidCraft release list exceeded 20 pages; refusing an incomplete resolution.' >&2
    return 1
}

release_checksum() {
    local version=$1 archive="solidcraft-$1-linux-x86_64.tar.gz"
    curl -fsSL --retry 5 --retry-all-errors --connect-timeout 15 --max-time 120 \
        "https://github.com/tangency-labs/solidcraft/releases/download/v$version/SHA256SUMS.txt" \
        -o "$work/sums.txt"
    python3 "$helper" checksum "$archive" "$work/sums.txt"
}

if [[ ${1:-} == --fetch && $# -ge 2 && $# -le 3 ]]; then
    version=$2 checksum=${3:-}
    if [[ $version =~ ^[0-9]+\.[0-9]+$ ]]; then
        version=$(latest_patch "$version")
        checksum=$(release_checksum "$version")
    elif [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || ! $checksum =~ ^[0-9a-fA-F]{64}$ ]]; then
        echo 'A pinned x.y.z release needs its SHA-256.' >&2
        exit 2
    fi
    archive="solidcraft-$version-linux-x86_64.tar.gz"
    curl -fsSL --retry 5 --retry-all-errors --connect-timeout 15 --max-time 120 -o "$archive" \
        "https://github.com/tangency-labs/solidcraft/releases/download/v$version/$archive"
    printf '%s  %s\n' "$checksum" "$archive" | sha256sum -c - >&2
    tar xzf "$archive"
    printf '%s\n' "$version"
    exit
fi

if [[ $# != 2 || ! $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || ! $2 =~ ^[0-9]+\.[0-9]+$ ]]; then
    echo "Usage: $0 <oldest-x.y.z> <newest-x.y>" >&2
    exit 2
fi
oldest=$1 newest=$2
oldest_sha=$(release_checksum "$oldest")
# Verify that the chosen minor has a published patch and matching checksum metadata.
resolved=$(latest_patch "$newest")
release_checksum "$resolved" > /dev/null
printf 'pub const NEWEST_SOLIDCRAFT: [u64; 2] = [%s];\n' "${newest//./, }"
printf 'pub const WINDOW_OLDEST: [u64; 3] = [%s];\n\n' "${oldest//./, }"
printf '          - version: %s\n            sha256: %s\n' "$oldest" "$oldest_sha"
printf '          - version: "%s"\n' "$newest"
