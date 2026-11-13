# Testing Dutchman on your library

This guide is for anyone willing to run Dutchman over their own CAD library and tell us what
broke. You convert everything on your own machine and choose what, if anything, reaches us:
an **anonymous report**, which says what converted and what did not, by cause, with no file
names, paths, part names or geometry in it, or a report you review and send from the app.

If you used 0.1.3's report-anon.json, re-run with 0.1.4 before sharing it.

Fusion (`.f3d`, `.f3z`) libraries are the main target. Inventor (`.ipt`, `.iam`) and
SolidWorks (`.sldprt`) and FreeCAD (`.FCStd`) files are welcome too,
including ones you cannot open any more.

## Ask your AI about one result

In the desktop app, choose **Ask your AI about this file** on a converted card with
losses, then paste the copied prompt into your own assistant with local file and command
access. No server setup is needed. The prompt names the original, design, and adjacent
`<stem>.report.json`; it first reads the installed `AGENTS.md` and conversion skill, then
opens that plain JSON,
without converting the whole file again. Full guidance paths come from the running app's
folder; matching GitHub URLs are fallbacks when those files are absent in a source build.
The report has the CLI's `dutchman.convert/1` schema and includes private local paths, so
keep it local.

For a specific loss needing more evidence, the prompt uses the full `dutchman` command
path beside the app, with `--json`, and puts follow-up outputs in a new scratch folder
next to the result. It keeps the original and converted design untouched and asks before
sending anything anywhere. The button only copies text; nothing is sent.

## Everything stays on your machine

- Converting and checking your files makes **no network connections**. Dutchman reads your
  files, writes the designs and reports where you tell it to, and starts only two kinds of
  program: itself (one copy per file, so one bad file cannot stop the run) and, if you have it,
  SolidCraft's command-line tool to check the results. On macOS and Windows it also runs `ps` or
  `tasklist` to measure how much memory each file takes. The app's update check says nothing
  about your files; `DUTCHMAN_UPDATE_CHECK=0` turns it off.
- Your files are never changed. The converted designs go to a folder you name.
- Nothing about your files is sent anywhere unless you send it yourself, after reviewing it.

## 1. Install

1. Install Dutchman 0.1.4 or newer, as [the guide](install.md) shows: on a Mac, the
   disk image (`Dutchman Convert.app`, or `Dutchman.app` in 0.1.5 and earlier, with the
   `dutchman` command inside it, which the guide shows how to put on your `PATH`); elsewhere, the release archive. Check that `dutchman --version`
   prints the version.
2. Optional, recommended: install a current SolidCraft (any release from the last
   two weeks, currently 0.18.2–0.41). Older targets down to 0.18.2 get simplified features. With it, Dutchman checks every converted design
   in SolidCraft: whether a feature history replays to the same shape, and whether the
   geometry shows whole, with a volume. Dutchman finds the command-line tool inside an
   installed SolidCraft app (`SolidCraft.app` on macOS, under Program Files on Windows). If
   you have only the tool, `solidcraft-cli` from a
   [SolidCraft release](https://github.com/tangency-labs/solidcraft/releases), put it on your
   `PATH` or tell Dutchman where it is:

   ```sh
   export SOLIDCRAFT_CLI=/path/to/solidcraft-cli          # macOS, Linux
   ```

   ```powershell
   $env:SOLIDCRAFT_CLI = "C:\path\to\solidcraft-cli.exe"  # Windows
   ```

### Stable or nightly

Use the latest stable release. When an issue you filed says the fix is "merged, not released
yet", or a fix depends on a cadmpeg change newer than the stable build's, you can try it at once with the
[`nightly`](https://github.com/dutchman-cad/dutchman/releases/tag/nightly) pre-release:
Dutchman's and cadmpeg's latest code, rebuilt each day once every test passes. Run the same
batch with it (a new `--journal`, since a different version converts everything again), and
say in the issue which you used: its notes give the commits.

## 2. Run the batch

Pick a folder for the results **outside** your library, then run (one line on Windows,
without the `\`):

```sh
dutchman convert-all ~/CAD \
  --out-dir ~/dutchman-test/designs \
  --journal ~/dutchman-test/journal.jsonl \
  --report-anon ~/dutchman-test/report-anon.json \
  --report-key ~/dutchman-test/report-key.json
```

- `~/CAD` is your library; give several files or folders if you like. Subfolders are searched;
  files Dutchman does not read are skipped and counted.
- A thousand files take from minutes to a few hours, depending on their size and whether
  SolidCraft checks them. `--jobs N` sets how many convert at once (default: your cores, at
  most 8); use fewer if the machine is busy.
- A file that takes over 15 minutes, or that runs the computer low on memory, is stopped and
  counted as failed; set your own limits with `--timeout SECONDS` and `--max-memory MB`.
- **Stopped part way?** Run the same command again. The journal remembers every file already
  done, and only the rest are converted.
- Inventor assemblies find their part files as Inventor does: at the paths they store, else
  in the project the nearest Inventor project file (`.ipj`) above them describes, with its
  workspace, library and imported-components folders. A shared library kept elsewhere is
  searched too with `--search DIR` (give it more than once for several). See
  [Finding the part files](inventor.md#finding-the-part-files).
- As it runs, Dutchman prints each file's result, with what it lost grouped by component for
  assemblies; an assembly with parts left out is counted as converted but **incomplete**.
  Every loss of every file, in full, goes to `dutchman.log` in the `--out-dir` folder.
- When it ends, Dutchman prints the report's salt. Keep it to yourself (it is also in the key
  file). To make the next run's ids match this one's, add `--report-salt` with that salt.

## Check a folder with doctor

`convert-all`, above, gives you one anonymous report for a whole library. `dutchman doctor`, or
**Check a folder** in the app, goes further: it also diagnoses each failing design and groups
repeated findings, so you see each distinct problem once.

```sh
dutchman doctor ~/CAD --out ~/dutchman-test/checked --jobs 4 --memory-mb 16000
```

`--memory-mb` is the total for all the designs converting at once. Run the same command again
to resume. Keep the output folder outside your library. Only the top-level `report.json` in it is
anonymous; the converted designs, `local-key.json` and `private-checkpoints/` stay private.
Findings are advisory, and [Doctor calibration](doctor-calibration.md) gives their limits.

## 3. Read the report

`report-anon.json` is plain JSON; any text editor shows it. The parts that matter most:

- `totals` and `formats`: how many files converted, how (editable history, checked or not, or
  geometry only), how many converted incomplete (assemblies with parts left out), how many
  failed.
- `stages`: where the trouble was. `decode` and `project` are cadmpeg reading the file, `map`
  and `write` are Dutchman, `solidcraft` is SolidCraft checking the result, `input` is the file
  itself (missing parts beside an assembly, say).
- `errors`: every distinct reason a file failed, most files first.
- `losses`, `blocked_features` and `verification`: what converted files lost, which features
  kept a history from being written, and why histories were not checked.
- `timing` and `slowest`: how long files took.

Each cause lists up to 20 file ids (`f3a9c0e1d2b4f5a6`). Look one up in `report-key.json` to
see which of your files it is. [The report's fields](report-anon.md) are all described.

## 4. What is safe to share

| File | Share? |
|---|---|
| `report-anon.json` | **Yes**, after reading it. It holds no names, paths or geometry. If you find a name in it, leave that part out and tell us: it is a bug. |
| Parts of it (one cause, a few ids) | Yes. |
| `report-key.json`, the salt | **No.** They turn ids back into your file paths. |
| The journal, `dutchman.log`, `--json` output, the printed results | No. They hold your file paths and names. |
| The converted designs, your files | No, unless you choose to (below). |

## 5. Report what you found

**One failed or wrongly converted file, from the app.** Failed cards have
**Review and send report**. On a converted card, first expand
**Something wrong? Report this file** for the same review and private-send steps.
The review shows the exact anonymous metadata. There is no free-text description in
this envelope yet; describe the problem in the private DM or GitHub form instead. You see what the
report holds and choose before anything goes. No GitHub or Discord account is needed, and the app
says it is sent only after our report service confirms it. A summary holds no names and no
geometry; a cut-down piece of the failing design, when Dutchman can prove one, holds geometry and
is a separate choice. Reports are limited to 25 MB each and about 5 an hour. If one does not go
through, it stays on your computer. When a later update fixes it, the app tells you. See
[Reporting a problem](desktop.md#reporting-a-problem).

**A whole library, or if you prefer GitHub.** File every report on **Dutchman's issues**,
<https://github.com/dutchman-cad/dutchman/issues>, with the **Library test report** form
(for a single file, the
[**A file didn't convert**](https://github.com/dutchman-cad/dutchman/issues/new?template=file-did-not-convert.yml)
form).
Even when the report says `upstream: cadmpeg` or `upstream: solidcraft`, file it on Dutchman,
not there: we work out where the fix belongs and take it upstream, then tell you in
the issue when a Dutchman release fixes it. To help with a fix yourself, follow the
[optional contribution tiers](#want-to-go-further) after reporting here.

- **One issue per distinct cause**: one entry of `errors`, `losses` or `blocked_features`.
  Ten files failing for one reason are one issue.
- In each: the format, source program and version, the cause as the report gives it
  (message or code, stage, upstream), the counts, a few file ids, and the report's
  `versions` block copied verbatim. Include `dutchman --version` and
  `solidcraft-cli --version` output too, or "none" if no SolidCraft was installed.
  Use the installed SolidCraft tool's path or `SOLIDCRAFT_CLI` if it is not on PATH.
  A report's `solidcraft: null` means none checked that run; keep it unchanged.
  Read the source version from metadata already reported by Dutchman or the source
  program's About screen. If the exact build is unknown, give the program, desktop or
  web, and the years the files are from, for example "Fusion desktop, unknown builds,
  files from 2020 to 2025". Agents ask the user once when this information is missing.
- Attach the whole `report-anon.json` to one issue (or the first) if you are happy to.
- **Never attach the original design or a private key to an issue.** You may attach a
  small rebuilt reproduction only if it reproduces and you are free to share it. Use the
  [minimum-reproduction procedure](minimum-reproduction.md) to compare its failure stage
  and finding codes. Keep a body copied from the original local while investigating.

Want to go further? See [the four tiers](#want-to-go-further) and say how far to go.

## Want to go further?

After the report come four optional tiers, from chasing the problem down to the full pull
requests:

1. **Chase it down:** a minimal synthetic reproduction of the failure.
2. **Find the cause:** the decode step that fails, found in cadmpeg with that synthetic file.
3. **Fix it:** a cadmpeg pull request.
4. **Finish it:** a Dutchman pull request, when the fix needs Dutchman too.

**Your own design files never leave your machine.** Only synthetic files, anonymous reports,
and code are shared, at every tier.

An agent shows the four tiers together, once, and asks one question: "How far do you want me
to go? (stop here / Tier 1 / Tier 2 / Tier 3 / all the way)". It then does every tier up to
your answer without asking again. It stops early only when a tier fails, or needs something
only you can give: rebuilding the small design by hand in your CAD program (Tier 1), or your
GitHub login to open a pull request (Tiers 3 and 4). Pull requests go out under your own
account.

### Report

Follow [Report what you found](#5-report-what-you-found). For one file or a folder:

```sh
dutchman --version
solidcraft-cli --version
dutchman convert-all --out-dir converted --report-anon report-anon.json part.f3d
```

Use fresh output paths. Run the SolidCraft command only if installed, using its full path
or `SOLIDCRAFT_CLI` when needed; otherwise record "none". Share the reviewed anonymous
report and one Dutchman issue per cause, with the three required versions described above.
Your own files, keys, salts, logs, and journals stay on your machine.

### Tier 1: Chase it down (a minimal synthetic reproduction)

Follow [Make a minimum reproduction](minimum-reproduction.md):

```sh
dutchman repro part.f3d --out original-reproduction
dutchman repro small.f3d --out small-reproduction
```

Replace the filenames with your format and use a fresh output folder for every trial.
Follow `original-reproduction/kit/next-steps.md` to rebuild a small design **by hand in the
source program**, in a new file with generic shapes and simple public dimensions. Do not
copy geometry, names, or dimensions from your private design. Compare the two kits'
`anonymous.json`: the failure stage and complete set of finding codes must match; counts
may shrink. A broad category with no structured codes proves only a candidate match.
If it does not reproduce, say so and keep working locally or stop.

Export a STEP of the synthetic design from the source program. After review, share only
the matching synthetic native file, its STEP export, the anonymous report, and a public
licence statement such as "I dedicate this synthetic design and its STEP export to the
public domain under CC0-1.0." Add them to the Dutchman issue. Your own files and
`local-key.json` never leave your machine; a body copied out for local investigation is
still private.

### Tier 2: Find the cause (diagnose in cadmpeg locally)

Use only the synthetic file from Tier 1. Read cadmpeg's
[CLI guide](https://github.com/cadmpeg/cadmpeg/blob/main/crates/cadmpeg/README.md),
[CONTRIBUTING.md](https://github.com/cadmpeg/cadmpeg/blob/main/CONTRIBUTING.md), and
[LEGAL.md](https://github.com/cadmpeg/cadmpeg/blob/main/LEGAL.md). With Rust installed:

```sh
git clone https://github.com/cadmpeg/cadmpeg.git
cd cadmpeg
cargo build -p cadmpeg --bin cadmpeg
target/debug/cadmpeg --version
target/debug/cadmpeg inspect /path/to/synthetic/small.f3d --json
target/debug/cadmpeg check /path/to/synthetic/small.f3d --json
target/debug/cadmpeg dump /path/to/synthetic/small.f3d -o small.cadir.json
```

On Windows use `target/debug/cadmpeg.exe`. These commands are defined in cadmpeg's CLI
source and guide: `inspect` reads the container; `check` decodes and checks the model;
`dump` decodes to CADIR without checking. The decode command is named `dump`.
Read the error, check findings, and decode losses, then trace the failing operation in
`crates/cadmpeg/src/loader.rs` and the relevant `crates/cadmpeg-codec-*/src/` decoder.
If the synthetic file passes in cadmpeg, record that result: the failure may be in Dutchman's
mapping or SolidCraft rather than native decoding. Do not invent a decode failure.

Share the exact cadmpeg version or commit (`git rev-parse HEAD`), commands, and findings
from the synthetic file in the Dutchman issue. Review outputs for local paths before
posting. Your own files never leave your machine; share only synthetic files, anonymous
reports, and code, not dumps or logs made from private files.

### Tier 3: Fix it (a cadmpeg pull request)

Work on a branch in your fork, using only synthetic tests and
public information allowed by cadmpeg's contribution and legal rules. Use its corpus
donation pipeline for any CAD fixture, as its CONTRIBUTING.md requires, and record how it
was made. Never use vendor SDKs, decompiled binaries, confidential material, or private
models. From the cadmpeg repository root, its documented local CI gate is:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo build --workspace
cargo test-fast
cargo test --workspace --doc
```

For IR or native record type changes also run:

```sh
cargo test -p cadmpeg-ir --features schema --lib
```

Cadmpeg requires DCO sign-off on every contribution. Its CONTRIBUTING.md allows
`git commit -s -m "Describe the fix"`, or a `Signed-off-by: Your Name <your@email>` line
in the pull request description for a squash merge. Use your own identity and certify
only contributions you have the right to submit; an agent must not sign for you.
Decoder or format-specification pull requests also need this exact, truthful sentence:

> I derived this contribution solely by analyzing CAD files I am legally entitled to possess and from publicly available information. I did not use vendor SDKs, decompiled binaries, confidential or NDA-covered material, or any source listed as forbidden in LEGAL.md.

Open a cadmpeg pull request with the code, synthetic regression test, measured result,
check results, sign-off, and declaration where required. Link it from the Dutchman issue.
Your own files never leave your machine; share only synthetic files, anonymous reports,
and code, with no private geometry, dimensions, names, or paths.

### Tier 4: Finish it (a Dutchman pull request, when needed)

A mapping or message fix belongs in Dutchman; native decoding belongs
in cadmpeg. Follow [CONTRIBUTING.md](../CONTRIBUTING.md) and [AGENTS.md](../AGENTS.md).
Branch from `main` in your fork and, from the Dutchman repository root, run the documented
checks CI runs:

```sh
scripts/solidcraft-window.sh --print
python3 scripts/check-version-gates.py --base HEAD
python3 -m unittest discover -s scripts/tests -v
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Add a `changelog.d/<short-name>.md` fragment and update affected docs and skills in the
same pull request. Validate the fragment with `python3 scripts/changelog.py check` and
review it with `python3 scripts/changelog.py preview`. Keep the default output unchanged
unless the change is opt-in, and document the origin of synthetic tests. If it needs an
upstream fix, keep the pull request as a draft until cadmpeg merges it and Dutchman's pin
includes it; see [building against local cadmpeg](../CONTRIBUTING.md#build-dutchman-against-a-local-cadmpeg)
for local diagnosis.

Share the code, tests, changelog fragment, docs, check results, and links to the Dutchman
issue and any upstream fix. Your own files never leave your machine; only synthetic files,
anonymous reports, and code are shared. Finish when you choose; no step commits you to
another one.

## Let your own AI check a folder

Connect `dutchman-mcp` to your assistant ([client setup](mcp.md#set-up-a-client)). Its diagnostic
tools return anonymous results, and `report_submit` needs an explicit yes, so paste this prompt:

```text
Check my CAD folder <LIBRARY FOLDER> with Dutchman, writing to <RESULTS FOLDER> outside it. Use the
doctor tool, with limits that leave this computer usable. If it stops, run it again; it resumes.
Run dutchman --version and solidcraft-cli --version if installed (use its full path or
SOLIDCRAFT_CLI if needed); record "none" only when no SolidCraft was installed. Copy any
report's versions block verbatim. Doctor's report.json has no versions block, so include
these command outputs separately. State the source program and version from metadata already
reported by Dutchman; ask me once if missing. If exact builds are unknown, record the program,
desktop or web, and the years the files are from.
Do not read my design files, the converted designs, local-key.json or private-checkpoints/.
Read only the anonymous report.json. List each distinct problem, most files first, and narrow the
biggest ones with diagnose_narrow. For a report I might send, call report_preview and show me
the choices, the part roles and the sizes. Offer only an anonymous summary from my own files,
never a bundle containing their geometry. Call report_submit only after I say yes to that report,
that summary choice and the public-test box. Never send anything else, anywhere. For a GitHub issue,
draft it for me to review and post, with those versions and anonymous facts only.
End by showing the four tiers from this guide's "Want to go further?" and asking once:
"How far do you want me to go? (stop here / Tier 1 / Tier 2 / Tier 3 / all the way)". Then do every tier up to my answer without asking again, and stop early only if a tier fails or needs something from me (such as my GitHub login or a small design rebuilt by hand).
```

## Let your own Claude do the legwork

If you use Claude (Claude Code, or Claude with the `dutchman-mcp` server and the
`dutchman-converting` skill from this repository), paste this prompt. It runs the batch,
groups what went wrong and drafts the issues **for you to review**; it does not post anything.

```text
I am testing Dutchman (https://github.com/dutchman-cad/dutchman) on my CAD library. My files
must never leave this machine: do not upload, paste or attach any design file or its contents
anywhere, and do not post anything to GitHub or elsewhere yourself.

Run dutchman --version and solidcraft-cli --version if installed, using its full path or
SOLIDCRAFT_CLI when needed; record "none" if no SolidCraft was installed. State the source
program and version from metadata already reported by Dutchman; ask me once if missing.
If exact builds are unknown, give the program, desktop or web, and years the files are from.

1. Run Dutchman's batch over my library at <LIBRARY FOLDER>, writing to <RESULTS FOLDER>
   (outside the library), with a journal, an anonymous report and a key:
   dutchman convert-all <LIBRARY FOLDER> --out-dir <RESULTS FOLDER>/designs
     --journal <RESULTS FOLDER>/journal.jsonl
     --report-anon <RESULTS FOLDER>/report-anon.json
     --report-key <RESULTS FOLDER>/report-key.json
   (or the dutchman-mcp convert_all tool with the same options). If it stops, run it again;
   the journal carries on. Do not show me the salt or the key's contents in your summary.
2. Read report-anon.json only (not the key, not the journal, not dutchman.log). Summarise totals, formats,
   stages and the slowest files in a few lines.
3. Group the causes in errors, losses (Degraded and Dropped only) and blocked_features into
   distinct issues, most files first. Merge entries that are clearly one cause.
4. For each, draft a GitHub issue for dutchman-cad/dutchman in the "Library test report"
   template's shape: title, format, stage, upstream, the cause as the report gives it, counts,
   up to five file ids, and the report's versions block copied verbatim. Add the command
   outputs and source program/version collected above; keep nulls in the block unchanged.
   Use only report-anon.json for the other technical facts. Check
   each draft for anything that looks like a file name, path, part or company name, and
   remove it.
5. Save the drafts as Markdown files in <RESULTS FOLDER>/issues/ and list them for me. I will
   read them and post the ones I choose. End by showing the four tiers from this guide's
   "Want to go further?" and asking once: "How far do you want me to go? (stop here / Tier 1 / Tier 2 / Tier 3 / all the way)";
   then do every tier up to my answer without asking again, and stop early only if a tier fails or needs something from me (such as my GitHub login or a small design rebuilt by hand).
```

## Let your own AI make a minimum reproduction

For one failing file you cannot share, use the
[minimum-reproduction guide](minimum-reproduction.md). This prompt works with a local
command-line assistant, any MCP client using `dutchman-mcp`, or a chat assistant coaching
you while you run the command. A remote assistant needs only anonymous.json; look up
private names yourself and do not paste them into chat.

```text
Help me make a minimum reproduction of a Dutchman conversion failure. My original CAD
file and its contents must stay local. Do not upload, paste, attach or post any file,
geometry, name, path, entity id or parameter value from it. Do not post an issue for me.

Run dutchman --version and solidcraft-cli --version if installed, using its full path or
SOLIDCRAFT_CLI if needed; record "none" if no SolidCraft was installed. State the source
program and version from metadata already reported by Dutchman; ask me once if missing.
If exact builds are unknown, give the program, desktop or web, and years the files are from.

1. If you can run local tools, run:
   dutchman repro <ORIGINAL FILE> --out <FRESH RESULTS FOLDER>
   or use the MCP repro tool with input and out. Otherwise coach me to run it locally.
   Read kit/anonymous.json and kit/next-steps.md. If it says no_failure_to_reproduce,
   explain that this run succeeded and stop.
2. Keep kit/local-key.json local. Read it only if you are running locally and keeping
   its contents local; otherwise ask me to look up the anonymous ordinal myself without
   pasting names or key contents. Do not enable MCP include_local_key for a remote chat.
   Never read or upload the original design, ordinary logs or journals.
3. Explain the failure stage, complete finding-code set and narrowing limits. Coach me,
   one step at a time, to build a small design in my CAD program using next-steps.md.
   Save and run repro on each candidate into a fresh folder. Keep a candidate only when
   the failure stage and complete code set match. Counts may decrease from 88 to 1.
   A broad category without structured codes is only a candidate, not a proven match.
4. Simplify until the remaining design reproduces with generic shapes and simple public
   dimensions. A body copied from the original always stays local; rebuild the small
   shape from scratch before proposing it for sharing.
5. Draft an issue for dutchman-cad/dutchman for me to review and post. Use anonymous.json
   facts only: copy its versions block verbatim, plus format, stage, category, finding
   codes/counts, kinds and anonymous ordinals. Add the command outputs and source program/version
   collected above. If no rebuilt design reproduces, say so. Exclude private names, paths, ids,
   geometry and parameter values, including the MCP response envelope's local paths.
6. Tell me to attach anonymous.json and the small rebuilt file only if it reproduces
   and I am free to share it, with its STEP export and a public licence such as CC0.
   Never the original file. Never local-key.json. Upload nothing.
7. End by showing the remaining tiers from this guide's "Want to go further?" and asking once:
   "How far do you want me to go? (stop here / Tier 2 / Tier 3 / all the way)"; then do every tier up to my answer without asking again, and stop early only if a tier fails or needs something from me (such as my GitHub login or a small design rebuilt by hand).
```

## Questions

Ask on Dutchman's issues; anything you would rather not ask in public, say so there and we
will find a private way.
