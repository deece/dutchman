<h1>
  <img alt="Dutchman Convert" src="assets/logo/dutchman-lockup.png#gh-light-mode-only" width="252">
  <img alt="Dutchman Convert" src="assets/logo/dutchman-lockup-night.png#gh-dark-mode-only" width="252">
</h1>

Converts Fusion, Inventor, SolidWorks, FreeCAD, CATIA, Rhino, Creo View, ACIS, and STEP designs into SolidCraft designs, keeping editable features where they map.

<a href="https://dutchman-cad.github.io/view.html?src=https://raw.githubusercontent.com/dutchman-cad/dutchman/main/docs/images/readme-result-day.png&alt=Dutchman%20Convert%20in%20Day%20theme%3A%20a%20CC0%20FreeCAD%20bracket%20converted%2C%20with%20its%20mapped%20feature%20history%20and%20Open%20in%20SolidCraft"><img alt="Dutchman Convert in Day theme: a CC0 FreeCAD bracket converted, with its mapped feature history and Open in SolidCraft" src="docs/images/readme-result-day.png#gh-light-mode-only" width="100%"></a>
<a href="https://dutchman-cad.github.io/view.html?src=https://raw.githubusercontent.com/dutchman-cad/dutchman/main/docs/images/readme-result-night.png&alt=Dutchman%20Convert%20in%20Night%20theme%3A%20the%20same%20CC0%20bracket%20conversion%20and%20its%20result%20card"><img alt="Dutchman Convert in Night theme: the same CC0 bracket conversion and its result card" src="docs/images/readme-result-night.png#gh-dark-mode-only" width="100%"></a>

See it first: [converted designs in your browser](https://dutchman-cad.github.io/demo/), including two complete 3D printers.

## Download

- **Mac:** [download the app](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-macos-universal.dmg), or `brew install --cask dutchman-cad/tap/dutchman-convert`.
- **Windows:** [download the 64-bit installer](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-pc-windows-msvc.msi) and open **Dutchman Convert** from Start (no administrator rights needed). A [portable ZIP](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-pc-windows-msvc.zip) is also available; or `scoop bucket add dutchman-cad https://github.com/dutchman-cad/scoop-bucket` then `scoop install dutchman-cad/dutchman`.
- **Linux:** download for [x86_64](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-unknown-linux-gnu.tar.gz) or [ARM64](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-aarch64-unknown-linux-gnu.tar.gz), extract it, and open `dutchman-gui`; or `brew install dutchman-cad/tap/dutchman`.

The Windows installer is unsigned. If SmartScreen says **Windows protected your PC**, click
**More info**, then **Run anyway**. See [Windows first run](docs/install.md#windows-first-run).

Release packages include local `skills/`, Markdown `docs/`, and an `AGENTS.md` explaining
commands and saved reports ([locations and omitted images](docs/install.md#included-guidance)).

Every download holds the desktop app (`dutchman-gui`), the `dutchman` command, and the `dutchman-mcp` server. On macOS, the app needs a [one-time approval](docs/install.md#opening-the-app-the-first-time). [Install](docs/install.md) has Docker, nightly builds, and building from source.

<details>
<summary>All downloads, with checksums</summary>

<!-- downloads -->
| Your computer | Download | Checksum |
|---|---|---|
| Mac (any Mac): the desktop app | [dutchman-macos-universal.dmg](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-macos-universal.dmg) | [SHA-256](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-macos-universal.dmg.sha256) |
| Mac command line (any Mac) | [dutchman-macos-universal.tar.gz](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-macos-universal.tar.gz) | [SHA-256](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-macos-universal.tar.gz.sha256) |
| Windows, 64-bit: installer | [dutchman-x86_64-pc-windows-msvc.msi](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-pc-windows-msvc.msi) | [SHA-256](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-pc-windows-msvc.msi.sha256) |
| Windows, 64-bit: portable ZIP | [dutchman-x86_64-pc-windows-msvc.zip](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-pc-windows-msvc.zip) | [SHA-256](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-pc-windows-msvc.zip.sha256) |
| Linux, x86_64 | [dutchman-x86_64-unknown-linux-gnu.tar.gz](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-unknown-linux-gnu.tar.gz) | [SHA-256](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-x86_64-unknown-linux-gnu.tar.gz.sha256) |
| Linux, ARM 64-bit (aarch64) | [dutchman-aarch64-unknown-linux-gnu.tar.gz](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-aarch64-unknown-linux-gnu.tar.gz) | [SHA-256](https://github.com/dutchman-cad/dutchman/releases/latest/download/dutchman-aarch64-unknown-linux-gnu.tar.gz.sha256) |
<!-- /downloads -->

</details>

## How to use it

1. **Drop files or folders in.** Or choose **Add files…**, then choose an output folder.

   <a href="https://dutchman-cad.github.io/view.html?src=https://raw.githubusercontent.com/dutchman-cad/dutchman/main/docs/images/readme-drop-day.png&alt=Start%20screen%20with%20the%20drop%20area%20and%20Add%20files%20button"><img alt="Start screen with the drop area and Add files button" src="docs/images/readme-drop-day.png" width="100%"></a>

2. **Pick Fast, Standard, or Full.** Fast keeps the shape; Standard keeps history, sketches, and joints within the memory you have free; Full keeps everything that maps. The estimate shows memory and time before you start.

   <a href="https://dutchman-cad.github.io/view.html?src=https://raw.githubusercontent.com/dutchman-cad/dutchman/main/docs/images/readme-preset-day.png&alt=A%20CC0%20bracket%20queued%20with%20Standard%20selected%20and%20a%20live%20memory%20and%20time%20estimate"><img alt="A CC0 bracket queued with Standard selected and a live memory and time estimate" src="docs/images/readme-preset-day.png" width="100%"></a>

3. **Convert, then Open in SolidCraft.** The result card says what came across and what didn't. Your design is a `.sc` file.

   <a href="https://dutchman-cad.github.io/view.html?src=https://raw.githubusercontent.com/dutchman-cad/dutchman/main/docs/images/readme-result-day.png&alt=The%20CC0%20bracket%27s%20result%3A%20mapped%20feature%20history%20and%20Open%20in%20SolidCraft"><img alt="The CC0 bracket's result: mapped feature history and Open in SolidCraft" src="docs/images/readme-result-day.png" width="100%"></a>

## What's new in 0.2.8

- **Right shapes, or none:** a shape Dutchman can't prove is refused or marked, not kept wrong. Fusion extrudes use the sketch region their end face proves, a rebuilt history is checked against the bodies Fusion stored, and the result card says when they don't match.
- **More kinds of features kept:** extrudes from or to a face, holes at several points, counterbores and countersinks with a drill point, fillets with several radii, Move Face, Inventor plates attached at 90 degrees, and placed cylinders and cones (SolidCraft 0.24.1 or newer). A Fusion Canvas image no longer stops a part's history.
- **Honest counts:** results count only features actually rebuilt and confirmed. The Voron V1.8 shows 253 of its 1,102 features that need rebuilding; 0.2.7 counted 290, including features it couldn't confirm.
- **Less memory, measured not guessed:** the Voron Trident peaks at 5.6 GB instead of 10.5 GB, the V1.8 at 8.5 GB instead of 11.7 GB, and a large Fusion archive at 17.5 GB instead of 21.5 GB. A conversion isn't refused on an estimate any more; it stops with a clear message only when the computer really runs low, and keeps the design.
- **No terminal windows on Windows:** converting in the desktop app no longer opens a terminal window for each SolidCraft check, and each file converts in its own process, so a crash ends that file and not the app.
- **Cards that fit:** result cards fit the window, any result with a loss has **Report this file…**, and **Ask your AI about this file** gives a short prompt every time.

[Release notes](https://github.com/dutchman-cad/dutchman/releases/latest) · [Changelog](CHANGELOG.md)

## Command line

```sh
dutchman part.f3d                          # writes part.sc beside it
dutchman convert part.f3d out.sc --preset fast
dutchman inspect part.ipt                  # what would carry across, without writing
dutchman convert-all project/ --out-dir converted/
```

[Install the macOS command-line archive](docs/install.md#from-a-release-archive), including
checksum verification and adding the programs to your PATH.
[The command line](docs/command-line.md) covers every option.

## What converts

Fusion assemblies default to **Smart** part positions. Decoder-bound displayed poses take
precedence when available; otherwise current poses are accepted when
joints close, assembly bounds are respected, and repeated inserted designs move together.
Other parts keep their first placement. Choose **As last shown in the original program** to use all
available current poses, or **As first placed** to restore the old placement choice:
`--part-positions first-placed`, also available under **Advanced → Part positions** in the
desktop app. Joints work from the selected placements. Reports count the placement rules.

Early development: **Reads** says whether a format's geometry can be read, and each conversion's report says what that file kept. Run `dutchman formats` for the live list.

<!-- formats: generated from dutchman formats --markdown -->
| Format | Files | Reads | Level | Converts |
|---|---|---|---|---|
| Fusion archive | `.f3d` | Reads geometry | L5 | Geometry; feature history when every feature maps; components and mapped joints |
| Fusion multi-document archive | `.f3z` | Reads geometry | L5 | As a Fusion archive, with referenced designs merged in by cadmpeg |
| Inventor part | `.ipt` | Reads geometry | L3 | Geometry; feature history when every feature maps |
| Inventor assembly | `.iam` | Reads geometry | L3 | Placed parts and sub-assemblies, read from their files in the project |
| SolidWorks part | `.sldprt` | Reads geometry | L3 | Geometry; feature history waits on cadmpeg binding sketches and extrude profiles |
| SolidWorks assembly | `.sldasm` | Reads geometry | L3 | Placed parts and sub-assemblies, read from their files in the project |
| FreeCAD document | `.FCStd` | Reads geometry | L5 | Geometry and components; feature history and joints when they map |
| CATIA part | `.CATPart` | Reads geometry | L3 | Geometry through STEP; solids and surfaces SolidCraft can import |
| Creo part | `.prt`, `.prt.N` | Opens, no geometry yet | L1 | Not yet |
| NX part | `.prt` | Opens, no geometry yet | L1 | Not yet |
| Creo View | `.pvz` | Reads geometry | L3 | Geometry through STEP; solids and surfaces SolidCraft can import |
| Rhino | `.3dm` | Reads geometry | L2 | Geometry through STEP; solids and surfaces SolidCraft can import |
| ACIS | `.sat`, `.sab`, `.smt`, `.smb` | Reads geometry | L3 | Geometry through STEP; solids and surfaces SolidCraft can import |
| STEP | `.step`, `.stp` | Reads geometry | L8 | Geometry through STEP; solids and surfaces SolidCraft can import |
| IGES | `.igs`, `.iges` | Recognised, not converted | – | Import it in SolidCraft directly (File > Import) |
| Unidentified part (Creo or NX) | `.prt` | Recognised, not read yet | – | Not yet |

FreeCAD TechDraw drawings also convert to DXF (`--to dxf`): page curves, dimension labels, annotations, and templates, with unsupported content reported as Dropped.

Output is a SolidCraft design (`.sc`), or, experimental, a FreeCAD rebuild macro (`.FCMacro`, `--to freecad`).
<!-- /formats -->

Designs open in SolidCraft 0.18.2 to 0.41; 0.41 is the newest known target.

A design Dutchman can't finish reading is stopped after 20 minutes with no progress (60 in all); in a Fusion `.f3z` archive it's left out and named, and the rest converts. Set `DUTCHMAN_DESIGN_TIME_LIMIT` to more minutes to give a large design longer.

[Formats](docs/formats.md) has the details for each program, and [Feature history](docs/history.md) what maps onto SolidCraft.

## Report a file that fails or converts wrong

Make a small public test file from a [fixtures recipe](https://dutchman-cad.github.io/fixtures/),
or send a file that converts wrong, with its STEP export, to Dutchy in
[Discord](https://discord.gg/yTcseBEdeu). Share only designs you own and can share.

Converted cards have a closed **Something wrong? Report this file** section. Expand it
for **Review and send report** and the private-send steps. The anonymous review shows
exactly which metadata will be sent; descriptions go in the private DM or GitHub form.
The CLI's `dutchman report` and MCP's `report_preview` / `report_submit` also support
converted-file summaries. See [Reporting a problem](docs/guide.md#report-consent).

With diagnostic logging enabled, each result card can **Show this run's log** or
**Copy log to…**. Its name includes the date, time, Dutchman version and result, with no
names or contents from the design. Report review and the copied Discord DM identify that
same log. [Reporting a problem](docs/desktop.md#reporting-a-problem) explains what is shared.

![Failed synthetic conversion in Day theme, with Show this run's log and Copy log to buttons](docs/images/readme-run-log-day.png)

## Use from scripts and AI agents

For a desktop result with losses, **Ask your AI about this file** copies a short prompt
naming the saved report and the run's log; your assistant reads the bundled guidance from there
and uses the bundled command line for focused follow-up. No server setup is needed. See [desktop usage](docs/desktop.md#ask-your-ai-about-this-file).

`dutchman --json` prints one machine-readable report, and `dutchman-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server on stdio with conversion, diagnosis, and reporting tools. The core conversion tools return the same reports as their matching `--json` commands:

| Tool | Arguments | Returns |
|---|---|---|
| `inspect` | `input` | `dutchman.inspect/1` |
| `convert` | `input`, `output`, and the convert options ([all arguments](docs/mcp.md#tools)) | `dutchman.convert/1` |
| `convert_all` | `inputs` (files and folders), `out_dir`, and optionally `jobs` and the convert options | `dutchman.convert-all/1` |
| `formats` | none | `dutchman.formats/1` |
| `read_report` | `input` (saved `.report.json` or its `.sc`; local paths stay private) | `dutchman.convert/1` plus `loss_groups` |

For Claude Code:

```sh
claude mcp add dutchman -- dutchman-mcp
```

For other clients:

```json
{
  "mcpServers": {
    "dutchman": {
      "command": "dutchman-mcp"
    }
  }
}
```

[The MCP server](docs/mcp.md) has every argument, client setup, and safety; [JSON reports](docs/json.md) has every field. Exit codes: `0` converted, `1` could not convert, `2` usage error.

## Docs

**Using Dutchman**

- [Getting started](docs/guide.md): install, a first conversion, the loss report, and opening the result.
- [Install](docs/install.md): Homebrew, Scoop, release archives and checksums, Docker, nightly, and source.
- [The desktop app](docs/desktop.md): presets, estimates, the SolidCraft version pill, and updates.
- [The command line](docs/command-line.md): options, `inspect`, whole folders, and watching a folder.
- [Choosing what to keep](docs/presets.md): Fast, Standard, Full, and the memory and time estimate.
- [Troubleshooting](docs/troubleshooting.md): common problems and diagnostic logs.
- [Find what breaks in a large private file](docs/diagnose.md): `dutchman diagnose` ranks the failing, slow and large parts by number, and `--part` converts one.
- [Make a minimum reproduction](docs/minimum-reproduction.md): isolate a failure locally and rebuild a small file you are free to share.

**Formats and results**

- [Formats](docs/formats.md): how each format is recognised and what converts.
- [Fusion](docs/fusion.md), [Inventor](docs/inventor.md), [SolidWorks](docs/sldprt.md), and [FreeCAD](docs/freecad.md): what converts from each program today, and what stands in the way.
- [Feature history](docs/history.md): what maps onto SolidCraft, how it is checked, and partial history.
- [Testing Dutchman on your library](docs/testing-your-library.md): convert your own files and report what broke, without sharing a file.
- [The anonymous report](docs/report-anon.md): what is safe to share.

**Scripts, servers, and agents**

- [The MCP server](docs/mcp.md): every tool and argument, client setup, and safety.
- [JSON reports](docs/json.md): every field of the `--json` reports.
- [Docker](docs/docker.md): convert or run the MCP server in a container.
- [Dutchman Web](docs/web.md): convert in a browser, on a server you host.
- [Agent skill](skills/README.md): teaches an AI assistant to convert files with Dutchman.
- [Pilot](docs/pilot.md): drive SolidCraft from plain-language requests.
- [Dutchman Print](https://github.com/dutchman-cad/print): send designs to a 3D printer.

**Contributing**

- [CONTRIBUTING.md](CONTRIBUTING.md), and [AGENTS.md](AGENTS.md) for AI contributors.
- [Architecture](docs/ARCHITECTURE.md), [Releasing](docs/releasing.md), and [Triage](docs/triage.md).
- Report a file that didn't convert on the [**A file didn't convert**](https://github.com/dutchman-cad/dutchman/issues/new?template=file-did-not-convert.yml) form, with no design file. Questions: the [Discord](https://discord.gg/yTcseBEdeu).

## Licence

Dutchman is pure Rust, licensed under the [Apache License, Version 2.0](LICENSE). Native files are read by [cadmpeg](https://github.com/cadmpeg/cadmpeg); Dutchman maps them onto [SolidCraft](https://github.com/tangency-labs/solidcraft).
