# The MCP server

## Read an existing result

`read_report` takes `input`, the absolute path of a `.sc` design or its adjacent
`<stem>.report.json`. Legacy `.solidcraft`/`.solvecraft` design paths and explicit
`.failed.json` reports also work. It reads only the saved JSON; neither the original nor
the design needs to exist, and nothing is decoded, converted, verified again, or written.
Desktop conversions and watch mode save these per-file reports. A missing sidecar is
reported as an error; the tool never falls back to converting.

```json
{"name":"read_report","arguments":{"input":"/models/converted/part.report.json"}}
```

The reply preserves the `dutchman.convert/1` report, including `losses`, feature outcomes
under `history` (including per-part history), placements under `completeness`, and
`verification` when recorded. It adds `loss_groups`, grouped by `(code, stage)` with
`count` and every original loss in `losses`; absent codes/stages are `null`. Fields absent
from the saved report remain absent. A saved failure retains `ok: false` and is returned
as an MCP tool error with its original error details. Reports over 64 MiB, unreadable JSON,
and other schemas are refused.

This is a **local report tool**, like `convert` and `inspect`: its full reply includes
private paths, file/feature names and messages. Use it only with a client keeping that
content local; do not paste the reply into an issue or send it to a remote assistant.
The anonymous diagnostic tools below continue to exclude these details. `read_report`
sends nothing, but an MCP client controls where it forwards a tool reply.

The desktop **Ask your AI about this file** prompt needs no MCP connection: it opens the
installed instructions and conversion skill first (with GitHub URL fallbacks when absent),
then the same JSON directly, and uses the bundled command line only for focused follow-up work.

SolidWorks parts convert in their saved configuration when its body membership is known.
Conversion reports include an Info note when bodies from other configurations are left out.

## Project inputs

Files whose names start with `~$` are lock files. They are ignored in project planning
and excluded from job, failure and skipped-file counts.

ZIP directory data and ZIP64 extensible records are limited to 16 MiB each before
the ZIP directory is allocated; entry budgets count every declared entry.

The `convert_all` tool accepts project ZIPs in `inputs` as well as files and folders;
`out_dir` remains required for the tool. ZIPs unpack privately within 10,000 entries and
4 GiB of extracted data, with traversal paths refused and symbolic links skipped.
Planning reads Inventor and SolidWorks assembly references, selects assemblies no other
assembly references, and converts them with their project parts, then unused parts. Adjacent same-name STEP exports
are used only when native conversion fails. The batch report adds `plan`; a fallback's
conversion result adds `native_failure` with the original native report.

The `dutchman` command also dispatches installed Dutchman CAD tools such as `pilot` and
`fixtures` through `dutchman-<tool>` beside the command or on `PATH`. `dutchman --help`
lists them. These external commands provide their own interfaces; the conversion MCP tools
and JSON reports described here retain their existing schemas.

The full CLI also accepts `dutchman part.f3d` (or `dutchman convert part.f3d`) and derives
the output beside the input from `--to`. An existing derived output is kept with exit 2;
choose `-o PATH` or use `--force` to replace it. `--force` conflicts with `--no-clobber`.
Explicit outputs retain their replacement behaviour. MCP output paths remain required except for `convert` with `dry_run: true`.

`dutchman-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server. It lets
an AI assistant convert CAD files with Dutchman on your machine: it provides conversion and diagnosis
tools. Conversion tools return the same JSON report as the matching `dutchman … --json` command
([json.md](json.md)).

It comes in every release archive, the Windows per-user MSI, and with `cargo install`
(see the [guide](install.md)). After installing the MSI, restart your MCP client to pick up
the user PATH entry for `%LOCALAPPDATA%\Programs\Dutchman`. Release packages also keep the
conversion skill in `skills/` and this Markdown guide in `docs/`, beside an installed
`AGENTS.md` explaining full-path commands and saved reports.
It speaks JSON-RPC over stdio, one message per line, and takes no arguments: the client starts
it and talks to it.

A part that holds an imported solid becomes a base-body Import feature at its place in the
timeline. Later features keep editable history when they map. Stored geometry has no recipe
to lose: the report says, at Info level, “This part holds an imported solid; there is no
feature history to bring over”. A part made only of stored geometry is geometry without
history, with no flattened-history warning.

Fusion consumed tools and removed bodies are omitted only when cadmpeg marks them hidden
and no component owns them, with an Info `bodies` note giving the count. User-hidden
component members stay available; mixed shown/hidden parts are shown whole, with an Info
note. See [Fusion visibility](fusion.md#parts-and-history).

Sheet-metal history follows the same mapping limits in [Feature history](history.md#sheet-metal) as the CLI:
base flanges and full-edge flanges on unchanged rectangular base sheets map when their own
input-edge geometry proves each pick. Unsupported flange dimensions or selections, and hems,
appear as the first blocked feature's reason; the conversion keeps the geometry. K-factor and
relief use the target rule defaults because CADIR does not supply them.

FreeCAD signed horizontal and vertical sketch distances convert to positive dimension
magnitudes at their solved positions. Their signed parameters and expressions remain
available; a dependent magnitude parameter drives each negative distance in SolidCraft.

CLI progress is separate from tool results: `convert --json`, `convert-all --json`,
`inspect --json`, and `watch --json` emit `dutchman.progress/1` JSON lines on stderr
while keeping the final report or watch-event stream on stdout (`--quiet` hides
progress). The MCP server has no progress notification mechanism and continues to return
only the final report. The web app also has no streaming conversion progress mechanism.
Desktop file cards show stage, elapsed time, and five-second heartbeats, with design
counts and a determinate bar when the total is known. Fusion `.f3z` member counts are
not available yet.

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

Both `convert` and `convert_all` accept `threads`, a whole number at least zero.
`0` (default) selects at most two workers for assembly parts; `1`
uses the serial path. Part count and available memory cap the actual worker count.
Component slices prepare independently and identical empty holders share a model.
SolidCraft parts also prepare independently, including checks in separate temporary folders,
and enter the design in their original order. Designs and ordered reports are identical
apart from `threads: {requested, used}`. Failed preparations retry serially. Native
decoding remains serial; other writers use shared component slicing only. Batch `jobs` limits
files, while `threads` limits part workers per file. `no_memory_check` leaves this
worker cap enabled.

## Choosing what to keep and estimating cost

Both `convert` and `convert_all` accept `preset` (`fast`, `standard`, or `full`, default
`standard`), `history`, `sketches`, `joints`, and `no_memory_check`. The three retention
booleans default to true and only narrow the preset: true cannot enable history in
`fast`. `history: false` is also available as the existing `import_only: true`.
Fast keeps shape only and skips the memory guard. Standard keeps history, sketches,
and joints, leaving history out when its estimate exceeds free memory and
the history-free estimate fits. Full never drops history for memory. Reference sketches
are controlled by `sketches`; sketches needed by retained history stay. Disabling
joints retains the selected placements and adds an Info report line. FreeCAD output honours
history only; sketch and joint choices do not control that writer.

An estimate above free memory doesn't stop a conversion; a dry run's `fits` reports it. `convert`
runs in a child process, and if memory really runs low it stops with the same message as the CLI:
`stopped before this computer ran out of memory: ...`. Set a lighter `preset` and try again, or set
`no_memory_check: true` to turn that stop off. Existing input and batch worker limits still apply.
Standard's decision to leave history out is reported as one Info line under `history`, as
described in the [guide](presets.md).

Requested SolidCraft checking is included in `estimate.peak_bytes`. Conversion-only
preflight can allow a run whose check will not fit. After reading, checking compares the
same extra check figure used by the desktop estimate with current free memory and
rechecks before geometry replay. It does not multiply the memory already held by the
decoded design.
Geometry-only conversion also checks the converted shape by default, using a separately
calibrated check allowance; `verify: false` keeps either kind of checking off.
If it cannot fit, the successful report includes `history_check_skipped` with
`needed_bytes`, `available_bytes`, and `reason`:
`History check skipped: needs about N GB, M GB free`. The original shape stays usable
and retained editable history stays unverified. This applies to `convert` and each
`convert_all` result, as in the CLI, desktop and web. `verify: false` always keeps checking
off; `no_memory_check: true` forces requested checks. Automatic part work uses at most two
workers and budgets each detailed part's temporary peak and the retained preparation queue.

Use `convert` with `dry_run: true` for a quick estimate without decoding or writing;
`output` is optional:

```json
{"name":"convert","arguments":{"input":"/models/assembly.f3z","preset":"standard","dry_run":true}}
```

The result contains `ok: true`, `estimate: {peak_bytes, seconds, seconds_exact, band, memory_band, time_band}`,
`choices: {preset, history, sketches, joints}`, and `fits` (boolean, or `null` when
memory or the estimate is unknown). `estimate` itself can be `null`. ZIP directory
reads are bounded and no design is decoded. Memory includes fixed process overhead and independent measured read, conversion and checking costs where calibrated. A pre-read cannot confirm archive assembly structure,
so Standard's budgeted choices are provisional until normal decoding. An over-budget
dry run returns `fits: false`; it writes nothing and does not attempt conversion.
Normal convert reports, including batch results, carry the same `estimate` and
`choices` fields ([json.md](json.md)).

The current phase fits use six public Fusion assemblies and five small Fusion, Inventor
and SolidWorks parts, measured on Linux with four requested workers and SolidCraft 0.30.1.
Total-peak RAM bands are ±45% for `.f3d` and ±30% for `.f3z`; Inventor/SolidWorks larger-part
growth remains a ±100% heuristic, and FreeCAD keeps ±50%. Component costs and their worst
misses are documented in [Choosing what to keep](presets.md). These are empirical total-peak
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

## Set up a client

### Claude Code

```sh
claude mcp add dutchman -- dutchman-mcp
```

`dutchman-mcp --version` prints the version and `dutchman-mcp --help` prints a short usage
note; both exit at once without starting the server.

On macOS, a blocked SolidCraft check explains how to open SolidCraft once and allow it
with **Open Anyway** in **System Settings > Privacy & Security**, then try again.
Dutchman checks the quarantine attribute for the user-approved flag without changing it.

Optional: if Dutchman cannot find `solidcraft-cli` on `PATH` or inside an installed SolidCraft
app, pass its path. Replace `/path/to/solidcraft-cli` with the installed executable's absolute
path before running this command:

```sh
claude mcp add dutchman --env SOLIDCRAFT_CLI=/path/to/solidcraft-cli -- dutchman-mcp
```

`claude mcp list` shows whether it connected.

### Codex

In `~/.codex/config.toml`:

```toml
[mcp_servers.dutchman]
command = "dutchman-mcp"
```

Optional, inside that same table, add the following line only after replacing the placeholder
with the installed executable's absolute path:

```toml
env = { SOLIDCRAFT_CLI = "/path/to/solidcraft-cli" }
```

### Other clients

Most clients (Claude Desktop, Cursor, Windsurf, and others) take an `mcpServers` entry:

```json
{
  "mcpServers": {
    "dutchman": {
      "command": "dutchman-mcp"
    }
  }
}
```

If needed, add an optional `"env": {"SOLIDCRAFT_CLI": "/path/to/solidcraft-cli"}` member beside
`command`, with a comma between them. Replace the placeholder with the installed executable's
absolute path; on Windows, use `.exe` and escape backslashes as `\\` in JSON.

If the client cannot find `dutchman-mcp`, give its full path as `command` (on Windows,
including `.exe`). `SOLIDCRAFT_CLI` is optional: without it, Dutchman looks on the server's
`PATH` and inside an installed SolidCraft app, and reports the history unverified if neither
provides a usable CLI.

## Tools

Repeated stderr notes of the same kind are grouped with a count. `--verbose`, `--log`,
and JSON retain each individual loss. Fusion’s material warning can also appear for a
default material: the decoded model does not establish that no source appearance was lost.

A Dropped appearance binding names its body or face target. “Its target is absent from the
decoded model” means no decoded geometry can receive it; “its target exists in the decoded
model but has no placed component geometry” means the target was decoded but is not placed.
Bindings to consumed or removed Fusion geometry have a separate explanation. “Its body has
no geometry the STEP writer can write” means the decoded target exists, but its shape cannot
be carried through STEP. Compare the named part with the source design when reviewing these losses.

`formats` lists all recognised formats, their status `words`,
and what Dutchman converts. Inputs without geometry support or a codec in this build
return a failure describing the reader's current capability. Creo and NX `.prt` inputs
are distinguished by contents. CATIA, Creo View, Rhino, ACIS and STEP use the geometry
path. Each conversion report's `input` also carries `words`, `program` and `extension`.
Status words are `reads geometry`, `opens, no geometry yet`, `recognised, not read yet`,
or `open it in SolidCraft directly` for IGES (`.igs`, `.iges`). IGES is recognised and
refused before decoding by `inspect`, `convert` and `convert_all`, with `error.message`:
`Dutchman doesn't convert IGES yet: open it in SolidCraft directly (File > Import).`
Batch results retain that refusal; IGES does not trigger the CLI or GUI fixtures invitation.
Numbered `.prt.N` saves are identified by contents too; a directory conversion keeps only
the highest revision per part and returns older saves in `skipped_revisions` (`path`, `newer`).
Numbered saves use the part's base output name and keep existing output files.
The CLI and desktop volunteer invitation is absent from MCP and `--json` output.

For `inspect`, `ok: true` means the file was read. Check `conversion.possible` and
`conversion.reason` before converting: inspection can succeed even when conversion cannot.

Unused FreeCAD binders and datum helpers are `not_needed`, with the Info note
`helper not converted: nothing uses it`. Used datum points and coordinate systems can be
carried by resolved sketch placements, pattern axes, or another resolved datum. A used helper without the required
resolved placement or binder source geometry remains a blocker, with its reason in the report.
These Info notes remain when another feature or a replay mismatch flattens the history.
A body inside a hidden container is hidden even when its own flag says it is shown.

SolidCraft assembly conversions retain reflections through shared mirrored component
geometry, including nested placements. Mirrored copies use imported geometry only;
`losses` explains any history or affected joints left out. Info `placements` notes count
reflections; Dropped notes name each non-finite, scaled, or sheared occurrence left out by
index and kind. Tiny numeric drift is corrected.

SolidWorks `.sldasm` uses the same `project` and `search` arguments as Inventor assemblies.
External parts and sub-assemblies resolve recursively; unreadable occurrences and unavailable
referenced configurations appear in the shared conversion report.

FreeCAD external Body and Part links are resolved beside the assembly first, then through
`project` and `search`. A missing linked `.FCStd` file is left out and named in losses and
`completeness.left_out`; available components still convert. Link arrays place each element
once. Grounding and supported Assembly-workbench joints map like other assembly joints.

FreeCAD fillets and chamfers use their base feature's stored edges only when the earlier
result and each pick on SolidCraft's replay are proven. Otherwise `history.features[].reason`
says why the edge selection is refused, and conversion keeps the original geometry.

For sweeps, `history.features[].reason` identifies unsupported corner transitions, section
interpolation, tangent-path extension, or simplification. Paths and guide rails may select
ordered sketch curves or a whole sketch with one non-construction curve. A blocked sweep
keeps the original geometry when converted.

Constant-radius helical sweeps, including FreeCAD AdditiveHelix and SubtractiveHelix,
map to sketch-profile coils when targeting SolidCraft 0.25 patch 1 or newer. Older targets block
the helix with `needs SolidCraft 0.25 patch 1 or newer` and the target version, preserving the
original geometry. The CLI equivalent is `--solidcraft-version x.y.z` on `convert`,
`convert-all`, and `watch` (default: found CLI version, even with `--no-verify`, else
the newest release known at build time, currently 0.30). The rolling 14-day window
is currently 0.18–0.30; older targets down to 0.18.2 get simplified features.
Pitch/height/turn laws, handedness, axis, reversed travel, and Boolean operation carry across. Feature parameter
bindings are retained when available, otherwise dimensions use resolved values. Conical
and spiral sweeps report the missing SolidCraft cone-angle/radial-growth option.

Hole history keeps chamfered entries and tapped holes (the bore and an ISO metric thread
label); thread details it can't keep are `Degraded` losses. Unsupported hole forms,
placements, and ends appear in `history.features[].reason`, and conversion keeps the
geometry when the history can't map.
Entry recesses with drill points report "SolidCraft holes take a drill point only on a
plain drilled hole"; SolidCraft 0.21.0 ignores `tip_angle` on counterbores and countersinks. SolidCraft 0.22.1 and newer support those drill
points, but Dutchman retains this limit for compatibility with older releases.

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

| Tool | Arguments | Returns | Writes |
|---|---|---|---|
| `inspect` | `input`, and optionally `project`, `search` | `dutchman.inspect/1` | nothing |
| `convert` | `input`, `output` (optional for `dry_run`), and optionally `threads`, `preset`, `history`, `sketches`, `joints`, `no_memory_check`, `dry_run`, `import_only`, `verify`, `solidcraft`, `solidcraft_version`, `verify_timeout`, `overwrite`, `project`, `search` | `dutchman.convert/1` | the `output` file |
| `convert_all` | `inputs`, `out_dir`, and optionally `jobs`, `timeout`, `max_memory_mb`, `journal`, `report_anon`, `report_salt`, `report_key`, `threads`, `preset`, `history`, `sketches`, `joints`, `no_memory_check`, `import_only`, `verify`, `solidcraft`, `solidcraft_version`, `verify_timeout`, `overwrite`, `project`, `search` | `dutchman.convert-all/1` | designs and the default log inside `out_dir`, plus `journal`, `report_anon`, and `report_key` at their specified paths |
| `formats` | none | `dutchman.formats/1` | nothing |
| `read_report` | `input` (saved report or its design) | `dutchman.convert/1` plus `loss_groups`, with private local names and paths | nothing |

The formats report includes each reader's status and cadmpeg's published headline
`level` (`L0`–`L9`, or `null` when none is published). The level supplements the status.

| Argument | Type | Meaning |
|---|---|---|
| `input` | string | Absolute path of a native CAD file: `.f3d`, `.f3z`, `.ipt`, `.iam` or `.sldprt` or `.FCStd`. |
| `output` | string | Absolute path of the `.sc` file to write (`.solidcraft` and `.solvecraft` also work). An existing file is replaced, unless `overwrite` is false. |
| `inputs` | array of strings, at least one | Absolute paths of files, and of folders to search (with their subfolders) for files Dutchman knows. |
| `out_dir` | string | Absolute path of the folder for the designs, keeping each input folder's layout. Created if missing. |
| `jobs` | integer, at least 1 | How many files to convert at once. Default: the machine's cores, at most 8. |
| `timeout` | integer, at least 1 | Seconds one file may take before it is stopped and reported as failed. Default 900. |
| `max_memory_mb` | integer | Megabytes one file may hold before it is stopped and reported as failed. Default 0, no limit; a file also stops if the computer really runs low on memory. |
| `journal` | path | A file recording each file's result as it finishes; files it already records, unchanged since, are not converted again. |
| `report_anon` | path | Also write the anonymous report ([`dutchman.report-anon/1`](report-anon.md)) there. The result then carries `report_anon: {"path", "salt"}`; the salt stays private. |
| `report_salt` | 32 hex digits | The salt for the anonymous report's file ids, to get the same ids as an earlier run. |
| `report_key` | path | Write the key from each anonymous id to its file's path there. Keep it private. |
| `preset` | string | `fast`, `standard` (default), or `full`; see [Choosing what to keep](#choosing-what-to-keep-and-estimating-cost). |
| `history`, `sketches`, `joints` | boolean | Narrow the preset; each defaults to true. |
| `no_memory_check` | boolean | Bypass the estimate refusal, Standard's history budget, and automatic history-check skipping; default false. |
| `dry_run` | boolean | `convert` only: return estimate, choices, and fits without decoding or writing; default false. `output` is optional when true. |
| `import_only` | boolean | Leave feature history out; keep geometry and the preset's reference sketches. |
| `verify` | boolean | Check a converted history in SolidCraft. Default `true`; skipped, and the history marked unverified, when no `solidcraft-cli` is found. |
| `solidcraft` | string | Path of `solidcraft-cli` for the check. Default: `$SOLIDCRAFT_CLI`, else `solidcraft-cli` on `PATH`, else the one inside an installed SolidCraft app (macOS `SolidCraft*.app`, newest first; Windows `Program Files\SolidCraft`). |
| `solidcraft_version` | string | Target release as x.y.z, at least 0.18.2. Default: the found CLI's version capped at newest known, else the newest known at build time (currently 0.30), including when `verify` is false. |
| `verify_timeout` | integer, at least 1 | Seconds each SolidCraft check may run before the history is written unverified. Default 120. |
| `overwrite` | boolean | Replace an existing `.sc` file (default true). When false, an existing file is left alone and that file's conversion fails with "already exists". |
| `project` | string | Absolute path of the project folder an assembly's part files are searched in when they are not at their stored paths. Default: the nearest folder above the assembly holding an Inventor `.ipj`, else two folders up; for `convert_all`, each folder given. See [Finding the part files](inventor.md#finding-the-part-files). |
| `search` | array of strings | Absolute paths of more folders to search, with their subfolders, for part files the project folder does not hold. |

An unknown argument is refused (`unknown argument bogus`), as is a missing required one
(`output is required`).

Paths are on the machine running the server. Give absolute ones: a relative path is read
against the server's working directory, which the client chooses.

### Diagnostic logs during conversion

Both `convert` and `convert_all` accept optional `diagnostics` (an absolute file path), or
`diagnostics_default: true` for a unique timestamp file in the platform's default diagnostics
folder. They cannot be combined. Explicit files are appended without retention; the default
folder keeps the newest 50 timestamp logs, including older time-only names. Automatic
names include the Dutchman version and result: `running` changes to `converted`, `failed`
or `stopped` after writers close; a crash or rename failure leaves `running`. Explicit
paths stay exactly as supplied. Reusing another version's log gives an advisory notice. [The guide](troubleshooting.md#diagnostic-logs-for-unfinished-runs)
lists each OS folder and how to review and send a log.

Batch output defaults to `.sc`; targets before SolidCraft 0.25.1 use `.solidcraft`.
Explicit output names are preserved.

For example, call `convert` with `{"input":"/abs/part.f3d","output":"/abs/part.sc",
"diagnostics":"/abs/run.jsonl"}`. `dry_run` writes no diagnostic log.
The log is [`dutchman.diagnostics/1`](json.md#dutchmandiagnostics1): records are flushed as
conversion proceeds, with disk syncs at least every five seconds and at stage changes,
and resource heartbeats every 30 seconds. It survives an interrupted process and contains
no names, paths, design contents, geometry, or messages. Per-member records appear when the
decoder reports them. `convert_all` with `report_anon` shares its salt and ids with diagnostics;
keep the salt and any `report_key` private. These extra files do not change conversion report
schemas or the output format.

## Example calls and replies

Each reply carries the report twice: as `structuredContent`, and as the same JSON in a text
`content` item for clients that read only text. The replies below show `structuredContent`,
with `content` shortened to `…`.

Inspect a file:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inspect","arguments":{"input":"/home/me/designs/solidworks_cube.sldprt"}}}
```

```json
{"jsonrpc":"2.0","id":2,"result":{"isError":false,"content":"…","structuredContent":{
  "schema":"dutchman.inspect/1","ok":true,
  "input":{"format":"solidworks_part","format_name":"SolidWorks part (.sldprt)","path":"/home/me/designs/solidworks_cube.sldprt"},
  "geometry":{"bodies":1,"faces":6},
  "conversion":{"possible":true,"reason":null},
  "mode":"import",
  "verification":{"status":"not_applicable","reason":"the design carries no feature history"},
  "history":{"features":[],"mapped":0,"total":0},
  "losses":[
    {"level":"Degraded","stage":"project","upstream":"cadmpeg","code":"sldprt/source.dialect-unverified","subject":"sldprt/source.dialect-unverified","message":"the document carries no swSolidWorks swVersion declaration, …"},
    {"level":"Degraded","stage":"project","upstream":"cadmpeg","code":"sldprt/config.inferred-without-native","subject":"sldprt/config.inferred-without-native","message":"1 configuration state(s) are inferred from geometry partitions without native configuration definitions."}]}}}
```

Convert one file:

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"convert","arguments":{"input":"/home/me/designs/synthetic_cube.ipt","output":"/home/me/designs/cube.sc"}}}
```

```json
{"jsonrpc":"2.0","id":3,"result":{"isError":false,"content":"…","structuredContent":{
  "schema":"dutchman.convert/1","ok":true,
  "input":{"format":"inventor_part","format_name":"Inventor part (.ipt)","path":"/home/me/designs/synthetic_cube.ipt"},
  "output":{"format":"solidcraft/23","path":"/home/me/designs/cube.sc"},
  "mode":"import",
  "verification":{"status":"not_applicable","reason":"the design carries no feature history"},
  "history":{"features":[],"mapped":0,"total":0},
  "losses":[
    {"level":"Degraded","code":"inventor/rse.segment-pair-untyped","subject":"inventor/rse.segment-pair-untyped","message":"Retained 1 structurally paired RSe segment(s) without record semantics."},
    {"level":"Info","code":"step/attribute.source-record-not-written","subject":"step/attribute.source-record-not-written","message":"7 source attribute record(s) were not written to STEP"},
    {"level":"Info","code":"step/native.source-record-omitted","subject":"step/native.source-record-omitted","message":"67 source-native record(s) were not represented in STEP"},
    {"level":"Info","code":null,"subject":"history","message":"flattened: the design carries no feature history"}]}}}
```

SolidCraft history checks compare area and volume within the measuring mesh tolerances.
Face-count differences alone produce no geometry loss, nor do open-shell warnings when area
and volume agree. Evaluation errors, unmeshable faces, bodies inside out, solids without
volume, and solids that geometry-only output opens as open shells are still reported;
sheets remain exempt from volume and closed-shell checks. Geometry-only output has no
independent source measurements. These use the existing report fields described in
[JSON reports](json.md).

If cadmpeg's model fails its checks, the reader retries after removing history, sketches,
parameters and associated design records, including parameter links on joints. Geometry and
assembly structure still undergo validation. A successful retry reports a `Degraded` loss
with subject `history`, saying parameters were left out and giving the original failure.

A conversion that fails is a tool error (`isError: true`) carrying its failure report:

```json
{"jsonrpc":"2.0","id":4,"result":{"isError":true,"content":"…","structuredContent":{
  "schema":"dutchman.convert/1","ok":false,
  "input":{"format":"unclassified_part","format_name":"Creo or NX part (.prt)","path":"/home/me/designs/a.prt"},
  "error":{"message":"Creo or NX part (.prt): reading geometry is not supported yet. Export a STEP file for SolidCraft to open."}}}}
```

Arguments a tool cannot take (a missing `output`, an unknown argument, a mistyped value) fail
the same way: a tool error carrying a failure report of the tool's schema, whose `error.message`
says what is wrong. `input` is described when the call names one, else `null`:

```json
{"jsonrpc":"2.0","id":6,"result":{"isError":true,"content":"…","structuredContent":{
  "schema":"dutchman.convert/1","ok":false,
  "input":{"format":"fusion_archive","format_name":"Fusion archive (.f3d)","path":"/home/me/designs/part.f3d"},
  "error":{"message":"output is required"}}}}
```

So a client checks `isError`, and finds the reason in `structuredContent.error.message` (or the
same report as text in `content`) whichever way the call failed.

Convert a folder. When any file fails, the whole call is a tool error, but the other files are
still converted and each has its own report (shortened here):

```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"convert_all","arguments":{"inputs":["/home/me/designs/project"],"out_dir":"/home/me/designs/converted","jobs":2}}}
```

```json
{"jsonrpc":"2.0","id":5,"result":{"isError":true,"content":"…","structuredContent":{
  "schema":"dutchman.convert-all/1","ok":false,"converted":3,"failed":1,
  "results":[
    {"ok":true,"input":{"path":"/home/me/designs/project/generated_unit_cube.f3d", …},"output":{"path":"/home/me/designs/converted/generated_unit_cube.sc", …},"mode":"import", …},
    {"ok":false,"input":{"path":"/home/me/designs/project/sub/container_metadata_only.f3d", …},"error":{"message":"cannot read this design: no geometry could be read from this Fusion archive (.f3d) file"}},
    {"ok":true,"input":{"path":"/home/me/designs/project/sub/solidworks_cube.sldprt", …},"output":{"path":"/home/me/designs/converted/sub/solidworks_cube.sc", …},"mode":"import", …},
    {"ok":true,"input":{"path":"/home/me/designs/project/sub/synthetic_cube.ipt", …},"output":{"path":"/home/me/designs/converted/sub/synthetic_cube.sc", …},"mode":"import", …}]}}}
```

FreeCAD conversions retain numeric spreadsheet cells and supported formulas as parameters,
resolving qualified object properties to parameter names. Text cells are omitted silently;
unused numeric cells produce `Info` notes. Expression fallback losses name the cause,
such as an unsupported function or an unresolved reference. The CLI and MCP use the same
losses and verification fields. Verification compares the initial geometry; it cannot
certify source dependencies that the decoder did not expose. See
[FreeCAD expressions](freecad.md#spreadsheet-parameters-and-expressions) before relying on
parameter edits, particularly named sketch constraint expressions and object-label references.

## Safety

- **Everything stays on your machine.** The server reads and writes local files only and
  sends nothing over the network. It runs `solidcraft-cli` locally for the history check.
- **What it writes.** `inspect` and `formats` write nothing. `convert` writes only its
  `output` file, matching the selected format. `convert_all` writes converted designs and
  its ordinary log inside `out_dir`. If supplied, `diagnostics`, `journal`, `report_anon`,
  and `report_key` are also written at their specified paths. `diagnostics_default` writes
  in the platform diagnostics folder and retains its newest 50 timestamp logs. A failed file creates no design; a mixed batch
  can still write successful designs, logs, and requested reports. A design is written only
  once it is complete. The server has no list of allowed folders: it can read and write
  wherever the account running it can. Point `output` and `out_dir` at a folder for converted
  designs, and check what your assistant asks for before you approve it.
- **Existing files.** `convert` replaces an existing `output` file, and `convert_all`
  replaces designs of the same name in `out_dir`. Use a fresh folder when that matters.
- **Caps.** Input files over 1 GiB are refused before they are read. One `convert_all`
  converts at most 10,000 files and searches folders at most 32 levels deep. An assembly
  brings in at most 1,000 part files and 100,000 placements, nested at most 32 deep. Each
  SolidCraft check stops after `verify_timeout` seconds. A message over 1 MiB is answered
  with an error and skipped.
- **Hostile files.** A damaged or unexpected file comes back as an error report, never as a
  crash of the server.

For an unreadable archive, the error keeps cadmpeg's cause and adds advice
to download or copy it again, check its extension, and report it on the
[A file didn't convert](https://github.com/dutchman-cad/dutchman/issues/new?template=file-did-not-convert.yml)
form with the file's anonymous report if the original opens in its CAD program.
For failure reports, request `report_anon`, review that file before sharing, and keep normal
JSON, logs, the journal, the key, and the salt private. Do not attach design files publicly.

Sketch history uses the same conic and constraint mappings as the CLI. Bounded parabolas,
hyperbolas, elliptical arcs up to one turn, and rational quadratic Béziers retain editable
geometry. Periodic sketch B-splines retain their exact shape through endpoint knot insertion.
Clamped uniform splines stay one spline; other spacing and repeated interior knots become
chains of exact spline pieces with shared endpoints (up to 200 pieces). Unequal weights
remain unsupported except for the rational quadratic Béziers above. Constraints that cannot
apply to a chain are left out with an Info message; the geometry remains exact.
An unsupported relation appears in `losses` as a counted `Degraded` sketch
message with its reason, while the solved sketch remains mapped. Static sketch patterns
retain point and radius associations; driving count and span parameters still produce a
constraint loss.

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

A flattened conversion keeps mapped sketches with resolved planes hidden after the imported
geometry for reference, with no features built on them. Its losses include one Info line,
`sketches: kept N sketches for reference`, with `; M left out` if any were omitted. Body
verification is unchanged. `import_only` leaves out history and retains reference sketches when the preset keeps
sketches. Use `sketches: false` as well, or `preset: "fast"`, for geometry alone.

SolidCraft assemblies with multiple top-level designs group each design's features in a
uniquely named timeline folder, keeping feature order and leaving joints outside folders.
Each folder added is listed in an Info loss with subject `timeline folders` and message
`"Name": N feature(s) grouped`. Parts and assemblies with one top-level design get no folders.

### FreeCAD output (experimental)

`convert` and `convert_all` accept `to: "freecad"` (default: `"solidcraft"`). For example:

```json
{"input":"/designs/part.f3d","output":"/designs/part.FCMacro","to":"freecad"}
```

Use a `.FCMacro` output path for `convert`; `convert_all` chooses that extension for its
jobs. The GUI uses the same output setting. The result uses `output.format:
"freecad-macro/1"` and the existing history and loss fields. Each macro embeds original STEP
geometry and rebuilds supported features through FreeCAD's public scripting API; parts
whose histories cannot be fully rebuilt retain their mapped prefix and mapped sketches
in a hidden **Rebuilt history (partial)** group, with STEP as the visible final shape when
the prefix differs. The report says `rebuilt N of M features; kept as reference`.
FreeCAD output is unverified and does not invoke SolidCraft or FreeCAD. `import_only` writes STEP geometry alone. See
[FreeCAD output](freecad.md#freecad-output-experimental) for feature coverage and running the macro.

Experimental Assembly output keeps supported joints, through sub-assemblies too, with
resolved connector frames, limits, and grounding. Unsupported operands or joint options
produce specific `joints` losses. An inferred grounded part is reported as `Info`. The macro
checks that solving preserves stored placements; failure removes the joints, restores those
placements, and warns in FreeCAD. `import_only` leaves joints out.

The CLI additionally supports experimental `--to fcstd` to run FreeCAD headlessly, save
and verify a native document, and `--to freecad --verify` to check a macro without saving
a document. These options are currently CLI-only; the MCP FreeCAD macro output above
remains unverified. `FREECAD_CMD` selects the headless executable and
`--freecad-timeout SECONDS` sets its 300-second default limit. See [FreeCAD output](freecad.md#freecad-output-experimental) for discovery,
measurement tolerances, fallback reporting and the `differs` verification status.

For self-hosted browser conversions, [Dutchman Web](web.md#settings) documents the shared
zip budget, concurrent upload and saved-data limits, private data folder, and allowed browser
origins. Uploads may be refused with 403 (origin check) or 503 (capacity); the CLI and MCP
reports are unchanged.

FreeCAD TechDraw drawing output is available from the CLI:
`dutchman convert part.FCStd --to dxf [--page NAME]`. It writes one
`part-PageName.dxf` per page; `-o` accepts an existing directory or a file base
(an exact file path with `--page`). Defaults require `--force` to overwrite;
explicit outputs replace unless `--no-clobber` is set. Page names are FreeCAD
object names. Pages keep neutral view curves, dimension labels, annotations, template frames, and
resolved title-block text in millimetre page space. Unsupported content is reported
in per-page JSON `pages` results. No page is an error. See [FreeCAD coverage](freecad.md#techdraw-to-dxf).
DXF conversion is not supported yet by convert-all, the MCP convert tool, desktop,
or web. The MCP formats report includes the additive `drawings` capability list.

The desktop app also offers Fast, Standard (default), and Full presets, with Advanced
checkboxes that leave history, sketches, or joints out. Estimates below 0.1 GB show
**under 0.1 GB**, and times below 60 seconds show **under a minute**, using the CLI’s text
formatter. The file row shows a live total, counting checking only with a usable CLI and checking on. Choices and the checking preference are remembered. Calibrated formats split reading, extra
conversion work after decoding and extra checking in the desktop. MCP keeps the existing
numeric estimate fields; the split is desktop presentation. History
Info messages, including history left out by Standard, appear on its result cards.

SolidCraft conversion results include `solidcraft: {target, found, known_newest, status}`: the selected
target release, discovered CLI release (or null), newest known release, and installed-version
status, even when verification is disabled.
Standard fixes its history budget before decoding for parts and assemblies; if only the
history-free estimate fits, conversion proceeds with one Info explanation. Memory refusals
happen before decoding. See [JSON reports](json.md) for diagnostic platform limits and UTC
log filenames.

With an explicit `solidcraft_version` and `verify: false`, version discovery uses only
cached results; `solidcraft.found` is null when not checked. Otherwise the CLI version
probe has a three-second limit and runs at most once per resolved CLI path per process.
Successful versions are cached across runs until the CLI's size or modification time
changes; see [SolidCraft versions](command-line.md#solidcraft-versions) for cache locations.

SolidCraft reports also include `solidcraft.known_newest` (a version string) and
`solidcraft.status`: `newer`, `too_old`, `older_loses_history`, `current`, or
`none_found`. History notices follow the selected target’s writer gates. These
fields require no network check. See [JSON reports](json.md).

SolidCraft version presentation uses minors: `solidcraft.target` and
`solidcraft.known_newest` now contain `x.y` strings (for example, `"0.30"`), changing their
previous `x.y.z` representation. `solidcraft.found` keeps the full CLI release (for example,
`"0.30.0"`). Status comparisons use minors, so all 0.30 patches are current; 0.31 is newer
and 0.17 is too old. Patch-sensitive writer gates and the internal 0.18.2 verification
floor remain unchanged. Explicit `solidcraft_version` inputs still accept `x.y.z` for
precise gate control.

Anonymous diagnostic logs include timed decoder preparation phases and, on failure, the
failure stage and error category. Model check failures add check-code counts and available
referenced curve-kind counts. A geometry-only recovery rechecks the decoded model in memory
only when failing checks involve history, sketches, or parameters; geometry failures stop
without repeating the decode. Fusion archives also record anonymous `member_classified`
timings for nested scanning, layer classification and reference metadata extraction before
the first member decode starts. See [diagnostic fields](json.md#dutchmandiagnostics1).

## `repro`: make a minimum reproduction

```json
{"name":"repro","arguments":{"input":"/models/part.f3d","out":"/results/reproduction"}}
```

Runs the default SolidCraft conversion with local diagnostics, then tests independent
archive members and retained CADIR subsets where structured findings permit it. `input`
and `out` are paths on the server; use a fresh output folder. Returns
[`dutchman.repro/1`](json.md#dutchmanrepro1): summary, kit paths and `anonymous.json`
content. `ok: true` means the kit was written; inspect `anonymous.status` to distinguish
failure from `no_failure_to_reproduce`. Errors without structured finding codes cannot
establish equivalent failures; the kit records narrowing limits.

Only `kit/anonymous.json` is safe to share. `kit/local-key.json` maps ordinals to source
names and stays with the user. Its content is omitted from the response by default.
`include_local_key: true` explicitly returns it **for a client keeping the response local**.
Do not enable that flag for a remote assistant, paste its response into an issue, or
upload the key. The response envelope contains local output paths even without that flag.
A chat assistant can coach from anonymous content while the user looks up names locally.
Follow `kit/next-steps.md` to rebuild a small design and rerun `repro` in a fresh folder.
Send anonymous.json and the rebuilt file only if it reproduces and the user is free to
share it. Never send the original design or local-key.json. The tool uploads nothing.

## Anonymous diagnostic tools

These tools reuse the CLI diagnosis engine. Their results contain part numbers, counts,
fixed error categories and signatures; they exclude source paths, design names and geometry.
Argument or I/O failures return a fixed anonymous error; inspect the local folder or use the
CLI locally for detailed errors. Paths supplied as arguments stay on the local machine.

| Tool | Required arguments | Behavior |
|---|---|---|
| `diagnose` | `input`, `out` | Rank parts; return the anonymous report. |
| `diagnose_narrow` | `input`, `out` | Rank, isolate and bisect; resume from checkpoints; return signature, known-problem advice and recipe. |
| `convert_part` | `input`, `part` | Try a numbered part and discard output geometry. |
| `stop_at_feature` | `input`, `part`, `feature` | Try history through that feature; 0 tries stored shape only. |
| `same_failure` | `input`, `out` | Check a small rebuild against the original signature in out. |
| `make_report` | `input`, `out` | Write the checked small file and anonymous facts to a local ZIP; send nothing. |

All except `make_report` accept `time_limit` (whole seconds or, for example, `30s` or `5m`) and a positive
`memory_limit_mb`. The two part-trial tools also accept `neighbours: true`. They return
`dutchman.part-trial/1`: `converted` records the trial outcome; `ok` says the tool ran.
The other tools use the existing diagnose, narrow, same-failure and made-report schemas,
with local `paths` omitted. No tool submits a report. The local report ZIP has the fixed
name `diagnose/dutchman-report.zip` under `out`; ask the user to review it before sharing.
Use `doctor` for folder checks, as described below.

See the [paste-in prompt](troubleshooting.md#diagnose-with-an-assistant).

### Local folder and STEP checks

Assembly completeness separates `placed` (geometry or child holders) from
`placed_empty` (leaf parts with no bodies), with both included in `placements`.
The summary and desktop and web cards show empty placements separately and mark the
assembly incomplete. Empty leaves add Degraded losses naming their anonymous part
numbers; child holders do not. Solid regions reduced to open sheets add a counted
Degraded geometry line; intended sheets do not. Doctor includes anonymous placement
counts and findings for empty placements and solids reduced to sheets.

`doctor` takes `input` (folder or ZIP), `out`, optional `jobs`, `memory_mb` (total
converter memory), `time_limit` (seconds per part), `step_path`, and `part` (report
target). It returns `dutchman.doctor/1` with anonymous file numbers and findings grouped
by signature. Checkpoints, converted designs and the local key stay in `out`.

`diagnose` also accepts `step_path`, `part`, `memory_mb` and `narrow`. Its anonymous
`report` includes the STEP comparison; `narrow: true` returns the same recipe and
known-problem results as `diagnose_narrow`. Both ranking tools and `compare_step` accept
`memory_mb` (at least 256) as an alternative to `memory_limit_mb`; supply only one.
`compare_step` takes `input`, optional `step_path`, `part`, `time_limit` and either
memory limit, and returns `dutchman.compare-step/1`. Without `step_path`, an unambiguous
same-name adjacent STEP is used. Comparisons return anonymous evaluated body numbers
and measurements, worst first. Body numbers are not assembly diagnose part numbers.
`unavailable` or `incomplete` is not a passing geometry check. Findings are advisory;
ask for confirmation before treating a visual difference as a defect. Neither tool
sends a file or opens a report. STEP files may be shared only through private sending.
Doctor's `geometry_checks` use the same anonymous measurement structure for stored source
versus converted bodies, with the conversion's joint setting and assembly pose.
Bounds and placement allowances have a 0.1 mm floor and
a 0.5 mm ceiling; volume and area allow 1%. Structural findings also include
`geometry_untrimmed_torus` for a toroidal face with no trim loops sharing a shell
with other faces. A whole torus in its own shell is allowed. Placement sanity also
uses the conversion's joint setting. See [calibration results](doctor-calibration.md)
for the observed false-positive rate and coverage. Optional local snapshot
pairs never enter the tool result.

<a id="report-preview-and-sending"></a>
Converted files are reportable too. For lost features, wrong placement or missing colours,
call `report_preview` with a fresh `folder`, the file's diagnostic `report_id`, its
`source_format` (for example `f3d`) and `signature: "converted-with-problem"`. This prepares
an anonymous summary without opening or sending the design. Review the returned `metadata`
with the user, then call `report_submit` with `choice: "summary"` only after consent.
There is no description field in the supported anonymous envelope; use the private DM
or GitHub form for a description without private names or paths.
The CLI equivalents are `dutchman report --folder FOLDER --report-id ID --source-format f3d
--signature converted-with-problem`, followed after review by
`dutchman report --folder FOLDER --send-summary --yes`.

`report_preview` takes `folder` and, for a new report, `report_id`, `source_format`
and `signature`. Reuse the diagnostic file id. The result shows parts and sizes,
never bundle contents or claim secrets. `metadata` shows the exact summary envelope with
default consent. Use the folder saved by `extract_repro` to
preview a proven bundle; a new report without a bundle offers summary. Nothing is sent.
`report_submit` takes `folder`, `choice` (`bundle` or `summary`), `public_test`
(default false) and `explicit_yes`. Ask the user for explicit consent to this exact
preview and choice before passing true. A failure does not authorize a later resend.
No tool accepts whole files for anonymous upload. Use the private-send card instead.

Report summaries identify the authoring app from the native format, such as Fusion or
Inventor. Exchange formats and unclassified `.prt` use `unknown` because their authoring
app cannot be inferred. The build version is included alongside this information.

The desktop report review selects the anonymous summary when no bundle is available.
Its **Send** button becomes **Retry** after a delivery failure.

`extract_repro` takes absolute `input` and `out` paths, with optional `time_limit` and
`memory_limit_mb` as for diagnose. It cuts, proves and shrinks a local reproduction bundle
without narrowing. `verify_repro` takes the absolute `folder` containing the saved
`bundle.json.gz` and `prepared.json` and rechecks its signature. Both return anonymous
results: availability or reproduction status, sizes, signature hash, and the manifest;
never paths, checker messages, entity identities, or geometry. Neither sends anything.
`report_preview` can use that same folder to review the saved bundle. Bundles are the
default when available. The entire multipart request must fit 25 MiB; a 64 KiB
reserve keeps bundles below the cap. Preview checks metadata and framing overhead
before marking a bundle sendable. Use the private-send card or summary when it will not fit.
Decode refusals have no bundle yet.

A cut refused by the fragment limits returns a normal unavailable-bundle result,
with a plain explanation and summary or private-send options; it is not a tool error.

## Assembly placements

Conversion tools accept `part_positions`: `"smart"` (default), `"last-shown"`, or
`"first-placed"`. Smart accepts current poses only when touching joints close (or there are
none), geometry fits in first-placement assembly bounds grown by 10%, and repeated inserted
designs move together. It keeps first placements when a check fails or cannot be established.
Last-shown uses available current poses without those checks; first-placed ignores later
moves throughout. Joints work from the selected placements.

Smart and last-shown prefer decoder-bound saved-scene displayed poses, without current-pose
guards. Only bound rows are used; ambiguous and unbound rows are skipped, and an absent
arena supplies none. Fallback current poses may be stale. To restore the old
placement choice, pass `part_positions: "first-placed"`. The boolean `stored_placements: true`
remains an alias; conflicting choices are rejected. Reports and dry runs include
`choices.part_positions`; successful reads add an Info summary and `part_positions` rule
counts and fallback reasons. `choices.stored_placements` remains a compatibility boolean,
true only for first-placed.

Missing assembly holder poses are inferred bottom-up from displayed children only when
their world-pose estimates agree within 0.001 mm and 1e-6 rotation entries. Conflicts keep
the existing placement source and are reported. JSON `part_positions` adds `derived_holders`
and `holder_conflicts`; `bound_displayed` and `derived_holders` partition `displayed`, preserving the displayed/current/first-placement total.

Local CLI and desktop progress names the running part or stage and its elapsed time,
including “Still …” notices and full-sentence check-limit consequences. Durations use one
format, such as `1 min 1 s` or `2 min`. Desktop folder runs keep the longest-running file,
its stage and step visible below the completed count, with the number of other active files. See
[progress during conversion](command-line.md#progress-during-conversion). MCP final reports
are unchanged; running names are excluded from anonymous diagnostic logs.
