# Instructions for AI contributors

Dutchman converts native CAD files (Fusion `.f3d`/`.f3z`, Inventor `.ipt`/`.iam`, SolidWorks
`.sldprt`, FreeCAD `.FCStd`) into SolidCraft designs (`.sc`; `.solidcraft` for SolidCraft before 0.25.1). It is pure Rust, Apache-2.0. Native files
are decoded by [cadmpeg](https://github.com/cadmpeg/cadmpeg); Dutchman maps cadmpeg's neutral
model (CADIR) onto SolidCraft. [CONTRIBUTING.md](CONTRIBUTING.md) has the full rules for
everyone; this file is the short version, plus what an AI working in the repository needs.

## Read first

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): the pipeline, the crates, the `Writer` seam,
  how history is mapped and checked.
- [docs/json.md](docs/json.md): the JSON reports. Field names are stable within a schema
  version.
- The format pages: [docs/fusion.md](docs/fusion.md), [docs/inventor.md](docs/inventor.md),
  [docs/sldprt.md](docs/sldprt.md).

## Layout

| Path | What is there |
|---|---|
| `crates/dutchman-core` | Format detection (`format.rs`), the one place cadmpeg is called (`decode.rs`), assemblies read from part files (`assembly.rs`), `Error` and `Report`, the `Writer` trait and `convert`. Knows nothing about SolidCraft. |
| `crates/dutchman-solidcraft` | The SolidCraft writer: the design container (`container.rs`, `document.rs`), the STEP restatement (`step.rs`), feature history (`history/`), assemblies (`assembly.rs`), the check in `solidcraft-cli` (`verify.rs`). Knows nothing about native formats. |
| `crates/dutchman` | The `dutchman` command (`main.rs`, `batch.rs` for `convert-all`, `formats.rs`), the `dutchman-mcp` server (`mcp.rs`, `bin/dutchman-mcp.rs`) and the JSON reports both return (`json.rs`). |
| `crates/dutchman-web` | The `dutchman-web` server: a page and an API that convert uploads with `dutchman::batch` (`lib.rs`), uploads and project zips (`upload.rs`), reports made fit for the browser (`report.rs`), the pages (`templates/`). |
| `crates/dutchman-serve` | Shared HTTP guards, private data folder checks (`hardening.rs`), and bounded zip extraction (`upload.rs`). |
| `crates/dutchman-core/tests/fixtures` | Synthetic test files, with their origin in the folder's README. |
| `crates/dutchman-solidcraft/tests/golden` | Expected `document.json` output. |
| `scripts/solidcraft-roundtrip.sh` | Opens converted fixtures in a SolidCraft CLI. |
| `docs/`, `skills/` | User docs, the MCP guide, and the agent skill for people converting files. |

## Checks

Run what CI runs (`.github/workflows/ci.yml`) before every commit:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI also builds with Rust 1.88 (`cargo check --workspace --locked`) and tests on Windows and
macOS, so don't assume a case-sensitive filesystem or `/` as the only path separator. The
history tests replay in SolidCraft when `SOLIDCRAFT_CLI` points at a `solidcraft-cli`.

## Rules

- **SolidCraft versions.** Keep the 0.18.2 opens-in floor, its CI job and all existing writer checks.
  Before starting, run `scripts/solidcraft-window.sh --print`: new work targets only newest plus releases
  published in the last 14 days. Never back-port features or add gates older than that window;
  flatten new features for older targets with a named reason, keeping the file opening.
- **Never crash.** No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`
  or slice indexing outside tests; no `unsafe` (the workspace lints enforce all of this). A
  hostile file comes back as an `Error`. Every call into cadmpeg goes through the existing
  guards, which turn a panic into an error. Cap sizes and counts that come from a file.
- **Keep the crates apart.** `dutchman-core` must not learn about SolidCraft;
  `dutchman-solidcraft` must not learn about native formats. Native decoding belongs in
  cadmpeg: if a format needs new decoding, the fix goes upstream there. Reading any part of a
  native file's bytes (a zip member, a stream such as `BulkStream.dat`, a table) is decoding;
  Dutchman uses only what cadmpeg reports.
- **Stable reports.** Only add fields within a schema version, and document them in
  `docs/json.md`. The CLI's `--json` and the MCP tools return the same reports, built in one
  place (`crates/dutchman/src/json.rs`).
- **Messages.** Plain, specific, sentence case; say what to do next; "not supported yet" for
  our gaps, never "corrupt" or "invalid" for the user's file; "convert", not "import".
- **Docs move with the code.** A change to a command, flag, tool, report field or message
  updates `docs/guide.md`, `docs/mcp.md`, `docs/json.md` and `skills/` in the same pull
  request.
- **Defaults stay put.** A change to what an existing file converts to by default (orientation,
  units, placement, names) is opt-in behind a flag or option, and the default output stays
  byte-for-byte the same. Never edit golden files, `corpus/placements/` or CI expectations to
  make a change pass: a golden changes only when the pull request means to change that output,
  and the pull request says why for each one.
- **Checks see the source.** Blend proving, the partial-history body comparison and the
  SolidCraft replay compare the written design with the source's own coordinates. A change
  that transforms the whole output (rotating, scaling, re-rooting) runs those checks on the
  untransformed design and applies the transform only where the final file is written; a test
  shows a fillet still maps with it on.
- **Land what works.** A change that needs an unmerged cadmpeg change waits, as a draft, until
  that change is merged and Dutchman's pin includes it. No commented-out code, `TODO` gates,
  `#[ignore]`d tests, or docs and help text announcing something the build can't do yet.
- **Measure the numbers.** Numbers that drive decisions (memory and time estimates in
  `choices.rs`, tolerances) come from measurements written in the pull request, not copied
  from another format.
- **Changelog fragments are for users.** One to three short bullets on what a user gets; the
  codec detail goes in the format's docs page.

## Clean-room rule

Proprietary formats are decoded from public information and from files their owners made
themselves only.

- Never use leaked or confidential material, a CAD vendor's SDK samples or help text,
  decompiled programs, or another product's source code.
- Never add third-party CAD files to the repository: no models from libraries, vendors,
  employers or customers. Test files are synthetic (built in code, from cadmpeg's public
  test suite, or written with cadmpeg's encoders) and their origin goes in the fixtures
  README.
- Never upload a user's design files anywhere while working on a problem with them.
