#!/usr/bin/env bash
# Print window constants/matrix entries, or fetch a verified CI release.
# Usage: solidcraft-window.sh <oldest-x.y.z> <newest-x.y>
#        solidcraft-window.sh --print [--releases saved-releases.json]
#        solidcraft-window.sh --fetch <x.y|x.y.z> [pinned-sha256]
set -euo pipefail
if [[ ${1:-} == --print ]]; then
    shift
    exec python3 "$(dirname "$0")/solidcraft-version-policy.py" "$@"
fi
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

# Fetches one asset of release v<version> to <out>. The release download URL can fail for
# hours on one asset (a 504 on SHA256SUMS.txt while the archives still download), so after
# its retries the same asset is fetched through the API's asset endpoint.
fetch_asset() {
    local version=$1 name=$2 out=$3 id
    curl -fsSL --retry 3 --retry-all-errors --connect-timeout 15 --max-time 600 -o "$out" \
        "https://github.com/tangency-labs/solidcraft/releases/download/v$version/$name" && return
    echo "release download of $name failed; fetching it through the API" >&2
    local auth=()
    [[ -n ${GITHUB_TOKEN:-} ]] && auth=(-H "Authorization: Bearer $GITHUB_TOKEN")
    curl -fsSL --retry 5 --retry-all-errors --connect-timeout 15 --max-time 120 "${auth[@]}" \
        -H 'Accept: application/vnd.github+json' \
        "https://api.github.com/repos/tangency-labs/solidcraft/releases/tags/v$version" \
        -o "$work/release.json"
    id=$(python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); print(next(a["id"] for a in r["assets"] if a["name"]==sys.argv[2]))' "$work/release.json" "$name")
    curl -fsSL --retry 5 --retry-all-errors --connect-timeout 15 --max-time 600 "${auth[@]}" \
        -H 'Accept: application/octet-stream' -o "$out" \
        "https://api.github.com/repos/tangency-labs/solidcraft/releases/assets/$id"
}

release_checksum() {
    local version=$1 archive="solidcraft-$1-linux-x86_64.tar.gz"
    fetch_asset "$version" SHA256SUMS.txt "$work/sums.txt"
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
    # SOLIDCRAFT_CACHE (a folder CI restores between runs) holds archives already fetched. A
    # cached archive is used only when it matches the checksum, like a fresh download.
    cache=${SOLIDCRAFT_CACHE:-}
    if [[ -n $cache && -f $cache/$archive ]] \
        && printf '%s  %s\n' "$checksum" "$cache/$archive" | sha256sum -c - >&2; then
        cp "$cache/$archive" "$archive"
    else
        fetch_asset "$version" "$archive" "$archive"
        printf '%s  %s\n' "$checksum" "$archive" | sha256sum -c - >&2
        if [[ -n $cache ]]; then
            mkdir -p "$cache"
            cp "$archive" "$cache/$archive"
        fi
    fi
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
