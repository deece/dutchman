# Troubleshooting

For a question, or to talk a problem through first, post in the Convert help forum on the
[Dutchman CAD Discord](https://discord.gg/yTcseBEdeu).

For a failed conversion, or a design that looks wrong in SolidCraft, start with an anonymous
report (replace `part.f3d` with your file or folder):

```sh
dutchman convert-all --out-dir converted --report-anon report-anon.json part.f3d
```

In the desktop app, **Report anonymously on GitHub** on the failed file (under **Help us fix it**)
does this for you. Review
`report-anon.json`, then paste it into the
[**A file didn't convert**](https://github.com/dutchman-cad/dutchman/issues/new?template=file-did-not-convert.yml)
form on Dutchman's issues (for a whole library, the **Library test report** form). Keep
ordinary logs, JSON output, the journal, the report key, and the salt private: they contain
names, paths, or a way to identify files. Keep the original design private. You may attach a
small rebuilt reproduction only if it reproduces and you are free to share it. See
[Report what you found](testing-your-library.md#5-report-what-you-found) for the three
required versions and [Want to go further?](testing-your-library.md#want-to-go-further)
for the four optional tiers, which use only a synthetic reproduction; an agent asks once how
far to go.

On Linux, run `dutchman-gui` from a terminal and confirm that your X11 or Wayland desktop
session has working OpenGL and graphics drivers. Include the terminal diagnostics when
reporting startup trouble. A graphics startup error prints that advice and exits with
status 1, as does a start that stalls for 30 seconds. The command-line `dutchman convert` works without
a desktop session.

Every error is one line on stderr, and the exit code is 1 (2 for a mistake on the command
line). A failed file creates no design; a batch can still write successful designs, logs,
and requested reports.

| Message | What to do |
|---|---|
| `x.step: output must be a .sc file` | Name the output `something.sc`. |
| `a.txt: Dutchman does not convert this kind of file. It converts …` | Choose one of the formats listed by `dutchman formats`. STEP exchange files convert too. |
| `Dutchman doesn't convert IGES yet: open it in SolidCraft directly (File > Import).` | Open the `.igs` or `.iges` file in SolidCraft using File > Import. |
| `… reading geometry is not supported yet. Export a STEP file and convert that instead.` | The format is recognised, but its reader has no geometry path yet. Export STEP from the program that made the design and convert that. |
| `cannot read this design: its contents are inventor data, not Fusion archive (.f3d); rename it to match` | The file's extension does not match its contents. Rename it with the right extension. |
| `cannot read this design: no geometry could be read from this Fusion archive (.f3d) file` | The file holds no geometry Dutchman can read yet, for example a design saved without bodies, or one whose geometry is stored in a form cadmpeg does not read yet. |
| `cannot read this design: malformed container: …` | Download or copy the file again, or check its extension. If the original opens in the program that made it, use the anonymous reporting route above. |
| `cannot convert this design: the model has no solids or surfaces SolidCraft can import` | The file was read but held nothing SolidCraft can hold as a solid or surface: no body, or no face whose bounds can be written. When only some faces of a body can be bounded, those faces convert as surfaces and the report lists the rest. |
| `missing.f3d: No such file or directory (os error 2)` | Check the path. |
| `part.f3d: the file is … bytes, over Dutchman's 1073741824-byte limit` | Dutchman opens files up to 1 GiB. |
| `history: …; unverified: …` | Not an error: the history was written without the SolidCraft check. Install `solidcraft-cli`, or point `--solidcraft` or `SOLIDCRAFT_CLI` at it, and convert again. |
| `history: …; unverified: no solidcraft-cli found …` | Dutchman looked for `solidcraft-cli` in `SOLIDCRAFT_CLI`, `PATH`, beside `dutchman` and the SolidCraft install folders and found none. `SOLIDCRAFT_CLI` is an environment variable holding the full path of the `solidcraft-cli` to use; the search order is in [How Dutchman finds SolidCraft](desktop.md#how-dutchman-finds-solidcraft). The JSON report's `solidcraft.search` lists each place tried. |
| `history: flattened: replay differs (…)` | Not an error: the history did not rebuild the same shape in SolidCraft. A retained editable timeline is hidden beside the imported final shape when possible. `--no-verify` keeps the history, unchecked. |

## A large file takes hours, runs out of memory, or fails in one part

Run `dutchman diagnose big.f3z --out findings`. It converts the design part by part on your
machine, each part under a time and memory limit, and writes a ranked report that names parts
by number: which fail, which are slow, which take the most memory. Then
`dutchman convert big.f3z --part 17` converts only that part, in seconds. A conversion that
cannot fit in the machine's free memory stops early and names the largest parts by number
instead of running for hours. `findings/diagnose/report.json` and `report.txt` are safe to
share; `local-key.json` is not. See [Find what breaks in a large private file](diagnose.md).

To have Dutchman narrow it all the way down by itself, with no AI assistant:

```sh
dutchman diagnose --narrow big.f3z --out findings            # run it again to resume after a stop
dutchman diagnose --same-failure small.f3d --out findings    # after rebuilding the failure
dutchman diagnose --make-report small.f3d --out findings     # one zip and a prefilled issue link
```

`--narrow` finds the failing part and the first feature it fails at, and records the failure's
signature (kind of feature, options, profile shape, error code; never names, paths or real
dimensions). `findings/diagnose/kit/next-steps.md` tells you what to rebuild. `--same-failure`
says whether your small file fails the same way, and `--make-report` gathers the report.
Nothing is sent: the zip stays on your machine until you send it. See
[Let Dutchman do all of it](diagnose.md#let-dutchman-do-all-of-it---narrow).

## Replay a history by hand

To find which feature of a converted history goes wrong, convert with `--write-replay`:

```sh
dutchman convert part.f3d --write-replay
solidcraft-cli run part.replay.json --in part.sc --results --allow-errors --json-out steps.json
```

`part.replay.json` is the command script Dutchman gives `solidcraft-cli` to rebuild the
history: for each feature in order, one `timeline.roll_to` command that rolls the timeline to
just after it, then one `document.inspect` that lists the bodies there. It is written whether
or not the history is checked, so it also works with `--no-verify`. Delete the commands after
the feature you want to look at, or keep only one pair, and run it again to see the bodies at
that step. `convert-all --write-replay` writes one script per `.sc` design.

A `.sc` design is a ZIP archive. `document.json` holds the design (its features, sketches and
parameters) and `blobs/*.step` the imported shapes. Any ZIP tool opens it, which shows what
a design holds without opening SolidCraft.

## "SolidCraft refused feature ..."

The history stopped because SolidCraft itself would not rebuild that feature; it is not a
Dutchman mapping limit. The rest of the model is kept as the imported shape. The report's
`history.features[]` entry carries `source: "solidcraft"` and a `category`. For
`fillet-overflows-neighbouring-face` (the blend is larger than the neighbouring faces) the
`upstream` link is https://github.com/tangency-labs/solidcraft/issues/4782. Shrinking that fillet
in the source usually avoids it.

## Send the file privately

If a file opens in the app that made it but Dutchman can't convert it, it's a bug in Dutchman,
and the file is the quickest way to fix it. A failed file's card in the desktop and web apps
has the steps (in the desktop app, open **Help us fix it** under **Review and send report**):

1. **Join our Discord server.** Already in? Skip this step.
2. **DM Dutchy.** The button copies a message and opens Dutchy's DM. Paste the message.
   It gives Dutchman's version, the error's code, the report id, the file's size, and whether a
   STEP is at hand. It names no file, and the report id is the one in the anonymous report and
   the diagnostic log.
   - A file of up to 10 MB: drag it in. The desktop app opens its folder.
   - A larger file: Dutchy replies with a private upload link, for up to 300 MB for the design,
     300 MB for its STEP, or a 1.5 GB zip, for 72 hours.
3. **Have a STEP export?** If you can export a STEP of the same design, add it too. It helps us
   compare, but it isn't required.

Only the Dutchman team sees it. It's never published or used in public tests, and we delete it
on request.

To send no file, use **Report anonymously on GitHub** instead: no file, no names.

## When you can't share the file

Make a local reproduction kit:

```sh
dutchman repro part.f3d --out reproduction
```

Use a fresh output folder. Dutchman runs the conversion with diagnostics, then tests
independent archive members and model subsets where the failure stage allows it. A
successful conversion writes a kit saying **no failure to reproduce**. On failure, the
kit gives finding codes and kinds, anonymous ordinals and explicit narrowing limits.

- `reproduction/kit/anonymous.json` is safe to share: fixed categories, coarse size class
  and counts, with no source names, paths, entity ids, geometry or parameter values.
- `reproduction/kit/local-key.json` maps ordinals back to your own names. **Do not share it.**
- `reproduction/kit/next-steps.md` tells you how to rebuild a small candidate in your CAD
  program and compare the failure stage and complete code set. The recipe is a starting
  point; a seam-crossing shape does not guarantee a decoder failure.

Send anonymous.json and the small rebuilt file only if it reproduces and you are free
to share it. Never send the original file or local-key.json. No file is uploaded by the
command. A remote chat assistant can coach from anonymous.json while you look up names
locally; the MCP tool returns private key content only with an explicit local-only flag.
See the [worked example](minimum-reproduction.md) and
[paste-ready AI prompt](testing-your-library.md#let-your-own-ai-make-a-minimum-reproduction).

## Diagnostic logs for unfinished runs

Enable diagnostics **before** reproducing a conversion that hangs or stops unexpectedly:

```sh
dutchman convert --diagnostics-default part.f3d converted.sc
dutchman convert-all --diagnostics-default --out-dir converted parts/
dutchman watch inbox --diagnostics-default --out-dir converted
```

Each invocation creates one JSON Lines file named by UTC start time, Dutchman version and
result, such as `2026-11-02_163710_dutchman-0.2.6_failed.jsonl`. The name starts with a
`running` result and changes to `converted`, `failed` or `stopped` when the run ends.
A crash leaves the `running` name. A counter keeps simultaneous starts unique. Names and
contents from the design never enter the log or its filename. If renaming is unavailable,
the log keeps its `running` name and the conversion result is unaffected. The default folder is:

| System | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\Dutchman\diagnostics\` |
| macOS | `~/Library/Logs/Dutchman/` |
| Linux | `$XDG_STATE_HOME/dutchman/diagnostics/`, or `~/.local/state/dutchman/diagnostics/` when unset |

The newest 50 automatically placed logs are kept. Starting a run removes older logs
whose filenames match this pattern, including older time-only names such as
`2026-10-29_201500.jsonl`; unrelated files are left alone.
`--diagnostics /absolute/path/run.jsonl` instead appends to an explicit file, without
retention or renaming. Reusing a log from another version shows an advisory notice, for
example, "This log is from Dutchman 0.2.5; this is 0.2.6." Conversion still proceeds.
Choose either `--diagnostics-default` or `--diagnostics PATH`.
The MCP `convert` and `convert_all` tools accept `diagnostics` (an explicit path) or
`diagnostics_default: true`.

The desktop app always keeps a diagnostic log of each run (no names or contents from your
file) in the default folder, which **Open diagnostics folder** opens.
Each result card offers **Show this run's log** and **Copy log to…** under its "Run log:" line,
available when the run ends.
Show selects the exact file on Windows and macOS, or opens its folder on Linux. Copy opens
a save dialog with the same filename and copies the log to the location you choose.
A batch shares one log, so every card opens or copies that same file. A retry has its own
log; the other cards keep their original log. Windows folder paths use backslashes.
On launch, an unfinished conversion in the newest default log shows one line, **The last run
didn't finish.**, with **Open its log folder** and **Dismiss** (hover for the log's place). In a
wide window it sits at the top of the Results column, so the main screen still fits.

Default diagnostic filenames use UTC on every platform, formatted from the system clock
without launching a timestamp command. Diagnostic OS and total-memory metadata are read
only when logging is enabled. On Windows the OS version field contains `windows`.
Available memory, total memory, and heartbeat process memory and CPU time are read through
safe system APIs on Windows and macOS, without launching memory-probe commands. Heartbeat
resource fields are available on Linux, Windows, and macOS; a measurement that cannot be
read is null. Linux keeps its `/proc` readers and honours cgroup memory limits when
checking available memory.

Every record is appended and flushed immediately, with a disk sync at least every five
seconds and on every stage change. A background heartbeat every 30 seconds records the
stage, elapsed time, process memory, CPU time, and last decoder member id when available.
Fusion archives include anonymous member sizes; per-member records appear when the decoder
reports them, including re-decodes and fixed failure categories. No progress bar total is
inferred from these decode attempts. A stopped process leaves its complete lines usable;
an interrupted final line may be incomplete. A supervised batch worker that times out or
is cancelled gets an error `end` record; killing the supervising process can leave no `end`.

The schema is [`dutchman.diagnostics/1`](json.md#dutchmandiagnostics1). It holds versions,
machine resources, selected conversion choices, input and archive byte sizes, timings,
salted ids, preparation phase durations, anonymous failure categories, and check/loss codes
with counts. Carrier check failures also count referenced curve kinds when available. It contains no file or design names, paths, geometry,
file contents, or loss/error messages. Salts use the anonymous report's random source and
hashing scheme; when `convert-all` also requests `--report-anon`, both share its salt and
file ids, including `--report-salt` when supplied. Keep that salt and any report key private.

Review the JSONL file, then attach it to Dutchman's **A file didn't convert** issue or send
it with your problem report. No design file is needed. Diagnostics work while conversion
is still running; [the anonymous report](report-anon.md) adds a batch summary after it ends.
Ordinary `--log`, JSON conversion reports, and journals contain names and stay private.

### Model checks and long Fusion archive preparation

When cadmpeg's model fails checks involving history, sketches, or parameters, Dutchman
removes those arenas and checks the existing decoded model as geometry alone. It keeps
that geometry if the recheck passes. A failure confined to geometry/carriers stops
immediately and says a geometry-only retry was not needed. Neither path reads the
container again. Drawing conversions keep their drawing graph and do not take this retry.

A long `container_prepare` phase before the first `.f3z` member starts belongs to cadmpeg.
Its `archive_scan`, `archive_classify` and `archive_session_open` phases say which step took
the time.
During `archive_classify`, `member_classified` records identify each classified F3D member
by the same anonymous id as member decode rows, including unreachable members. `ms` is its
total preparation time; `nested_scan_ms`, `layers_ms` and `metadata_ms` show time spent
expanding and scanning the nested container, classifying and merging layers, and extracting
reference metadata. These records arrive before the first member decode starts, so a slow
preparation can be traced to a member and stage without exposing its path. Records are
capped at 100,000 per conversion and include completed work before a refusal.
The actual member decode reopens its scan again. Final composition and document hashing
after the last member are timed as `container_compose`.

The estimate uses fixed overhead plus rates on declared archive content sizes, with a
bounded nested-directory inspection. Large nested members beyond its metadata budget
use their outer declared size. It has no separate size-dependent term for the repeated
inflation, classification, and metadata scans above; a content-rate estimate can therefore
miss lengthy preparation. Memory and time estimates are planning approximations, not
measured limits. Diagnostic instrumentation does not change their calibration.

## Diagnose with an assistant

Without an assistant, choose **Diagnose this file** on the desktop failure card, or run
`dutchman diagnose --narrow FILE --out RESULTS` and follow `diagnose/kit/next-steps.md`.
With dutchman-mcp configured locally, paste this prompt and substitute your local paths:

> Use Dutchman's anonymous diagnostic tools on INPUT, writing local results under OUT.
> Run dutchman --version and solidcraft-cli --version if installed, using its full path
> or SOLIDCRAFT_CLI if needed; record "none" if no SolidCraft was installed. Copy the
> report's versions block verbatim for any issue draft, and add those outputs separately.
> State the source program/version from metadata already reported by Dutchman; ask me once
> if missing. If exact builds are unknown, give the program, desktop or web, and file years.
> Start with diagnose_narrow; it resumes a stopped run and checks shipped known problems.
> Explain any workaround first. If useful, try convert_part with the anonymous part number,
> optionally with neighbours, and stop_at_feature to test a history prefix. Show me the
> numbered recipe for my CAD program and help me rebuild a small file with round sizes.
> After I save it, use same_failure on that small file with the original OUT. If it converts
> fine or differs, explain which recorded options or profile counts to revisit. Once checked,
> use make_report to prepare the local ZIP. Tell me where to review it locally. Do not read
> local-key.json, the original file, the rebuilt file or the ZIP into the conversation.
> Do not upload anything or open a submission link; I will review and choose what to send.
> End by showing the remaining tiers from "Want to go further?" in Testing Dutchman on your
> library and asking once: "How far do you want me to go? (stop here / Tier 2 / Tier 3 / all the way)";
> then do every tier up to my answer without asking again, and stop early only if a tier fails or needs something from me (such as my GitHub login or a small design rebuilt by hand).

The assistant sees categories, counts and signatures. The report ZIP itself contains the
small file's geometry. A broad matching error category alone does not prove the same
failure; the check says when that limitation applies.
