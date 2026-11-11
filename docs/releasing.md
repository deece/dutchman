# Releasing

Each release is built once, as its final release candidate. A candidate is a tag `vX.Y.Z-rc.N` on a `main` commit; it runs the full release build. When one is good, the release tag `vX.Y.Z` on the same commit turns that build into a draft release in about five minutes, with no second build. Ordinary pull requests and pushes to `main` run only the quick debug build and tests.

1. Refresh the SolidCraft window alongside the cadmpeg pin. `WINDOW_OLDEST` stays an
   exact, checksum-pinned release from the rolling 14-day window, never below the internal
   `OLDEST_SOLIDCRAFT` floor (0.18.2). `NEWEST_SOLIDCRAFT` names only the newest known
   minor, currently `[0, 30]`; a patch release requires no Dutchman constant or docs update.
   Choose the oldest release and newest minor from SolidCraft's release dates, then run
   `GITHUB_TOKEN=... scripts/solidcraft-window.sh 0.18.2 0.30` with a read-only GitHub token.
   The script reads the authenticated GitHub releases list, selects the highest published
   stable patch of that minor, and checks its checksum metadata. It prints the constants,
   the oldest pinned checksum, and the newest minor's dynamic CI matrix entry. Copy those
   into `crates/dutchman-solidcraft/src/verify.rs`; the CI matrix's `newest` entry follows
   that constant, so `.github/workflows/ci.yml` needs no edit for a new minor.
   CI resolves that newest minor again at each run with the workflow's `GITHUB_TOKEN`,
   downloads its `SHA256SUMS.txt`, and verifies the exact archive before extracting it.
   Drafts, prereleases, missing minors, and ambiguous checksums fail rather than falling back.
   Keep the oldest entry pinned. Update minor names in README and docs when the newest
   minor changes. Internal writer targets default to newest-minor.0; explicit and found
   targets retain patches for feature gates and the 0.18.2 verification floor.
   Run target-version and roundtrip checks at both window bounds in CI.

   A new minor does not have to wait for a person: the `SolidCraft window` workflow
   (`.github/workflows/solidcraft-window.yml`) checks SolidCraft's releases every day and,
   when a new minor `x.Y.0` appears, runs `scripts/solidcraft-window.py 0.Y 0.Y.Z` and opens
   the "Refresh the SolidCraft window to 0.18–0.Y" pull request, then starts CI on its
   branch. The script moves the newest end of the window only; the oldest end and the design
   format stay as they are, so check the release notes for a format change before merging.
   Patch releases do nothing. The repository setting "Allow GitHub Actions to create and
   approve pull requests" must be on for the workflow to open the pull request.
2. Bump the version in its own small pull request, before the first candidate. Set the
   workspace `version` in `Cargo.toml` to `X.Y.Z` (and the `version` of the `dutchman-core` and
   `dutchman-solidcraft` workspace dependencies), and run `cargo build` so `Cargo.lock` follows.
   Candidates carry the final version, so the programs, archives and installer say `X.Y.Z` and
   "rc" appears only in the tag name. Only quick checks run on the bump. Leave the README's
   download table alone: it links the latest published release's version-less files
   (`releases/latest/download/`), which the release uploads beside the versioned ones, so it
   follows a release only once that release is published (a test checks the table).
3. Write `release-notes/X.Y.Z.md` in a documentation-only pull request, before the tag: one
   intro paragraph, then `## What's new` with at most 7 bullets, each with a bold lead phrase. Use
   plain words and Oxford commas; describe the highlights rather than copying the CHANGELOG.
   Add `## Known issue` if needed. Images are kept for milestones, such as a design that now
   converts fully: commit one under `docs/images/releases/X.Y.Z/` (from a public, licensed
   file) and place it after the intro as
   `![description](https://raw.githubusercontent.com/dutchman-cad/dutchman/vX.Y.Z/docs/images/releases/X.Y.Z/<file>)`,
   so the release page and announcements show the file at the tag. The release-notes check
   fails when a shown image has no alt text or its file is not committed.
   Preview the full body with
   `python3 scripts/release-notes.py X.Y.Z`; it adds the downloads, install commands, and
   a link to the CHANGELOG on `main` (its entry for the release is added after the tag, step 7;
   once it exists, the link points at the entry). Documentation-only, so it starts no full build.
4. Tag a candidate and push it: `git tag vX.Y.Z-rc.1 <commit> && git push origin vX.Y.Z-rc.1`.
   `release-build.yml` runs the full build (see "Release candidates" below) and keeps the
   release files for 30 days. If it finds a problem, fix it in a pull request and tag
   `vX.Y.Z-rc.2` on the new `main` commit; a newer candidate cancels a candidate build still
   running. Candidates install over each other (the MSI allows same-version upgrades).
5. Cut the release: tag the candidate commit that passed with `vX.Y.Z` (no `-rc`) and push the
   tag: `git tag vX.Y.Z <commit> && git push origin vX.Y.Z`. The `Release` workflow
   (`release.yml`) does not build. It finds the successful candidate run for exactly that
   commit, downloads its release files, checks every SHA-256, the exact file set and that
   `dutchman --version` from the Linux archive reports `X.Y.Z`, adds the version-less copies,
   and creates the draft GitHub release from `release-notes/X.Y.Z.md` with the files attached.
   That takes about five minutes. It fails if a release for that tag already exists, if the tag
   does not match the `Cargo.toml` version, or if no successful candidate run exists for the
   commit (it says so; tag a candidate on that commit and wait for it). A maintainer verifies
   the body and downloads, then publishes the release. Before publishing, check that the draft
   shows tag `vX.Y.Z`, not `untagged-…`.
   Publishing the release (not the tag or the draft) runs `Release image`
   (`release-image.yml`), which publishes `ghcr.io/dutchman-cad/dutchman:X.Y.Z` and `:latest`
   from the published release's checksum-verified Linux archives, with no second Rust build;
   a draft never reaches the registry. To republish a release's image, run it on demand with
   that tag (`gh workflow run release-image.yml -f tag=vX.Y.Z`). Candidate runs and packaging
   pull requests build both image architectures without logging in or pushing.
6. crates.io: releases ship as GitHub release archives only, until the core API settles. The
   crates.io `dutchman` crate stays the 0.0.1 placeholder. When crates are published, the
   order matters: `dutchman` depends on `dutchman-core` and `dutchman-solidcraft`, so those go
   first (`cargo publish -p dutchman-core`, then `-p dutchman-solidcraft`, then `-p dutchman`).
   Both are `publish = false` today, and publishing them makes their APIs public. Run
   `cargo publish --dry-run -p <crate>` before each.
7. After the release is published: in a documentation-only pull request, run
   `python3 scripts/changelog.py release X.Y.Z` to fold the fragments into a dated `CHANGELOG.md`
   entry (use `--date YYYY-MM-DD` to override today), review it, add any known issue, and stage
   the deleted fragments. It follows the release tag, so it builds nothing.

In short: bump pull request, notes pull request, `vX.Y.Z-rc.1`, fixes and `rc.2`, `vX.Y.Z` on
the chosen candidate's commit, draft in about five minutes, publish, fold the CHANGELOG.

If a release cannot be drafted because its candidate files are gone (they expire after 30
days) or the candidate run cannot be used, tag a new candidate on the commit, wait for it to
pass, then run `gh workflow run release.yml -f tag=vX.Y.Z`. The release workflow never builds:
a release always ships a candidate's tested files.

### Release candidates

`release-build.yml` is the full release build. It runs for:

| Trigger | Runs |
|---|---|
| Candidate tag `vX.Y.Z-rc.N` | Full build and packaging of that commit; files kept 30 days. Fails first if `X.Y.Z` differs from `Cargo.toml`'s version. A newer candidate cancels a running one. Saves the Rust caches. |
| Tag `v0.0.0-rc.dryrun*` | The same, without the version check (for measuring) |
| Release tag `vX.Y.Z` | `release.yml`: the candidate's files become a draft release; nothing is built |
| Tag `v0.0.0-dryrun*` | The same, but no release is created and the elapsed time is logged |
| Pull request that changes packaging, or with the `release-build` label | Full build; superseded runs are cancelled |
| Any other pull request | `ci.yml` only: debug build and tests on every platform |
| Push to `main` | `ci.yml` only; it saves the debug Rust caches that pull requests restore |
| Run by hand (`gh workflow run release-build.yml --ref <branch>`) | Full build |
| Nightly | Its own daily build of `main`, unchanged |

For just the Linux x86_64 tarball and its checksum, dispatch with `targets: linux-x86_64`: `gh workflow run release-build.yml --ref <branch> -f targets=linux-x86_64`.

A full build makes each platform's archive
(`scripts/package-archive.sh`: `dutchman`, `dutchman-mcp` and `dutchman-gui`, plus `dutchman-web` on Linux, with LICENSE,
NOTICE, README and a SHA-256), the Windows per-user MSI, and the macOS disk image
(`scripts/macos-dmg.sh`: Dutchman Convert.app, universal, signed ad hoc), and keeps exactly the
files a release attaches, with their `.sha256`, as workflow artifacts
(`dutchman-X.Y.Z-<target>.tar.gz` or `.zip`, with the MSI, and `dutchman-X.Y.Z-macos`). The two Mac targets
build in parallel on separate Apple silicon runners (Intel is cross-compiled). A small
packaging job checks and unpacks both archives, then uses their binaries to build the
universal app with `lipo` and make the disk image, without compiling again. It also makes
`dutchman-X.Y.Z-macos-universal.tar.gz` with universal `dutchman`, `dutchman-mcp`, and
`dutchman-gui` binaries, using the same archive layout and checksum format as the per-chip
archives. The per-chip Mac archives remain release assets for Homebrew. On Windows the job
installs the MSI, upgrades it from a lower version and over a second build of the same version
(`MajorUpgrade` allows same-version upgrades, so rc.2 installs over rc.1),
refuses a downgrade, installs as a standard user, uninstalls, and runs the release-mode updater
tests. Rust caches are kept per target and profile for those checks and saved only by pushes to `main`. The full release builds (candidates, packaging pull requests, runs by hand) restore and save no cache: what users download is always built from scratch.

The `plan` job in `release-build.yml` starts the builds for a pull request that:
   - changes `.github/workflows/`, `Cargo.toml` other than its version lines (it holds the
     profiles), `Dockerfile`,
     `docker/`, `compose.yaml`, `scripts/*msi*`, `scripts/package*`, `scripts/macos-dmg.sh`,
     `scripts/archive-readme.py`, `scripts/downloads.py`, `assets/install/`, `assets/dmg/`,
     `assets/windows/`, `skills/`, the app icon and `examples/ico.rs`, or
     `crates/dutchman-gui/src/update.rs` and `update/`;
   - carries the `release-build` label (the label starts the run when it is added), for any
     other change that needs the full builds, such as one to `Cargo.lock` that might break a
     release link.

Without one of those, the workflow still reports on the pull request, with its builds skipped.

The release tag finds the candidate with `scripts/find-release-run.sh`: a successful run of
`release-build.yml`, started by pushing a tag `vX.Y.Z-rc.*`, for exactly the tagged commit, which
still has every release artifact for the version (the newest such run). No other commit
counts: the release tag goes on the candidate's own commit. A dry run
(`git tag v0.0.0-rc.dryrun1 <commit>`, then `git tag v0.0.0-dryrun1 <commit>`; push each, then
delete them with `git push origin :refs/tags/<name>`) does all of it but creating the release.

Programs between releases: `scripts/fetch-ci-binary.sh main <target> <dir>` takes the newest
nightly build of `main`, `rc` or a tag such as `v0.2.8-rc.1` takes a candidate's, and a pull
request number takes its full build when it has one (CONTRIBUTING.md).

## Packaging

Release and nightly Mac downloads offer a universal `.dmg` for the desktop app and a
universal `.tar.gz` for the command line. Both run on Apple silicon and Intel. Stable
releases also attach version-less copies, including `dutchman-macos-universal.tar.gz`
and its `.sha256`. Only after a release containing that archive is published, switch
`scripts/downloads.py`'s `latest` table to the universal archive and regenerate the README
download table with `python3 scripts/downloads.py latest README.md`.

Linux release and nightly archives include `dutchman-web`; it needs no desktop libraries.
Windows and macOS archives currently leave it out: native builds of the web server have not
yet been verified there. Build it from source on those platforms until that check is complete.

`scripts/macos-dmg.sh` builds the universal app and a compressed, read-only HFS+ disk
image. The installation window opens in icon view with about 640 × 440 points of content,
96-point app and Applications icons under the ends of a teal arrow, and the read-me below
them. The toolbar, status bar and sidebar are hidden.

The original background and its provenance live in `assets/dmg/`; regenerate both PNG
sizes from the SVG using the Rust example documented there. Packaging uses macOS `sips`
and `tiffutil` to combine @1x and @2x images into a hidden multi-resolution TIFF.
It needs no additional packaging dependencies.

macOS does not provide a standard headless writer for Finder's `.DS_Store` layout.
The script mounts a temporary writable image and asks Finder to save the view, positions
and background before detaching and compressing it. Finder needs a logged-in session and
permission for the build to control it (GitHub's macOS runners have both). Finder has a
30-second request timeout and an outer 45-second process limit. If styling fails, including when Finder is
unavailable or automation access is denied, the script discards that image and builds
the plain app, Applications link and read-me image from the untouched staging folder.
Image creation, signing, detaching and final `hdiutil verify` errors still stop packaging.

On macOS, mount the finished image afresh, check the icon positions and background in
both appearances and on Retina and standard-density displays, open the read-me, then
drag the app into Applications and launch it. Eject and remount to check persistence.
Also build with a failing `osascript` earlier on `PATH` to check that the plain fallback
still mounts, retains all three items and passes `hdiutil verify`.

## Stable and nightly

A stable release depends on cadmpeg at one commit of its `main` branch, taken from cadmpeg's
own repository (`https://github.com/cadmpeg/cadmpeg`) and pinned as the same `rev` for every
cadmpeg crate in the workspace `Cargo.toml`. Move the pin after a batch of
cadmpeg merges: set the new commit, run `cargo update -p cadmpeg-core -p cadmpeg-ir -p
cadmpeg-registry`, then the full tests, the SolidCraft round trip and the anonymous-report
leak check on a real library before merging it. The pin pull request must pass the "Placements" check
(`corpus/placements/`); if a golden legitimately changes, regenerate it in the same pull request
after looking at the converted picture (see `corpus/placements/README.md`). A fix merged on cadmpeg's `main` after the
pinned commit reaches users through the nightly channel: the `nightly` pre-release, rebuilt
each day from Dutchman's `main` against cadmpeg's `main` (`scripts/use-cadmpeg-main.sh`, see
`.github/workflows/nightly.yml`). The stable release after the pin moves carries it.

Nightly and release macOS builds use the same release profile, target triples, packages, and
default features. The nightly keeps its own Rust cache per target (`macos-release-<target>`);
release candidate builds use no cache and build from scratch.

Regular releases build with the `release` profile: thin LTO and 16 codegen units. A major or
stable release, starting with 0.3.0, builds with a slower, fully optimised profile instead
(one codegen unit and fat LTO).

All cadmpeg crates, including cadmpeg-registry, come directly from the pinned git revision.
`scripts/use-cadmpeg-main.sh <sha>` updates the dependencies and checks their lockfile sources.

## Candidate builds

`docs/candidates.toml` lists builds that are neither stable nor nightly: one Dutchman commit
against one cadmpeg commit (a fix not yet merged, say), for a few targets (by default
Windows x86_64 and Linux x86_64). A change to the file on `main` runs
`.github/workflows/candidates.yml`, which builds each with the `release` profile and replaces
the rolling `candidates` pre-release with the archives, their SHA-256 files and
`candidates.json`. `dutchman investigate` compares a user's design against them and the
nightly. Remove an entry once its fix is released; an empty list publishes an empty release.

## Container image

The Linux archive builds run in Debian bookworm so their glibc requirements fit the
`debian:bookworm-slim` runtime. `docker/release.Dockerfile` installs the CLI and MCP
server, plus `dutchman-web` whenever it is included in the archives, and retains the
licence files. Image licence metadata comes from `Cargo.toml`. Only the image publishing
jobs receive `packages: write`. The nightly image job waits for the nightly pre-release
publication to succeed, then publishes `ghcr.io/dutchman-cad/dutchman:nightly` from the
same run's Linux artifacts.

Check the image job in the workflow, then check the published manifest and executable:

```sh
docker buildx imagetools inspect ghcr.io/dutchman-cad/dutchman:X.Y.Z
docker run --rm ghcr.io/dutchman-cad/dutchman:X.Y.Z --version
```

The manifest must include `linux/amd64` and `linux/arm64`; test `--version` on each
architecture. On first publication, check that the GHCR package is public so anonymous
pulls work. See [Docker](docker.md) for conversion, MCP and Compose examples.

## Public corpus sweep

The **Public corpus sweep** workflow (`.github/workflows/corpus-sweep.yml`) converts the CAD
files of public repositories on GitHub's free runners, so a sweep over a real-world corpus does
not need anyone's machine. `corpus/public.toml` lists the sources: only public repositories under
an open licence, each with its SPDX licence. The workflow reads the licence from the GitHub API
and skips a source whose licence no longer matches. A repository with no open licence is not
listed (the file says which were left out). It fetches only the CAD files, never the whole
repository, and publishes nothing: the results are workflow artifacts and the run summary.

Dispatch it from the Actions tab, or with the GitHub CLI:

```
gh workflow run corpus-sweep.yml -f sources=PrintersForAnts/Crucible -f step=true
```

| Input | Meaning |
|---|---|
| `sources` | Comma list of `owner/name` to restrict the run to (default: every source). |
| `dutchman` | `branch` (the code of the branch the run is on; the default) or `nightly` (the nightly release build). To sweep a pull request, run the workflow on its branch: `gh workflow run corpus-sweep.yml --ref <branch>`. |
| `cadmpeg` | `pinned` (the commit Dutchman pins) or `main` (cadmpeg's main, as the nightly builds it). |
| `step` | Include `.step` and `.stp` files. |
| `preset` | `standard` (default), `fast` or `full`. |
| `keep_sc` | Comma list of path fragments: files matching keep their converted design as an `sc-*` artifact. Others are deleted after conversion. |

The `build` job builds the Linux binary once, `plan` lists each repository's files and groups
them into at most 256 chunks (files over 40 MB and `.f3z` alone, the rest about 20 at a time),
and one `convert` job per chunk converts each file with `--no-verify`, a 90-minute limit per
file, and records exit code, wall time and peak memory. A file that needs more memory than the
runner has, refused by Dutchman's memory check or killed for memory, is reported as "needs more
memory (runner)", not as a conversion failure. The `report` job builds `REPORT.md` (also shown in
the run summary) and `results.json`, uploaded as the `corpus-report` artifact. The report has, per
source, the files by type, converted, failed and needs-memory counts, how many kept full, partial
or flattened history, assemblies placed against total, and the slowest file's time and peak
memory; then the top failure causes with an example and the layer that owns each (cadmpeg,
Dutchman or SolidCraft), and the top reasons history stopped. `scripts/corpus_report.py` is tested
by `scripts/tests/test_corpus_report.py`.

For report fix notices, include the portal's `fixes_reports` hashes in release notes
as the hidden `dutchman-release` JSON comment documented in [json.md](json.md).
Use hashes of the exact diagnostic report ids, including their existing prefix.
The app reads this through its normal release check and compares locally.
Release builds send reports to the service address in `crates/dutchman/src/report_send.rs`.
`DUTCHMAN_REPORT_ENDPOINT` at build time replaces it, for example for a staging service.
