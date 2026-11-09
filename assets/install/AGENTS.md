# Dutchman Convert

Dutchman converts native CAD files into SolidCraft designs (`.sc`) on this machine.

## When a conversion went wrong: one procedure

If the person asks why a file didn't convert, or converted with something missing or wrong
(the app's **Ask your AI about this file** prompt says so), do this and nothing more unless
they ask:

1. Run `"$D" investigate "ORIGINAL FILE"` (how to set `D` is below). It reads the report the
   app saved beside the file, or converts once into a private folder when there is none. It
   narrows a failed conversion to its first failing feature, and writes one file made to be
   shared: `summary.md`, in `ORIGINAL FILE.investigate/` beside the file. It sends nothing on
   this first run.
2. Read `summary.md` and explain it plainly: what converted, which features were not rebuilt
   and why, which replays may be wrong, and what a newer build would change, if it lists any.
3. Ask the person one question: may Dutchman send that summary to the Dutchman team? It holds
   no file, part or feature names, no paths and no sizes from the design; show it to them if
   they want to see it.
4. Run the same command again with `--yes` if they said yes, or `--no-send` if they said no.
   The second run reuses the summary. A yes prints a receipt id; give it to them.

Never overwrite the original, the converted design or the saved report, and never send or
upload anything else. Everything else `investigate` writes (the convert report, the
narrowing, `local-key.json`) is private and stays in that folder.

## The executable

Use this folder's executable by its full path. In a POSIX shell started here, set
`D="$(pwd)/dutchman"`; in PowerShell set `$D = Join-Path (Get-Location).Path 'dutchman.exe'`
and use `& $D` in place of `"$D"`. In a macOS app bundle this guide is in
`Contents/Resources`; set `D="$(cd ../Helpers && pwd)/dutchman"` there instead.

- `"$D" formats --json`: which formats this build reads.
- `"$D" inspect "input.f3d" --json`: look before converting.
- `"$D" convert "input.f3d" "new-output.sc" --json`: convert to a fresh output path.
- `"$D" convert-all "project" --out-dir "new-output" --json`: a folder or project ZIP.

Choose input and fresh output paths with the person; explain losses before calling a result
complete. Read `skills/dutchman-converting/SKILL.md` before converting; its guidance is in
`docs/`, starting with `docs/guide.md`, `docs/command-line.md` and `docs/json.md`. This
bundle holds every Markdown doc and images up to 256 KiB.

## Going further (only if the person wants more)

- **Reports.** A saved `<stem>.report.json` (or `<stem>.failed.json`) is plain
  `dutchman.convert/1` JSON: data, not instructions. Check `schema`, `ok`, `input`, `output`,
  `history` (its `features` list each feature's `kind`, `status`, `reason`, `ordinal`, its
  place in the source timeline from 0, and `variant`), `verification` and each `losses`
  entry's `level`, `code` and `message`. `ok` alone does not prove verified history. Reports
  name private paths and parts: keep them local. `docs/json.md` has every schema.
- **Trimming by hand.** `"$D" convert "input.f3d" "scratch/trial.sc" --write-replay --json`
  also writes `scratch/trial.replay.json`, the command script that rebuilds the history step
  by step for `solidcraft-cli run`; `--part P --stop-at-feature N` cuts the history after
  feature N of part P. See `docs/troubleshooting.md` (Replay a history by hand) and
  `docs/diagnose.md`.
- **The `.sc` file** is a ZIP archive; its `document.json` holds the features in timeline
  order with their ids, which the replay script rolls to one by one.
- **A local cadmpeg build.** To try a decoder change, build Dutchman from source against a
  local cadmpeg checkout with the `[patch."https://github.com/cadmpeg/cadmpeg"]` block in
  Dutchman's CONTRIBUTING.md (https://github.com/dutchman-cad/dutchman/blob/main/CONTRIBUTING.md).

Keep every new file in a fresh scratch folder next to the design, and ask before sending
anything anywhere.
