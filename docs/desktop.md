# The desktop app

## Ask your AI about this file

Each completed conversion saves its full `dutchman.convert/1` JSON report beside the
design as `<stem>.report.json`, the same schema as CLI `convert --json`. Failed files
save `<stem>.failed.json` in the output folder instead. Reports include local paths,
file and feature names, and detailed messages: they stay local and are not anonymous.
If saving fails, the app shows a notice; the conversion result is still available.

A converted card with losses, and a failed card, offers **Ask your AI about this file**.
Clicking it always copies a short prompt (a paragraph) and opens it in a read-only text box
under the button (about five lines tall, scrolling inside), with **Copy** to copy it again and **Close**. **Copied. Paste it into your AI
assistant.** shows for about two seconds after each copy. The box stays until you close it or
the card is replaced by a new conversion. The prompt names the original file, the converted
design (not for a failed file), the saved report (`<stem>.report.json`, or `<stem>.failed.json`)
and the log of that run. It tells the assistant to start by running
[`dutchman investigate`](command-line.md#look-into-a-conversion-that-went-wrong-investigate) on
the original file, as the `AGENTS.md` beside the app explains (on GitHub for a source build),
and to explain the `summary.md` it writes. That guide asks you one question, whether to send
that summary, and explains how to go further in a fresh scratch folder, leaving the original,
design and report untouched. Nothing is sent and no server
connection or extra setup is needed. Use an assistant that can read local files and run
local commands. The prompt says to ask before sending anything anywhere; pasting it into an
assistant is your choice, so check that assistant's privacy settings before sharing private
paths or reports.

Each file converts in its own process, so running low on memory or a crash ends that file and not the app, and closing the app ends what is still converting. A file stopped because it ran out of memory alongside others converts again on its own at the end of the run, and its card says so. On Windows these processes open no terminal windows.

While a file converts, its card shows the design-read count and a separate running-step
line beneath it: **Building history for Bracket: 12 s** or **Checking Bracket: 12 s**. The time belongs
to that step and updates every five seconds, even while the count stays at, for example,
261 of 264. Parallel conversion shows the current step that has run longest. Reading,
checking the decoded model, and saving also have their own stage lines.

During a folder run, the bottom bar keeps the longest-running file visible below the
completed-file count, even when its card is offscreen. It shows the filename, stage and
current step on two lines, for example **Assembly.iam: building history (2 other files running)** above
**Still checking Bracket (rebuilt history): 1 min 1 s**. The other-running count excludes queued and finished files;
selection follows total file time, even when a part's step clock resets.

Times use one format throughout: **45 s**, **1 min 1 s**, **2 min**, **1 h 5 min**, or **1 h**.
Past the usual duration the line says **Still checking Bracket: 1 min 1 s**.
A target check that reaches its enforced limit says what happens next: the check stops,
and history stays unverified, imported geometry stays visible, or document conversion
stops, according to the output and check being run. Usual durations are notices rather
than deadlines. See [progress during conversion](command-line.md#progress-during-conversion)
for each threshold and limit. Part and design names in this area are local text and do
not enter the anonymous diagnostic log.

Every desktop run keeps a local diagnostic log in the diagnostics folder (**Open diagnostics
folder** opens it), never beside your designs. It holds no names or contents from your file
and stays on your computer. The folder keeps the newest 50 logs.

If a run was cut off, the next launch says **The last run didn't finish.**, with a button to its
log folder. **Dismiss** keeps that run's notice away after a relaunch; another unfinished run
shows its own.

## Project inputs

Files whose names start with `~$` are lock files. They are ignored in project planning
and excluded from job, failure and skipped-file counts.

ZIP directory data and ZIP64 extensible records are limited to 16 MiB each before
the ZIP directory is allocated; entry budgets count every declared entry.

Drop a CAD file, a folder, or a project `.zip`. The card shows assembly, part and STEP
counts before conversion. A new folder beside the input, named `Project (SolidCraft)`,
is selected automatically; **Choose…** can select another output folder. Existing folders
receive a numbered alternative.

ZIP projects unpack into a private temporary folder, limited to 10,000 entries and 4 GiB
of extracted data. Paths outside the project are refused and symbolic links are skipped.
Temporary files are removed when the conversion and its retained retry rows are released.
Inventor and SolidWorks assembly references select the top-level roots, which convert
with their parts resolved inside the project; unused parts convert afterwards. A same-name STEP export beside
a native file is kept as a fallback if the native conversion fails.

The Inventor project folder setting appears only for inputs containing Inventor files.
Available memory is labelled as memory and hidden when unknown. Automatic SolidCraft
selection compares the versions found on PATH, beside Dutchman and in installed applications, choosing
the newest; `SOLIDCRAFT_CLI` remains an explicit override.

### How Dutchman finds SolidCraft

Dutchman checks histories with `solidcraft-cli`, the command-line tool that comes with
SolidCraft. The desktop app, the command line and the server all search for it the same way,
in this order:

1. `SOLIDCRAFT_CLI`, an environment variable you can set to the full path of a
   `solidcraft-cli` (`solidcraft-cli.exe` on Windows). When it names an existing file, that
   file is used and nothing else is searched.
2. `PATH`, then the folder `dutchman` runs from. If several are found, the newest release is
   used.
3. The folders SolidCraft installs into: on Windows `Program Files`, `Program Files (x86)`
   and your per-user programs folder (`%LOCALAPPDATA%`), then the install folder the
   installer recorded in the registry; on macOS `SolidCraft*.app` in `/Applications` and
   `~/Applications`.

A file you chose with **Choose solidcraft-cli…** in the SolidCraft version window is used
instead, until you press **Find automatically**; one that no longer answers as SolidCraft is
ignored. When nothing is found, that window lists the places that were searched. Opening it
searches again, so a SolidCraft installed (or slow to start) after Dutchman opened is picked
up without restarting. To set the variable on Windows, search for "environment variables" in
the Start menu, add `SOLIDCRAFT_CLI` as a user variable, and restart Dutchman.

Dutchman Convert (`dutchman-gui`) converts without a terminal. [How to use it](../README.md#how-to-use-it) shows it step by step.

## Converting

Open `dutchman-gui`, add design files, folders or project ZIPs, and press
**Convert**. Convert stays visible while you scroll through settings or results. The
**Preset** selector starts at **Standard**:

- **Fast**: shape only, least memory.
- **Standard**: history, sketches, and joints; leaves history out if it would
  not fit in free memory.
- **Full**: everything, most memory.

Under **Advanced**, **Feature history**, **Sketches**, and **Joints** let you leave content
out of the selected preset. A changed checkbox makes the selector read **Custom (based on
Standard)** or **Custom (based on Full)**. Choosing a preset again resets these switches.
Fast already leaves all three out; choose Standard or Full to enable them. These choices
are remembered between launches. Older `import_only` settings restore with **Feature
history** unchecked and Standard as the base.

Queued files show **about N GB, about M min** when an estimate is available. Files in queued
folders are listed with their estimates too. Estimates below 0.1 GB show **under 0.1 GB**,
and times below 60 seconds show **under a minute**, just as in the CLI. **M GB of memory free** appears
once when the machine's free memory can be read. On a Mac this counts memory the system can give back
(not in use by apps, system or compression), not only the few pages it keeps empty. These are estimates, not limits or promises; an unavailable estimate
leaves no label. Planning and estimation run in the background.

Pressing Convert starts the conversion at once; there is no memory dialog. The estimate in the
file row and box comes from the file's size and can be far off either way, so it never
refuses or delays a conversion. Dutchman measures memory while it converts and stops with a
clear message if the computer really runs out. When the estimate was above free memory, the
result card's **memory** Info note says so. Preset descriptions appear on hover or keyboard
focus in the preset selector. The **Convert and check history even if memory is low** control
in Advanced stays.

Archive sizes are read in the background once; changing choices updates the figures without
reading the archive again.

When Standard leaves history out to fit in memory, its result card shows the
history Info message even with the full loss log collapsed. The original geometry is kept.

When a Fusion Base Feature holds its body as the design ends and later joins or cuts re-apply to it,
the card counts those features apart ("43 of 235 features rebuilt, 76 kept and re-applied to the
final body") and says "Base Feature 1 holds the body as the design ends. The features after it are
kept but re-apply to that body, so editing them may not change the shape."

A result card ends with how many rebuilt extrudes may use the wrong sketch region, such as "2
rebuilt extrudes may use the wrong sketch region; it couldn't be checked against the source."
They replay, but nothing proves they swept the right region; the JSON report marks each with
`profile_unverified`.

When a partial history replays but its bodies differ from what the source stores where the
history stops, the card says so and what to do, such as "The rebuilt features don't match the
original where the history stops (0 bodies where it has 1). The final shape is correct; check the
rebuilt features before editing them." In an assembly it gives how many parts. The JSON report has
the loss `history/body-mismatch`.

Expanded **Losses** lists wrap long feature ids inside their result card. Ordinary text
wraps at spaces.

A converted design opens in SolidCraft from its card: the app that `SOLIDCRAFT_APP` names, else
`solidcraft` on `PATH` or beside Dutchman, else the system's app for `.sc` files. When an Inventor assembly's part
files are not found, its card names them: **Search a folder…** or **Find a file…** converts it
again, searching there too. The app follows the system's light or dark setting, or can be set to
Day or Night, and its **Output format** choice also offers a FreeCAD macro.

## SolidCraft version

The header’s SolidCraft status pill opens a small panel with **Target SolidCraft** and a
notice explaining when the installed SolidCraft is newer than Dutchman knows, too old
(before minor 0.18), or older and the selected target loses editable history. Only the last case suggests newer SolidCraft for more history;
0.25 patch 1 through 0.41 currently need no such notice. The notice never blocks conversion.
The pill shows the detected release, or **SolidCraft not found** with a download link in its
panel. **Advanced > SolidCraft version…** opens the same panel. Narrow windows show only
the version or **not found**, shorten **Update**, and put theme choices in a menu.
When space is limited, header controls sit on a second row below the title and version,
so Settings, the SolidCraft status, themes, problem reports, and updates stay reachable.
For an installed release newer than known, the panel has an **Update Dutchman** button, which
opens the update window, when the update check already knows a newer Dutchman. Too-old releases have a
[Download SolidCraft](https://github.com/tangency-labs/solidcraft/releases) link.

**Choose solidcraft-cli…** in the panel picks the command-line tool that checks histories and
remembers it; **Find automatically** goes back to searching. Choosing the SolidCraft app uses the
`solidcraft-cli` beside it. Without one, the panel says so: on Windows, the single-file SolidCraft
download leaves it out, and the portable ZIP and the MSI installer include it.

Choose the target with the **Target SolidCraft** selector: it defaults to the found CLI's version, else the built-in newest (currently 0.41), even when history checking is off. **Automatic** follows the detected installation; **0.25 and newer** keeps supported FreeCAD helices as editable coils.

History checking stays on by default. Before a SolidCraft rebuild, Dutchman compares the
memory needed for a second build with free memory, using what the design holds after
reading. If it will not fit, conversion still succeeds: the original shape stays visible,
retained editable history stays hidden and unverified, and the card and report say
**History check skipped: needs about N GB, M GB free**. Other designs are checked normally.

Turning **Check feature history in SolidCraft** off stays off. Advanced also offers
**Convert and check history even if memory is low**, the equivalent of CLI
`--no-memory-check`. This forces the requested work and can run out of memory; it does
not turn checking back on when you switched it off. The override lasts for the current
app run. Automatic part preparation uses at most two workers, with fewer when a detailed
part's temporary peak or the retained preparation queue would consume free memory.
Small imports avoid extra preparation workers.

The window opens where and at the size it was last closed. A saved size smaller than the
app's minimum, 520 by 420 points, is ignored, and the window opens at its default size instead.

With no saved size, the window opens centred at 1180 by 800 points, or 90% of the screen where
that is smaller, which fits the whole main screen with the **Advanced** choices open and nothing
scrolled. From 1100 points wide, **Designs** and **Output folder** stand in a left column and
**Results** in a right column. Results that do not fit scroll inside their own column, and a
long list of designs scrolls inside **Designs**; the rest of the screen stays in place. Below
1100 points the main screen is one column, **Designs**, **Output folder**, then **Results**,
and scrolls as one page (down to the 520 point minimum).

## Reporting a problem

The window's title and top row show its version beside **Dutchman Convert**, for example
**v0.2.6**; a nightly shows its nightly version. **Report a problem**, at the top right beside the
**System / Day / Night** theme switch, opens the
[A file didn't convert](https://github.com/dutchman-cad/dutchman/issues/new?template=file-did-not-convert.yml)
form. `dutchman-gui --version` and `dutchman-gui --help` print and exit without opening a
window (on Windows, read the version with `dutchman --version`).

Each result card shows its run's log path as a small "Run log:" line (hover it for the whole path),
then **Show this run's log** and **Copy log to…**, available when
the run ends. Show selects that file on Windows and macOS, or opens its folder on Linux.
Copy lets you save it elsewhere,
using the same filename by default. Every card in a batch uses the same log; retries keep
the other cards linked to their original run.

Log names include UTC date and time, Dutchman version and result, for example
`2026-11-02_163710_dutchman-0.2.6_failed.jsonl`. A live or crashed run keeps `running`;
completed runs use `converted`, `failed` or `stopped`. **Open diagnostics folder** opens the
folder containing the newest 50 logs, including older time-only names.

**Review and send report** names this run's log automatically and lets you show or copy it.
The log contains no names or contents from the design. Send uploads only the summary or
bundle described in that review; the log stays local. The private-send card also shows
this log, and the copied Discord DM names it so you can attach it without guessing which
file belongs to the run. An older log's version is shown as an advisory notice, such as
"This log is from Dutchman 0.2.5; this is 0.2.6." It never blocks conversion.

When a file converts with any loss (history not kept or only partly kept, features flattened
or blocked, parts unplaced, colours dropped), its card has a **Report this file…** button.
It opens **Something wrong? Report this file** and brings it into view. A clean result has no
button, but the same section is there, closed, if it looks wrong anyway. Choose
**Review and send report** to review the exact metadata before sending an anonymous
summary, or follow the same private-send steps as a failed card to send the original.
**Diagnose this file** stays on failed cards: it narrows reproducible conversion failures,
not visual problems in a converted result. **Add matching STEP** can check a converted
file locally against an export.

Descriptions are not supported in the anonymous report envelope yet. Describe what is
wrong in the private DM or GitHub form, leaving out private names and paths. Converted
summaries use the signature `converted-with-problem` and contain no design geometry;
they never reuse an earlier failure's bundle. The review shows the exact metadata,
including sharing choices, and nothing is sent until **Send** is clicked.

On a failed card, these other ways are folded under **Help us fix it**, below **Review and send
report**, so the whole card fits the window; a converted result has them under **Something
wrong? Report this file**. To send a file privately, follow the steps on the card:
join the Discord server, then **Copy and open DM** copies a message and opens Dutchy's
DM. For a converted result, the copied message starts with a note that the result is
wrong, followed by the standard routing card with `Error: converted-with-problem`.
The routing card keeps its fixed wording so automatic upload links work.
A file within Discord's upload limit is dragged in, and its folder opens for that; for a
larger one, Dutchy replies with a private upload link. See
[Send the file privately](troubleshooting.md#send-the-file-privately).

**Report anonymously on GitHub** sends no file and no names. It opens the same form in your
browser with Dutchman's version, your system and the error's stage, upstream and code filled
in, and copies the file's anonymous report, which names no file and no folder. Paste it into
the form's **Anonymous report for this file** box and read it before you submit. Never attach
the design; a link to a file that is already public is welcome. The link stays under 2,000
characters so GitHub opens it while you are signed in. For converted files, the form says
that the result is wrong and asks what differs from the original.

The card takes the invite, the limits, and the message from a small policy built into the app.
Once a day at most, where update checks run, the app reads a newer copy from
<https://dutchman-cad.github.io/report-policy.json> in the background. The request says nothing
about your files. `DUTCHMAN_UPDATE_CHECK=0`, turning off automatic update checks, and source
builds all stop it, and the built-in copy is used.

## Updates

Release builds check for updates at startup and every hour while open, reading GitHub's
releases feed (the GitHub API is asked only if the feed cannot be read). Those background
checks may reuse an earlier answer. A small **Update** button beside the theme switch appears
when a newer release is known, and it never opens a window by itself. Pressing it, or
**Check now** in Settings, always asks GitHub for the latest release at that moment and
ignores earlier answers: the window shows **Checking for updates…**, then the newest version
with its release notes, **Dutchman is up to date**, or why the check failed (no connection,
GitHub limiting requests, or another error) with **Retry**. The notes are shown with their
headings, bold text, `code` and bullet lists; links show as their text. **Download** opens the package for this platform (or the release page).
Homebrew and Scoop installations offer **Update with Homebrew** or **Update with Scoop**.
Homebrew runs `brew update` followed by `brew upgrade --cask dutchman-cad/tap/dutchman-convert`
(or `brew upgrade dutchman-cad/tap/dutchman` for the formula), with
`HOMEBREW_NO_AUTO_UPDATE=1` for the upgrade. The window shows progress and the last 12 output
lines. Closing the update window keeps Homebrew running; reopening it shows the same progress.
After success, **Restart Dutchman** opens the installed app and closes this copy. Homebrew can
replace the running app on macOS.

Scoop needs Dutchman to close before Windows can replace its executable. **Update with Scoop**
first explains the hand-off; **Update and restart** confirms it, and **Cancel** leaves the app
open. A detached, hidden PowerShell helper waits up to two minutes for Dutchman to exit, runs
`scoop update` then `scoop update dutchman` for the `dutchman-cad/dutchman` package, and opens
Dutchman from its Scoop shim whether the update succeeds or fails. Settings save on normal
close. Finish active conversions before confirming. Output goes to
`%LOCALAPPDATA%\Dutchman\diagnostics\scoop-update.log`. On the next start, a failed or
incomplete result opens the update window with its error, output tail, and **Copy command**;
the log is consumed once and deleted. Success opens no update window.

Windows copies have two update paths, using the versioned package and its `.sha256` from
the same stable release. Nothing changes until the SHA-256 matches; a failed download or
checksum check deletes the download and leaves the installed copy untouched. **Download**
remains available for a manual install.

- **MSI: Install (closes Dutchman).** Detection requires the running executable at
  `%LOCALAPPDATA%\Programs\Dutchman\dutchman-gui.exe` and a matching `InstallDir` string in
  `HKCU\Software\Dutchman CAD\Dutchman Convert`, written by the MSI. Paths are compared
  without case, separator, trailing separator, or Windows extended-prefix differences.
  After download and verification, the install window reopens if hidden. Dutchman asks
  before closing if conversions are running:
  **Finish files and install** stops new files from starting, lets active files finish,
  then closes and installs. If the files finish before you answer, **Close and install**
  still asks for your confirmation. **Cancel install** deletes the prepared download and
  keeps Dutchman open; a queue already asked to stop stays stopped. New conversions wait
  while the installer is being prepared. Close an open diagnosis window after it finishes.
  With no running conversions, Dutchman closes normally so settings save. A detached
  PowerShell helper waits for the process to exit, runs `msiexec /i <file> /qb /norestart`
  without administrator rights, and restarts Dutchman after the installer finishes.
  Its verbose log is `%LOCALAPPDATA%\Dutchman\diagnostics\msi-update.log`. The helper
  waits up to two minutes for exit and up to 15 minutes for installation. Failed or
  incomplete installations show the log location on the next start. Uninstall keeps settings and logs.
- **Portable ZIP: Update now.** When neither MSI nor Scoop identifies the copy, all three
  release programs beside the Windows executable identify a portable folder. Dutchman
  downloads and verifies the ZIP, stages all three programs and bundled guidance on the
  install volume, renames the originals to `.old`, and puts the new files beside them.
  If replacement fails, it attempts to restore the originals and explains any recovery
  needed. The running app keeps working; **Restart now** becomes available once conversions
  finish. A detached helper waits for this copy to exit before opening the updated app.
  The next start deletes the `.old` backups. A folder without write access offers the manual
  download fallback. Other Windows copies keep **Download**.

Scoop and Homebrew retain their package-manager paths. macOS and Linux downloads are unchanged.

No package-manager commands run until confirmed. Missing commands, failed commands, and a
15-minute update timeout retain the copy-command fallback. **Copy command** remains a
secondary action for both package managers; close Dutchman before running it with Scoop.
**Skip this version** hides that release; **Remind me later** hides updates for three days.
These choices persist in the desktop settings. **Settings** offers **Check for updates
automatically** (on by default) and **Check now**, which also shows skipped releases.
`DUTCHMAN_UPDATE_CHECK=0` disables all checks, including **Check now**. Builds from source
never check; only release-workflow builds enable this feature.

## Installing it

On macOS it comes as **Dutchman Convert.app** in a disk image: open it and drag it onto Applications in the window that appears (see
[opening it the first time](install.md#opening-the-app-the-first-time): approve it once
and the commands inside it run too). It ships in the
release archives beside `dutchman`, or build it from source with Rust 1.88 or newer:

```sh
cargo install --git https://github.com/dutchman-cad/dutchman dutchman-gui
```

On macOS, let SolidCraft itself run once before the first check: open SolidCraft from
Applications, click **Done** when macOS stops it, then click **Open Anyway** in **System
Settings > Privacy & Security**, the same steps as for Dutchman Convert. Dutchman reads the
app's quarantine attribute and its user-approved flag without changing them. The desktop app
retries a blocked check when its window regains focus or you start a conversion.

On Linux it needs a desktop session (X11 or Wayland with OpenGL); the file dialogs use the
desktop portal, and dropping files works without it.

## What the update check sends

For direct downloads, MSI installations, and portable Windows ZIP copies, each check sends an HTTPS GET to
`https://github.com/dutchman-cad/dutchman/releases.atom` with
`User-Agent: Dutchman-update-check/<version>` and, on repeat checks, `If-None-Match`
containing the feed ETag GitHub returned. An unchanged feed (304) needs no API request.
Only a newer stable `vX.Y.Z` release triggers one GET to
`https://api.github.com/repos/dutchman-cad/dutchman/releases/latest` for package links;
if the feed fails, one API request is tried instead, then the next automatic check is in
an hour.

For both Windows MSI and portable ZIP installations, the automatic and manual checks fetch
exactly the Atom feed and, under the conditions above, the latest-release API. They fetch no
package-manager metadata. Clicking **Install (closes Dutchman)** additionally fetches:

- `https://github.com/dutchman-cad/dutchman/releases/download/vX.Y.Z/dutchman-X.Y.Z-x86_64-pc-windows-msvc.msi.sha256`
- `https://github.com/dutchman-cad/dutchman/releases/download/vX.Y.Z/dutchman-X.Y.Z-x86_64-pc-windows-msvc.msi`

Clicking **Update now** in a portable ZIP copy additionally fetches:

- `https://github.com/dutchman-cad/dutchman/releases/download/vX.Y.Z/dutchman-X.Y.Z-x86_64-pc-windows-msvc.zip.sha256`
- `https://github.com/dutchman-cad/dutchman/releases/download/vX.Y.Z/dutchman-X.Y.Z-x86_64-pc-windows-msvc.zip`

These are the offered stable release's published checksums and packages. All four asset
requests use the same User-Agent, without an ETag. Each download follows at most five redirects,
only over HTTPS to this repository's GitHub release-download path,
`release-assets.githubusercontent.com`, or `objects.githubusercontent.com`. Checksums are
limited to 4 KiB, packages to 512 MiB, and selected ZIP contents to 512 MiB across at most
1,000 archive entries. No other URL is fetched by either Windows update path. Installing,
swapping, cleanup, and **Restart now** make no additional update requests.

For package-manager installations, the hourly check first sends an HTTPS GET to exactly one
of these public package files, using the same User-Agent and a conditional `If-None-Match`
header with that file's ETag:

- Homebrew cask: `https://raw.githubusercontent.com/dutchman-cad/homebrew-tap/main/Casks/dutchman-convert.rb`
- Homebrew formula: `https://raw.githubusercontent.com/dutchman-cad/homebrew-tap/main/Formula/dutchman.rb`
- Scoop: `https://raw.githubusercontent.com/dutchman-cad/scoop-bucket/main/bucket/dutchman.json`

An unchanged package file (304) causes no further request. Only a package version newer than
the version already known triggers a releases-feed request for that exact version's notes;
no releases API request is needed. The pill appears only once the package has the newer
stable version. If its feed entry is missing, the window links to the release page.
Package metadata failures retry in an hour without offering a feed-only update.

These update-check requests send no cookies, identifiers, usage data, file names, paths, or
file contents. Clicking **Update with Homebrew** or confirming **Update with Scoop** runs
that package manager, which contacts its own configured package sources and download hosts
using its own network and privacy settings. GitHub sees the connecting IP address, ordinary HTTPS request metadata,
the app version in the User-Agent, and the public feed or package ETag. Clicking **Download** opens
the release asset or page in the default browser, under that browser's privacy settings.

## Diagnose a failed file

Choose **Diagnose this file** on a failed conversion card. The wizard runs the same local
narrowing as `dutchman diagnose --narrow`, showing numbered steps and feature progress.
Its checkpoint and reports live beside the source, in `<filename>.diagnosis/diagnose`.
After a crash or restart, choose the action for that same file again to resume. An open
wizard also has **Resume diagnosis** to rerun from its checkpoint.

Follow the recipe for your CAD program, then choose **Check my small file** and pick the
new file. The result says whether it fails the same way, converts fine, or differs.
**Make the report** becomes available after a matching or explicitly broad check. It writes
a local ZIP containing the small file, its signature and anonymous.json. **Open diagnosis
folder** lets you review it. The ZIP contains geometry; share it only if you are free to
share that small file. Nothing is sent by the wizard.

## Check a folder and matching STEP

Use **Check a folder** to run local folder diagnosis with concurrent-design and total
memory controls. Completed folder checks resume on the next run. Results and converted
designs are kept in a sibling `(Doctor)` folder. Only the top-level `report.json` is
anonymous; keep all other output private. **Add matching STEP** on a result card opens
local checking for that file. A same-name STEP is shown automatically; choose another
export if needed, set a numbered report target, then click **Start local checks**.
Comparisons are advisory measurements, not a rendered visual proof. Nothing is sent.

When the installed SolidCraft CLI supports snapshots, flagged comparisons show stored
source and matching STEP or converted geometry side by side. These are whole-design
views. On a file check, click **Target part N** to save the report target. Comparison
body numbers still need local resolution to the correct assembly part.

Failed-file cards and expanded converted-file reporting sections offer **Review and send report**. Review the part roles and exact
sizes; bundles contain geometry. The public-test permission is separate and unticked.
Extraction is not supported yet: choose anonymous summary, or use the private-send
card for the whole file. Send/retry always requires a click. If the endpoint does
not answer, the report remains in the output folder under `reports/<id>`.

## Assembly placements

Open **Advanced → Part positions** to choose:

- **Smart** (default): accept current poses only when touching joints close, geometry fits
  within first-placement assembly bounds grown by 10%, and repeated inserted designs move
  together. Keep first placements when a check cannot be established.
- **As last shown in the original program**: use current poses where available, then first placements.
  The desktop app names the program (for example **As Inventor last showed them**) when
  every queued design comes from one.
  Current poses can be stale when a bound displayed pose is unavailable.
- **As first placed**: place parts where they were first placed, ignoring later moves.

The choice is remembered. **As first placed** restores the old placement choice. Joints work
from the selected placements. The report's Info note counts the placement rules and explains
Smart's fallbacks. Smart and **As last shown in the original program** first use decoder-bound saved-scene
displayed poses when available, without the current-pose guards. Ambiguous and unbound
scene rows do not select placements; older decoders can omit the arena.
