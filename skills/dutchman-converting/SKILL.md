---
name: dutchman-converting
description: Use when someone wants a CAD file (Fusion, Inventor, SolidWorks, FreeCAD, CATIA, Creo View, Rhino, ACIS or STEP) turned into a SolidCraft design (.sc) with Dutchman, through the dutchman-mcp tools or the dutchman command. Covers inspecting first, reader status and refusals (including IGES, Creo and NX), converting one file or a folder, checking the result in SolidCraft, explaining losses in plain words, and making private minimum reproductions of failures.
---

# Converting CAD files with Dutchman

On Windows, the per-user MSI installs `dutchman`, `dutchman-mcp`, and `dutchman-gui` under
`%LOCALAPPDATA%\Programs\Dutchman` and adds that folder to the user PATH. Start a new
terminal or restart the MCP client after installation. MSI copies update from the app's
**Install (closes Dutchman)** button; Scoop copies keep **Update with Scoop**, and portable
ZIP copies offer **Update now**, then **Restart now** after verification and replacement. Uninstall keeps settings and diagnostic logs.

The `dutchman` command also dispatches installed Dutchman CAD tools such as `pilot` and
`fixtures` through `dutchman-<tool>` beside the command or on `PATH`. `dutchman --help`
lists them. These external commands provide their own interfaces; the conversion MCP tools
and JSON reports described here retain their existing schemas.

The full CLI also accepts `dutchman part.f3d` (or `dutchman convert part.f3d`) and derives
the output beside the input from `--to`. An existing derived output is kept with exit 2;
choose `-o PATH` or use `--force` to replace it. `--force` conflicts with `--no-clobber`.
Explicit outputs retain their replacement behaviour. MCP output paths remain required.

SolidWorks parts convert in their saved configuration when its body membership is known.
An Info note counts bodies left out from other configurations and names the saved configuration;
bodies with unknown membership stay.

Dutchman converts native CAD files into SolidCraft designs. Use the `dutchman-mcp` tools when
they are connected (`inspect`, `convert`, `convert_all`, `formats`, `repro`); otherwise run the
`dutchman` command with `--json`. Both return the same reports, described in Dutchman's
`docs/json.md`.

A part that holds an imported solid becomes a base-body Import feature at its place in the
timeline. Later features keep editable history when they map. Stored geometry has no recipe
to lose: the report says, at Info level, “This part holds an imported solid; there is no
feature history to bring over”. A part made only of stored geometry is geometry without
history, with no flattened-history warning.

For `convert`, `convert-all`, `inspect`, and `watch`, progress goes to stderr: stage and
design updates plus a still-working heartbeat every five seconds with elapsed time.
`--json` makes these `dutchman.progress/1` JSON lines; parse stdout as the final report
(or the existing watch-event stream). `--quiet` suppresses progress. Desktop file cards
show the same information, with a determinate bar when the design total is known. Fusion
`.f3z` updates show stage and elapsed time without a percentage; diagnostic member decode
attempts can repeat and do not establish a unique design completion count. MCP and web conversions return only their final reports; no
progress stream is available.

Sketch point-to-line dimensions and parallel-line spacing retain their parameter expressions;
point-on-curve coincidences also carry across. For a size edit, check the editable body's
bbox or volume separately from the final imported shape. Cosmetic metric thread annotations
require resolved cylindrical face geometry; a historical face ID alone is reported as
having no resolved input surface geometry.

When a feature blocks SolidCraft history conversion, earlier editable features and
resolved sketches are kept in a hidden **Editable history (partial)** component. The
original final shape remains visible as an Import after the kept history. Show the
hidden component to continue editing it. The report names the blocker and counts
retained source features; `mode: partial_history` and verification `partial` distinguish
this from full verified history. The reason says whether retained features replayed or
remain unverified. Later sketches with unresolved planes or external geometry are
reported and left out. A sketch that fails replay is retried without constraints, with
the loss reported. If there is no body-building prefix, resolved sketches remain hidden
beside the imported shape for reference. A final shape mismatch preserves the mapped
timeline for correction when it replays; it does not identify which feature needs
correction. SolidCraft's numerical eval includes hidden bodies as well as the visible
final shape.

Assembly visibility reports count source-hidden placements separately from retained
editable history. An Info `visibility` note, `N editable-history component placement(s)
are placed hidden`, counts the latter across repeated and nested placements. Both use
the same bounded hidden-occurrence paths; a Degraded note reports paths left shown
past the 4096-path limit.

## Conversion choices and memory

Use `threads: 0` (default) for automatic assembly part concurrency, or
`threads: 1` for serial preparation; a positive number bounds workers. Part count and
available memory cap the actual count. The report records `threads: {requested, used}`.
The core slices components in parallel and reuses identical empty holder models. SolidCraft
parts prepare independently and enter the design in the original order, preserving the
design and loss order; worker failures retry serially. Native decoding remains serial.
Batch `jobs` limits simultaneous files, while `threads` limits part workers per file.
The CLI equivalent is `--threads N` on `convert`, `convert-all`, and `watch`.
`no_memory_check` does not remove the part-worker memory cap.

Use `preset: "standard"` by default. `fast` keeps geometry at the selected placements,
without history, reference sketches, or joints; `full` keeps everything that maps and
never drops history to fit memory. Standard drops history when its
estimate exceeds free memory and the history-free estimate fits, with one Info line
under `history` explaining the choice. `history: false` (also `import_only: true`),
`sketches: false`, and `joints: false` narrow any preset. Sketches needed by retained
history stay; with joints off, the selected placements stay. FreeCAD output honours history
only, not the sketch or joint choices.

Before a costly conversion, call `convert` with `dry_run: true`; omit `output` if desired.
This pre-reads metadata and bounded ZIP directory information without decoding or
writing. Read `estimate` (`peak_bytes`, integer `seconds`, fractional `seconds_exact`, legacy `band`, `memory_band`, nullable `time_band`, or `null`), `choices`
(`preset`, `history`, `sketches`, `joints`), and `fits` (boolean or unknown `null`).
Estimates use measured read, conversion and checking costs where calibrated, with fixed process overhead separated from content growth.
Calibrated read, conversion and check costs add to the existing numeric totals; no JSON
split fields are added. The desktop shows the read peak and extra conversion and checking
memory separately, with time for each and the same total in the file row. Whole-second
times round up only after adding the stages. Assembly parts
stored in separate files are not pre-read, and archive assembly structure is confirmed
only during normal decoding. A dry run does not promise that conversion will succeed.

The current phase fits use six public Fusion assemblies and five small Fusion, Inventor
and SolidWorks parts, measured on Linux with four requested workers and SolidCraft 0.30.1.
Total-peak RAM bands are ±45% for `.f3d` and ±30% for `.f3z`; Inventor/SolidWorks larger-part
growth remains a ±100% heuristic, and FreeCAD keeps ±50%. Component costs and their worst
misses are documented in `docs/presets.md`. These are empirical total-peak
envelopes, not guarantees for unseen designs or individual phases.

Time fits include the requested checking cost, including default geometry checking with
history off. `seconds_exact` adds fractional phase times and `seconds` is its ceiling.
`time_band` stays null: other platforms, worker counts and timeout settings have no fresh
timing calibration. The old multipart speedup fraction is not reused after the shared-index
performance change. Worker scheduling itself is unchanged. `band` remains the shared
compatibility field; use `memory_band` and nullable `time_band` for separate coverage.

`threads.used` is the largest bounded Dutchman part-worker limit selected across conversion
stages and embedded members. It excludes threads inside SolidCraft child processes and does
not measure simultaneously busy CPU cores. Failed worker tasks can retry serially.

Both converting tools refuse an over-budget run with a message naming the estimate,
free memory, and lighter preset. Prefer that lighter preset. Only use
`no_memory_check: true` when running despite the estimate is intended; it bypasses the
refusal and Standard's history budget, while input and batch worker limits remain.
The CLI equivalents are `--preset`, `--no-history`, `--no-sketches`, `--no-joints`, and
`--no-memory-check`. CLI scripts should use `--yes` to forbid interactive preset
switches; `--yes` and `--json` refuse over-budget runs with exit 2 rather than accepting
extra memory use. Normal JSON reports include `estimate` and effective `choices`.

Keep SolidCraft history checking on by default. The peak estimate includes checking;
geometry-only conversion also checks its converted shape, using a separate measured
allowance. The desktop's **Check in SolidCraft** control applies with history on or off.
Conversion-only preflight can allow a run whose check cannot fit. If memory after reading
will not fit the same extra check figure shown by the desktop estimate, conversion keeps
a usable original shape and unverified
retained history. Explain `history_check_skipped` (`needed_bytes`, `available_bytes`,
`reason`) when it appears: `History check skipped: needs about N GB, M GB free`.
Use `verify: false` / `--no-verify` when checking is unwanted. Only use
`no_memory_check: true` / `--no-memory-check` to force requested checks intentionally;
it does not enable a check the user turned off. Desktop Advanced offers **Convert and
check history even if memory is low**. Automatic assembly preparation uses at most two
workers, reduced for dense parts or a large retained preparation queue.

## Ground rules

- **The user's designs stay on the user's machine.** Never upload, paste, attach or send a
  design file, or its contents, anywhere: not to a web service, an issue tracker, a chat or
  another tool. Work with local paths only. If a bug report needs a file, say so and let the
  user decide what to share; suggest a small design they made themselves for the purpose.
- **Use absolute paths.** Paths are on the machine running the server; a relative one is read
  against the server's working directory.
- **Don't overwrite without asking.** `convert` replaces an existing output file and
  `convert_all` replaces same-named designs in `out_dir`. Write to a new file or folder
  (for example `converted/` next to the originals) unless the user said otherwise.
- **Say "convert", not "import".** The result is a SolidCraft design; the user's file is not
  changed.

## 1. Check the format

Call `formats` for the installed build's formats, status `words` and `converts`.
Its `level` adds cadmpeg's published headline (`L0`–`L9`), or `null` when none is
published; reader status still determines whether conversion can be attempted.
Fusion, Inventor, SolidWorks parts, FreeCAD, CATIA, Creo View, Rhino, ACIS and STEP
can convert geometry. Creo and NX parts are identified by contents
(they share `.prt`) and currently open without geometry, so conversion is refused. SolidWorks
assemblies are recognised without a reader. `input.words` appears on
conversion successes and failures. Reader status describes capability; the actual
history and verification report says what the file kept. CLI `--quiet` hides the volunteer
invitation; JSON and MCP reports never include it.

Directory conversion keeps the highest numbered `.prt.N` revision per part, reports older
saves as skipped, and uses the part's base output name without replacing an existing file.
After a refused conversion (except IGES) or a history that did not fully map, the invitation
links to test-file recipes and offers private help: send the file and its STEP export to
Dutchy in Discord.

What the installed build converts is what its `formats` says; check it rather than this
list when unsure.

For anything else, don't try workarounds such as renaming the file. Tell the user plainly:

- **STEP (`.step`, `.stp`)**: converts through the geometry path; SolidCraft also opens STEP directly.
- **IGES (`.igs`, `.iges`)**: recognised with `input.words: "open it in SolidCraft directly"`
  and refused before decoding: "Dutchman doesn't convert IGES yet: open it in SolidCraft
  directly (File > Import)." Use SolidCraft's own importer. Batch results retain the same
  refusal; do not invite the user to make IGES test files.
- **SolidWorks assemblies (`.sldasm`)** convert placed parts and sub-assemblies. Keep part
  files beside the assembly, or use `project` / `search` (`--project` / `--search`). Missing
  and unreadable occurrences are left out and reported. Unavailable referenced configurations
  fall back to the saved active configuration with a Degraded loss. Suppressed and mirrored
  components follow cadmpeg's loss notes.
- **Creo and NX**: reading geometry is not supported yet.
  Suggest exporting STEP from the program that made the design and converting that.
- **Inventor assemblies (`.iam`)** convert, but need their part files next to the assembly
  (or at the paths it stores). If parts are missing, the report lists them as `Dropped`.

## 2. Inspect first

Call `inspect` with `input`. It writes nothing. Read:

- `conversion.possible` and `conversion.reason`: whether conversion can succeed and why it
  cannot. `ok: true` means inspection succeeded, even when conversion is impossible.
- `geometry.bodies`, `geometry.faces`: what was read.
- `mode`: `history` (editable feature history) or `import` (solid geometry only).
- `verification.reason`: why there is no history, when there is none.
- `history.features`: each feature's `status`; a `blocked` one has a `reason`.
- `losses`: everything that will not carry across intact.

Sweep histories support twist, end scale, supported orientations, and one guide rail, with
single-curve whole sketches accepted as paths or rails. A blocked sweep's reason names
unsupported corner transitions, section interpolation, tangent-path extension or
simplification. Explain that the converted design keeps its original geometry.

Holes keep chamfered entries and tapped holes (the bore plus an ISO metric thread label; a
thread's hand, class, and length are listed as losses). Taper, exit treatments, modeled
threads, and entry recesses with drill points block history: explain the reported reason.
"SolidCraft holes take a drill point only on a plain drilled hole" means SolidCraft 0.21.0
ignores drill points on counterbores and countersinks; conversion keeps the original geometry.

Tell the user what to expect in one or two sentences before converting, for example: "This
part has one body. Its history won't carry across (the fillet's input edges can't be proven),
so you'll get the solid shape without the feature history."

Sheet-metal histories map base flanges and full-edge flanges on unchanged rectangular base
sheets when their own input-edge midpoint, tangent, and vertices prove the pick. Fixed
outer-face heights, inside/outside placement, angles, and bend radii map. Other dimensions
or unproven selections keep geometry and explain the blocked feature. CADIR's hem developed
length has no exact mapping to SolidCraft's outer-face length. Do not claim
that K-factor or relief settings were preserved: CADIR does not supply them, so sheet rules
use SolidCraft defaults. Contour flange, bend, unfold/refold and corner-relief operations
are not present in the pinned CADIR version.

## 3. Convert

- One file: `convert` with `input` and `output` (ending in `.sc`; legacy `.solidcraft`
  and `.solvecraft` names also work). Default names use `.solidcraft` for targets before
  SolidCraft 0.25.1.
- Many files or a folder: `convert_all` with `inputs` and `out_dir`. One failure does not stop
  the rest; the call is an error when any file failed, and each file has its own report in
  `results`.
  Designs and the default log go inside `out_dir`; optional `journal`, `report_anon`, and
  `report_key` files go to their specified paths, which may be outside it.
- `solidcraft_version: "0.25.1"` (CLI `--solidcraft-version 0.25.1` on `convert`,
  `convert-all`, or `watch`) chooses the target release. It must be x.y.z and at least
  0.18.2. The default is the found CLI's version capped at newest known, else the newest known at build time
  (currently 0.30), even with CLI `--no-verify` or MCP `verify: false`.
  Dutchman targets the current SolidCraft with a rolling 14-day window (currently
  0.18–0.30); older targets get simplified features with a named reason.
- `preset: "fast"` when the user wants only the shape. `history: false` (also
  `import_only: true`) removes history while keeping the preset's reference sketches and
  joints. Leave `verify` at its default.

A failed conversion comes back with `ok: false` and `error.message`. The message already says
what to do next; pass it on in plain words. A failed file creates no design. A mixed batch
may still write successful designs, logs, and requested reports.

For unreadable archives, suggest downloading or copying again and checking the extension.

### Reporting a problem

When a file fails, loses something it should not, or looks wrong in SolidCraft, and the
original opens in its CAD program, point the user to Dutchman's
[**A file didn't convert**](https://github.com/dutchman-cad/dutchman/issues/new?template=file-did-not-convert.yml)
form (for a whole library, the **Library test report** form and
`docs/testing-your-library.md`). It asks for the format, the versions, the operating system,
what was expected and what happened, and an anonymous report for that one file:
`convert_all` with only that file in `inputs` and a `report_anon` path (or `dutchman
convert-all --out-dir converted --report-anon report-anon.json /abs/part.f3d`). The user
reads that report before pasting it; ordinary JSON, logs, the journal, the key, and the salt
stay private. Do not attach designs publicly; a user who wants to share a reproducer can
offer it privately in the issue. The issue goes on Dutchman even when the report says
`upstream: cadmpeg` or `upstream: solidcraft`. Draft the issue for the user to post; don't
post it yourself.

When conversion hangs or gets killed, enable a live diagnostic log **before** retrying:
MCP `convert`/`convert_all` accept `diagnostics: "/abs/run.jsonl"`, or
`diagnostics_default: true` for an automatically named log. CLI `convert`, `convert-all`,
and `watch` accept `--diagnostics PATH` or `--diagnostics-default`. Explicit files append;
default folders retain ten timestamp logs. They are Windows
`%LOCALAPPDATA%\Dutchman\diagnostics\`, macOS `~/Library/Logs/Dutchman/`, and Linux
`$XDG_STATE_HOME/dutchman/diagnostics/` (default `~/.local/state/dutchman/diagnostics/`).
The app has **Save a diagnostic log (no names or contents from your file)** and
**Open diagnostics folder** settings, **Show the log** after Stop/completion, and an
unfinished-run notice on next launch. Logs flush every record and sync at least every five
seconds and at stage changes. They hold stages, resource heartbeats, anonymous member
sizes and decoder events, and loss-code counts; no names, paths, contents, geometry, or
messages. Per-member records appear when the decoder reports them. For a slow `.f3z`
preparation, compare `member_classified` records by anonymous member id: `ms` is the total,
and `nested_scan_ms`, `layers_ms` and `metadata_ms` locate the stage taking time. These
records precede member decoding and are capped at 100,000 per conversion. Ask the user to review
and share this JSONL log with the Dutchman issue; keep ordinary logs private. A combined
`report_anon` shares its salt and file ids with diagnostics; keep the salt/key private.
See `docs/troubleshooting.md#diagnostic-logs-for-unfinished-runs` and `docs/json.md` for the schema.

## 4. Verify in SolidCraft

Never report success on the reply alone.

If macOS blocks SolidCraft, follow the report's first-run guidance: open SolidCraft once
from Applications, then click Open Anyway in System Settings > Privacy & Security, and
try again. Dutchman checks whether the quarantine attribute has the user-approved flag.
Never remove or change the quarantine attribute.

- `verification.status` `verified`: SolidCraft replayed the history and it matches the
  original in area and volume within the measuring mesh tolerances. `unverified`: written
  without that check (the reason says why, usually no `solidcraft-cli` was found); mention
  that the history has not been checked. `partial`: an editable prefix and resolved
  sketches are hidden beside the visible imported shape; the reason says whether retained
  features replayed. `flattened`: no body-building prefix could be retained, so the design
  keeps the imported shape and any
  mapped sketches with resolved planes, hidden for reference with no features built on them.
  An Info `sketches` line counts sketches kept and, when applicable, left out.
- Mirrored assembly placements use shared mirrored component geometry, including nested
  reflections. Normal copies keep their original history; mirrored definitions use imported
  geometry only and report history left out. Explain Info `placements` notes as preserved
  reflections, and Dropped `placements` notes as non-finite, scaled, or sheared occurrences
  omitted by zero-based index and kind. Ask the user to check those source placements and
  convert again. Joints touching mirrored components are left out with their pose retained.
- Assemblies with multiple top-level designs group each design's features in uniquely named
  timeline folders, keeping feature order and leaving joints outside folders. Each folder
  added has an Info `timeline folders` note: `"Name": N feature(s) grouped`. Parts and
  assemblies with one top-level design get no folders.
- Open the design and look at it: `solidcraft-cli eval /abs/path/design.sc`, or with
  SolidCraft's MCP server, start it on the design (`solidcraft-cli mcp --in
  /abs/path/design.sc`) and call `measure`.
- For partial history, use SolidCraft's `document.inspect` to check each body's `visible`
  flag; numerical `eval` includes hidden editable bodies in its body count and totals.
  Compare the visible bodies with the source's final bodies.
- For full history or an import, check `body_count` against `geometry.bodies` from
  `inspect`, that no timeline entry has an `error` or meshing warning, and that area
  and volume are plausible for the part (ask
  the user for a known dimension if you have nothing to compare with).
- A face-count difference alone does not establish a defect: SolidCraft can merge faces on
  one surface. Geometry-only output has no independent source measurements; timeline
  errors, unmeshable faces, bodies inside out, solids without volume, and solids opened as
  open shells still matter. Sheets remain exempt from volume and closed-shell checks.
- If something is off, say exactly what ("SolidCraft could not mesh 3 faces"), and don't
  claim the conversion is fine.

## 5. Explain the loss report

Repeated stderr notes of the same kind are grouped with a count. `--verbose`, `--log`,
and JSON retain each individual loss. Fusion’s material warning can also appear for a
default material: the decoded model does not establish that no source appearance was lost.

A Dropped appearance binding names its body or face target. “Its target is absent from the
decoded model” means no decoded geometry can receive it; “its target exists in the decoded
model but has no placed component geometry” means the target was decoded but is not placed.
Bindings to consumed or removed Fusion geometry have a separate explanation. “Its body has
no geometry the STEP writer can write” means the decoded target exists, but its shape cannot
be carried through STEP. Compare the named part with the source design when reviewing these losses.

The raw messages are technical. Group them by level and translate:

- **Info**: no effect on the shape. Usually say nothing, or "a few internal records that
  SolidCraft doesn't use were left out."
- **Degraded**: carried across in a weaker form. Name the thing affected:
  `f3d/material.not-transferred` says "Materials and appearances were not carried over."
  Ask the user to check the colours and set any missing materials in SolidCraft; the IR
  cannot rule out a default-only material. A sketch dimension "retaining native
  operands" means "the sketch is the right size, but that dimension may not drive it."
- **Dropped**: left out. Always mention these, with the part or feature named.
- History: say which feature stopped it, from `verification.reason` or the first `blocked`
  feature, and that the shape itself is complete.

Keep it short, lead with what the user got ("You have an editable design with 7 features"
or "You have the solid shape, without its feature history"), then what is missing. Don't
call the user's file corrupt or invalid; when Dutchman cannot read something, it is not
supported yet.

Offset datum planes need no building feature when their support resolves or all their
consumers carry independent resolved sketch placements. Otherwise the history reason
names the unresolved plane reference; resolved skipped planes count as `not_needed`.

Shells require removed faces from their own historical input topology, a resolved
thickness and inward or outward direction, and one earlier named body. A shell input-face
blocker means the source selection or historical boundary is not supported yet; final-body
faces cannot establish which face existed before shelling.

A through-all cut with no earlier named body is `blocked` before replay, with a reason
asking for its body-producing feature to resolve first. Source storage order does not
change replay order: the neutral ordinal controls it.

## Command line equivalents

```sh
dutchman formats --json
dutchman inspect --json /abs/part.f3d
dutchman convert --json /abs/part.f3d /abs/converted/part.sc
dutchman convert-all --json --out-dir /abs/converted /abs/project
```

Exit codes: 0 converted, 1 could not convert (report has `ok: false`), 2 usage error.

FreeCAD signed horizontal and vertical sketch distances convert to positive dimension
magnitudes at their solved positions. Their signed parameters and expressions remain
available; a dependent magnitude parameter drives each negative distance in SolidCraft.

When a sketch reports `constraints not carried`, translate the count and reason as lost
editability: the solved geometry remains exact. Coordinate, intersection, coradial,
endpoint tangency, curvature, point symmetry, regular polygon, and repeated dimensions can
now carry across. Static patterns keep point and radius associations, but driving count
or span parameters still need a loss. Bounded parabolas and hyperbolas, elliptical arcs
up to one turn, rational quadratic Béziers, equal-weight rational splines, and clamped
polynomial splines map. Periodic splines are clamped exactly by endpoint knot insertion.
Nonuniform spacing and repeated interior knots become chains of exact spline pieces with
shared endpoints, up to 200 pieces; already uniform splines stay one spline. Constraints
that cannot apply to a chain are left out with an Info message. Unequal spline weights
remain unsupported except for the rational quadratic Béziers above. FreeCAD compact periodic sketch knots currently
need normalization in cadmpeg before reaching the mapper. Selected multi-piece elliptical
arcs, unbounded conics, other weighted splines, and text without solved outlines still prevent the sketch's history from mapping.

Fusion consumed tools and removed bodies are omitted only when cadmpeg marks them hidden
and no component owns them, with an Info `bodies` note giving the count. User-hidden
component members stay available; mixed shown/hidden parts are shown whole, with an Info
note. See the Fusion format documentation.

FreeCAD numeric spreadsheet cells and supported formulas can remain editable parameters.
Qualified object properties bind through decoded parameter identities; renaming preserves
links. Text cells are omitted silently, and unused numeric cells get `Info` notes.
A value-only expression loss gives a specific cause: unsupported function or conditional,
unresolved or ambiguous reference, cycle, or unsupported syntax. A verified initial shape
does not certify edit behavior. The current decoder omits named sketch constraint bindings
such as `.Constraints.width`; their dimensions stay at their evaluated values. Object-label
references require decoded label identities. Check geometry after editing parameters; see
[FreeCAD expressions](../../docs/freecad.md#spreadsheet-parameters-and-expressions).

Constant-radius fillets and equal-setback chamfers map when the feature's own input edge
geometry proves which edge each one acts on in SolidCraft's replay before the blend.
Unused FreeCAD binders and datum helpers are `not_needed`, with the Info note
`helper not converted: nothing uses it`. Used datum points and coordinate systems can be
carried by resolved sketch placements, pattern axes, or another resolved datum. A used helper without the required
resolved placement or binder source geometry remains a blocker, with its reason in the report.
These Info notes remain when another feature or a replay mismatch flattens the history.
A body inside a hidden container is hidden even when its own flag says it is shown.

FreeCAD can supply that geometry from an earlier base feature's stored shape, including
when its intermediate body is hidden. Unproven source membership or an ambiguous replay
pick stops body-building history before the blend, with the reason reported. Earlier editable
features and resolved sketches remain hidden beside the imported final shape when possible.
`--no-verify` cannot prove blends.

Whole-sketch hole placements exclude construction entities; explicit entity selections
can still use construction points or circle centres.

For mirror-containing composite patterns and repeated full-turn endpoints, an attempted
replay that cannot finish keeps the original geometry and reports why. Turning verification
off, or having no usable CLI, still writes unverified history.

Composite feature patterns containing resolved mirrors can map to several ordered timeline
features while retaining one result per source feature in the history report. Each stage
repeats the previous instance set. Chained stages require a target of SolidCraft 0.25 patch 1
or newer; older targets report `needs SolidCraft 0.25 patch 1 or newer` and keep the original
geometry. Single-stage mirrors and patterns still map on every supported target.
A full-turn circular pattern with a repeated seed endpoint
keeps each distinct location once. Replay differences still flatten the history and report
why; body-seeded composite mirrors and unresolved planes remain unsupported.

Body mirror patterns and join/cut/intersect body combines (optionally keeping the tool
bodies) can keep editable history; their bodies must be named by earlier mapped features.
Shells and drafts are refused, since their faces are known only on the final body, where
the feature has already removed or tilted them; they wait on input face geometry.
Replacing a source shape with its mirror is not supported yet: SolidCraft's mirror keeps
the original. Cosmetic threads lack a resolved standard and pitch in CADIR, so assigning
SolidCraft's ISO metric designation would invent thread data; these stop the editable prefix.

Constant-radius helical sweeps (FreeCAD AdditiveHelix and SubtractiveHelix) keep their
sketch profile, axis, pitch/height/turn law, handedness, reversed travel, and Boolean
operation as sketch-profile coils only for targets SolidCraft 0.25 patch 1 or newer. Older
targets block the feature with `needs SolidCraft 0.25 patch 1 or newer` and the target version,
preserving the original geometry. Feature parameter
bindings drive dimensions when available; otherwise their resolved values are kept.
Conical helices and spirals remain blocked because SolidCraft's coil has no cone-angle
or radial-growth option. Check replay verification as for other mapped history.

When cadmpeg's model fails its checks, Dutchman retries after removing history, sketches,
parameters and associated design records, including parameter links on joints. A successful
retry retains geometry and assembly structure, validates them, and reports a `Degraded`
`history` loss explaining that parameters were left out and naming the original check failure.
Duplicate parameter names cannot block this geometry-only validation.

FreeCAD assemblies may hold only external `App::Link` references. Keep their sibling
`.FCStd` parts beside the assembly, or supply `project` and `search` folders. Missing links
are named losses in `completeness.left_out`; the remaining components still convert.
Link arrays place each element once. Grounding fixes the part, and unsupported assembly
joints are reported while stored placements are retained.

### FreeCAD output (experimental)

When FreeCAD is the requested destination, pass `to: "freecad"` to `convert` or
`convert_all`; use an output path ending in `.FCMacro` for a single conversion. The CLI
form is `dutchman convert INPUT --to freecad -o OUTPUT.FCMacro`. This does not require the
FreeCAD input feature. Tell the user to run the result in FreeCAD with **Macro > Macros… >
Execute**, or `freecadcmd OUTPUT.FCMacro`, then save the document as `.FCStd` if needed.
The macro embeds original STEP geometry; parts whose features cannot be rebuilt remain
imported geometry, with the mapped prefix and mapped sketches hidden in **Rebuilt history
(partial)** when available. Explain `rebuilt N of M features; kept as reference`,
`history.features` and their reasons, and that the history is unverified until the macro
runs. The original remains available for comparison; a runtime
failure restores it and warns in FreeCAD. Parameter values are kept; source expressions
remain inert text. See `docs/freecad.md#freecad-output-experimental` for the supported cases and limits.
Assembly macros also write supported joints (through sub-assemblies too) with frames,
limits, and grounding. Explain `joints` losses: unsupported operands/options retain stored
placements, and an ungrounded joint chain gets an inferred grounded part (`Info`). A failed
Assembly solve removes the joints, restores stored placements, and warns in FreeCAD.
`import_only` leaves joints out.

For CLI conversions, `--to fcstd -o OUTPUT.FCStd` runs FreeCAD headlessly, saves the native
document and verifies every part against embedded STEP. `--to freecad --verify` checks a
macro without saving a native document. Set `FREECAD_CMD` to one headless executable path (or the Flatpak form)
when discovery cannot find it; `--freecad-timeout SECONDS` defaults to 300 and kills a slow
process. Explain `differs` using the named part and reason; a runtime fallback restores
STEP and adds a specific loss. A macro may be written with `not checked: FreeCAD not found`;
native output requires working FreeCAD. These extra flags are CLI-only and experimental.

For browser conversions, consult [Dutchman Web](../../docs/web.md) for upload and storage
limits and HTTPS setup. A 403 means the browser origin was refused; a 503 means capacity is
unavailable. Retry after saved jobs expire or adjust the documented server settings.

## Watch an inbox with the CLI

For a local, self-hosted inbox use `dutchman watch INBOX --out-dir OUTPUT --json`.
Polling defaults to 5 seconds and settling to 10 seconds (`--interval`, `--settle`).
`--once` handles one scan for cron, and uses the usual bounded batch child workers and
conversion flags. The inbox is only read. Results and `dutchman.convert/1` report sidecars
(`.report.json` or `.failed.json`) go under OUTPUT with relative folders preserved.
Existing unrelated outputs are skipped. SHA-256 state in `.dutchman-watch.json` prevents
retries of recorded content, including failures, after restart. `--json` is an NDJSON stream
of `dutchman.watch-event/1`, not a batch report. Ctrl-C/SIGTERM allows running work a
30-second grace period and saves state. This command has no MCP counterpart.

FreeCAD TechDraw drawing output is available from the CLI:
`dutchman convert part.FCStd --to dxf [--page NAME]`. It writes one
`part-PageName.dxf` per page; `-o` accepts an existing directory or a file base
(an exact file path with `--page`). Defaults require `--force` to overwrite;
explicit outputs replace unless `--no-clobber` is set. Page names are FreeCAD
object names. Pages keep neutral view curves, dimension labels, annotations, template frames, and
resolved title-block text in millimetre page space. Unsupported content is reported
in per-page JSON `pages` results. No page is an error. See [FreeCAD coverage](../../docs/freecad.md#techdraw-to-dxf).
DXF conversion is not supported yet by convert-all, the MCP convert tool, desktop,
or web. The MCP formats report includes the additive `drawings` capability list.

The desktop app also offers Fast, Standard (default), and Full presets, with Advanced
checkboxes that leave history, sketches, or joints out. Estimates below 0.1 GB show
**under 0.1 GB**, and times below 60 seconds show **under a minute**, using the CLI’s text
formatter. Its memory warning offers the same choices with live costs and a total matching
the file row. Checking is counted only with a usable CLI and checking on. **Convert with
these choices** checks queued files again when the conversion fits; if it still will not
fit, proceeding bypasses memory checks for that run only. Choices and the checking preference
return to the main window and are remembered. Numeric MCP estimates stay unchanged. History
Info messages, including history left out by Standard, appear on its result cards.

SolidCraft conversion reports include `solidcraft.target` and `solidcraft.found` (null
when no CLI version is available), even with verification disabled. Standard decides
whether history fits before decoding, for both parts and assemblies; an accepted
history-free budget cannot turn into a memory refusal after decode. Default diagnostic
log filenames use UTC. Heartbeats record process memory and CPU time on Linux, Windows and
macOS.

With an explicit target and verification off, Dutchman uses only cached CLI versions for
`solidcraft.found`; null can mean not checked. Version probes run at most once per resolved
CLI path per process, with a three-second limit. Successful versions also use a disk cache
keyed by the CLI path, size, and modification time; see `docs/guide.md` for its location.

SolidCraft reports also include `solidcraft.known_newest` (a version string) and
`solidcraft.status`: `newer`, `too_old`, `older_loses_history`, `current`, or
`none_found`. History notices follow the selected target’s writer gates. These
fields require no network check. See [JSON reports](../../docs/json.md).

SolidCraft names and report targets use minors: `solidcraft.target` and
`solidcraft.known_newest` are `x.y` strings such as `"0.30"`. `solidcraft.found`
retains the full installed version. All patches of the newest minor count as current;
writer gates still use full versions internally. Explicit target inputs remain `x.y.z`.

Diagnostic end records include anonymous failure stage, category, check-code counts, and
available referenced curve kinds. Timed phase records distinguish container preparation
from member decoding and model validation. Geometry-only recovery reuses the decoded model
and runs only for failures involving history, sketches, or parameters; a geometry failure
says the retry was not needed. Do not suggest repeating a long decode to bypass those checks.

## Make a minimum reproduction

Use this when a conversion fails and the original file must stay private.

1. Run `dutchman repro FILE --out FRESH_FOLDER`, or MCP `repro` with `input` and `out`.
   A kit with `status: no_failure_to_reproduce` means this conversion succeeded; stop
   and explain that there is no failure to reproduce in this run.
2. Read `kit/anonymous.json` and `kit/next-steps.md`. Use `kit/local-key.json` only in
   a client keeping those names local. A remote chat assistant must ask the user to
   look up the anonymous ordinal themselves, without pasting the key or names into chat.
   MCP omits key content by default; `include_local_key: true` explicitly requests a
   local-only response. Keep the response envelope's local paths out of the issue too.
   Do not read, upload or paste the original design, geometry, ordinary logs or journals.
3. Explain the stage, finding-code set, counts and narrowing limits. Coach one CAD step
   at a time using `next-steps.md`: start a new design, build a simple body, save it,
   and rerun `repro` into another fresh folder. Let the user do the native CAD operations
   unless they have asked for local CAD automation. Do not claim a feature caused a
   geometry check merely because it references the final body.
4. Compare each candidate with the original anonymous report. Keep the same failure
   stage and complete finding-code set; counts may decrease, such as 88 to 1. If no
   structured codes are available, explain that a matching category is only a candidate
   and cannot prove the same failure. Respect the reported limits instead of inventing
   an isolated unit. Continue local reduction or follow the generic body-copy recipe.
   A copied body is for local investigation; rebuild with simple, freely shareable
   dimensions before proposing a shareable file.
5. Once the small rebuilt design reproduces, draft an issue for the user to review and
   post on Dutchman's issues. Use only anonymous.json for the title and technical facts:
   versions, format, stage, category, finding codes/counts, kinds and anonymous narrowing
   path. Remove source names, paths, entity ids and geometry/parameter values. If rebuilding
   has not reproduced, draft an anonymous-facts-only issue and state that limit.
6. List only anonymous.json and the small rebuilt file as proposed attachments, with the
   file included only if it reproduces and the user is free to share it. Never attach the
   original file or local-key.json. Do not post, attach or upload anything yourself.

The [minimum-reproduction guide](../../docs/minimum-reproduction.md) has a worked
synthetic example and a prompt usable with any chat assistant or MCP client.

## Project inputs

`convert_all` accepts ZIP projects in `inputs`, using private bounded extraction (10,000
entries, 4 GiB). It reads Inventor and SolidWorks assembly references, converts assemblies
no other assembly references with their project parts, then converts unused parts. Adjacent same-name
STEP exports are fallbacks when native conversion fails. Read the batch `plan` and the
fallback result's `native_failure` when explaining outcomes. On the command line,
`dutchman convert project.zip` or a folder works directly and defaults to a new
`Project (SolidCraft)` folder beside the input; `convert-all --out-dir` is optional.

Project planning ignores `~$` lock files for every format; they produce no conversion,
failure or skipped-file count.

ZIP directory data and ZIP64 extensible records are limited to 16 MiB each before
the ZIP directory is allocated; entry budgets count every declared entry.

### Anonymous narrowing and rebuilds

Use MCP `diagnose_narrow(input, out)` to rank, isolate and bisect with checkpoints. Results
include a numbered recipe for Fusion, Inventor, SolidWorks and FreeCAD and any exact shipped
known-failure advice. Prefer a workaround when one is reported. For individual trials use
`convert_part(input, part, neighbours?)` and `stop_at_feature(input, part, feature,
neighbours?)`; output geometry is discarded. Trial feature 0 keeps stored shape only.
`diagnose(input, out)` ranks without narrowing. These tools accept `time_limit` and
`memory_limit_mb`; their responses exclude names, paths and geometry.

Ask the user to rebuild the candidate recipe with simple round sizes. Run
`same_failure(input=<small file>, out=<original results>)`, explain any differences, and
only then `make_report(input=<checked small file>, out=<original results>)`. The ZIP is
local at `out/diagnose/dutchman-report.zip` and contains geometry. Do not read the design,
local-key.json or the ZIP into model context. Have the user review it locally and choose
what to share. No diagnostic tool sends anything. Use `doctor` for folder checks, as described below.

## Folder doctor and matching STEP

Assembly completeness separates `placed` (geometry or child holders) from
`placed_empty` (leaf parts with no bodies), with both included in `placements`.
The summary and desktop and web cards show empty placements separately and mark the
assembly incomplete. Empty leaves add Degraded losses naming their anonymous part
numbers; child holders do not. Solid regions reduced to open sheets add a counted
Degraded geometry line; intended sheets do not. Doctor includes anonymous placement
counts and findings for empty placements and solids reduced to sheets.

For a folder or ZIP, use MCP `doctor` (`input`, `out`, optional `jobs`, total `memory_mb`,
per-part `time_limit`, `step_path`, `part`) or `dutchman doctor FOLDER --out CHECKED
--jobs 2 --memory-mb 8192 --json`. Repeat the same command to resume. Only top-level
`report.json` is anonymous; checkpoints, the local key and converted designs stay private.
Findings group signatures locally with counts; broad categories are candidates only.

MCP `diagnose` accepts `input`, `out`, optional `narrow`, `step_path`, `part`,
`time_limit`, `memory_mb` (or `memory_limit_mb`, never both). Durations accept whole
seconds or strings such as `30s`. `narrow: true` retains the recipe and known-problem
results of `diagnose_narrow`; ordinary diagnosis returns an anonymous `report` wrapper.
`compare_step` accepts the same measurement arguments without
`out` or `narrow`. CLI: `dutchman diagnose FILE --step EXPORT.step --out CHECKED`.
Unambiguous same-name adjacent STEP exports are discovered automatically. Tool output
contains anonymous numbers only. STEP body numbers differ from assembly diagnose part
numbers; never use a comparison body number as `convert --part` without resolving it
locally. `--part` on doctor/diagnose records a target and does not limit comparison.
Thresholds: 1% volume/area; 0.1% diagonal clamped to 0.1–0.5 mm for bbox and placement.
Face counts alone never flag differences. `incomplete`/`unavailable` is not a pass.
Checks are advisory; confirm findings before reporting them. The observed body-row
false-positive rate on the two placement-golden assemblies is 0/329 (0%); incomplete
rows are not full geometry passes. Placement sanity follows the selected joint pose.
`geometry_untrimmed_torus` means a toroidal face with no trims shares a shell with
other faces. Review the boundary; a whole torus in its own shell is allowed.
Nothing is sent; the STEP itself can be shared only through the private-send flow after the user chooses it.
Doctor's `geometry_checks` compare converted bodies with stored source geometry,
using the conversion's joint setting and assembly pose, except
partial-history outputs with intentionally extra hidden bodies. Private whole-design
snapshot pairs may accompany flags when SolidCraft supports them; never attach these
to an anonymous public report. A file check's Target part buttons save a local target.

For a converted result with missing features, wrong placement or missing colours, use a
fresh report folder with `report_preview`, the diagnostic file id, source format and
`signature: "converted-with-problem"`. Review its `metadata`; submit only summary after
explicit consent. CLI users can preview with `dutchman report --folder FOLDER --report-id ID
--source-format f3d --signature converted-with-problem` and send after review with
`dutchman report --folder FOLDER --send-summary --yes`. No free-text description is
supported in the anonymous envelope. Describe the problem privately or in the GitHub
form without private names or paths. The desktop offers these actions under the closed
**Something wrong? Report this file** section. Failure narrowing is for failed files.

For report sending, call `report_preview` first and show the user the choices, part
roles and sizes. Never show or transmit bundle contents through an assistant. Call
`report_submit` with `explicit_yes: true` only after the user's explicit yes to that
report, its bundle/summary choice and separate public-test permission. The public-test
box defaults false. Whole-file sending uses the private-send card. On delivery failure
keep the local report; retry later only when the user asks. GitHub gets the anonymous
summary only, never a geometry bundle. The desktop app selects summary when no bundle is available;
Send becomes Retry only after delivery fails. Optional Discord claims are copied locally and pasted by the user.

Report summaries identify the authoring app from the native format, such as Fusion or
Inventor. Exchange formats and unclassified `.prt` use `unknown` because their authoring
app cannot be inferred. The build version is included alongside this information.

When no proven bundle exists choose summary or the private-send card.

For a check failure, `dutchman diagnose INPUT --bundle --out OUT` or MCP `extract_repro`
cuts and proves a private local fragment without narrowing. MCP `verify_repro` rechecks
the saved report folder. Review `report_preview`: it shows only the manifest, roles and
sizes. Bundles contain numeric geometry and default to the bundle choice when available.
Obtain explicit consent for sending. The complete multipart request must fit 25 MiB, including metadata and overhead;
bundles reserve 64 KiB below that cap. Check `bundle_sendable` in the preview and
use the private-send card or summary when it is false. Decode refusals have no bundle yet; never substitute the original as a bundle.

Treat refused cuts as a normal no-bundle outcome, including large assembly closures
that exceed the fragment byte limit. Offer summary or the private-send card.

### Finding a run's diagnostic log

Automatic diagnostic filenames include UTC date and time, Dutchman version and result.
They start as `running` and become `converted`, `failed` or `stopped` once writers close;
a crash or rename failure leaves `running`. Retention keeps 50 logs, including legacy
time-only names. Explicit `--diagnostics PATH` files keep their exact path. A version notice
when reusing an older log is advisory; conversion proceeds. Desktop result cards offer
**Show this run's log** and **Copy log to…** for the shared batch log, while retries keep
other cards linked to their original run. Report review and the copied Discord DM identify
that run's log. The log contains no names or contents from the design. Send uploads only
the summary or bundle reviewed in the consent screen; the log stays local.

## Start from an existing desktop result

When the person wants to know what went wrong, start with `dutchman investigate ORIGINAL`
and follow the procedure in the installed `AGENTS.md`: read the `summary.md` it writes,
explain it, ask the one question whether to send it, then run it again with `--yes` or
`--no-send`. The rest of this section is for going further when they ask.

The desktop **Ask your AI about this file** button copies a short prompt naming the original
file, the converted design, the saved `<stem>.report.json` (`<stem>.failed.json` when the
file did not convert, with no design) and the log of that run, plus this app's `AGENTS.md`.
Open the report directly from disk: it is plain `dutchman.convert/1` JSON. Explain the saved
losses grouped by code and stage, feature outcomes (including history parts), placements and
verification, and say what the report proves and what stays unverified. The log is JSON
lines with the stage reached, timings and loss counts; it has no names or contents from the
design. Do not convert the whole file again, and no server installation or connection is
needed.

Use the `dutchman` executable beside the app, by its full path (`Contents/Helpers/dutchman`
inside a macOS app bundle), for focused follow-up only when a loss needs more evidence, and
with `--json`, redirecting each result to a file in a fresh scratch folder next to the
design (if it exists, pick an unused numbered suffix). `inspect INPUT --json` reads without
converting. `diagnose INPUT --out SCRATCH/diagnosis --time-limit 30s --json` reads the
original and tries parts separately. For a known part use `convert INPUT -o
SCRATCH/trial.sc --part P --stop-at-feature N --no-clobber --json`; P is the part number
diagnosis found, N counts active timeline features and 0 means stored shape only; always
keep `--part` on a trial. Only after diagnosis has identified a failing part, use `repro
INPUT --out SCRATCH/repro --from-diagnose SCRATCH/diagnosis --json` to reuse it. Keep every
new output in the scratch folder and never overwrite the original, the converted design,
the saved report or the log. The paths, the report and any reproduction geometry are private:
do not upload files or file an issue without the person's permission, and ask before
sending anything anywhere.

If a local MCP client is already connected, optional `read_report(input)` reads the same
saved report without decoding; `.sc` resolves to the adjacent `.report.json` and explicit
`.failed.json` works too. It returns every original field plus `loss_groups` entries with
`code`, `stage`, `count`, and original `losses`. Unlike anonymous diagnostic tools, this
local tool returns private paths, names and messages, like `convert` and `inspect`.
Keep the report and reply local; do not put them in an issue or remote assistant.

### Assembly placements

Part positions defaults to Smart. Use CLI `--part-positions smart|last-shown|first-placed`,
MCP `part_positions`, or desktop Advanced → Part positions: Smart / As Fusion last showed
them / As first placed. The desktop remembers the choice. First-placed restores the old
placement choice; `--stored-placements` and MCP `stored_placements: true` remain aliases.
Smart admits current poses only when touching joints close, geometry fits inside
first-placement assembly bounds grown by 10%, and repeated inserted designs move together.
Missing or unsupported checks keep first placements. Last-shown uses available current
poses without those checks; these can be stale. Joints work from the selected placements.
Smart and last-shown prefer decoder-bound saved-scene displayed poses without current-pose
guards. Ambiguous and unbound rows are skipped; an absent arena supplies none. An unbound
scene matrix is not evidence for a named occurrence's placement. Explain the Info
summary and JSON `part_positions` rule counts and fallback reasons. Requested selection is
`choices.part_positions`; compatibility `choices.stored_placements` is true only for
first-placed.

Missing assembly holder poses are inferred bottom-up from displayed children only when
their world-pose estimates agree within 0.001 mm and 1e-6 rotation entries and reproduce
every displayed child. A conflicted holder keeps its first placement, even inside a holder
that moves, and is reported. Smart also keeps the first placement of a displayed holder (and
everything inside it) when a part below it would leave the bounds grown by 10%. JSON `part_positions` adds `derived_holders`
and `holder_conflicts`; `bound_displayed` and `derived_holders` partition `displayed`, preserving the displayed/current/first-placement total.

CLI and desktop progress includes the running part or stage and its elapsed time. “Still …”
means the usual duration has passed, not a deadline. A check-limit notice states the actual
retention or stop policy in a full sentence. Durations use a single format such as `1 min 1 s`
or `2 min`. Desktop folder runs keep the longest-running file, stage and step visible below
the completed count, with the number of other active files. Names in progress are private local text; do not copy them into
anonymous diagnostic reports. See docs/command-line.md for per-step limits.
