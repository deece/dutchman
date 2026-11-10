# Contributing to Dutchman

Thank you for helping. Dutchman converts native CAD files into SolidCraft designs; reading the
native files is done by [cadmpeg](https://github.com/cadmpeg/cadmpeg). Bug reports, small
synthetic test files and pull requests are all welcome.

## Reporting a problem

Start with an anonymous report:

```sh
dutchman convert-all --out-dir converted --report-anon report-anon.json part.f3d
```

Replace `part.f3d` with your file or folder. Review `report-anon.json`, then open an issue
with the cause, counts, and versions it records. Ordinary logs, JSON output, and journals
contain file names and paths; keep them, the report key, and the salt private.

Keep the original design private. You may attach a small rebuilt reproduction if it
reproduces and you are free to share it; follow the
[minimum-reproduction guide](docs/minimum-reproduction.md). Never share the original or
local-key.json. Do not share designs from your employer or customers, or models downloaded
from libraries. See
[Testing Dutchman on your library](docs/testing-your-library.md#5-report-what-you-found).

Always report it here, on Dutchman, even when the report names cadmpeg or SolidCraft (an
`upstream` field, or a `f3d/…`, `inventor/…` or `sldprt/…` loss code). Don't file it on cadmpeg
or SolidCraft yourself: we work out where the fix belongs, take it upstream, and tell you here
when a Dutchman release has it. [docs/triage.md](docs/triage.md) says how.

To test Dutchman on a whole library of your own without sharing a file, see
[Testing Dutchman on your library](docs/testing-your-library.md) and its issue form.

## Building and checking

You need Rust 1.88 or newer.

```sh
cargo build --release
```

Before you open a pull request, run the checks CI runs (`.github/workflows/ci.yml`):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI runs the debug-profile tests on Linux (x86_64 and ARM64), Windows and macOS, and checks that the workspace builds with
Rust 1.88. A further job opens every converted fixture in a released SolidCraft
(`scripts/solidcraft-roundtrip.sh`) and replays the history tests in it. It is informational
and does not block a merge. To run it locally, put a SolidCraft release's `solidcraft-cli`
where `SOLIDCRAFT_CLI` points:

```sh
SOLIDCRAFT_CLI=/path/to/solidcraft-cli cargo test -p dutchman-solidcraft --test history --test assembly
```

### Build Dutchman against a local cadmpeg

The workspace `Cargo.toml` pins every cadmpeg crate it names directly (`cadmpeg-codec-f3d`,
`cadmpeg-core`, `cadmpeg-ir` and `cadmpeg-registry`) to one commit of cadmpeg's repository:
`git = "https://github.com/cadmpeg/cadmpeg", rev = ...`. To try a change in a cadmpeg
checkout without pushing it, patch that source to your checkout. Put this in
`.cargo/config.toml` in the Dutchman checkout, with cadmpeg cloned beside it (change the
paths if yours is elsewhere). Every cadmpeg crate in `Cargo.lock` needs a line, because the
registry pulls in the codecs too:

```toml
[patch."https://github.com/cadmpeg/cadmpeg"]
cadmpeg-asm = { path = "../cadmpeg/crates/cadmpeg-asm" }
cadmpeg-codec-catia = { path = "../cadmpeg/crates/cadmpeg-codec-catia" }
cadmpeg-codec-creo = { path = "../cadmpeg/crates/cadmpeg-codec-creo" }
cadmpeg-codec-f3d = { path = "../cadmpeg/crates/cadmpeg-codec-f3d" }
cadmpeg-codec-freecad = { path = "../cadmpeg/crates/cadmpeg-codec-freecad" }
cadmpeg-codec-iges = { path = "../cadmpeg/crates/cadmpeg-codec-iges" }
cadmpeg-codec-inventor = { path = "../cadmpeg/crates/cadmpeg-codec-inventor" }
cadmpeg-codec-nx = { path = "../cadmpeg/crates/cadmpeg-codec-nx" }
cadmpeg-codec-pvz = { path = "../cadmpeg/crates/cadmpeg-codec-pvz" }
cadmpeg-codec-rhino = { path = "../cadmpeg/crates/cadmpeg-codec-rhino" }
cadmpeg-codec-sat = { path = "../cadmpeg/crates/cadmpeg-codec-sat" }
cadmpeg-codec-sldprt = { path = "../cadmpeg/crates/cadmpeg-codec-sldprt" }
cadmpeg-codec-step = { path = "../cadmpeg/crates/cadmpeg-codec-step" }
cadmpeg-container = { path = "../cadmpeg/crates/cadmpeg-container" }
cadmpeg-core = { path = "../cadmpeg/crates/cadmpeg-core" }
cadmpeg-ir = { path = "../cadmpeg/crates/cadmpeg-ir" }
cadmpeg-measure = { path = "../cadmpeg/crates/cadmpeg-measure" }
cadmpeg-parasolid = { path = "../cadmpeg/crates/cadmpeg-parasolid" }
cadmpeg-protein = { path = "../cadmpeg/crates/cadmpeg-protein" }
cadmpeg-registry = { path = "../cadmpeg/crates/cadmpeg-registry" }
```

Then build as usual (`cargo build -p dutchman`). The patch changes `Cargo.lock`, which now
names the local paths. Keep both the `Cargo.lock` change and `.cargo/config.toml` out of
your commits: do not stage them, or run
`git update-index --assume-unchanged Cargo.lock .cargo/config.toml`
(undo with `--no-assume-unchanged`) and `git checkout Cargo.lock` when you are done.

To build against a commit of cadmpeg's `main` instead of a local checkout, run
`scripts/use-cadmpeg-main.sh <cadmpeg commit sha>`. It moves every pin to that commit and
updates `Cargo.lock`; do not commit what it changes either.

### Check one file quickly

A `--release` build takes a long time, and a plain debug build converts a large design too
slowly to wait for. A debug build optimised enough (`opt-level = 2`, no debug information)
converts one design in reasonable time, and it builds much faster than `--release`:

```sh
cargo build -p dutchman --config profile.dev.opt-level=2 --config profile.dev.debug=0
target/debug/dutchman convert path/to/design.f3d --no-verify
```

Use `--release` for timings and for anything you report as a speed.

### Use CI-built binaries instead of building locally

Pull requests and pushes to main build the debug profile and run the tests, which is what
checks them. The optimised programs (thin LTO, the same `dutchman`, `dutchman-mcp` and
`dutchman-gui` a release ships) come from three places:

- `main`: the nightly build, made every day from main and kept for 7 days. The script says which
  commit and day it took.
- a release candidate (`rc`, or a tag such as `v0.2.8-rc.1`): the full release build of a
  candidate tag, kept for 30 days.
- a pull request: its full build, which exists only for packaging changes, the `release-build`
  label, or a run started by hand (`gh pr edit 123 --add-label release-build`, or
  `gh workflow run release-build.yml --ref my-branch`). The script says so when it has none.

To try a change without building it:

```sh
scripts/fetch-ci-binary.sh main aarch64-apple-darwin /tmp/dutchman-main           # newest nightly of main
scripts/fetch-ci-binary.sh 123 x86_64-unknown-linux-gnu /tmp/dutchman-pr-123      # pull request 123 (if it has a full build)
scripts/fetch-ci-binary.sh v0.2.8-rc.1 x86_64-pc-windows-msvc /tmp/dutchman-rc    # a release candidate (or: rc)
```

The script needs the `gh` command, signed in, and prints the path of the unpacked `dutchman`.
To fetch by hand: `gh run download <run> -n <artifact>`; a downloaded
artifact has lost its executable bit, so `chmod +x` the programs.

## Code style

- **Never crash.** A damaged or unexpected file must come back as an error, never a panic.
  The workspace lints deny `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`,
  `unreachable!` and slice indexing outside tests, and forbid `unsafe`. Use `Result`, `?`,
  `get()` and checked arithmetic. Every call into cadmpeg is guarded against panics.
- **Treat file contents as hostile.** Cap sizes and loop counts that come from a file.
- **Messages are for people.** Say what happened and what to do next, plainly, in sentence
  case. Say "not supported yet" for a gap in Dutchman or cadmpeg; don't call the user's file
  corrupt or invalid. Use "convert", not "import", for what Dutchman does.
- **JSON reports are stable.** Within a schema version (`dutchman.convert/1`), fields are only
  added. A change that would break a reader needs a new version. Update
  [docs/json.md](docs/json.md) with any field you add.
- **Tests use synthetic files only.** See [Test files](#test-files).

## Pull requests

1. Fork the repository and branch from `main`.
2. Keep each pull request to one topic, with tests for what changed.
3. Update the docs it affects: [docs/guide.md](docs/guide.md) for commands and messages,
   [docs/mcp.md](docs/mcp.md) for the MCP tools, [docs/json.md](docs/json.md) for report
   fields, the format pages, and [skills/](skills/) when a tool or message the skill
   describes changes.
4. Add a [changelog.d/](changelog.d/) fragment for the change; do not edit `CHANGELOG.md`.
   Run `python3 scripts/changelog.py check` and `python3 scripts/changelog.py preview`.
5. Open the pull request against `main` and make sure CI passes.

A few things reviewers ask about most:

- **Keep the default output.** A new behaviour that changes what an existing file converts to
  (orientation, units, placement, names) is opt-in behind a flag, and the default output stays
  the same. Change a golden file or a placement expectation only when the change means to, and
  say why in the pull request.
- **Checks compare with the source.** Blend proving, the partial-history body comparison and the
  SolidCraft replay compare the written design with the source's coordinates, so a change that
  transforms the whole output applies it only where the final file is written, after them.
- **Land what works.** If your change needs an unmerged cadmpeg change, keep it as a draft until
  cadmpeg merges it and Dutchman's pin includes it, then switch everything on in one pull
  request. No commented-out code, ignored tests, or docs announcing what the build can't do yet.
- **Measure numbers that drive decisions**, such as the memory and time estimates in
  `crates/dutchman/src/choices.rs`, and put the measurements in the pull request.
- **Changelog fragments are for users:** one to three short bullets on what they get. Detail
  belongs in the docs.

Write commit messages and pull request titles in the imperative, in sentence case, saying
what changes for the user: "Map patterns of features", "Convert SolidWorks parts".

## Licence

Dutchman is licensed under the [Apache License, Version 2.0](LICENSE). By contributing, you
agree that your contributions are licensed under the same terms (section 5 of the licence).
There is no separate contributor agreement or sign-off.

## Clean-room rule

Proprietary formats are decoded from public information and from files their owners made
themselves, never from leaked or confidential material, decompiled programs, or someone
else's private designs. Native decoding belongs in cadmpeg, not in Dutchman: reading any part
of a native file's bytes (a zip member, a stream such as `BulkStream.dat`, a table) is
decoding, and goes there.

- Don't add code, test files or documentation that comes from another CAD program's source,
  SDK samples or help, or from material under a confidentiality agreement.
- Don't add third-party CAD files: no designs from model libraries, vendors, employers or
  customers.
- New test files are synthetic: built in code, or taken from cadmpeg's public test suite, with
  their origin written down in `crates/dutchman-core/tests/fixtures/README.md`: the merged
  cadmpeg commit and path a file was copied from, or how it was generated.
- Tests for bugs found on private files use generic dimensions and synthetic geometry: never
  copy a private design's numbers, names or shapes into a test.

## Test files

The files in [`crates/dutchman-core/tests/fixtures/`](crates/dutchman-core/tests/fixtures/)
are synthetic inputs from cadmpeg's test suite, or generated with cadmpeg's own encoders;
none is a real design. Their README records where each came from. Expected results are kept
as golden files in `crates/dutchman-solidcraft/tests/golden/`.

## Moving the cadmpeg pin

Dutchman pins cadmpeg to one commit of its `main` (the `rev` in the workspace `Cargo.toml`).
Move it after a batch of cadmpeg merges, never for one commit alone: run the tests, the
SolidCraft round trip and the anonymous-report leak check first (docs/releasing.md).

## Adding a format

Dutchman never parses a native format itself. A new format starts in cadmpeg: once cadmpeg
publishes a codec for it, Dutchman can convert it. Open the Dutchman pull request when the
codec is merged into cadmpeg's `main` and the pin includes it, or keep it as a draft until then.

1. Turn the codec on in the `cadmpeg-registry` features in the workspace `Cargo.toml`.
2. Add the format to `Format` in `crates/dutchman-core/src/format.rs` (its id, extension,
   name, family and `codec_id`), and to `ALL`. Format ids are part of the JSON reports, so
   choose them once.
3. Add its row to `dutchman formats` in `crates/dutchman/src/formats.rs`, saying plainly what
   converts.
4. Add a synthetic fixture with its provenance, and tests: decoding in
   `crates/dutchman-core/tests/decode.rs`, the written design against a golden file in
   `crates/dutchman-solidcraft/tests/end_to_end.rs`. The hostile-input test
   (`crates/dutchman-solidcraft/tests/robustness.rs`) picks up every fixture in that folder
   by itself. Add the fixture to the round-trip job's list in `.github/workflows/ci.yml`.
5. Write `docs/<format>.md`: what converts, what does not yet, and which upstream gaps stand
   in the way. List the new extension in the README, [docs/guide.md](docs/guide.md),
   [docs/json.md](docs/json.md) (`input.format`) and the MCP tool descriptions.
