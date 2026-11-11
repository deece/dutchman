# The command line

## Continue from a desktop result

The desktop **Ask your AI about this file** prompt reads the installed
`AGENTS.md` and conversion skill first (with GitHub URL fallbacks when absent), then the
saved plain `<stem>.report.json`. It names the full path of `dutchman` beside the running app
(`dutchman.exe` on Windows, `Contents/Helpers/dutchman` in a macOS app bundle). There is
no server to connect. Optional follow-up commands
use `--json` and a fresh scratch folder next to the converted design. `inspect INPUT --json`
reads without converting; `diagnose INPUT --out SCRATCH --time-limit 30s --json` tries
parts separately. For a specific trial use
`convert INPUT -o SCRATCH/trial.sc --part P --stop-at-feature N --no-clobber --json`.
`--stop-at-feature` requires `--part`; feature numbers count active timeline features,
and 0 tries stored shape only. Once diagnosis has identified a failing part,
`repro INPUT --out FRESH_SCRATCH --from-diagnose DIAGNOSIS --json` reuses that diagnosis.
Do not run a whole-file conversion again or overwrite the original, converted design,
or saved report. Nothing is sent without asking. See [desktop usage](desktop.md#ask-your-ai-about-this-file).

## Look into a conversion that went wrong: investigate

```sh
dutchman investigate design.f3d [--out FOLDER] [--report FILE] [--no-compare] [--yes | --no-send] [--json]
```

One command for the AI a user asks for help. It reads the report the desktop app saved beside
the design (`design.report.json` or `design.failed.json`), or converts the design once into
the private folder when there is none. A conversion that failed is then narrowed to its
first failing feature, as `diagnose --narrow` does. Everything goes into one private folder,
`design.f3d.investigate` beside the design unless `--out` names another.

The one file made to be shared is `summary.md`: versions, format and outcome, the replays that
may be wrong (a kept feature whose profile the check did not confirm, or a profile region that
doesn't match the source), the first blocked feature of each kind with its place in the
timeline, SolidCraft's own refusals with where they are tracked, the failure and what the
narrowing found, and what the conversion lost. It is built from the
[anonymous report](report-anon.md) data and checked in code before it is written: it holds no
file, part or feature name, no path and no number from the design. If that check fails,
nothing is written and the error says so.

It also converts the design with newer builds and says what each changes: the newest
[nightly](install.md) and every candidate build in [`docs/candidates.toml`](candidates.toml)
made for this platform. They are downloaded from this project's GitHub releases, checked
against the SHA-256 published beside each, and kept in the cache folder (`~/.cache/dutchman/builds`,
`~/Library/Caches/dutchman/builds` or `%LOCALAPPDATA%\dutchman\builds`), so a second run
downloads nothing. No Rust toolchain is needed and nothing about the design is sent.
The summary lists, regressions first, whether each converts, how many features it rebuilds,
and each feature it rebuilds that this version doesn't, or the reverse. `--no-compare` skips
this; `--compare-time-limit` (default 30m) bounds each build's conversion.

Then it asks once: `Send this summary to the Dutchman team? [y/N]`. Yes sends `summary.md`,
and nothing else, through the anonymous report endpoint with the signature `investigate`,
and prints the receipt id. No leaves everything where it is. `--yes` and `--no-send` give the
answer in advance, for an AI the user has already answered. Without a terminal and without
either flag, nothing is sent; the result says to ask the user and run again with one of them.
That second run reuses the summary the first one made instead of converting again; `--fresh`
starts over. `--json` prints [`dutchman.investigate/1`](json.md#dutchmaninvestigate1).

## Project inputs

Files whose names start with `~$` are lock files. They are ignored in project planning
and excluded from job, failure and skipped-file counts.

ZIP directory data and ZIP64 extensible records are limited to 16 MiB each before
the ZIP directory is allocated; entry budgets count every declared entry.

`dutchman convert project.zip` and `dutchman convert project-folder` convert a whole
project. The short forms `dutchman project.zip` and `dutchman project-folder` also work.
The default output is a new folder beside the input, named `project (SolidCraft)`;
use `-o DIR` to choose another. `convert-all` also accepts ZIP inputs and its `--out-dir`
is optional, defaulting beside the first input. ZIP extraction uses a private temporary
folder, with a 10,000-entry and 4 GiB limit; traversal paths are refused, links are skipped,
and extracted files are cleaned up after the run.

The plan reads Inventor and SolidWorks assembly references to select assemblies no other
assembly in the project references. These convert with parts resolved inside the project,
followed by parts no assembly uses. Reference cycles remain visible for error reporting. A STEP export beside a same-name native file is used only if native
conversion fails. The batch report's `plan` describes its conversion list;
`native_failure` retains the native report when a STEP fallback was attempted.

Everything the desktop app does is also a command. `dutchman convert --help` lists every option.

## Output files

Omit the output to write beside the input, using its stem and the extension chosen by
`--to`: `.sc` (default), `.FCMacro`, or `.FCStd` for model output.
Targets before SolidCraft 0.25.1 get `.solidcraft` by default.
`--to dxf` writes one `part-PageName.dxf` per TechDraw page; see
[TechDraw drawing output](freecad.md#techdraw-to-dxf). If a model default exists, Dutchman
exits 2 without changing it: choose `-o PATH` or pass `--force` to replace it. A default
that is the input itself is always refused. `--force` conflicts with `--no-clobber`.
Explicit output paths are replaced unless `--no-clobber` is given. Built-in commands win
over files with the same name; use `dutchman convert pilot` for a file named `pilot`.
Installed tools take precedence over bare files; otherwise an existing file or a recognised
CAD extension selects conversion.

## Options

| Option | What it does |
|---|---|
| `--preset fast`, `--preset standard`, `--preset full` | What to keep; Standard is the default. See [Choosing what to keep](presets.md). |
| `--no-history`, `--import-only` | Leave feature history out; keep geometry and the preset's reference sketches. |
| `--no-sketches` | Leave reference sketches out; sketches needed by retained history stay. |
| `--no-joints` | Leave assembly joints out and retain the selected placements before joint solving. |
| `--part-positions MODE` | Part positions: `smart` (default), `last-shown`, or `first-placed`. |
| `--stored-placements` | Alias of `--part-positions first-placed`: place parts where they were first placed, ignoring later moves. |
| `--no-memory-check` | Bypass the memory refusal, Standard's history budget, and automatic history-check skipping. |
| `--yes` | Never prompt to switch presets; refuse an over-budget conversion with exit 2. |
| `--solidcraft PATH` | The `solidcraft-cli` that checks a converted history. Default: `$SOLIDCRAFT_CLI`, else `solidcraft-cli` on `PATH` or beside `dutchman`, else the one inside an installed SolidCraft app (macOS `SolidCraft*.app`, newest first; Windows `Program Files\SolidCraft`, else the install folder recorded in the registry). The same search runs in the desktop app; see [How Dutchman finds SolidCraft](desktop.md#how-dutchman-finds-solidcraft). When none is found, the report's `solidcraft.search` lists the places tried. |
| `--solidcraft-version x.y.z` | Target SolidCraft release, at least 0.18.2. Default: the found CLI's version capped at newest known, even with `--no-verify`, else the newest known at build time (currently 0.30). |
| `--no-verify` | Write the history without checking it in SolidCraft. Cannot be combined with `--solidcraft`. |
| `--write-replay` | For a `.sc` design, also write `<design>.replay.json` beside it (the design's name with `.replay.json` in place of `.sc`): the command script that rebuilds the history step by step. See [Replay a history by hand](troubleshooting.md#replay-a-history-by-hand). Works with `convert` and `convert-all`; the JSON report names it as `replay_script`. |
| `--verify-timeout SECONDS` | How long each SolidCraft check may run before the history is written unverified. Default 120. |
| `--no-clobber` | Do not replace an existing `.sc` file: that file's conversion fails instead. By default it is replaced. |
| `--project DIR` | The project folder an assembly's part files are searched in when they are not at their stored paths. Default: the nearest folder above the assembly holding an `.ipj`, else two folders up. See [Finding the part files](inventor.md#finding-the-part-files). |
| `--search DIR` | Another folder to search for part files the project folder does not hold, such as a shared library. May be given more than once. |
| `-o PATH`, `--force` | Write to `PATH`; `--force` replaces an existing default output (see [Output files](#output-files)). |
| `--to freecad`, `--to fcstd`, `--to dxf` | Write a FreeCAD macro or document, or TechDraw pages as DXF (see [FreeCAD](freecad.md)). |
| `--threads N` | Bound the assembly part workers; `1` is serial (see [Assembly workers](#assembly-workers)). |
| `--diagnostics`, `--diagnostics-default` | Write an anonymous diagnostic log while converting (see [Diagnostic logs](troubleshooting.md#diagnostic-logs-for-unfinished-runs)). |
| `--json` | One final JSON report on stdout, with progress JSON lines on stderr. |
| `--quiet` | Hide conversion progress and the volunteer invitation. |
| `--verbose` | Print every loss, each note in full. Without it, a file with more than 12 losses is summarised by component (below), repeated notes of the same kind are grouped with a count, and long notes are cut to their first sentence. The JSON report always has them all in full. |
| `--part N`, `--with-children` | Convert only part N, as `dutchman diagnose` numbers it (with the parts inside it, for a sub-assembly). See [Find what breaks in a large private file](diagnose.md). |
| `--log FILE` | Also write the whole text report, every loss in full, grouped by component, to `FILE`. |

History checks remain enabled by default. After reading, Dutchman estimates the extra
memory needed for a SolidCraft rebuild from resident memory, then rechecks free memory
before replay. If it cannot fit, the design is saved with its original shape visible and
retained history unverified. Text output and JSON report
`History check skipped: needs about N GB, M GB free`; JSON also includes
`history_check_skipped` with byte counts. The printed peak estimate includes checking
unless `--no-verify` is selected. A check that cannot fit does not by itself refuse conversion
or remove otherwise retainable history. `--no-memory-check` forces the requested check;
`--no-verify` still keeps checking off, including with that override.

## Find what breaks in a large file: diagnose

```sh
dutchman diagnose big.f3z --out findings [--time-limit 5m] [--memory-limit 16] [--json]
```

Reads the design once, converts each part on its own under a time and memory limit, and
writes `findings/diagnose/report.json` (schema [`dutchman.diagnose/1`](json.md#dutchmandiagnose1)),
`report.txt` and `local-key.json`. The first two name parts by number only and are safe to
share; the key is not. `dutchman convert big.f3z --part N` then converts one part alone, and
`dutchman repro big.f3z --out findings` goes straight to the first failing part. See
[Find what breaks in a large private file](diagnose.md).

```sh
dutchman diagnose --narrow big.f3z --out findings [--step-time-limit 2h]
dutchman diagnose --same-failure small.f3d --out findings
dutchman diagnose --make-report small.f3d --out findings
```

`--narrow` runs the rest by itself: it ranks the parts, isolates a failing part, alone and then
with its neighbours, bisects its history to the first failing feature, and records the failure's
signature, printing `step 3 of 5: narrowing part 17 (feature 41 of 80)` as it goes. Each step has
its own limits. Run it again with the same `--out` to resume after a stop.
`--same-failure` converts a small rebuilt file and compares its signature; it exits 0 only for
the same failure. `--make-report` writes one zip and a prefilled issue link for a file that
passed that check, and sends nothing. `--json` prints
[`dutchman.narrow/1`](json.md#dutchmannarrow1),
[`dutchman.same-failure/1`](json.md#dutchmansame-failure1) or
[`dutchman.made-report/1`](json.md#dutchmanmade-report1).

`dutchman convert big.f3z --part 17 --stop-at-feature 41` converts part 17 with its history cut
after feature 41 (0: no history), counting the features not suppressed, in timeline order. Its
stored shape stays whole.

## Look before you convert: inspect

`inspect` reads a file and says what converting it would carry across. It writes nothing and
does not run SolidCraft.

`design_sketch_constraints.f3d`, from the test fixtures folder ([crates/dutchman-core/tests/fixtures/](../crates/dutchman-core/tests/fixtures/)), is for inspecting only: its history is a
single sketch, with no solid or surface to convert.

```sh
dutchman inspect design_sketch_constraints.f3d
```

```text
Fusion archive (.f3d): 1 body, 1 face
would write: nothing; converting it fails: the model has no solids or surfaces SolidCraft can import
  mapped     sketch "Sketch 1"
dutchman: Degraded: f3d/feature.edge-reference-lost: 1 source parametric edge reference(s) were marked EDGE_REFERENCE_LOST and have no independent complete selection proof.
dutchman: Degraded: f3d/sketch.relation-native: 2 sketch relation(s) retain native operands because no unique neutral relation was resolved.
dutchman: Degraded: f3d/sketch.dimension-native: 1 sketch dimension(s) retain native operands because no unique neutral dimension was resolved.
```

The first line counts the bodies and faces read. The second says whether the design would keep
its feature history or be written as geometry, and why, or, as here, that converting it would
fail and why. Then comes each feature of the history
with its status (`mapped`, `not_needed`, `left_out` or `blocked`, with the reason for a
blocked one), and the loss report. `inspect --json` prints the same as a
`dutchman.inspect/1` report.

## Progress during conversion

`convert`, `convert-all`, `inspect`, and `watch` show the current stage, designs read, and
elapsed time, including the running part or stage and its own elapsed time, on stderr.
Durations use seconds below a minute, minutes and seconds below an hour, and hours and
minutes thereafter, omitting zero units: `45 s`, `1 min 1 s`, `2 min`, `1 h 5 min`, or `1 h`.
The desktop app shows the same. A terminal gets one updating line.
When stderr is redirected or piped, text mode prints no
progress, so logs and scripts see only errors and warnings; `--verbose` asks for a line at
each stage or design completion and a running-step update every five seconds there. `--quiet`
suppresses progress everywhere. With `--json`, stderr contains `dutchman.progress/1` JSON lines; stdout
stays one final report (or the existing watch-event stream for `watch`). Elapsed time starts
with each file, including a separate assembly's part files.

The desktop app shows these updates on each converting file's card. When the design total
is known it shows a determinate bar for designs read; otherwise it keeps a spinner alongside
the stage and elapsed time. The bar measures reading designs, not the remaining conversion time.
Stage ids are `input` (opening the input and finding part files), `decode` (reading geometry and history), `project`
(checking the decoded model), `map` (building and checking the output design), and `write`
(saving the output). A model-check fallback can return to `decode` to read geometry alone.
The fallback keeps geometry and assembly structure, and drops history, sketches, parameters and what named them.
A heartbeat confirms the worker is running; it does not measure decoder activity.

A running step reads **Checking Bracket: 12 s**. Once it takes longer than its usual
time it reads **Still checking Bracket: 1 min 1 s**. This is elapsed time,
not an estimate of time left. With parallel workers the oldest current step is shown;
a check takes the place of preparation on its worker until it ends. Completed-read counts
can stay unchanged while parts are prepared or checked.

Available split reading, conversion and checking estimates supply usual times for whole
stages, including a named whole-design read or preparation. A whole-file estimate is not
assigned to each assembly part or selected-part conversion. Without a matching estimate,
usual-time thresholds are 10 s for opening, checking the decoded model, and writing;
30 s for reading a design or preparing a part; and 60 s for a target-application check.
These per-step thresholds apply when the existing whole-file estimates cannot describe an
individual part or subprocess. They are notices, not deadlines.

SolidCraft's `--verify-timeout` defaults to 120 s per subprocess evaluation. Comparing a
history with the original can run two evaluations, each with its own clock. The line
distinguishes **Checking Bracket (original geometry)** and **Checking Bracket (rebuilt
history)**, so a fresh clock identifies the next evaluation. Blend proofs, retained
history and converted geometry also identify their check. Blend proofs
share that budget across their two runs; retained-prefix checking shares it across repair
attempts. Past a limit the line names the consequence, for example:
**Bracket took longer than the 2 min check limit: the check stopped, and the history is kept
unverified.** Writers configured to hide unverified history keep it hidden beside the
visible import. A pattern that needs replay proof falls back to imported geometry, with
any retained history kept separately. A timed-out blend proof
leaves the unproven feature out of the editable history. A timed-out prefix check keeps
retained history unverified beside the visible import. A geometry check that times
out keeps the converted geometry unchecked. The final report records the actual outcome.

The limit line always ends in a full stop. The name and limit (2 min for SolidCraft,
5 min for FreeCAD by default) are followed by the applicable outcome:

| Check | Outcome text |
|---|---|
| History | the check stopped, and the history is kept unverified. |
| Hidden history | the check stopped, and the history is kept unverified and hidden beside the imported geometry. |
| Pattern proof | the check stopped, and the imported geometry is kept with any retained history. |
| Retained prefix | the check stopped, and the retained history is kept unverified while the imported geometry stays visible. |
| Blend proof | the check stopped, and the unproven feature was left out of the editable history while the imported geometry is kept. |
| Converted geometry | the check stopped, and the converted geometry is kept unchecked. |
| FreeCAD macro (5 min) | the check stopped, and the macro is kept unverified. |
| FreeCAD document (5 min) | conversion stopped, and no output document was written. |


FreeCAD's `--freecad-timeout` defaults to 300 s for its headless run. A native document
conversion stops without writing an output document; a macro-only check keeps the macro
unverified. Reading, mapping and writing have no independent wall-clock limit.
`convert-all --timeout` is a separate whole-file limit (900 s by default): it stops that
file's worker and reports failure. The desktop has no practical whole-file timeout. **Stop** starts no further files;
files already converting finish. Closing the app ends its workers through their parent-exit
guard. The shared CLI supervisor separately allows a 30 s grace period when a shutdown
flag is supplied; the desktop does not supply that flag. CLI version queries stop after 3 s; a CLI that gives no version can still be used for
checks. Memory guards remain independent of these time limits.

Fusion `.f3z` archives are decoded in one call. Member decode attempts can repeat, so
progress does not infer unique design counts or a percentage from them. Optional
[diagnostic logs](troubleshooting.md#diagnostic-logs-for-unfinished-runs) record these attempts. Separate assembly part files report completions as they are read,
with an unknown total while parts are discovered. These updates do not change the final report.

## Convert a whole folder: convert-all

`convert-all` converts every file named, and every file Dutchman knows inside the folders
named (with their subfolders; symbolic links are not followed). The designs go to
`--out-dir` when supplied, otherwise a new folder beside the first input, keeping the folder layout. A file that fails does not stop the others.

```sh
dutchman convert-all --jobs 2 --out-dir converted project
```

```text
Fusion .f3d: reads geometry
converted  project/generated_unit_cube.f3d -> converted/generated_unit_cube.sc (geometry without history)
           Degraded: f3d/material.not-transferred: Materials and appearances were not carried over.
           Info: step/attribute.source-record-not-written: 7 source attribute record(s) were not written to STEP
           Info: step/native.source-record-omitted: 45 source-native record(s) were not represented in STEP
           Info: history: flattened: the design carries no feature history
Fusion .f3d: reads geometry
failed     project/sub/container_metadata_only.f3d: cannot read this design: no geometry could be read from this Fusion archive (.f3d) file
Inventor .ipt: reads geometry
converted  project/sub/synthetic_cube.ipt -> converted/sub/synthetic_cube.sc (geometry without history)
           Degraded: inventor/rse.segment-pair-untyped: Retained 1 structurally paired RSe segment(s) without record semantics.
           Info: step/attribute.source-record-not-written: 7 source attribute record(s) were not written to STEP
           Info: step/native.source-record-omitted: 67 source-native record(s) were not represented in STEP
           Info: history: flattened: the design carries no feature history
2 converted, 1 failed, 1 skipped (not in a format Dutchman knows)
Want to help Dutchman read Fusion, Inventor better? If you have Fusion, Inventor, you can make a few small test files from short recipes: https://dutchman-cad.github.io/fixtures/ or type /volunteer in our Discord (https://discord.gg/yTcseBEdeu).
```

Each converted file's loss report is listed beneath it, summarised by component when it has
more than 12 losses. The whole report, every loss in full, is written to `dutchman.log` in the
output folder (`--log FILE` puts it elsewhere). This log, ordinary JSON, and the journal hold
file names and paths; keep them private. For an issue, request `--report-anon FILE`, review
it, and share that report as [Troubleshooting](troubleshooting.md) shows. Files in the folders that are not in
a format Dutchman knows (`notes.txt`, say) are skipped and counted at the end. The exit code is 0
only when every file converted, else 1. `--jobs N` sets how many files
convert at once (default: the machine's cores, at most 8). It takes the same options as
`convert`, and `--json` prints one `dutchman.convert-all/1` report holding a
`dutchman.convert/1` report per file. One run converts at most 10,000 files and searches
folders at most 32 levels deep.

Large batches:

- **Each file in its own process.** A file that takes longer than `--timeout SECONDS` (default
  900) or holds more than `--max-memory MB` (default `0`, no limit) is stopped and
  reported as failed. Any file also stops if the computer really runs low on memory, and the rest carry on. A file that crashes the converter fails alone.
- **Carry on where you stopped.** `--journal FILE` records each file's result as it finishes.
  Run the same command again and the files already recorded, unchanged since, are not
  converted again; their results come from the journal. Files another version of Dutchman
  recorded, or an older journal format (before 0.1.4), are converted again.
- **Every positional argument is an input.** `convert-all` takes files and folders only; the
  output folder is `--out-dir DIR`. A missing argument that looks like an output path (no
  extension, or an output extension such as `.sc`) draws a warning on stderr before
  anything is converted, and is still reported as a missing input.
- **A report you can share.** `--report-anon FILE` (also accepted by `convert`, for one file,
  with `--report-salt` and `--report-key`) also writes an anonymous report: counts by
  format, every loss and failure by cause and stage, and timings, with no file names, paths or
  geometry. See [Testing Dutchman on your library](testing-your-library.md) and
  [the report's fields](report-anon.md).

Everything runs on your machine: Dutchman makes no network connections.

FreeCAD assemblies can link Bodies and Parts in sibling `.FCStd` documents. Dutchman
tries the assembly's folder first, then the same project and `--search` folders used for
Inventor below. Missing linked files become named losses while the available parts convert.
Link arrays place each element once; grounded parts stay fixed. See [FreeCAD](freecad.md).

For an Inventor assembly (`.iam`), Dutchman finds its part files as Inventor does: at the
paths it stores, else anywhere under the project folder, the nearest folder above it holding a
`.ipj`. Give another with `--project DIR`; `convert-all` searches each folder it is given. See
[Finding the part files](inventor.md#finding-the-part-files).

## Watch a folder

```sh
dutchman watch /mnt/inbox --out-dir /mnt/converted
# One scan for cron (files modified within the last 10 seconds wait for the next run):
dutchman watch /mnt/inbox --out-dir /mnt/converted --once
```

The watcher polls recursively every `--interval` seconds (default 5), keeping relative
folders in the output. It ignores hidden files and folders, `~$*`, `*.part`, `*.tmp`,
`*.crdownload`, unknown formats and symbolic links. Like `convert-all`, it selects the
newest numbered `.prt.N` save and derives its output from the base part name. Same-stem
inputs in one scan receive numbered output names as in `convert-all`.

A file is ready after its size and modification time stay unchanged for `--settle`
seconds (default 10), counted from when the watcher first sees it: a copy can keep its
source's old modification time (Finder, `cp -p`, `rsync -t`), so that time alone does not
mean the copy has finished. With `--once`, which looks only once, a file is ready when its
modification time is at least `--settle` seconds old; future timestamps always wait. Use
`--settle 0` for files already uploaded.
The inbox is only read: mount it read-only in a container if desired. Choose a separate
output folder, with neither folder inside the other and no symbolic links in output subfolders.

Each conversion runs in the existing isolated batch worker. `--jobs` defaults to the
machine's cores, at most 8. `--timeout` defaults to 900 seconds per file, and
`--max-memory` to 0, no limit. A file also stops if the computer really runs low on memory. The same conversion options as
`convert-all` apply, including `--to`, `--import-only`, `--no-verify`, `--solidcraft`, `--solidcraft-version`,
`--project` and `--search`.

For `parts/cube.f3d`, the default results are `parts/cube.sc` and
`parts/cube.report.json` under the output folder. Failures write `parts/cube.failed.json`
there, with the ordinary `dutchman.convert/1` error report. A failed update keeps the
previous design. Successful updates replace the design and report; obsolete report sidecars
are removed only when they still contain the watcher's bytes. Unrelated or externally
edited outputs and sidecars are skipped, as are destinations assigned to another input. `--no-clobber` also keeps existing watcher outputs.

The output folder's `.dutchman-watch.json` records SHA-256 input hashes and outcomes,
plus hashes proving which outputs the watcher owns. It is saved by temporary file and
atomic rename after each result. Recorded content is never retried, including failures
and skips, across restarts and option changes. Keep this state with the outputs; choose a
fresh output folder to retry unchanged files. Run one watcher per output folder.

Each result prints one line with `converted`, `failed`, or `skipped`, a relative input path
and duration. `--json` prints one `dutchman.watch-event/1` object per line instead (see
[JSON reports](json.md)). Hash skips and ignored or unsettled files print nothing.
`--once` scans once, drains all ready work, then exits 0 if no attempted conversions failed,
or 1 otherwise. A previous failure remembered in state does not fail a new run. Ctrl-C or
SIGTERM stops scanning and starting queued work, gives running conversions up to 30 seconds
to finish (their per-file limits still apply), saves state and exits. Inputs changed during
a conversion are skipped without recording that hash, then settled again on a later scan.

## Assembly workers

`convert`, `convert-all`, and `watch` accept `--threads N` for assembly part work.
`0` (the default) uses at most two workers; `1` runs the serial path. A positive number
requests a larger limit. The actual count is limited by the number of leaf parts and
free memory, using CADIR entity counts as well as body counts to estimate each part's
temporary peak, with a floor for mapping and a SolidCraft check process. Preparation
falls back to serial work when its retained results would use more than half of free memory.
Small imported parts use serial SolidCraft preparation to avoid worker overhead. Workers share only the memory that stays free
after a quarter of it (at least 1 GiB) is kept back for the rest of the conversion. The core precomputes component slices and
reuses an identical empty model for assembly holders, then builds the tree in its
original order. SolidCraft STEP restatement, history mapping, blend proving, and per-part
replay checks prepare independently. Parts then enter the design in source order, keeping
IDs, names, blobs, losses, and history records identical. Failed worker preparations are
retried serially at their original positions.

```sh
dutchman convert assembly.f3z assembly.sc --threads 4
dutchman convert-all project/ --out-dir converted/ --threads 1
```

`--jobs` limits simultaneous files in a batch; `--threads` limits part workers within
each file. Account for both when tuning memory use. Native decoding remains serial.
Other writers use the shared slicing stage only. The JSON report's `threads` field records the
requested limit and the number of workers used. `--no-memory-check` does not disable
the part-worker cap.

## SolidCraft versions

Dutchman is made for the current SolidCraft (any release from the last two weeks);
older versions get simplified features, or pass `--solidcraft-version x.y.z` on
`convert`, `convert-all`, or `watch` (at least 0.18.2).
The target defaults to the found CLI's version, even with `--no-verify`, else the
newest minor known at build time (currently 0.30). The current window is 0.18–0.30.
Constant-radius FreeCAD helices keep editable sketch-profile coils only for targets
0.25 patch 1 or newer; older targets preserve the original geometry and report the version needed.
SolidCraft names and status comparisons use minors; patch-sensitive writer gates use full
versions internally. CI tests the
latest published patch of the newest minor, while the oldest window release stays pinned.
Designs open in their target release and newer releases.


Before starting new writer work, run `scripts/solidcraft-window.sh --print` to see the newest
stable release, the oldest release published in the last 14 days (always including newest),
and the separate 0.18.2 opens-in floor. It uses the dated GitHub release list already used by
the window tooling; `--releases saved-releases.json` reads a saved copy offline. Keep the floor,
its CI job, and existing writer checks. New features target only this window: never back-port
one or introduce a gate for an older release. Older targets flatten the new feature with a
named reason and still open. CI's `scripts/check-version-gates.py` checks version expressions
and capability uses against the pull-request base; `scripts/version-gates-allowed.txt` records
historical expressions and may only shrink. Release metadata failures fail the check rather
than falling back to the writer's older pinned `WINDOW_OLDEST` value.


SolidCraft conversion reports include the selected target and the discovered CLI version,
even with verification off. Text output includes `Target SolidCraft 0.30 (found 0.30.0)`,
or `(none found)` when no version can be read. `--solidcraft-version` overrides the target;
otherwise the found supported version is used, capped at the newest known release, falling
back to 0.30. Successful conversions print the same installed-version notice beside the
target line, with the SolidCraft download URL for too-old releases. The CLI never checks
for Dutchman updates over the network. Failed or refused files print neither line.
No installed release means no notice. Other output
formats omit this line and the JSON `solidcraft` object.

CLI version discovery runs at most once per resolved CLI path in each Dutchman process,
including when the CLI gives no answer. The version command has a three-second limit.
Successful versions are also cached across runs in `solidcraft-versions.json` under
`$XDG_CACHE_HOME/dutchman` (or `~/.cache/dutchman`) on Linux,
`~/Library/Caches/Dutchman` on macOS, and `%LOCALAPPDATA%\Dutchman` on Windows.
The disk entry must match the CLI's resolved path, file size, and modification time;
changing the executable causes a fresh probe in the next process. Unreadable or damaged
caches are ignored, and timeouts are remembered only for the current process.
With an explicit target (`--solidcraft-version` or the desktop target selector) and
verification off, Dutchman does not run the version command. Reports reuse a matching
cached version, or leave `solidcraft.found` null when it has not been checked.

## Other Dutchman CAD tools

Install `dutchman-pilot` or `dutchman-fixtures` next to `dutchman` or on `PATH`; the copy
next to `dutchman` wins. `dutchman --help` (or `dutchman` alone) lists installed tools with
their versions. An installed Pilot receives its arguments unchanged, taking precedence over
the built-in Pilot. Without it, builds with the default `pilot` feature use the built-in
Pilot; other missing tools print their installation link and exit 1.

`dutchman printers` and `dutchman print FILE --printer ID` run [Dutchman Print](https://github.com/dutchman-cad/print),
`dutchman-print`, when it is installed.

Pilot-only installations can ship the small `dutchman-launcher` binary renamed to `dutchman`.
It runs installed tools and identifies itself as `dutchman launcher <version>` with
`--version`. Get [Dutchman Convert](https://github.com/dutchman-cad/dutchman) for `convert`,
`convert-all`, `watch`, `inspect`, and `formats`.

## Assembly placements

`--part-positions smart|last-shown|first-placed` selects **Part positions** for `convert`,
`convert-all`, and `watch`:

- `smart` (default): use current poses only when touching joints close, geometry stays inside
  first-placement assembly bounds grown by 10%, and repeated inserted designs move together.
- `last-shown`: use current poses where available, including later moves, then first placements.
- `first-placed`: place parts where they were first placed, ignoring later moves.

Decoder-bound saved-scene displayed poses take precedence in Smart and last-shown,
without the current-pose guards. Ambiguous and unbound rows are skipped. If the decoder
omits the arena or an occurrence has no bound display pose, last-shown relies on current
poses, which may be stale. Smart keeps first
placements when a check cannot be established. Joints work from the selected placements.
`--stored-placements` is an alias of `--part-positions first-placed` and restores the old
placement choice. JSON reports record `choices.part_positions` and per-rule `part_positions`
counts; an Info note summarises them.
