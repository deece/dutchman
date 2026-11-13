# The anonymous report: `dutchman.report-anon/1`

When filing an issue, copy `versions` verbatim, including `versions.dutchman`. See
[Report what you found](testing-your-library.md#5-report-what-you-found) for the required
Dutchman, SolidCraft, and source program versions and the optional steps after reporting.

`dutchman convert-all --report-anon FILE …`, `dutchman convert --report-anon FILE …` for one
file (the report of a batch of one, with `--report-salt` and `--report-key` as for
`convert-all`, and the same stderr notice with the salt), or the MCP `convert_all` tool's
`report_anon` writes, beside the usual results, one JSON file that says how a batch went without saying
which files were in it. It is made to be shared: posted in an issue, sent to the maintainers.

For a conversion that never finishes, enable [a live diagnostic log](troubleshooting.md#diagnostic-logs-for-unfinished-runs)
before trying again: `--diagnostics-default`, or `--diagnostics PATH` for an explicit file.
Unlike this completed-batch report, [`dutchman.diagnostics/1`](json.md#dutchmandiagnostics1)
writes progress and resource heartbeats during the run, with no messages at all. A run using
both reports shares this report's salt and file ids. The desktop app always keeps diagnostics
and shows or copies the exact run's log from each result card, after a stopped run or on its
next launch. Automatic names include UTC date and time, Dutchman version and result
(`running`, `converted`, `failed` or `stopped`); they never include design names. Explicit
`--diagnostics PATH` files keep their chosen path. Report review names the run's log, but
uploads only the summary or bundle reviewed there. The Discord DM names that same log;
you choose whether to attach it. The log has no names or contents from the design.

## Converted files with a wrong result

Converted files are included in this report as well as failures. The desktop's
**Something wrong? Report this file** section offers anonymous review and private-send
steps for every converted result that has a saved report, whatever it lost: history not kept
or only partly kept, features flattened or blocked, parts left unplaced, or colours dropped,
at any loss level. The card has a visible **Report this file…** button that opens the section;
the section itself starts closed. Only an assembly whose part files were not found
is offered the missing-parts search instead.
The separate endpoint summary envelope (`dutchman.report-send/1`) supports only bounded
metadata and sharing choices; it has no free-text description field. The review displays
that exact envelope. Converted-file summaries use `converted-with-problem` as their
signature, with no geometry or older failure bundle. No description is added to either
anonymous format. Describe what is wrong in the private DM or GitHub form, leaving out
private names and paths. The CLI and MCP summary paths are described in
[the guide](guide.md#report-consent).

## What it never holds

- **No file or folder names, no paths.** Each file appears only as an id, `f` and 16 hex
  digits: a SHA-256 hash of the file's full path and a secret salt. Nobody can turn an id back
  into a path without the salt and the path.
- **No part, component, feature or sketch names.** Messages keep their wording, but every
  name the conversion met is replaced by `<name>`: the names of the files and folders it read
  from (the project folder, the Inventor project file and the folders it names, every part
  found), and every name the design gives a part, component, body, feature, sketch, parameter
  or property (each convert report lists them in `names`). Every path, file name, quoted name
  (`"…"`, `'…'`, `` `…` ``, `“…”`), word with a path separator in it, and long hexadecimal id
  (a GUID, a hash) is replaced by `<path>`, `<file>`, `<name>` or `<id>` too. A name is taken
  out where it stands as a word of its own; plain lower-case words (`import`, `parts`) are
  left, as messages are made of them.
- **No names in codes.** A loss code is cadmpeg's own fixed code, or `dutchman/` and one of a
  fixed set of words (`history`, `feature`, `sketch`, `joints`, `parts`, …, `other`) or an exact [appearance cause](#appearance-loss-causes).
- **No geometry**: no coordinates, sizes, volumes or models.
- **No salt.** The salt is printed on your screen when the run ends, and written to the key
  file if you ask for one. It is never in the report.

Read the report before you share it. It is plain JSON. If you find a name in it, leave that
line out and say so in your issue; it is a bug in Dutchman.

## Matching an id to your file

The ids depend on the salt, so the same file gets the same id whenever you use the same salt:

- `--report-key FILE` writes the key, `{"salt": "…", "files": {"f…": "/path/to/file.f3d"}}`.
  Keep it to yourself.
- `--report-salt HEX` runs with a salt you already have (the one printed last time), so the
  ids match an earlier report.

When a maintainer asks about `f3a9c0e1d2b4f5a6`, look it up in your key.

## Fields

```json
{
  "schema": "dutchman.report-anon/1",
  "versions": {"dutchman": "0.1.5", "cadmpeg": "0.6.5+63ebadd", "solidcraft": "0.18.2",
               "solidcraft_search": null, "os": "linux", "arch": "x86_64"},
  "run": {"verify": true, "import_only": false, "jobs": 8, "timeout_s": 900,
          "max_memory_mb": null, "resumed": 0},
  "totals": {"files": 105, "converted": 77, "history_verified": 3, "history_unverified": 0,
             "geometry_only": 74, "degraded": 77, "incomplete": 3, "failed": 28,
             "skipped": 2},
  "formats": {"fusion_archive": {"files": 40, "converted": 38, …, "timing": {…}}, …},
  "skipped_by_extension": {"txt": 1, "pdf": 1},
  "stages": {"decode": {"failed": 25, "losses": 119, "files": 83}, …},
  "errors": [{"message": "cannot read this design: …", "stage": "decode",
              "upstream": "cadmpeg", "files": 23, "occurrences": 23,
              "formats": {"fusion_archive": 23}, "file_ids": ["f…", …]}],
  "losses": [{"code": "f3d/geometry.not-transferred", "level": "Dropped", "stage": "decode",
              "upstream": "cadmpeg", "example": "ASM BREP geometry was not transferred: …",
              "files": 4, "occurrences": 4, "formats": {…}, "file_ids": […]}],
  "blocked_features": [{"kind": "fillet", "reason": "fillet is not mapped yet",
                        "stage": "map", "files": 2, …}],
  "verification": [{"status": "flattened", "reason": "feature \"<name>\": …", "files": 6, …}],
  "timing": {"count": 105, "p50_ms": 657, "p90_ms": 1386, "p99_ms": 2061, "max_ms": 2557,
             "total_ms": 77854},
  "slowest": [{"file": "f…", "format": "fusion_archive", "ms": 2557,
               "outcome": "geometry_only", "stage": null}]
}
```

| Field | Meaning |
|---|---|
| `versions` | Dutchman's version, the cadmpeg it was built with (its version and the commit Dutchman pins), the SolidCraft CLI that checked the designs (`null` when none did), `solidcraft_search` (when none did: how the search for `solidcraft-cli` went, as `{"outcome", "places"}` with only generic place names such as `SOLIDCRAFT_CLI`, `PATH` and `Program Files` and an outcome for each, never a path; `null` otherwise), and the operating system and processor. |
| `run` | How the batch ran: whether designs were checked in SolidCraft, `--import-only`, files at once, one file's time and memory limits, and how many results came from `--journal`. |
| `totals` | Every file: `converted`, then how: `history_verified` (editable history, checked against the original in SolidCraft), `history_unverified`, `geometry_only`; `degraded` counts converted files with at least one `Degraded` or `Dropped` loss; `incomplete` counts converted assemblies with placements left out; `failed`; `skipped` (not in a format Dutchman knows). |
| `formats` | The same counts per input format (`fusion_archive`, `fusion_multi_document`, `inventor_part`, `inventor_assembly`, `solidworks_part`, `solidworks_assembly`, `freecad_document`, `unknown`), each with its `timing`. |
| `skipped_by_extension` | The skipped files by extension: lower case, `none`, or `other` for one that is not 1–8 letters and digits. |
| `stages` | For each stage (`input`, `decode`, `project`, `map`, `write`, `solidcraft`; see [Stages](json.md#stages)): the files that `failed` there, the `losses` that arose there, and the `files` with either. |
| `errors` | Every distinct failure, most files first. Messages that differ only in their numbers are one cause. |
| `losses` | Every distinct loss: by cadmpeg's loss `code`, or for Dutchman's own by a fixed appearance cause code (counts only), or `dutchman/<kind>` (what it is about: `appearance`, `assembly`, `bodies`, `components`, `constraints`, `feature`, `full`, `geometry`, `history`, `joints`, `occurrence`, `parameter`, `parts`, `pattern`, `profile`, `sketch`, `validation`, `writer`, else `other`) and message. `example` is one message, cleaned, except for appearance cause codes, which carry counts only. |
| `blocked_features` | The source features that kept a history from being written, by `kind` and `reason`. Each cause also lists `features`, in timeline order: the feature's `ordinal` (its position number in the design's timeline, as the source model numbers it and as the `ordinal` of the same feature in the [JSON report](json.md)), its `variant` (a short form such as `extrude, to a plane, start from a face`, `hole, counterbore, through all` or `fillet, 2 radius groups`, from the feature's extent, start, hole form or group count, never a name or a size) and `count`, the blocked features of that ordinal and variant across the batch. At most 50 positions are listed. A feature SolidCraft refused also carries `source` (`solidcraft`), `category` and `upstream` (a fixed category name and an issue link, never a design name). |
| `verification` | Why histories were written unchecked (`unverified`), not written (`flattened`), or not applicable, by reason. |
| `timing` | Times per file, in milliseconds: the nearest-rank 50th, 90th and 99th percentiles, the longest and the total. Files that were never started (a path that does not exist) are not timed. |
| `slowest` | The 20 slowest files, slowest first: id, format, time, outcome and, for a failure, its stage. |

Every cause carries `files` (how many files show it), `occurrences` (how many times in all),
`formats` (files per format) and `file_ids` (up to 20 of those files' ids).

The report's fields are added to, never renamed or removed, while its schema stays
`dutchman.report-anon/1`.

## Appearance loss causes

Appearance assignments left out of the output use these fixed codes in both the anonymous
report and the diagnostic log's `end.losses` counts. These categories never contain a target
id, binding id, colour name or component name. Anonymous appearance entries have no `example`.

| Code | Cause |
|---|---|
| `dutchman/appearance.redundant-face` | Face colour equals its body's colour, with no visible effect. Reserved: these assignments are retained and produce no loss count. |
| `dutchman/appearance.wire` | Target is a wire body or saved free edge. |
| `dutchman/appearance.face-pick-missed` | Face geometry could not be matched to an output face with a resolved interior pick. |
| `dutchman/appearance.target-absent` | Target is absent from the decoded model. |
| `dutchman/appearance.unwritable-body` | Target belongs to a body with no geometry the writer can write. |
| `dutchman/appearance.other` | Another appearance mapping gap, such as a missing colour, conflicting assignments or no placed component geometry. |

The exact codes above are allow-listed; arbitrary `dutchman/appearance.*` strings are not
accepted as categories. Assignments kept as body colours, uniform face colours or individual
face colours are not counted as dropped. Multiple dropped assignments count individually,
even when they share a target. Ordinary conversion reports retain their human explanations.
