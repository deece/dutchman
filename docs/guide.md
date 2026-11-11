# Getting started

## Ask your assistant about a saved desktop result

A desktop conversion saves `<stem>.report.json` beside its design (`.failed.json` for a
failure), using the CLI's full per-file JSON schema. These reports contain private local
paths. A converted card with losses (and a failed card) has **Ask your AI about this file**: it
copies a short prompt, shown under the button too, naming the original, the design, the saved
report and the run's log. Paste it into an assistant with local file and command access.
It sends the assistant to the installed `AGENTS.md` (or its GitHub copy when absent), which
explains how to read them and the `dutchman` commands to use in a fresh scratch folder. It does
not require a server connection and asks before sending anything. See [desktop usage](desktop.md#ask-your-ai-about-this-file).

## Project inputs

Files whose names start with `~$` are lock files. They are ignored in project planning
and excluded from job, failure and skipped-file counts.

ZIP directory data and ZIP64 extensible records are limited to 16 MiB each before
the ZIP directory is allocated; entry budgets count every declared entry.

Folders and project ZIPs can be passed directly to `dutchman convert` or `convert-all`.
The default output is a new `Project (SolidCraft)` folder beside the input. Project planning
selects top-level Inventor and SolidWorks assemblies by their references, converts them
with parts resolved inside the project, then converts parts no assembly uses. Same-name adjacent STEP exports are fallbacks
for failed native conversions. See [Project inputs](command-line.md#project-inputs).

Dutchman Convert turns native CAD files into [SolidCraft](https://github.com/tangency-labs/solidcraft)
designs (`.sc`). This page takes you from install to an open design in four steps: install, convert, read the report, and open the result; the
[docs list](../README.md#docs) has everything else.

What changed in each release: the [0.2.6 release notes](../release-notes/0.2.6.md) and the
[changelog](../CHANGELOG.md).

A design stores its geometry compressed, so a large assembly stays small: SolidCraft refuses design files over its limit (256 MB by default).

## Install

- **Mac:** `brew install --cask dutchman-cad/tap/dutchman-convert` for the app, or
  `brew install dutchman-cad/tap/dutchman` for the commands.
- **Windows:** the [per-user installer](install.md#windows-the-installer) adds the app to Start
  and the commands to your user PATH without administrator rights. Or `scoop bucket add dutchman-cad https://github.com/dutchman-cad/scoop-bucket`,
  then `scoop install dutchman-cad/dutchman`.
- **Any system:** a [release archive](install.md#from-a-release-archive), [Docker](docker.md),
  or [from source](install.md#from-source).

[Install](install.md) has every option, checksums, and the one-time
[first-run approval](install.md#opening-the-app-the-first-time) on macOS. Install
[SolidCraft](install.md#solidcraft-optional-recommended) too: Dutchman uses it to check each
converted history. The supported SolidCraft window is currently 0.18–0.30.

## Your first conversion

You need a native CAD file. Dutchman has no public sample designs yet, but its tests use
small synthetic files that are fine to try: they are in
[`crates/dutchman-core/tests/fixtures/`](../crates/dutchman-core/tests/fixtures/), with
where each came from in that folder's README. None is a real design. `solidworks_cube.sldprt`
is a 10 mm cube; `generated_unit_cube.f3d` and `synthetic_cube.ipt` are the same cube as
Fusion and Inventor files. To download them:

```sh
fixtures=https://raw.githubusercontent.com/dutchman-cad/dutchman/main/crates/dutchman-core/tests/fixtures
curl -fLO "$fixtures/solidworks_cube.sldprt"
curl -fLO "$fixtures/generated_unit_cube.f3d"
curl -fLO "$fixtures/synthetic_cube.ipt"
```

```sh
dutchman convert solidworks_cube.sldprt cube.sc
```

```text
SolidWorks .sldprt: reads geometry
converted  solidworks_cube.sldprt -> cube.sc (geometry without history)
dutchman: Degraded: sldprt/source.dialect-unverified: the document carries no swSolidWorks swVersion declaration, so no declared identity was verified. …
dutchman: Degraded: sldprt/config.inferred-without-native: 1 configuration state(s) are inferred from geometry partitions without native configuration definitions.
dutchman: Info: step/design.parametric-records-omitted: 1 parametric/design record(s) were not represented in STEP: configuration=1
dutchman: Info: step/native.passthrough-record-omitted: 2 uninterpreted passthrough record(s) were not represented in STEP
dutchman: Info: step/native.source-record-omitted: 1 source-native record(s) were not represented in STEP
dutchman: Info: history: flattened: the design carries no feature history
dutchman: (repeated notes are grouped and long notes shortened; --verbose prints every note in full)
Want to help Dutchman read SolidWorks better? If you have SolidWorks, you can make a few small test files from short recipes: https://dutchman-cad.github.io/fixtures/ or type /volunteer in our Discord (https://discord.gg/yTcseBEdeu).
```

The synthetic SolidWorks cube lacks native version and configuration metadata, so those
warnings are expected for this sample. The internal records omitted from STEP do not change
its shape: the written geometry should be one 10 mm cube (1,000 mm³). The sample has no
editable feature history. Check the result in SolidCraft as [below](#open-the-result-in-solidcraft).

The exit code is 0, and the first line, on stdout, says what was written where: here the
geometry, as this file has no feature history (else `editable history, verified in SolidCraft`
or `editable history, unverified`). After progress updates, the lines on stderr are the loss report, explained
[below](#reading-the-loss-report). Long notes are cut to their first sentence, marked `…`;
`--verbose` prints them whole. An explicitly named existing output file is
replaced (`--no-clobber` keeps it, and the conversion fails instead), and nothing is written
when the conversion fails.

SolidCraft output accepts `.sc`, `.solidcraft`, and `.solvecraft`; each uses the same container.
Other targets use their own extension.
To get one machine-readable report on stdout instead
of the text, add `--json` ([json.md](json.md) describes every field):

```sh
dutchman convert --json generated_unit_cube.f3d cube.sc
```

```json
{"history":{"features":[],"mapped":0,"total":0},
 "input":{"format":"fusion_archive","format_name":"Fusion archive (.f3d)","path":"generated_unit_cube.f3d"},
 "losses":[
  {"code":"f3d/material.not-transferred","level":"Degraded","message":"Materials and appearances were not carried over.","subject":"f3d/material.not-transferred"},
  {"code":"step/attribute.source-record-not-written","level":"Info","message":"7 source attribute record(s) were not written to STEP","subject":"step/attribute.source-record-not-written"},
  {"code":"step/native.source-record-omitted","level":"Info","message":"45 source-native record(s) were not represented in STEP","subject":"step/native.source-record-omitted"},
  {"code":null,"level":"Info","message":"flattened: the design carries no feature history","subject":"history"}],
 "mode":"import","ok":true,
 "output":{"format":"solidcraft/23","path":"cube.sc"},
 "schema":"dutchman.convert/1",
 "verification":{"reason":"the design carries no feature history","status":"not_applicable"}}
```

(Printed on one line; wrapped here.)

SolidWorks parts convert in their saved configuration when its body membership is known.
An Info note counts bodies left out from other configurations and names the saved configuration.

SolidWorks assemblies (`.sldasm`) convert with their external part files. Keep parts beside
the assembly or give `--project DIR` and `--search DIR`. Missing or unreadable occurrences
are reported and left out; unavailable referenced configurations use the saved active
configuration with a Degraded loss. See [SolidWorks assemblies](sldprt.md#assemblies-sldasm).

Before a history is built, Dutchman checks each extrude's profile region against the
source. When the design's only geometry-building feature is a blind one-sided extrude whose
end face survives in the stored model, the area of the region the extrude sweeps must match
that face's area (within 1 mm² or 3%, which absorbs arcs measured as fine polygons); a mismatch blocks the extrude, and the history stops
there. An extrude whose region cannot be checked this way stays kept and is marked
`profile_unverified` in the JSON report (see [json.md](json.md)).

History checks stay on by default and fit themselves to free memory. If a second build
will not fit after reading the design, conversion still saves a usable original shape
with retained history unverified; the card and report say
`History check skipped: needs about N GB, M GB free`. Peak estimates include checking.
The runtime guard uses the same calibrated extra check figure as the desktop estimate,
without multiplying the memory already held by the decoded design.
Geometry-only conversions also check the converted shape by default. Their allowance is
calibrated separately; **Check in SolidCraft** can turn checking off in the desktop.
Use `--no-verify` to keep checks off or `--no-memory-check` to force requested checks.
Desktop Advanced has the matching **Convert and check history even if memory is low**
control. Automatic assembly preparation uses at most two workers and reduces that count
for detailed parts or large retained preparations. See [presets](presets.md).

The desktop shows the same estimate in the file row and starts converting when you press
Convert; see [the desktop guide](desktop.md).

## Reading the loss report

An assembly's conversion first says how complete it is, under the line saying what was
written:

```text
converted  top.iam -> out/top.sc (geometry without history)
assembly: 18 of 20 placements placed; 2 left out:
  pin.ipt ×2: not found at its stored path or in the project folder /work/Robot
```

Empty leaf parts are counted separately as `placed empty`, and make the desktop and
web result card Incomplete. For example, `67 of 88 placements placed; 13 placed empty;
8 left out` means 13 component placements retain their joints but have no bodies.
Each empty leaf adds a Degraded line naming its anonymous part number, as used by
`diagnose` and `--part`. An assembly holder of children is not an empty leaf.
When a solid region becomes an open sheet, a Degraded geometry line counts the
affected regions. Intended surface bodies are not reported as degraded for being open.
Doctor includes these findings and the assembly's placement counts.

Mirrored assembly placements use a shared mirrored copy of the component's geometry;
ordinary copies keep the original. An Info `placements` note counts the converted
reflections. Mirrored copies retain imported geometry only, with a history note when
needed. Non-finite, scaled, or sheared placements beyond tolerance are left out with a
Dropped `placements` note naming the occurrence index and kind. Check those placements
in the source and convert again. Tiny numeric drift is corrected.

The count takes in every placement the assembly and its sub-assemblies record, each
sub-assembly as often as it is placed. Each cause names the file and how many placements it
cost. A part not found usually means its project folder is elsewhere: give it with
`--project`, or a library with `--search` (see
[Finding the part files](inventor.md#finding-the-part-files)). `inspect` says the same as
`would place: …`.

Every conversion lists what it could not carry across intact. A design with more than 12
losses, an assembly's usually, gets them by component instead, the design itself first, then
the parts with the most that matters:

```text
losses: 182 in 14 components
  the design: 1 Degraded, 3 Info
  pin.ipt: 1 Dropped
  base.ipt: history flattened (feature "Shell1": a native face reference is not supported yet: cadmpeg does not resolve it to a model face); 2 Degraded, 9 Info
  …
```

`--verbose` prints every line, and `--log FILE` writes them all to a file. In the desktop app,
each part's losses fold away under its own heading, and **Save the full log…** writes them
out.

Each loss line reads `LEVEL: SUBJECT: MESSAGE`:

| Level | Meaning | What to do |
|---|---|---|
| `Info` | No effect on the geometry. Something was left out that SolidCraft does not use, or the line just says what happened. | Nothing. |
| `Degraded` | Carried across in a weaker form. | Check that part of the design in SolidCraft. |
| `Dropped` | Left out of the design. | Compare with the original; redraw what is missing, or convert again when Dutchman or cadmpeg supports it. |

When cadmpeg's model fails its checks, Dutchman retries with geometry and assembly structure
alone. It removes history, sketches, parameters, and associated design records before checking
again, including parameter links on joints. A `Degraded: history` loss says these were left out
and gives the original check failure. Duplicate parameter names therefore cannot block this
retry; geometry checks still apply.

Repeated stderr notes of the same kind are grouped with a count. `--verbose`, `--log`,
and JSON retain each individual loss. Fusion’s material warning can also appear for a
default material: the decoded model does not establish that no source appearance was lost.

A Dropped appearance binding names its body or face target. “Its target is absent from the
decoded model” means no decoded geometry can receive it; “its target exists in the decoded
model but has no placed component geometry” means the target was decoded but is not placed.
Bindings to consumed or removed Fusion geometry have a separate explanation. “Its body has
no geometry the STEP writer can write” means the decoded target exists, but its shape cannot
be carried through STEP. Compare the named part with the source design when reviewing these losses.

Lines from the examples above, in plain words:

- `Info: history: flattened: the design carries no feature history`: the file has no
  history to convert, so the design is the solid geometry alone.
- `Info: step/native.source-record-omitted: 45 source-native record(s) were not represented
  in STEP`: internal records of the native file that have no place in the converted geometry.
  The shape is not affected.
- `Degraded: f3d/material.not-transferred: Materials and appearances were not carried over.`:
  the decoder could not confirm that materials carried over. Set any missing materials
  again in SolidCraft.
- `Degraded: f3d/sketch.dimension-native: 1 sketch dimension(s) retain native operands …`:
  a sketch dimension could not be tied to the sketch geometry it measures. The sketch is at
  the right size, but that dimension may not drive it.

`Dropped` lines name what was left out. For example, a part in an Inventor assembly whose
file cannot be found is reported as `Dropped: <part>: its part is not in a separate file
Dutchman can find`, and the rest of the assembly still converts.

The first part of a subject such as `f3d/material.not-transferred` is a stable code from
cadmpeg; in `--json` it is the `code` field, which scripts can match on.

Sketch dimensions keep parameter expressions when their operands map, including
point-to-line distances and parallel-line spacing. A loss saying a historical thread face
has no input surface geometry means the thread annotation could not be placed safely;
see [Feature history](history.md#feature-by-feature). Check size edits on the editable body
when the result also contains the original imported shape.

Clamped sketch splines with nonuniform spacing or repeated interior knots convert as chains
of exact spline pieces, up to 200 pieces, instead of leaving out the sketch. Periodic
splines are clamped first. Uniform splines stay one spline. Endpoints are shared, and
profiles and sweep paths select the whole chain. Constraints that cannot apply to a chain
are left out with an Info message; the solved geometry remains exact. A loft through such a
chain is not mapped yet (more edges in a section would change the lofted surface).

## Open the result in SolidCraft

In the SolidCraft app, use File, Open, or start it with the design:

```sh
solidcraft cube.sc
```

From the command line, `solidcraft-cli eval` opens the design, rebuilds its timeline and
prints its bodies as JSON:

```sh
solidcraft-cli eval cube.sc
```

Check that `body_count` is what you expect, that no `timeline` entry has an `error` or a
meshing warning, and compare each body's `area_mm2` and `volume_mm3` with the original design
within the measuring mesh tolerances. A face-count difference alone does not establish a
defect. An AI agent can do the same through SolidCraft's MCP server: open the design with
`solidcraft-cli mcp --in cube.sc` and call its `measure` tool.

Which SolidCraft a design is written for, and how Dutchman finds the installed one, is in
[SolidCraft versions](command-line.md#solidcraft-versions).

## Where next

- [The desktop app](desktop.md): presets, estimates, the SolidCraft version, and updates.
- [The command line](command-line.md): options, `inspect`, whole folders, watching a folder.
- [Choosing what to keep](presets.md): Fast, Standard, Full, and the memory and time estimate.
- [Formats](formats.md): what converts from each program.
- [Feature history](history.md): what maps onto SolidCraft, and what stops a history.
- [Troubleshooting](troubleshooting.md): common problems and diagnostic logs.

## Moved sections

This page used to hold everything. Each part now has its own page:

### Assembly workers

[Assembly workers](command-line.md#assembly-workers)

### Choosing what to keep

[Choosing what to keep](presets.md)

### Homebrew or Scoop

[Homebrew or Scoop](install.md#homebrew-or-scoop)

### macOS: the app

[macOS: the app](install.md#macos-the-app)

### Opening the app the first time

[Opening the app the first time](install.md#opening-the-app-the-first-time)

### From a release archive

[From a release archive](install.md#from-a-release-archive)

### Channels: stable and nightly

[Channels: stable and nightly](install.md#channels-stable-and-nightly)

### From source

[From source](install.md#from-source)

### Docker

[Docker](docker.md)

### Dutchman Web, in a browser

[Dutchman Web, in a browser](install.md#dutchman-web-in-a-browser)

### SolidCraft (optional, recommended)

[SolidCraft (optional, recommended)](install.md#solidcraft-optional-recommended)

### Desktop app

[Desktop app](desktop.md)

### Options

[Options](command-line.md#options)

### Look before you convert: inspect

[Look before you convert: inspect](command-line.md#look-before-you-convert-inspect)

### Progress during conversion

[Progress during conversion](command-line.md#progress-during-conversion)

### Convert a whole folder: convert-all

[Convert a whole folder: convert-all](command-line.md#convert-a-whole-folder-convert-all)

### Watch a folder

[Watch a folder](command-line.md#watch-a-folder)

### Formats

[Formats](formats.md)

### Pilot

[Pilot](pilot.md)

### Feature history and the SolidCraft check

[Feature history and the SolidCraft check](history.md)

Offset datum planes need no building feature when their support resolves or all their
consumers carry independent resolved sketch placements. Otherwise the history reason
names the unresolved plane reference; resolved skipped planes count as `not_needed`.

Shells require removed faces from their own historical input topology, a resolved
thickness and inward or outward direction, and one earlier named body. A shell input-face
blocker means the source selection or historical boundary is not supported yet; final-body
faces cannot establish which face existed before shelling.

A body made by a new-body feature is named from the source; when the source leaves it
unnamed, it is named "Body 1", "Body 2" and so on (the first number not yet taken), so later
features can still target it. An extrude stored as a join before any feature has built a
body makes that first body. A through-all cut with no earlier named body (no earlier
new-body feature mapped) is `blocked` before replay, with a reason asking for its
body-producing feature to resolve first. Source storage order does not
change replay order: the neutral ordinal controls it.

### Diagnostic logs for unfinished runs

[Diagnostic logs for unfinished runs](troubleshooting.md#diagnostic-logs-for-unfinished-runs)
include UTC time, Dutchman version and result in automatic filenames. Explicit
`--diagnostics PATH` files keep their chosen path. Reusing a log from another version shows
a notice and still converts. Desktop result cards show or copy their own run's log.

### Troubleshooting

[Troubleshooting](troubleshooting.md)

### FreeCAD output (experimental)

[FreeCAD output (experimental)](freecad.md#freecad-output-experimental)

### Conversion details

[Conversion details](command-line.md#solidcraft-versions)

### Presets and estimates

[Presets and estimates](presets.md#presets-and-estimates)

Anonymous diagnostic logs include timed decoder preparation phases and, on failure, the
failure stage and error category. Model check failures add check-code counts and available
referenced curve-kind counts. A geometry-only recovery rechecks the decoded model in memory
only when failing checks involve history, sketches, or parameters; geometry failures stop
without repeating the decode. Fusion archives also record anonymous `member_classified`
timings for nested scanning, layer classification and reference metadata extraction before
the first member decode starts. See [diagnostic fields](json.md#dutchmandiagnostics1).

## Make a minimum reproduction

When a failing design must stay private, run:

```sh
dutchman repro part.f3d --out reproduction
```

Use a fresh output folder each time. This runs the default SolidCraft conversion with
local diagnostics and writes `reproduction/kit/anonymous.json`, `local-key.json` and
`next-steps.md`. `--json` returns the summary, kit paths and anonymous content. Exit 0
means the kit was written, including when conversion failed; exit 1 means kit creation
failed. A successful conversion says **no failure to reproduce**.

Only `anonymous.json` is intended for sharing. `local-key.json` maps anonymous ordinals
to your own names and is marked **Do not share**. Follow `next-steps.md` in your CAD
program to rebuild a small design and compare the failure stage and finding-code set.
Temporary conversion output and diagnostic logs are removed after the command returns.
No files are uploaded. Narrowing uses retained CADIR subsets and stops where an independent
subset cannot reproduce the same structured check codes; it does not regenerate features.
The [kit schema](json.md#dutchmanrepro1) describes these limits.

## Rebuild a small failing example

`dutchman diagnose --narrow FILE --out RESULTS` now includes a numbered rebuild recipe for
Fusion, Inventor, SolidWorks and FreeCAD in `RESULTS/diagnose/kit/next-steps.md`. Recipes
use round starting sizes and the signature's options and profile counts. Unknown feature
classes get a generic recipe. An exact match in the release's known-problem list gives a
workaround or next-version fix advice before further narrowing when enough fields are known.

In the desktop app, choose **Diagnose this file** on the failure card for progress, resume,
the recipe, **Check my small file**, and **Make the report**. The report ZIP stays local
and contains the small file's geometry. Review it before choosing to share it.
The [MCP tools](mcp.md#anonymous-diagnostic-tools) provide the same steps for an assistant.

## Check a folder locally

`dutchman doctor designs --out checked --jobs 2 --memory-mb 8192 --json` plans
folders and project ZIPs with the same rules as `convert-all`: top-level assemblies,
their referenced parts, and unused parts. It converts in isolated workers, then runs
part diagnosis. `--memory-mb` is the total converter budget, divided across concurrent
designs; CPU availability caps `--jobs`. `--time-limit 5m` limits each diagnose part.
Run the same command again to resume. Completed checks are reused when the project
inventory, choices, budgets and matching STEP metadata agree. Keep the output outside
the input folder. Default output is a stable sibling named `designs (Doctor)`.

Only `checked/report.json` is anonymous. Converted designs, `local-key.json` and
`private-checkpoints/` stay private. Findings are grouped locally by signature with
counts. A broad error category is a candidate group, not proof of the same defect.
Nothing is sent. Review advisory findings before making a report.

`dutchman diagnose big.f3z --step big.step --out checked` compares stored geometry
against an export of the same design. An unambiguous same-name `.step` or `.stp`
beside a native file is found automatically, including inside a folder or ZIP.
`doctor --step reference.step` accepts a project with one conversion job. Use
`diagnose --part 17` or `doctor --part 17` to record a report target. This does not
change the geometry being compared or replace `convert --part`.

STEP results use evaluated **body numbers**, which can differ from diagnose part
numbers for assemblies and repeated occurrences. Matching uses geometry, placement
and stable order, never names. Results are worst first and contain volume, area,
bounding box, face counts where available, and placement differences. Current
thresholds are 1% volume/area. Bounds/placement allow 0.1% of the body diagonal,
with a 0.1 mm floor and a 0.5 mm ceiling, so a 1 mm move still flags on large bodies.
Face counts alone never flag a difference. Missing measurements are explicit. Checks remain advisory; unavailable comparisons do not count as passes.
No automatic report is sent. The observed false-positive rate for the two
placement-golden assemblies is 0/329 evaluated body rows (0%);
[calibration results and limits](doctor-calibration.md) give the coverage and denominators.
The STEP itself stays local; use the private-send card if you choose to share it.

The structural checks also flag `geometry_untrimmed_torus`: a toroidal face without
trim loops sharing a shell with other faces. Review its boundary before reporting a
defect. A whole torus in its own shell is allowed, including a separate cavity shell.
An isolated part cannot be called floating without an independent placement reference;
the comparison checks detect movement relative to stored geometry or a matching STEP.

Every successful SolidCraft result also gets an advisory body comparison against the
source's stored geometry, using the same joint setting and assembly pose as the
conversion. Partial-history outputs skip this comparison because eval
includes intentionally hidden editable bodies; the normal history verification still
runs. Where supported, SolidCraft writes private whole-design snapshot pairs for
flagged comparisons under `diagnose/preview/`. Their PNGs and staged designs carry
geometry. Review them locally; they are excluded from anonymous tool results.

Memory supervision includes kernel subprocesses on Linux, macOS and Windows; a stopped
worker stops its process group/tree too. These are sampled resident-memory limits,
not an operating-system reservation. The folder planner runs before worker supervision.
Resume fingerprints include resolved external assembly references (up to 10,000 files);
an incomplete dependency inventory is checked afresh.

Real SolidCraft controls on the three published synthetic cube fixtures (Fusion,
Inventor and SolidWorks) produced 0 flags in 3 comparisons against matching STEP.
A planted 1 mm move in a two-instance assembly was detected, and snapshot pairs were
produced. These small controls establish basic behavior, not a false-positive rate
for large assemblies or arbitrary designs.

<a id="report-consent"></a>
Report consent is per report. On a converted desktop card, expand
**Something wrong? Report this file**; on a failed card, the actions are already visible.
**Review and send report**
opens a review with bundle or summary. A bundle is selected when available; otherwise
only the anonymous summary is offered and selected: version, OS, format, source app and
problem signature only. Converted results use `converted-with-problem` and never
reuse a prior failure bundle. The review displays the exact metadata and sharing choices.
Descriptions are not supported in the anonymous envelope; describe the problem privately
or in the GitHub form without private names or paths. Whole files use the private-send card on Discord. A separate box,
**You may use this report in a public test case**, starts unticked.
Nothing is sent before clicking Send. A report is sent only when a matching receipt
arrives. A temporary failure gets one short retry, then keeps the report locally;
retrying later requires clicking **Retry**, which appears after a failure. A daily cap
asks you to try again tomorrow.
GitHub accepts the anonymous summary only; never attach a geometry bundle there. For a
converted result the form asks what differs from the original, without claiming conversion failed.

For the CLI, first make a local preview (always printed as JSON):

```sh
dutchman report --folder reports/converted --report-id f0123456789abcdef --source-format f3d --signature converted-with-problem
```

Replace the example id with the file id from your diagnostic log or private report key.
Use a fresh report folder for this converted result. No design is opened or uploaded.
Read the preview's `metadata`, then send that summary explicitly:

```sh
dutchman report --folder reports/converted --send-summary --yes
```

`--public-test` separately permits a public test case and defaults off. The same command
retries a failed submission only when you run it again. Saved desktop report folders can
also be reviewed with `dutchman report --folder FOLDER`. MCP offers the same path through
[`report_preview` and `report_submit`](mcp.md#report-preview-and-sending).

Release builds send to Dutchman's report service; `DUTCHMAN_REPORT_ENDPOINT` at compile
time replaces it. A debug build has no default: clicking Send keeps the report locally
and offers the same fallbacks, and development tests use a loopback HTTP stub.

Report summaries identify the authoring app from the native format, such as Fusion or
Inventor. Exchange formats and unclassified `.prt` use `unknown` because their authoring
app cannot be inferred. The build version is included alongside this information.

The desktop keeps its page scrollbar visible to reach every private-send option in the results.
Copied-report notices use the success colour and clear on the next action.
**Check a folder** keeps the app header; **Part to check** selects a part for local checks.

`dutchman diagnose part.f3d --bundle --out diagnosis` cuts a private reproduction
bundle without narrowing. Ordinary diagnosis, narrowing, and doctor also try to save a
bundle when cadmpeg's decoded model fails a check. A cut must reproduce the same
failure signature; Dutchman widens it if needed and keeps no bundle if proof fails.
A refused cut, including a closure wider than the current fragment byte limit, is a
normal no-bundle result; choose summary or the private-send card.
Shrinking runs for up to ten seconds (or the shorter time limit). Creating
`diagnose/cancel-bundle` in the output folder stops shrinking between candidates and
keeps the last proven cut.

The report folder `diagnose/repro` holds `bundle.json.gz`, `manifest.json`, and private
report metadata. The manifest lists retained arenas, row counts, sizes, and purposes;
previews never show their contents. Bundles contain numeric geometry. Review them locally
before choosing to send. A bundle is the default when available. The service caps the entire multipart request at 25 MiB, including metadata and
upload overhead. Dutchman keeps gzip bundles at least 64 KiB below that cap and checks the complete
size before enabling Send. If it will not fit, use the
private-send card or send a summary. Decode refusals have no bundle yet; choose
an anonymous summary or the private-send card. Nothing is sent by diagnosis.

## Assembly placements

**Part positions** offers **Smart** (default), **As last shown in the original program**, and
**As first placed**. Smart accepts current poses only when touching joints close, geometry
stays within first-placement assembly bounds grown by 10%, and repeated inserted designs
move together. It keeps first placements when a check fails or cannot be established.
Last-shown accepts available current poses without those checks. First-placed ignores later
moves and restores the old placement choice. Joints work from the selected placements.

Use CLI `--part-positions smart|last-shown|first-placed`, MCP `part_positions`, or desktop
**Advanced → Part positions** (remembered). `--stored-placements` remains a first-placed
alias. Smart and last-shown prefer decoder-bound displayed poses when available, without
current-pose guards. Ambiguous and unbound rows are skipped; an absent arena supplies none.
Fallback current poses may be stale. An Info note counts the selected rules. JSON includes
`choices.part_positions` and `part_positions` counts and fallback reasons.

Missing assembly holder poses are inferred bottom-up from displayed children only when
their world-pose estimates agree within 0.001 mm and 1e-6 rotation entries and reproduce
every displayed child. A conflicted holder keeps its first placement, even inside a holder
that moves, and is reported. Smart also keeps the first placement of a displayed holder (and
everything inside it) when a part below it would leave the bounds grown by 10%. JSON `part_positions` adds `derived_holders`
and `holder_conflicts`; `bound_displayed` and `derived_holders` partition `displayed`, preserving the displayed/current/first-placement total.

Local CLI and desktop progress names the running part or stage and its elapsed time,
including “Still …” notices and full-sentence check-limit consequences. Durations use one
format, such as `1 min 1 s` or `2 min`. Desktop folder runs keep the longest-running file,
its stage and step visible below the completed count, with the number of other active files. See
[progress during conversion](command-line.md#progress-during-conversion). MCP final reports
are unchanged; running names are excluded from anonymous diagnostic logs.
