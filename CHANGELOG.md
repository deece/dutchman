# Changelog

All notable changes to Dutchman are listed here. Versions follow [Semantic Versioning]; until
1.0, a minor version may change the command line, the Rust API or the JSON reports (a JSON
report's schema name changes when a field changes incompatibly).

Unreleased changes live in [changelog.d/](changelog.d/).

[Semantic Versioning]: https://semver.org/spec/v2.0.0.html

## 0.2.9 (2026-11-13)

### Known issues

- In SolidCraft 0.40, a loft whose first and last sections are regions of the same sketch has no
  mass. It opens correctly in SolidCraft 0.41 and 0.30
  ([tangency-labs/solidcraft#15263](https://github.com/tangency-labs/solidcraft/issues/15263)).
- In SolidCraft 0.40 and 0.41, a STEP file with an edge across a closed spline's seam can come in
  with faces left out, so a closed solid opens without volume
  ([tangency-labs/solidcraft#16118](https://github.com/tangency-labs/solidcraft/issues/16118)).

### Formats

- SolidWorks parts saved with several configurations leave out bodies belonging only to other configurations and report the saved configuration converted.
- A design that makes no progress for 20 minutes, or runs 60 minutes in all (in an archive, from when that design starts), counts as stuck and is stopped, in the desktop app too; `DUTCHMAN_DESIGN_TIME_LIMIT` sets the minutes (0 turns it off). In a Fusion `.f3z` archive the stuck design is left out and the rest of the archive converts; the report names it. Before, one stuck design could hold a whole archive for hours with nothing else converting.

### History

- A kept partial history is now checked more tightly against where the source has each body, so a feature that rebuilds a body in the wrong place is no longer kept.
- Rebuilt histories with large body position or box differences are no longer reported as verified.
- Curved histories can verify despite small box measurement differences. Position and box checks allow the larger of 0.01 mm and 0.02% of the larger box diagonal.
- A sketch region whose holes cross each other (a nut pocket drawn over a bolt hole) is now
  found. SolidCraft refused the extrude with "unknown complete source profile", because the
  region's hole is the union of the two; the region is now named by a point inside it.
- Keep partial history unverified when any source body centre rests on incomplete edge evidence, even if every body has a centre entry.
- A trimmed sketch region bounded partly by a piece of a line is now found. The piece was read
  as a distance along the line instead of the fraction of it that cadmpeg gives, so the region
  never closed. Such regions are now named by a point inside them, and their unused curves are
  written as construction.
- A loft section that takes several loops of one sketch is no longer mapped: the source can fill the outer loop where the loops read as a ring, so the rebuilt loft could have a hole the original does not.
- A Fusion feature whose profile selection stays native, in a sketch with one profile loop that other curves divide into several regions, now stops the history there instead of using every region. Before, a cut through such a sketch could remove the whole part.
- A feature that takes a list of a sketch's profiles no longer keeps that sketch's unused curves
  solid. A line in no profile (one from the origin to the centre of a ring, say) is now written as
  construction, as it already was for features that select regions, instead of splitting the
  profile so that SolidCraft refused the extrude ("unknown complete source profile").
- Write a spline SolidCraft cannot take directly as a chain of exact spline pieces instead of
  leaving out the sketch, including its profiles and sweep paths.
- Keep shared endpoints and smooth joins between spline pieces, and report constraints that
  cannot apply to the chain.
- A loft whose section or guide rail would use such a chain is not mapped yet, since more edges
  in a section would change the lofted surface.
- A sketch curve that bounds no region any feature uses is written as construction, even when it
  belongs to one of the sketch's closed loops. A tab no feature takes, drawn along part of the
  edge of the region an extrude takes, or a ring around a hole that region keeps, used to stay
  solid and split the region in SolidCraft, which then refused the extrude ("unknown complete
  source profile"). Features that select trimmed regions now count the curves they use instead
  of keeping every curve of the sketch solid, when the region's boundary closes.
- Regions that share part of an edge and are taken together (a tab and the plate it sits on)
  are named by a point inside each, so SolidCraft finds them where it splits the shared edge.

### Assemblies

- Carry a sketch from another component into a part placed more than once when exactly one instance puts the selected profile on the part. Ambiguous placements leave the sketch out.
- A Fusion component whose feature uses a sketch another component builds (designed in the
  context of the assembly) keeps its history past that feature, when each of the two components
  is placed once. The sketch comes into the component with its plane moved to where the other
  component puts it. Such features used to stop the component's history ("its sketch is not an
  earlier mapped sketch").

### Desktop app

- **Ask your AI about this file** now tells the assistant to start with `dutchman investigate`,
  and the installed `AGENTS.md` is one procedure: run it, explain `summary.md`, and ask once
  whether to send it.
- In a folder run, the bar under the counts names the file that has run longest, its stage and its current step, and how many other files are running.
- Progress shows one duration format ("45 s", "1 min 1 s", "2 min", "1 h 5 min"), and a check that took longer than its limit says in a full sentence what happened, for example "the check stopped, and the history is kept unverified".
- While converting, the progress card names the part or stage running now and how long it has been on it, beneath the design counts ("Checking Bracket (rebuilt history): 12 s").
- Past its usual time a step says "Still …", and past its check limit the card says what happens next, for example that the check is stopped and the history kept unverified.

### Command line and install

- Progress durations use one format ("45 s", "1 min 1 s", "2 min", "1 h 5 min") instead of repeating the total seconds, and a check that took longer than its limit says what happened to its output.
- The plain progress line and `dutchman.progress/1` records name the running part or stage, its elapsed time, its usual duration and the check limit that applies, with what happens past it. Part names stay out of the diagnostic log and anonymous reports.
- Release builds started by hand can build just the Linux x86_64 tarball and its checksum with `targets: linux-x86_64`.
- Designs open in SolidCraft 0.18.2 to 0.41.
- SolidCraft 0.41 is the newest known target and the default without an installed CLI.

### Reports

- `dutchman investigate` also converts the design with the newest nightly and the candidate
  builds in `docs/candidates.toml`, downloaded and checked against their published SHA-256,
  and the summary says what each changes, regressions first. `--no-compare` skips it.
- `dutchman investigate design.f3d` looks into a conversion that went wrong, for the AI a user
  asks for help. It reads the saved report (or converts once), narrows a failure, and writes
  one shareable `summary.md` with no names, paths, or sizes from the design. It then asks once
  whether to send that summary to the Dutchman team.

### cadmpeg

- The cadmpeg pin moves from `52ba5a1` to `f9d0a14` (cadmpeg 0.6.11).
- Inventor: more history forms decode, among them more fillet forms (no longer left native), state-2 Thicken face selectors, full-turn revolves, through-all surface extrusions over a solid body, work-plane body trims, and mirror planes from work planes.
- Fusion: face and fillet edge selections are read from the history their scope is bound to, and Cut and reordered Coil scopes decode.
- SolidWorks: closed faces without edges and ring edges are kept, more offset surfaces convert, and source bodies used by derived or mirrored geometry no longer appear as current bodies.
- Fusion: more threads, holes, healed face deletions, extrudes, fillets, shells, lofts, pipes, and patterns decode for history mapping. Sketches keep their timeline names, and historical body centres use sampled edges with incomplete evidence identified.
- Inventor: fixed-spacing rectangular body patterns and more face selections decode for history mapping. CATIA: more shared sheet boundaries and sheet trims convert.

### Docs

- CI tries the Linux ARM64 tests once more when GitHub shuts their runner down mid-job; any
  other failure still fails the required check.

## 0.2.8 (2026-11-08)

### History

- An extrude that starts or ends at a face as it was earlier in the history (a face that later
  features change) keeps its history: the face's plane is rebuilt from that earlier state.
- A fillet with several radius groups keeps its history: each group becomes its own blend, in
  the order the source applies them.
- A hole placed at several sketch points keeps its history, drilled at each point, when its
  placements are exactly the sketch's points and all drill the same way.
- An extrude whose profile region does not match the source is no longer counted as kept. When
  the design's only geometry-building feature is a blind one-sided extrude whose end face
  survives in the stored model, the area of the swept region (outer loops minus holes) must
  match that face's area to within 1 mm² or 3%. A mismatch blocks the extrude with the
  swept and source areas in its reason, records the loss code
  `history/profile-region-mismatch`, and stops the editable history there.
- A kept extrude whose region could not be checked this way carries `profile_unverified: true`
  in the JSON report's `history.features`, so counts do not claim more than was proven.
- Counterbore and countersink holes with a drill point keep both when the design targets
  SolidCraft 0.22.1 or newer; for older SolidCraft they are still refused, as before.
- A feature whose profile is several selected regions merged into one self-crossing loop now
  says so in the report, instead of blaming the sketch's other curves. Such a profile is still
  not mapped.
- An extrude that ends at the plane of a face, after earlier features, now keeps its history
  in Fusion and Inventor files: it ends at the same plane in SolidCraft. Extrudes ending at a
  work plane or point after earlier features are still left as stored geometry.
- An extrude that starts at a plane parallel to its sketch now keeps its history, as a start
  offset from the sketch.
- An extrude that selects one region between sketch lines sweeps that region: the lines that
  bound it stay solid instead of becoming construction lines, which merged the regions and
  swept the wrong area.
- A body the source file leaves unnamed (as Inventor parts do) is named "Body 1", "Body 2" and
  so on, so a later feature such as a through-all cut can still target it and the history
  continues past it.
- An extrude stored as a join before any body exists makes the first body, as it does in the
  source, instead of leaving later features with no body to work on.
- A sheet-metal attached plate and its paired bend now map to one SolidCraft edge flange when
  the plate is a rectangle spanning the whole attachment edge, square to the sheet at 90
  degrees, flush with the edge and of the sheet's thickness. Other plates report the specific
  shape that is not supported yet, and the geometry is still converted.
- A Move Face that moves planar faces along their normal is written as SolidCraft's offset-face
  feature, with each face picked by a point inside it. Rotations, translations along another
  direction, non-planar faces and unresolved selections are reported with their own reason.
- An extrude whose region is part of a circle crossed by another keeps its history: two crossing
  circles bound several regions with the same curves, so the region is now selected by a point
  inside it, taken from the stored boundary. A boundary that doesn't close stays refused.
- cadmpeg gives a Fusion Base Feature's body only as the design ends. Joins and cuts after it
  re-apply to that body without changing the shape, so they are kept but no longer counted as
  rebuilt: the report's `history.reapplied` counts them, and the result card says editing them
  may not change the shape. A Base Feature whose body any other later feature changes, such as a
  Move, is refused, naming that feature; it had replayed the body where the later features leave
  it, then applied them again.
- The SolidCraft replay that finds a blend's or a face profile's input stops at the last step it
  inspects, so a slow feature later in the history no longer slows each replay. On the public
  RailCore II 300ZLT this was one part with a 20 s extrude, replayed twice.
- Fusion cylinder and cone primitives keep their history, placed by their feature frame: a whole
  cylinder becomes a SolidCraft cylinder, and a whole cone a SolidCraft cone on SolidCraft 0.24.1
  and newer. They used to be refused as having no placement, which stopped the history of three
  public designs (a parametric pin, a T8 leadscrew and the Salad Fork's captive pins).
- A Fusion Canvas (a reference image) no longer stops a history: it builds no geometry, so it is
  counted as not needed. On the public Voron V1.8 it stopped the history of parts holding 810
  features.
- A feature whose profile is the whole of a sketch with several profiles is kept only when
  SolidCraft's check confirms it; otherwise the history stops before it. Such a cut could remove
  the whole body where Fusion keeps one.

### Assemblies

- Reference sketches in a large assembly are mapped from the sketch, feature, and parameter
  data alone, built once per conversion instead of a full copy of the model for each group of
  components, so the conversion needs less memory.
- The report and the result card name the parts whose position nothing confirms: a warning for
  parts at their design's origin because their placement could not be read, and a note for parts
  resting on a stored pose that no joint, saved view, or usable later pose confirms.

### Desktop app

- Show the notice about a run that didn't finish as one line, at the top of the Results column in a wide window, so the main screen still fits without scrolling.
- A failed file's card fits the window at its default size. **Review and send report** stays on
  the card; Discord, the DM with Dutchy, the STEP export and **Report anonymously on GitHub**
  are folded under **Help us fix it**.
- The small text in the report step boxes is left-aligned instead of justified with wide gaps.
- A converted result with any loss (history not kept or only partly kept, features flattened or
  blocked, parts left unplaced, colours dropped) now has a visible **Report this file…** button
  in its card. It opens the report section with **Review and send report**.
- **Ask your AI about this file** now copies a short paragraph naming your file, the
  converted design, its saved report, and the log of that run. The prompt also opens in a
  box under the button, with Copy and Close, and stays there until you close it or convert
  again.
- **Ask your AI about this file** copies every time you click it. "Copied" shows for about
  two seconds and then the button is back, instead of staying stuck after the first click.
  A new conversion starts with a working button.
- Failed files also offer **Ask your AI about this file**, using the saved failure report.
- Every run keeps a local diagnostic log in the diagnostics folder, with no names or contents
  from your files. The "Save a diagnostic log" checkbox is gone; **Open diagnostics folder**
  stays.
- A result card names its run log as "Run log: …" in small text, cut to fit with the whole
  path on hover, instead of a bare path that read like stray text.
- A design that carries no feature history says so once: "Geometry without history: the
  design carries no feature history." The same reason is no longer repeated on the line under it.
- The **Ask your AI about this file** prompt box shows about five lines and scrolls inside, so
  the whole result card still fits the window with it open.
- The desktop app now finds SolidCraft the way the command line does. A SolidCraft that was
  slow to answer when the app opened (a first start after installing) is no longer treated as
  missing for the whole session: opening the SolidCraft version window searches again. A
  remembered `solidcraft-cli` that no longer answers is ignored in favour of the automatic
  search, and the window lists the places that were searched when nothing is found.
- The Update button now asks GitHub for the latest release each time you press it, so it never offers an older version than the newest one. It shows "Checking for updates…", then the new version, "Dutchman is up to date", or why the check failed with a Retry button.
- Update checks read GitHub's releases feed instead of its rate-limited API.
- Release notes in the update window show their headings, bold text, code and bullet lists properly.
- On a Mac, the free memory shown for designs no longer reads about 0 GB when plenty of memory is available.
- Dismissing "The last run didn't finish." keeps it dismissed after a relaunch, until another run
  doesn't finish.
- On Windows, converting no longer opens a terminal window for each SolidCraft or FreeCAD check.
- The result card says when a partial history's rebuilt features don't match the original where
  the history stops, and that the final shape is still correct, so check the rebuilt features
  before editing them. Until now only the JSON report had it, with `history/body-mismatch`.
- The result card says how many rebuilt extrudes may use the wrong sketch region, because it couldn't
  be checked against the source. Until now only the JSON report marked them, with
  `profile_unverified`.

### Command line and install

- `convert-all` warns on stderr, before converting, when a missing positional argument looks
  like an output path, and its help says every positional argument is an input (use
  `--out-dir` for the output folder).
- `convert --report-anon FILE` writes the anonymous report of one file, with `--report-salt`
  and `--report-key`, as `convert-all` does.
- Progress lines on stderr are off when stderr is not a terminal; `--verbose` brings them back.
- `--write-replay` on `convert` and `convert-all` also writes `<design>.replay.json` beside a
  `.sc` design: the command script that rebuilds its history step by step, for
  `solidcraft-cli run`. The JSON report names the file as `replay_script`.
- `dutchman-mcp --version` prints `dutchman-mcp <version>` and `dutchman-mcp --help` prints a
  short usage note; both exit at once instead of starting the server and waiting on stdin.

### Reports

- The anonymous report now lists each blocked feature's timeline ordinal and a short variant
  (`extrude, to a plane`, `hole, counterbore, through all`, `fillet, 2 radius groups`), in
  timeline order, with no names or sizes. The JSON report's history features carry the same
  `ordinal` and `variant`.
- A face, edge or body selection Dutchman cannot map now says which form it was, such as "a
  face from an earlier history state is not supported yet" or "a native edge reference is not
  supported yet", instead of "not resolved".
- A feature SolidCraft refuses when it replays the history is now reported as
  `SolidCraft refused feature "X": ...`, without SolidCraft's internal site number, instead of
  reading like a Dutchman mapping limit. The JSON feature entry gains `source`, `category`,
  `upstream` and `reason_raw`; a fillet larger than its neighbouring faces has its own category
  with a link to the SolidCraft issue. The anonymous report carries the category and link.
  The same wording is used in the history summary, the verification reason and per-part
  reasons.
- When no SolidCraft is found, the JSON report's `solidcraft.search` and the anonymous
  report's `versions.solidcraft_search` say where Dutchman looked (`SOLIDCRAFT_CLI`, `PATH`,
  Program Files and the other install folders) and what it found in each. The anonymous report
  names the places only, never a path.
- When memory runs low during the SolidCraft check, the check stops and the built design is kept,
  marked unverified with the reason, instead of losing the file.
- An estimate from a file's size no longer refuses a conversion or leaves out feature history.
  The conversion runs, watching memory, and stops with a clear message if the computer really
  gets down to 1 GB free while the conversion holds at least 1 GB.
- Standard leaves feature history out only when what the read really took leaves no room for it.
- `convert-all`, `watch`, and the MCP batch tool no longer stop a file at 8 GB by default.
- The desktop no longer stops to warn about an estimate before converting.
- A file that runs out of memory alongside others in a batch is converted again alone at the end,
  once, and the report says so.
- The desktop converts each file in its own process, so a stop or a crash ends that file, not the
  app.
- The result card and summaries count features rebuilt, out of those that need one. `history.rebuilt`
  and `history.needing` are new in the JSON report; `history.mapped` also counts features that need
  nothing, such as component inserts and joints, so it overstated editable history.
- A partial history that replays but whose bodies differ from what the source stores at the same
  point, fewer of them or one away from where the source has it, is now reported, with the loss code
  `history/body-mismatch`, rather than counted as replaying cleanly. Fusion files store this.
  In a design with components, a stored list that features of more than one component carry is
  not used, since it isn't any one component's state.
  A base feature written with its body as the design ends isn't compared, since the source's
  state after it is the body before later features changed it.
- Where a Fusion file marks a history state's body box as exact, the partial-history body check
  compares its centre within 0.5 mm or 1% of the body's size, rather than half its size.

### cadmpeg

- The cadmpeg pin moves from `65d6e87` to `52ba5a1` (cadmpeg 0.6.10), now taken from cadmpeg's own repository
  rather than a fork of it.
- Fusion, since 0.2.8-rc.3: a through cut no longer reads its remaining far face as its selected
  profile (the wrong intermediate shape some through cuts had in 0.2.7 and 0.2.8-rc.3 is gone).
  Typed profile loops join the regions they bound, Delete Face and Chamfer selections with edge and
  face groups are read, Sweeps that select body faces and more Sweep and Boolean forms decode, work
  planes placed in more ways and Reverse Normal map, a pattern's body seed binds through the state
  before it, and a history state marks the bodies whose stored box is exact. Timeline
  transitions, split planes, absorbed bodies and hole-loop profiles are resolved, construction axes
  become datum axes, surface patches become filled surfaces, and compact Surface Extend forms decode.
- Inventor reads Mirror source bodies, more hole and edge selector forms, derived stored
  bodies, sketch box members and Thicken face operands; Creo transfers geometric tolerances; NX assembly placements and CATIA face colours are
  read more correctly.
- Fusion: extrude profiles are proven from their end caps and recipe operands are bound per
  counted operand, so extrudes no longer sweep the wrong sketch region where cadmpeg picked one
  by heuristic. Fillet, chamfer and face selections match their recipe faces exactly, and
  chamfer edges between faces of other features resolve.
- Fusion also reads text parameters, Design parameters in their comment form, sketch scopes
  that list deleted members, the 466-byte coil form, pipe groups, diagonal and frameless
  dimension annotations, more selection and scope forms, lofts to a sketch point, and work
  plane frames placed by their U direction. It resolves trimmed extrude regions, incomplete
  fillet groups, replaced fillet and chamfer support faces, Move Face and Offset Faces members,
  and a cylinder blend's edge by geometry. An unprofiled sketch curve that bounds a selected
  extrude region is reported at Info level, since nothing is lost.
- A cut that selects several separate regions, stored as one self-crossing loop, is split into
  those regions, and a through cut's regions are chosen from the faces it changed, so both map
  instead of being refused or sweeping the wrong area. Renamed scopes and Boolean and re-tagged
  Revolve forms are decoded, a cylinder blend reads its best-ranked edge and reports that
  inference, and more sketch, loft, body-recipe and construction-group forms are read.
- Fusion assemblies read occurrences that reference a source design, leading-flag local
  occurrences (keeping unregistered ones and preferring registered frames), version-4 and
  localized joint scopes, and joints of a standalone design bound by their relations only; a
  projected joint's assumed activation is reported, and a same-component body copy and paste
  reads as a move with one copy.
- Inventor: polygon sketches close, parameters keep their values when the unit type is
  unrecognised, bodies are named from the feature that owns them, fillet and chamfer input
  edges resolve on elliptical, spline and bounded ellipse carriers, with secondary names and
  through edge revisions, and attached plates in a rotated frame are read. Planar curve sketch
  members and construction lines with a conflicting carrier are kept native instead of misread.
- SolidWorks and NX parts read more of their node sequences (Deltas carriers and blends,
  patched points, terminated-stream nodes, attributes and NURBS lanes), and analytic and
  B-spline carriers are decoded once for both.

### Docs

- Troubleshooting explains how to replay a converted history by hand and what a `.sc` design
  holds.
- Contributing explains how to build against a local cadmpeg, how to build a fast optimised
  profile for checking one file, and that tests for bugs found on private files use generic
  dimensions and synthetic geometry.

### Formats

- Falling back to geometry alone no longer fails when a component's definition names sketches
  that the fallback removed.
- A Fusion body an active Remove feature deletes is left out of the converted design. Fusion
  keeps such a body in the file without showing it, and it used to come out as part of the
  design. An Info note on the Remove gives the count. A Remove whose bodies cadmpeg doesn't
  resolve is left as it is.

## 0.2.7 (2026-11-04)

### Assemblies

- Fewer false "colour dropped" notes on Fusion assemblies: colour assignments that do reach the design are no longer reported as lost, including on parts placed more than once and beside a hidden editable history copy. No visible change on the public test assemblies.
- Put face colours on the right faces where a pick landed on the wrong arc of a cylinder or inside a hole, for example on painted stepper motor bodies.
- Keep body colours when a component also contains saved wire edges or a body that cannot be written.
- Say plainly when a colour's target is missing from the decoded design.
- Change the Fusion assembly default to Smart part positions, accepting current poses only when joints, bounds, and repeated inserted designs agree. Use `--part-positions first-placed` to restore the old placement choice.
- Add Part positions choices Smart, As Fusion last showed them, and As first placed to the CLI, remembered desktop Advanced settings, and MCP. Keep `--stored-placements` as a first-placed alias.
- Report the selected mode, placement rule counts, and fallback reasons in JSON and an Info note.
- Compose current poses with the containing document's insertion and retain ambiguous decoded placements with reported counts.
- Prefer decoder-bound saved-scene displayed poses in Smart and As Fusion last showed them, without current-pose guards; skip ambiguous and unbound rows and support decoders without the arena.
- Keep joints closed at displayed poses by deriving missing holder placements from agreeing children, and report derived holders and conflicts.
- Keep groups whose joints already close at their stored poses before placing other joined parts, and prefer a non-identity stored world pose when starting an unplaced joint group.
- Carry the smallest connected holder or its origin body assembly when a joint places it, leaving unrelated children of identity organisers where they belong.
- Resolve isolated child pose markers within a posed component against already closing joints, while preserving explicit holder and independently contained poses.
- Apply joint offsets and angular offsets in the second connector's frame, keeping joined parts at their stored poses when those frames already close.
- Keep slider translation offsets and use the matching distance direction for aligned connector frames.
- Recheck joint closure after all parts are placed and suppress conflicting joints that would move them again on open.
- Convert large assemblies faster with less temporary memory while keeping their geometry, history and placements.
- Reduce memory used to prepare nested placements and reference sketches.
- Keep a sub-assembly shared across its copies when Fusion shows every copy with the same arrangement inside it; Smart and As Fusion last showed them now write the same components and occurrences as As first placed in that case.
- In Smart, keep the first placement of a sub-assembly, and everything inside it, when parts below it would leave the model's extent (grown by 10%) at the position Fusion shows.
- Keep a sub-assembly whose displayed parts do not agree on where it sits at its first placement, even inside a sub-assembly that moves, and use a sub-assembly position worked out from its displayed parts only when it reproduces all of them.

### Desktop app

- Save each conversion's full local JSON report beside its design, and save failed reports as .failed.json.
- Copy a prompt from converted cards with losses to ask an assistant about the saved result, reading installed guidance first and using plain JSON and the bundled command line without extra setup or another whole-file conversion.
- Read saved results through the optional read_report MCP tool, including losses grouped by code and stage, feature outcomes, placements and verification.
- Dutchman finds `solidcraft-cli` and the SolidCraft app when they sit beside it, and the SolidCraft
  version panel can choose the `solidcraft-cli` to check with and remembers it.
- When only the SolidCraft app is found, the panel says why that is not enough and which Windows
  downloads include `solidcraft-cli`.
- Open the window at a size that shows the whole main screen at once, centred on the screen, with no scrolling. A window size you saved is still restored.
- Show Designs and Output folder beside Results in a wide window; narrow windows keep one column.
- Scroll long results inside the Results column, and a long list of designs inside Designs, instead of scrolling the whole screen.
- Tighten the spacing of the options so more fit in view, and show the page scrollbar only when it is needed.
- Keep expanded loss notes inside their result card, including long feature ids, while wrapping ordinary text at spaces.
- Adjust presets, retained content and history checking in the memory warning, with live
  memory and time estimates matching the file row and choices remembered in the main window.
- Show when Standard leaves history out, when checking may be skipped, and when the choices
  still will not fit. Keep conversion and cancel buttons visible in narrow and short windows.
- Read archive sizes in the background and reuse them when choices change.
- Show measured estimates for reading, converting to SolidCraft and checking history, with memory and time for each and the same total in the file row.
- Use the displayed check allowance when deciding whether a history check fits in free memory.
- Count geometry checking when feature history is off and let Check in SolidCraft turn it off.
- Keep history memory and time estimates at least as high as the same conversion and check without history, including small parts.
- Add the displayed stage durations for the total time in the memory box and file row, preserving sub-minute bounds.
- Explain that estimates can be off by a few GB either way.
- Keep the title and every header control visible at narrow window widths, including when an update is available.
- The memory warning shows the same figure as the file's row: the memory converting needs. It
  showed about twice that, adding a history check that is skipped when it would not fit.
- Name the original program in the Part positions choice, such as "As Inventor last showed them", when every queued design comes from one program, and say "As last shown in the original program" otherwise.
- Report converted files with lost features, wrong placement or missing colours from a section that starts closed on the result card.
- Review the exact anonymous metadata before sending, or use the existing private-send steps to share the file.
- Preview and send converted-file summaries with `dutchman report` or the MCP reporting tools.
- Open at the default size when the saved window size is too small to use, for example after Dutchman Convert was closed while minimized, instead of restoring a window about 100 pixels wide.
- Find the right diagnostic log on each result card with Show this run's log and Copy log to…;
  batch cards share one log and retries keep the other cards linked to their original run.
- Include date, time, Dutchman version and result in automatic log names, keeping running
  names after crashes and recognising older logs when keeping the newest ten.
- Open the right diagnostics folder on Windows and select the run's log with native paths.
- Identify the run's log in report review and copied Discord messages, explain that it has
  no design names or contents, and show an advisory notice for logs from another version.

### Command line and install

- Install Dutchman Convert on Windows without administrator rights, with a Start menu shortcut and commands on your user PATH. The portable ZIP remains available.
- Update Windows installer copies with Install (closes Dutchman), a verified installer and automatic restart. Portable ZIP copies offer Update now and Restart now; Scoop copies keep Update with Scoop.
- Uninstall Windows installer copies while keeping settings and diagnostic logs.
- Include the agent skill, Markdown documentation, and a short AGENTS.md beside installed release programs.

### Reports

- Count dropped colours by cause in anonymous reports and diagnostic logs: wire bodies or saved free edges, unmatched face picks, absent targets, unwritable bodies and other mapping gaps.
- Keep retained colours out of dropped-assignment counts and omit appearance identifiers and example messages from anonymous cause entries.
- Diagnostic logs show anonymous per-member Fusion archive preparation timings for nested scanning, layer classification and reference metadata extraction before decoding starts.
- Diagnostic logs time each step of preparing a Fusion archive: scanning it, classifying its
  documents, and opening the session.

## 0.2.6 (2026-11-02)

### Formats

- Convert SolidWorks assemblies (`.sldasm`) as placed parts and nested sub-assemblies.
  Resolve external files case-insensitively beside the assembly and through project search
  folders, protect against reference cycles, and report missing parts and unavailable
  referenced configurations while keeping the parts that can be read.

### History

- A writer that keeps an unchecked history hidden (`hide_unverified_history`) now does so for
  each part of an assembly too: the part's original geometry stays visible, and its mapped
  history goes into a hidden "Editable history (unverified)" component.
- Empty regions without shells no longer consume body appearance targets on stored base
  features. Their STEP shapes and names stay aligned, preserving later editable features.
  Other shape-count mismatches omit explicit appearance targets while keeping history.
- A cylindrical face with its own colour (holes, bosses, rounded edges) now gets that colour in
  the written design, placed by a point on the face. Before, only planar faces did and the
  colours of curved faces were reported as left out.
- Delete face maps to SolidCraft's delete-face feature when cadmpeg resolves the faces in the
  feature's own input topology (planar and cylindrical faces, with the heal setting). When the
  selection is still native, the history note now says so instead of "not mapped yet".
- Extrudes ending at a resolved plane parallel to their sketch now keep editable history
  before any body-building feature, including placed work-plane ends. Plane ends after
  earlier geometry name the missing input face geometry; tilted planes and vertex ends
  also name their unsupported case.
- Hide partial and unverified editable-history copies from the History bar for targets of SolidCraft 0.30 or newer; their features remain editable in the Browser.
- Extrudes of a face as it was earlier in the timeline ("extrude this face") now keep editable
  history. The face is rebuilt as a sketch of its boundary from the edge geometry the design
  keeps for it, and checked against the face SolidCraft rebuilds before the extrude, which also
  decides which side the extrude runs on. Faces whose boundary is not lines and circular arcs,
  or whose edge geometry is missing, still stop the history and say why.
- Keep history checking on by default and skip checks that do not fit in free memory, reporting the reason while keeping the original shape usable and retained history unverified. Include checking in memory estimates, bound automatic assembly workers by part complexity, and offer a desktop override for the memory guard.
- Offset datum planes resolve signed distances through earlier datum planes, resolved
  support frames or a unique planar face. Independently placed sketch consumers can
  continue when the source support is missing; other unresolved consumers remain guarded.
- Body splits can use the retained offset plane frame.
- A parameter you edit now reaches more of the geometry it sets. An extrude's distance and a
  partial revolve's angle keep the parameter that sets them instead of the value it had; a
  sketch on the end of an earlier feature sits at the height that sketch's dimensions give
  (so a socket cut follows the screw's length); a ring drawn as an outer and an inner loop
  keeps its hole (an inner diameter parameter changes it); and a sketch with a parameter
  whose last loose point or two the source left free keeps that point where it was drawn, so
  one edit cannot fling it across the profile.
- Keep point-on-curve coincidences, point-to-line and finite-endpoint dimensions, and
  parallel-line spacing, with parameter expressions driving the editable geometry.
- Convert cosmetic ISO metric thread annotations with resolved diameter, pitch, length, and
  cylindrical face geometry, including sizes without a source designation.
- Tie a sketch profile that meets the sketch origin to that origin when the source does, so
  a parameter edit no longer slides the profile across a revolve axis.
- An extrude, revolve or other feature that takes a whole sketch, or a list of its profile loops,
  now treats loops inside other selected loops as holes. A shim drawn as two concentric circles
  is written as a ring instead of a filled disc, and a sketch of nested outlines (a logo plate)
  keeps its cut-outs, so these files no longer lose their editable history at the replay check.
- Shells with historical input-face boundaries map thickness, inward or outward direction
  and a single earlier body to SolidCraft. Unresolved input selections, final-body faces,
  unsupported offset modes and corner continuations remain specific history blockers.
- Fusion 3D sketches (spatial sketches) now map to SolidCraft 3D sketches, so the history
  continues after them instead of stopping at "spatial_sketch is not mapped yet". Lines,
  exact arcs, circles, splines and points are written at their solved positions, and a
  sweep path or guide rail picked from their curves resolves to them. Constraints, links to
  model edges and vertices, surfaces and source-native geometry are reported as not carried.
- Stored-geometry features (Inventor stored bodies, Fusion base bodies, and other formats'
  directly stored geometry) import exactly the bodies they output, solids and sheets, at their
  place in the timeline; a stored-geometry feature with no output bodies is reported as holding
  none rather than guessed.
- Sketch curves that no closed profile loop uses (open segments, lines laid across a
  region) are now written as construction when every feature using the sketch selects
  whole-loop regions and none names the curve, so they no longer divide the region the
  extrude, revolve or plate selects. An Info note counts them.
- Through-all extrusion cuts stop mapping when no earlier feature names a body to cut,
  explaining the missing producer before SolidCraft replay. Neutral ordinals continue to
  control replay order even when the feature records are stored in another order.
- A writer that cannot check history against the original (the SolidCraft importer plugin,
  with no SolidCraft CLI in its sandbox) can keep the mapped history in a hidden
  "Editable history (unverified)" part, with the original geometry as the visible model.

### Assemblies

- Reduce conversion time for assemblies with many coloured faces, including with the fast preset.
- Designs compress their stored geometry, so a large assembly stays well under SolidCraft's design file limit: a public 88-component assembly went from 333 MB to 10 MB.
- Joints into a Fusion component stored at the identity move the whole component only when internal joints or file poses bind the rest of what it holds to the joined part, the joints that reach into it agree on where it goes, and the file does not already place both joined parts. An outer joint that places a part inside from something placed (a part joined to the origin) keeps that part where it puts it. RailCore II 300ZL and the Voron 2.4 keep every occurrence where 0.2.5 put them; the robot arm's gripper still moves onto the arm.
- Mirrored placements retain reflected component geometry in shared mirrored definitions,
  including nested reflections. Mirrored copies keep imported geometry and report history
  and affected joints left out. Tiny placement drift is corrected; non-finite, scaled, or
  sheared placements beyond tolerance are left out with a note so the design can open.
  Mirrored geometry is written into the carriers (points, curves, surfaces, free-form
  poles) rather than as a reflecting transform, so SolidCraft keeps every mirrored body.
- A component whose instances hold different components, or the same components at different
  placements, now converts as an assembly instead of one flat import. It is written as one
  definition per layout (`<component>`, `<component> (2)`, ...), each instance at its own
  placement, and the report notes how many components were written as how many definitions.
  Instances with the same layout, within numeric noise, still share one definition, and mirrored
  placements combine with it.
- A component a joint places now carries the parts the file poses inside it. Before, an inserted
  design joined to the rest of an assembly by a part inside it was moved to that joint, but the
  parts the file poses inside it stayed at their old world positions, so they were left away from
  the rest of the design while their neighbours moved.
- A converted assembly writes each sketch under the part that owns it instead of in the
  assembly root, once per part however many times it is placed, including sketches of a
  component that holds no bodies. Sketches with no owner, such as those of the design's own
  root component, stay in the root, and the report counts them.
  Sketches a part's history builds nothing on, and sketches the source hides, start hidden.
- Reduce memory use when converting SolidWorks assemblies by releasing saved display meshes
  and sharing reads of the saved active configuration across sibling sub-assemblies.
- Convert top-level SolidWorks assemblies in folders and ZIP projects with their nested assemblies and parts, then convert unused parts.
- History is shorter on converted assemblies. A part repeated across sub-assemblies or layouts
  is one definition placed in each, and "Original geometry (imported)" is left out when a
  part's base features already hold every one of its bodies. A kept history of base features
  alone gets no hidden "Editable history (partial)" component.
- Joints to parts inside identity-placed Fusion components place the whole component,
  keeping its internal joints together. Components already held by a file pose or a joint
  stay in place, and conflicting joints remain suppressed.

### Desktop app

- Show copied-report notices in the success colour and clear them on the next action.
- Offer only available report choices, use plain consent wording, and show Retry after a failed send.
- Keep the app header during folder checks, clarify the part selector, and keep the page scrollbar visible to reach every private-send option.
- A failed file's card now shows how to send the file privately: join the Discord server, then
  one button copies a one-line message and opens Dutchy's DM. A file within Discord's limit is
  dragged in, with its folder open; for a larger one, Dutchy replies with a private upload
  link. **Report this on GitHub** is now **Report anonymously on GitHub**, and the report id
  matches the diagnostic log. The web app's failed cards show the same steps.
- **Report this on GitHub** opens a short link, so GitHub no longer answers it with a 406 when you are signed in. The anonymous report is copied for you to paste into the form.
- The desktop and web apps show **Dutchman Convert** with the running version beside it in
  their top row, at every window size. A nightly shows its nightly version.

### Command line and install

- `dutchman-core` has a Cargo feature per input format (all on by default, in `formats`), so a
  build that reads only some formats, such as the SolidCraft importer plugin, links only their
  cadmpeg codecs.
- New `dutchman diagnose --narrow FILE` runs the whole narrowing by itself: it ranks the parts,
  isolates a failing part, alone and then with its neighbours, bisects its history to the first
  failing feature, and records the failure's signature (kind of feature, options, profile shape,
  and error code; never names, paths, or real dimensions). Each step has its own limits, it says
  `step 3 of 5: narrowing part 17 (feature 41 of 80)` as it goes, and running it again resumes
  after a crash or a reboot.
- New `dutchman diagnose --same-failure small.f3d` converts your small rebuild and says whether
  it fails the same way, and `--make-report` then makes one zip with a prefilled issue link.
  Nothing is sent: the zip stays on your machine until you send it.
- `dutchman convert FILE --part N --stop-at-feature K` converts a part with its history cut after
  feature K.
- New `dutchman diagnose FILE`: for a large design that takes hours, runs out of memory, or
  fails in one part, it converts the design part by part on your machine, each part under a
  time and memory limit (`--time-limit`, `--memory-limit`), and writes a ranked report that
  names parts by number only (`diagnose/report.json`, `report.txt`; `local-key.json` maps the
  numbers back to your part names and stays private). A part that hangs, crashes or runs out of
  memory costs only that part, and the report is written as it goes.
- `dutchman convert FILE --part N` (with `--with-children` for a sub-assembly) converts only
  part N, as diagnose numbers it, so a failure that took hours to reach shows in seconds. A
  number the design does not have is refused with the range it does have.
- A conversion that cannot fit in the machine's free memory now also stops once the design is
  read, instead of running for hours, naming the largest parts by number; `--no-memory-check`
  still converts anyway.
- `dutchman repro` goes straight to the failing part of a diagnose report (found in `--out`, or
  given with `--from-diagnose`), and `next-steps.md` names the part number and the kind of
  feature to rebuild.
- **`dutchman doctor <folder>`** checks a whole folder or zip on your machine, with CPU and memory
  budgets, private checkpoints so it can resume, and an anonymous report. Repeat failures are
  grouped by signature. Nothing is sent.
- **`dutchman diagnose --step`** compares a design with its matching STEP export, worst part first.
  A STEP beside the file is found by itself. The comparison limits are advisory for now.
- The desktop app can check a folder and take a matching STEP, and the MCP server adds `doctor`,
  `diagnose`, and `compare_step`.
- Convert project ZIPs and folders directly, with private bounded ZIP extraction, a
  project plan, top-level Inventor assemblies and unused parts, and STEP fallback for
  same-name native files. Report unread SolidWorks assemblies while converting their parts.
- Select a new output folder beside dropped inputs, show Inventor settings only when
  applicable, hide unknown memory, and prefer the newest automatically found SolidCraft CLI.

- Show the project inventory before conversion. Web folder uploads preserve their relative
  paths and share the bounded upload budget; results show the project plan.
- Ignore `~$` lock files in project planning for every CAD format, excluding them from
  conversion, failure and skipped-file counts.
- Bound ZIP directory metadata and declared entry counts before allocation, using the
  same preflight for project extraction and conversion estimates.
- Recognize SolidCraft 0.30 as the newest known minor and default target without an
  installed CLI. The supported window is 0.18–0.30; CI checks the newest 0.30 patch
  and the pinned 0.18.2 release. Designs keep format 23, which 0.30 and older targets open.

### Reports

- Anonymous diagnostic logs include failure stages, error categories, model check counts,
  available referenced curve kinds, and timed decoder preparation phases. Geometry check
  failures stop without repeating the decode; history recovery rechecks the model in memory.
- Group repeated stderr loss notes with a count, including appearance bindings on faces.
  Keep individual details in --verbose, --log, and JSON reports.
- Diagnosis now includes numbered rebuild recipes for Fusion, Inventor, SolidWorks and
  FreeCAD, using round starting sizes and the anonymous failure signature. Unknown feature
  kinds get a generic recipe. A versioned release list can identify exact known failures
  before further narrowing and show a workaround or next-version fix advice.
- Failed desktop cards now offer **Diagnose this file**, with step progress, checkpoint
  resume, the recipe, **Check my small file**, and local **Make the report**.
- MCP adds anonymous `diagnose`, `diagnose_narrow`, `convert_part`, `stop_at_feature`,
  `same_failure` and `make_report` tools, with a paste-in troubleshooting prompt. Reports
  stay local until the user chooses to share them.
- Keep one millimetre placement differences detectable on large bodies by capping the advisory bounds and placement allowance at 0.5 mm.
- Check assembly placements and compare bodies in the selected joint pose so intentional joint placement does not produce look-wrong findings.
- Flag toroidal faces without trim loops when they share a shell with other faces; allow whole tori in their own shells.
- Count empty leaf placements separately from placements with geometry in assembly
  summaries, desktop and web cards, JSON, and doctor. Empty leaves report Degraded
  losses with anonymous part numbers; assembly holders of children are not empty.
- Report a counted Degraded geometry loss when solid regions convert to open sheets.
- Stop counting a possible ten-second macOS version-probe timeout as normal conversion
  work, and retain fractional assembly estimates in seconds_exact.
- Explain Fusion material losses as “Materials and appearances were not carried over.”
- Preserve the largest part-worker count when combining reports from assembly members.
  Clarify that worker counts exclude threads inside SolidCraft checks.
- A failed file's card can show you what a report holds and ask your consent before sending it.
  It sends the anonymous summary when a report endpoint is configured, and says "sent" only after
  a matching receipt. If sending fails, the report stays on your machine.
- Linking a report to Discord is optional, and an update can tell you it fixes a report you sent.
- The MCP server adds `report_preview` and `report_submit` for reviewing and sending reports.
- Sent reports identify the source app from native formats instead of always reporting unknown.
- Cut, prove and shrink private reproduction bundles for model check failures, with local gzip storage and a manifest of retained records and sizes.
- Add diagnose --bundle and anonymous MCP extract_repro and verify_repro tools; offer summary or the private-send card when no proven bundle is available.
- Keep report requests below 25 MiB including multipart overhead, show bundle Send when available, and retain source-app metadata.
- Read CADIR 11 feature histories while continuing to refuse unknown newer versions.
- `dutchman repro` and the MCP `repro` tool make a local minimum-reproduction kit with
  anonymous check facts, private ordinal-to-name lookup, and rebuild instructions.
  Structured check failures are narrowed through independent archive members and CADIR
  subsets without sharing the original design.

### Docs

- The README leads with download, use, and what converts, and its formats list is a table.
  The long guide is split into pages: install, the desktop app, the command line, choosing
  what to keep, formats, feature history, and troubleshooting. Old guide links still land
  on a pointer to the new page.
- `dutchman formats --markdown` prints the formats list as a Markdown table.
- Explain macOS archive download, checksum verification, installation, and first run, and
  make quarantine removal quiet when no flag is present.
- Link the disk image’s first-opening instructions to installation and command-line help.
- Explain how to make a minimum reproduction when the original CAD file must stay private,
  with a step-by-step skill procedure, a paste-ready AI prompt and a generated synthetic
  example of 88 use-curve range findings.
- A manually dispatched workflow converts the CAD files of public, openly licensed repositories
  on GitHub's free runners and reports what converted, what failed, and which layer owns each
  failure. See [Releasing](docs/releasing.md#public-corpus-sweep).

## 0.2.5 (2026-10-30)

### Formats

- Convert FreeCAD TechDraw pages to DXF with `convert --to dxf`, one file per page,
  or select a page with `--page NAME`. Keep neutral view curves, visible and hidden
  line types, dimension labels and measurements, annotations, template frames, and
  resolved title-block text in millimetre page space. Report unsupported content
  and entity counts per page in JSON.
- Leave out Fusion bodies kept only as consumed tools or removed bodies, with their exclusive
  geometry and an Info note. Keep user-hidden component members available.

### History

- A base feature the source proves holds no bodies is reported as not needed instead of
  stopping the part's history.
- One-sided partial solid revolutions keep editable history when their selected profile
  bounds prove the sweep side about the source axis, including reversed FreeCAD revolutions.
  Unproven sides keep the original geometry and report the missing proof.
- Uniform periodic sketch B-splines keep their exact shape as editable closed control splines,
  including equal-weight rational curves. Unequal weights and unsupported interior knots name
  the weights or custom knots missing from SolidCraft's control splines.
- Profile regions with holes made of whole circles replay when unused layout lines cross them.
  The lines stay editable as construction geometry when every active consumer selects
  explicit whole-loop regions.
- A whole-region profile is written with its curves and holes in sorted order, which every supported
  SolidCraft release accepts; older releases refused a profile with ten or more holes as damaged.
- An Inventor split of a body by a work plane maps to a SolidCraft split, and later features
  that select either piece keep their history. A split by a surface is reported as not
  supported yet.

### Assemblies

- Large Fusion assemblies convert with less memory and in less time: each component is cut
  from the decoded design without first copying the whole design. A 33-placement assembly
  peaks at 10.3 GB instead of 12.8 GB and converts in under six minutes instead of ten.
- Fusion components keep the features and parameters cadmpeg identifies as theirs, even when
  features have no output-body links. Features owned by another component stay out, and the
  design's loose-body component keeps root-owned history when it exists.
- Fusion components keep the feature history that builds their bodies: the features that make
  them, their own sketches, and what those depend on. Each component's history is then mapped
  and checked like a single part's, so the report names each component's first blocker instead
  of saying no history was found.
- A joint that cannot hold its parts where the file places them, because other joints already
  place them, is written suppressed. SolidCraft solves the joints when it opens a design, so an
  active conflicting joint moved those parts away from the machine; now they stay where the
  design has them, and the joint is kept for the user to fix or turn back on.
- Keep child placements used to resolve the poses of inserted Fusion designs, and suppress
  conflicting joints so those components stay in place when SolidCraft opens the design.
- Keep body and face appearance bindings on component definitions, including imported and
  stored solids, so every placement inherits their colours and opacity. Write uniformly
  painted faces as a body colour, retain mixed face looks, and report bindings left out.
- Slice component models in parallel and reuse identical empty assembly holders. Prepare
  SolidCraft assembly parts in parallel, including per-part replay checks, while
  preserving design bytes and report order. Bound workers with `--threads N` on `convert`,
  `convert-all`, and `watch`, or MCP `threads`; `0` selects automatically and `1` runs serially.
  Part count and available memory cap workers; JSON reports record requested and used counts.
  Failed worker preparations retry serially. Available memory also respects Linux cgroup
  limits and is detected on Windows.
- Parallel part preparation keeps a quarter of free memory (at least 1 GiB) back for the rest
  of the conversion, so a computer with little spare memory runs fewer workers instead of
  running out.

### Desktop app

- Choose Fast, Standard, or Full presets and leave feature history, sketches, or joints out
  under Advanced. Remember choices, restore older geometry-only settings, and keep Convert
  visible while settings and results scroll.
- Show memory and time estimates beside queued files, check memory before converting, and
  offer a lighter preset or a one-run override when a design may not fit.
- Show history Info messages on result cards, including assembly history left out by Standard.
- Move Report a problem beside the theme switch in the header. Explain memory warnings in
  two sentences using only the file name, and keep the dialog buttons in one row.
- Show under 0.1 GB and under a minute for small estimates in the app and CLI, using the
  same formatter and rounding longer times up to whole minutes.
- Release builds check for stable updates at startup and hourly using GitHub's conditional
  releases feed. A header button offers release notes and the appropriate download or package
  manager command, with persistent skip and three-day reminder choices and update settings.
- Homebrew and Scoop updates wait for the new package version. Homebrew updates in the
  background with live output and restart controls; Scoop confirms a close, update, and reopen
  hand-off so Windows can replace the executable, and reports failed updates once on restart.
  Failures retain a copyable command. The Homebrew cask is `dutchman-cad/tap/dutchman-convert`.
- Explain installed SolidCraft versions in a desktop header pill and beside the CLI target line,
  with download guidance for minors before 0.18 and history guidance from writer gates.
  The pill opens target settings and release guidance, and adapts to narrow windows.
  JSON reports include the newest known SolidCraft release and installed-version status.

### Command line and install

- Add Fast, Standard, and Full conversion presets, with history, reference-sketch, and joint
  choices. Standard budgets assembly history against available memory; Full never drops it
  for memory. FreeCAD output honours the history choice only.
- Estimate conversion memory and time before decoding. Offer a lighter preset interactively,
  or refuse an over-budget run with exit 2; `--yes` disables prompts and `--no-memory-check`
  bypasses the guard. JSON reports include estimates and effective choices. MCP accepts the
  same choices and a `dry_run` pre-read that writes nothing.
- Elapsed time during a conversion also reads in hours and minutes past a minute, such as
  `7261 s (2 h 1 min)`, on the command line and in the desktop app.
- Memory estimates for Fusion designs are calibrated on more measured conversions, including a
  large single design with 1,500 parts and runs on Windows, so the estimate no longer falls
  short of a conversion's real peak.
- Default SolidCraft output uses `.sc` for targets 0.25.1 and newer, and `.solidcraft`
  for older targets. Explicit `.sc`, `.solidcraft`, and `.solvecraft` outputs are accepted.
  Batch conversions, the desktop app, MCP, and web downloads use the same naming rule.
- Cache SolidCraft CLI versions within each process and across runs, with a three-second
  probe limit. An explicit target with verification off uses only cached versions in reports.
- Target the installed SolidCraft release, even with verification off, or the newest release
  known at build time (currently 0.27.0). Dutchman is made for releases from the last two
  weeks, currently 0.18.2–0.27.0 (the window never starts below 0.18.2, the oldest accepted
  target). Features a target cannot open keep the original geometry with a named reason.
- Counterbores and countersinks write their explicit flat tip for targets 0.22.1 and newer,
  the first SolidCraft release that applies a drill-point angle to them.
- Read available memory without launching PowerShell on Windows or vm_stat on macOS.
  Diagnostic heartbeats now record process memory and CPU time on Windows and macOS,
  as they already do on Linux, and Windows diagnostic logs include total memory.
- Decide memory refusals before decoding, and let Standard convert parts and assemblies
  without history when only the history-free estimate fits, with one Info explanation.
- Show the SolidCraft target and discovered version in conversion JSON and text reports.
- Name default diagnostic logs in UTC without a timestamp subprocess, and remove Windows
  shell probes for diagnostic OS and total-memory metadata. Document null Windows and
  macOS heartbeat resources under the workspace's unsafe-code ban.
- On Windows, Dutchman finds `solidcraft-cli` in the folder the SolidCraft installer recorded in
  the registry, so a SolidCraft installed to a folder of your choice is found and its history
  check runs. The desktop app's hint when none is found names `SOLIDCRAFT_CLI` and asks for the
  full path of `solidcraft-cli`.

### Reports

- Save anonymous diagnostic JSONL logs during conversion, including stages, resource heartbeats,
  Fusion member sizes and decode attempts, and final loss counts. CLI, MCP, and desktop settings
  support fixed diagnostic folders with ten-log retention; the app reopens unfinished logs after
  a restart. Each record is flushed immediately and synced at least every five seconds.
- Use a fixed process-memory floor when estimating conversions, so keeping history no
  longer multiplies that overhead. Refit tiny Inventor and SolidWorks floors and FreeCAD's
  content cost from Windows and macOS measurements. Keep time estimates separate from
  memory's history multiplier.
- Tighten measured RAM bands and give multipart Fusion a separate time band. JSON adds
  memory_band and nullable time_band while retaining the legacy shared band; text estimates
  show separate coverage and identify uncalibrated time estimates.
- Estimate tiny conversions in fractional seconds, with format and host startup costs
  separate from content work and Windows verification startup. JSON adds `seconds_exact`
  while keeping integer `seconds`; unsupported timing bands remain null and text remains
  “roughly”. Memory estimates and calibrated multipart Fusion timing are unchanged.
- Multipart Fusion archive time estimates account for requested part workers, with a serial
  component and the converter's CPU and memory limits. Memory estimates are unchanged.
- An assembly's JSON report lists each part's history under `history.parts`: its name, whether
  its history was written, checked, partial or flattened, the first blocker, and how many of
  its features mapped.
- Name and compare SolidCraft by minor version in the desktop app and CLI. JSON
  `solidcraft.target` and `known_newest` visibly change from `x.y.z` to `x.y` strings;
  `found` keeps the installed full version. Patch-sensitive writer gates remain intact.
- Resolve and checksum-verify the latest stable patch of the newest SolidCraft minor
  during CI roundtrips. The oldest window release stays pinned.

### cadmpeg

- The cadmpeg pin moves from `2e01486` to `7f08434` on raykholo/cadmpeg, which carries changes
  ahead of their upstream merge. Fusion components own the bodies their body containers hold,
  and operation-table poses apply the same way to every occurrence of a definition, so
  assemblies are placed closer to the design. Inventor derived parts read their stored bodies,
  positional two-section lofts are read, and rectangular patterns keep their suppressed
  instances as skipped grid positions. SolidWorks parts with unusual edge topology are refused
  rather than read wrong. Fusion extrude, fillet, split and loft selections stored as native
  links are resolved, Fusion files saved offline open, and Fusion mesh bodies that cannot be
  decoded are reported as such. Fusion extrude profiles resolve to sketch regions in more
  designs, a Base Feature that holds a stored solid becomes a typed base body (written as a
  checked SolidCraft import), components whose occurrence frames are stored separately are
  placed correctly, Fusion Generic appearances keep their transparency, large archives are read
  one design at a time, and a design inside an archive that fails to read is read once, not
  again at every place it is used. Inventor schema-15 parts are read, and SolidWorks constant-
  radius rolling-ball blends and intersection edges are read more exactly.
- A rectangular pattern that skips grid positions is not mapped yet: SolidCraft's rectangular
  pattern fills the whole grid. The report names the pattern and how many positions it skips,
  and the part keeps its exact geometry.

### Docs

- Lead the README with desktop screenshots, downloads, and a three-step conversion walkthrough;
  keep compatibility and history details in the guide.
- The README's download table links the universal Mac command-line archive for Apple silicon
  and Intel, beside the universal desktop disk image, now that a release carries it.

## 0.2.4 (2026-10-29)

### History

- FreeCAD MultiTransforms containing mirrors keep editable history as ordered mirror and
  pattern features when replay matches the original, targeting SolidCraft 0.25.1 or newer.
  Older targets keep the original geometry with `needs SolidCraft 0.25.1 or newer`
  for chained stages; single-stage mirrors and patterns remain available.
  Full-turn circular patterns omit a
  repeated seed endpoint while retaining every distinct instance.
  An unfinished replay check keeps the original geometry for these patterns.
- Whole-sketch hole placements exclude construction geometry, so reference origin points
  do not create extra holes. Explicit entity selections retain construction locations.
- Constant-radius FreeCAD additive and subtractive helices keep their sketch profiles,
  axes, pitch/height/turn laws, handedness, and reversed travel as editable coils when
  targeting SolidCraft 0.25.1 or newer. Older targets preserve the original geometry and
  report the required version. Conical helices and spirals report the missing coil option.
- Choose the target release with `--solidcraft-version x.y.z` on `convert`, `convert-all`,
  or `watch`, or `solidcraft_version` in MCP. The default is the found verification CLI's
  version, else 0.18.2; with `--no-verify`, it defaults to 0.18.2. Targets below 0.18.2 are
  refused. Counterbore and countersink holes retain flat bottoms on newer targets too.
- Skip unused FreeCAD binders and datum helpers with an Info note. Carry used datum
  placements through resolved sketches, pattern axes, and datums, and explain unresolved uses.
  Keep helper Info notes when another feature or replay check flattens the history.
- Respect hidden FreeCAD containers when filtering bodies, including children marked shown.
- Preserve supported spreadsheet formulas and qualified parameter references as live SolidCraft
  expressions. Omit text cells silently, identify unused numeric cells, and report specific
  reasons when expressions must retain their evaluated values.
- FreeCAD sketches with negative horizontal or vertical distances replay with positive
  dimension magnitudes, preserving their solved positions and signed parameter expressions.
- Keep editable features before the first blocking feature, plus resolved sketches, hidden
  beside the visible imported final shape. Reports name the blocker, count retained features,
  and distinguish partial replay from verification of the full history.
- Retain a replayable timeline for correction when its final shape differs from the source.
- Retry sketches that fail replay without constraints, retaining their solved geometry and
  reporting the lost constraints.
- Keep resolved reference sketches in parts and assemblies even when no body-building
  feature or source timeline can be retained.
- Convert stored geometry as an imported base body, keeping later mappable features.
  Report imported solids as Info, without a flattened-history warning when no recipe exists.

### Assemblies

- Convert FreeCAD assemblies that link Bodies and Parts in other `.FCStd` documents, finding
  siblings first and reporting missing parts by name. Keep grounding and supported joints.
- Place FreeCAD link-array elements once, without duplicate LinkElement occurrences.
- Fusion components and bodies the design hides open hidden in SolidCraft: a hidden occurrence,
  or one whose part holds only hidden bodies, is placed hidden instead of shown, and can be
  shown again there. A part with shown and hidden bodies is still shown whole.
- Group each top-level design's features into a named timeline folder when an assembly
  contains multiple designs, keeping feature order and leaving joints outside folders.

### Desktop app

- **Report this on GitHub** opens for signed-in GitHub users again. A long filled-in link made
  GitHub answer "406 Not Acceptable"; the link now stays under 4,000 characters, a report that
  does not fit is copied for you to paste, and a very long error message is shortened in the
  form (the anonymous report keeps all of it).

### Reports

- A design whose faces SolidCraft merges (faces that share one surface) is no longer reported
  as missing faces. The history check compares area and volume, so an open-shell warning
  there is no fault when they agree; unmeshable faces, bodies inside out, solids without
  volume, and solids that geometry-only output opens as open shells are still reported.
- Reads and conversions show their stage, designs read when available, and elapsed time, with a
  still-working heartbeat every five seconds. Desktop cards show a reading progress bar
  when the design total is known. CLI progress goes to stderr, as JSON lines with `--json`,
  and is hidden by `--quiet`; stdout keeps its final report. Fusion `.f3z` member counts
  are not available yet.

### cadmpeg

- The cadmpeg pin moves from `e06fa52` to `2e01486`. Fusion designs whose sweeps or rotational
  supports failed cadmpeg's model checks convert now. Inventor holes are drilled the way the
  hole points (they used to replay outside the part and fall back to exact geometry), browser
  work planes are read as datum planes, and a split of a solid is read and reported as not
  mapped yet. SolidWorks parts keep their stored face senses.

### Docs

- The container image (`ghcr.io/dutchman-cad/dutchman:X.Y.Z` and `:latest`) is published when a
  release is published, from that release's own archives, instead of when its tag is pushed,
  so a draft release never reaches the registry.
- Build macOS release and nightly targets in parallel, reuse their checked archives for the
  universal disk image, and share per-target Rust caches between nightly and release builds.
- Mac command-line downloads include one universal archive for Apple silicon and Intel.
  Release notes link it alongside the universal desktop disk image; per-chip archives
  remain available for Homebrew.

## 0.2.3 (2026-10-28)

### Formats

- FreeCAD parts no longer lose faces bounded by trimmed elliptical (or other trimmed) edges:
  leaving out hidden intermediate shapes kept the edge's curve but dropped the construction
  and full curve it is cut from, so the edge, its loop and its face were left out of the
  geometry.

### History

- FreeCAD fillets and chamfers can use edges of their base feature's stored shape, with
  each edge proven on SolidCraft's replay before the blend. Hidden intermediate shapes
  remain left out of the converted geometry.

### Assemblies

- A Fusion joint from a part inside one copy of a component placed more than once is left out
  when another joint already holds that copy itself to the same occurrence, as a rigid group
  holding both stores. It held the copy a second time, with another frame, so one of the two
  joints could not hold.
- A Fusion joint between a component and a part inside it, as each rigid group stores one for
  every member, moves the inner part and keeps the component where the design places it.
  Inserted designs anchoring their own rigid groups were moved to meet their parts, losing
  their placement in the assembly.
- A Fusion joint between a copy of a component placed more than once and a part inside that
  copy is the component's own joint, written through its first copy like the part. It joined
  the copy to the first copy's part, so every later copy moved onto the first.
- Joints that reach nothing placed, such as a design placed where it is stored that holds its
  parts in a rigid group, are placed from the outermost occurrence they join, which stays
  where the file puts it. They started from whichever joint came first, so a part-to-part
  joint could fix both parts where they were stored and leave the design's own joints
  unmet.
- A joint that moves a component leaves the parts inside it that other joints already placed
  where those joints put them. In a Fusion rigid group listing a design's parts before the
  component holding them, the later joint moved the parts away from the earlier ones.
- Fusion components the file places at the pose it shows them in stay there: their joints
  are checked where they are instead of moving them back to where they were inserted. A
  printer assembly's linear carriages, idler washers, and electronics now open where the
  design shows them.

### cadmpeg

- The cadmpeg pin moves from `fb1cd0a` to `e06fa52`. SolidWorks parts that 0.1.5 read and 0.2.0
  to 0.2.2 read empty, with fewer bodies, or failing cadmpeg's edge checks read again: bytes
  inside other records no longer hide or refuse a part's bodies, arcs stored past the seam keep
  their range, and solid snapshots stored with features are no longer read as sketches. Fusion
  reads rigid groups and their joints in full, with members mated aligned, and binds joint
  operands in other designs, in nested components and in merged member designs to the exact
  occurrence; places designs inserted at the top of an assembly at their current pose, not
  where they were first inserted; reads construction planes from their solved placement, typed
  local component occurrences, qualified and legacy external placement frames, edge flanges
  that meet at a corner, localized hole scopes, and through-all hole extents with drill tips;
  shares one definition for each merged external design; and reads fillet face groups, sketches
  placed on entities, parameters with detached owners, and pipe scopes. Inventor projects
  attached plates with their bends, sheet-metal corners that change nothing, mirrors about a
  work plane, single moves and rotations of bodies (which Dutchman reports as not mapped yet),
  and more revolves. NX reads change streams against their embedded schema. FreeCAD applies
  part and link placements once and counts a shell as closed when each edge is used as often
  each way. STEP output recovers edge curves a model leaves out, keeps the faces it can bound
  as surfaces when a body cannot be written whole, writes solids as advanced B-rep
  representations, and leaves a collapsed edge without a curve out of its loop. Fusion also
  places local occurrences at the root of a design, and external occurrences, at the pose the
  design shows them in, from the records that store that pose, instead of where they were
  inserted; when those records disagree, the ones that list the most owners decide. Inventor
  rectangular feature patterns with fixed spacing, in one or two directions, convert as
  SolidCraft rectangular patterns; Combine is read as a Boolean of bodies, which Dutchman
  reports as not mapped yet. SolidWorks parts that fall back to their display meshes keep them,
  and report that the meshes carry no exact geometry.

### Known issue

- A very large Fusion assembly can need more than 24 GB of memory to convert
  ([#227](https://github.com/dutchman-cad/dutchman/issues/227)); on a machine with less, the
  system stops the conversion before it writes anything.

## 0.2.2 (2026-10-26)

### Known issue

- Some SolidWorks parts that 0.1.5 read still decode empty, fail cadmpeg's edge checks, or come
  through with fewer bodies (reported as losses, not as a clean conversion); a fix is in
  progress in cadmpeg. If 0.1.5 worked better for a part, keep using it for that part for now.

### History

- Merged designs that reuse a parameter name keep their own references: an expression follows
  the parameters it depends on to their new names instead of naming another design's
  parameter of the same name. Such renames are reported as information, and only names
  SolidCraft cannot use are reported as changed.
- The SolidCraft replay check allows for the error in SolidCraft's measuring mesh, which grows
  with each body's area and size, and also compares body areas. Curved parts such as lofts and
  revolves that rebuild exactly are now verified instead of flattened.

### Assemblies

- A design's own component that holds no bodies and no other components is placed as an empty
  component, when every body of the design belongs to a component, instead of the whole design
  converting as stored geometry.
- A joint between two parts inside one copy of a component placed more than once is converted
  once, as that component's own joint, instead of being left out. A joint from such a part to
  something outside its copy holds the whole copy, with a note.
- A joint to a part inside a design inserted into a component, which Fusion names by the
  part's key alone, joins that part instead of the component that holds it.
- A component that is empty in the source is written as an empty component, so the joints to
  it are kept.

### Desktop app

- On macOS the app is shown as **Dutchman** in Launchpad, the Dock, Finder and the menu bar, so
  Launchpad no longer cuts its name short. Its file stays `Dutchman Convert.app`.

### Command line and install

- On macOS, `brew install --cask dutchman-cad/tap/dutchman-convert` installs the Dutchman
  Convert app in Applications.
- Homebrew installs pour a ready-made bottle on macOS and Linux, so they no longer need Xcode
  or current Command Line Tools.
- Dutchman Web can link converted designs directly to the SolidCraft web app with
  `--solidcraft` (`DUTCHMAN_WEB_SOLIDCRAFT`), using a same-origin path. Cards show
  **Open in SolidCraft**, and API outputs include `open`; FreeCAD macros keep only their download links.
- `dutchman-web --output-dir <dir>` also saves each converted design in a folder, such as a
  mounted network share, keeping its uploaded folders and name and numbering it rather than
  replace a file already there. Each result says where its design was saved.
- `dutchman-web --base-path /dutchman` serves every page and API route under that path, so it
  can share one address with other services behind a reverse proxy.

### Docs

- Add a Compose example for SolidCraft, Dutchman Web, and a watched NAS inbox on one origin.
- The release and nightly workflows download only Dutchman's own archives, so the container
  image's build record no longer stops the draft release.

## 0.2.1 (2026-10-25)

### Known issue

- Some SolidWorks parts that 0.1.5 read still decode empty, fail cadmpeg's edge checks, or come
  through with fewer bodies (reported as losses, not as a clean conversion); a fix is in
  progress in cadmpeg. If 0.1.5 worked better for a part, keep using it for that part for now.

### Formats

- Show cadmpeg's published headline level beside reader status in the CLI, MCP formats
  report, desktop format list, README and guide.
- A STEP file of one part goes into the SolidCraft design as it is, instead of being read by
  cadmpeg and written again as STEP: nothing is lost on the way, and the report no longer lists
  the writer's STEP losses. A STEP assembly of several parts still converts part by part into
  components.

### History

- A fillet or chamfer on edges a sheet-metal base flange generated maps the same way as one on
  an extrusion's generated edges: a sketch point's side edge runs the flange's thickness on the
  side it is drawn, and each edge is proven on SolidCraft's rebuilt body before the blend is
  written.
- A fillet or chamfer left unmapped for its edges says which case it is: a selection cadmpeg
  does not resolve to edges yet, edges only partly resolved in its input topology, or edges of
  the final body.
- A fillet or chamfer on edges an extrusion generated maps when cadmpeg names them by the
  sketch entity that made them, as it does for Inventor: a sketch point's side edge, or a sketch
  line's edge where the extrusion starts or ends. Each edge is proven on SolidCraft's rebuilt
  body before the blend is written.
- A counterbore, countersink or chamfered hole with a drill point at its bottom says why it is
  not mapped: SolidCraft holes take a drill point only on a plain drilled hole, and ignore it
  on one with an entry recess, which would leave the cone out of the geometry.
- An extrude, revolve or other sketch feature whose profile cadmpeg gives as whole sketch
  regions no longer makes a design SolidCraft refuses as damaged ("invalid analytic offset
  topology or boundary reference"). The one loop of a one-loop sketch is written as every
  profile. Otherwise each boundary is written in the order SolidCraft finds it:
  counter-clockwise, from its leftmost point. This affected Inventor parts read with cadmpeg's
  sketch-region profiles.

### Command line and install

- Each release is also published as a container image, `ghcr.io/dutchman-cad/dutchman`, for
  amd64 and arm64, tagged with its version and `latest` (nightly builds as `nightly`). It holds
  the same binaries as the Linux downloads; [docs/docker.md](docs/docker.md) covers pulling,
  converting a mounted folder, the MCP server, and updating. Linux downloads are built on
  Debian bookworm, so they run on systems with glibc 2.36 or newer.
- Run Dutchman in Docker: the `Dockerfile` builds an image with `dutchman` and `dutchman-mcp`
  for x86_64 and ARM64 Linux, optionally with SolidCraft to check histories
  ([docs/docker.md](docs/docker.md)).
- Convert from a browser with `dutchman-web`, a web server for a self-hosted Dutchman: drop
  design files or a project zip, read each file's report, and download the designs. Uploads are
  converted as `convert-all` converts them and deleted after a while; a password and upload
  limits are settings. The Docker image includes it, and `compose.yaml` runs it on port 8080
  ([docs/web.md](docs/web.md)).
- `dutchman watch <inbox> --out-dir <dir>` converts files dropped into a folder, for a
  self-hosted Dutchman on a shared or network folder. It checks the folder on an interval,
  waits until a file stops changing, converts it into the output folder with its report beside
  it, and never converts the same content twice (also after a restart). A file that fails
  gets a `.failed.json` report and is tried again only once it changes. The inbox is never
  written to. `--once` converts what is ready and exits, for cron and scripts.
- Dutchman Web shares zip limits across each upload, limits uploads being received, and
  refuses new jobs when saved data or available disk space reaches its configured limit.
  Data folders are checked for private access on Unix, and browser uploads check their origin.
  The web guide explains HTTPS setup for HTTP basic authentication.
- Linux release and nightly archives include `dutchman-web` alongside the command, MCP server
  and desktop app.

### Reports

- The geometry-only retry leaves out parameters as well as history and sketches before it checks
  the model, so a design whose parameters repeat a name (as merged Fusion multi-document
  archives can) converts as geometry; the report says parameters were left out.

### cadmpeg

- The cadmpeg pin moves from `66550bb` to `fb1cd0a`. Fusion keeps nested local component
  placements, more resolved face selections, and compact coil handedness; reads designs whose
  exact edge curves end slightly away from their vertices (as in some large multi-part
  assemblies) instead of refusing them; gives each design merged into a multi-document
  archive its own parameter scope; and assigns sketches to their components. Inventor reads
  more extrusion and hole extents, full revolves about a work axis, center-point holes,
  spotfaces and hole end targets, oriented chamfers, circular sweeps, unguided two-section
  lofts, closed sketch splines, stored-body base features, and bend-free sheet-metal plates as
  base flanges, and resolves more extrusion profiles and fillet and chamfer edge selections.
  SolidWorks corrects compact-curve edge orientation. Creo preserves plane axes and boundary
  orientation, avoids ambiguous operation states, reads circle radii from type-4 dimensions,
  and binds two-cap circular sweeps. FreeCAD reads point symmetry and improves mesh placement,
  spline domains, joint angles, Body visibility, and document restoration. STEP keeps
  unclaimed bodies, corrects replica transformations, and writes reflected conic pcurves as
  exact rational splines.

### Docs

- Each pull request adds a changelog fragment; release preparation assembles the fragments
  into one ordered entry, and CI checks fragment headings and repeated release headings.
- Tagged builds now create a draft release with short highlights, downloads, and a link to the
  full changelog.

## 0.2.0 (2026-10-25)

### Known issue

- Some SolidWorks parts that 0.1.5 read now decode empty, fail cadmpeg's edge checks, or come
  through with fewer bodies (reported as losses, not as a clean conversion); a fix is in
  progress in cadmpeg. If 0.1.5 worked better for a part, keep using it for that part for now.

### Formats

- Convert CATIA `.CATPart`, Creo View `.pvz`, Rhino `.3dm`, ACIS `.sat`/`.sab`/`.smt`/`.smb`,
  and STEP `.step`/`.stp` through the geometry path. Creo and NX parts (`.prt`, including
  numbered Creo saves) are recognised by contents and report when geometry reading is not
  supported yet. SolidWorks assemblies remain recognised without a reader.
- IGES `.igs`/`.iges` files are recognised and refused before decoding, with directions to open
  them in SolidCraft directly (File > Import). SolidCraft's own importer sews their faces into
  solids; cadmpeg does not yet. IGES refusals do not invite test files.
- The CLI, JSON reports, and desktop app show whether each format reads geometry, opens without
  geometry, is recognised without a reader, or should be opened in SolidCraft directly.
  `dutchman formats` lists that status alongside what Dutchman converts. After a refused
  conversion (except IGES) or a history that did not fully map, one invitation links to
  test-file recipes and private help with wrong conversions. `--quiet` hides it; JSON output
  contains no invitation.

### History

- Constant-radius fillets and equal-setback chamfers map from proven feature input edges on any
  prior history: their input edges are matched on the body SolidCraft rebuilds from the history
  before them (one replay per design), and are refused, saying why, without `solidcraft-cli`.
- A Fusion constant-radius fillet with a tangency weight of exactly 1 maps as a circular
  fillet, its blend being circular in section, with an Info note ("tangency weight 1.0 treated
  as a circular fillet"). Other weights are still refused.
- Sheet-metal edge flanges map supported dimensions on unchanged rectangular base sheets using
  proven input-edge midpoint picks. Other edge flanges say why they are not mapped when their
  edge is known only after bending. Hems say why their developed length or rolled, teardrop, or
  unresolved form has no SolidCraft equivalent.
- Sweeps accept single-curve whole-sketch paths and guide rails, and report specific reasons
  for unsupported corner transitions, section laws, tangent extension, and simplification.
- Lofts map in more forms: surface lofts become SolidCraft surface lofts, and ruled lofts
  through two sections map. Loft rails share the sweep path reader, so a single-curve
  whole-sketch rail maps and each unsupported rail kind gives its own reason.
- Hole history keeps chamfered entries and ISO metric thread labels on tapped holes (thread
  details SolidCraft has no field for are reported, not blocking), and names the unsupported
  hole forms, placements, and ends it refuses.
- Revolves keep more of their history: unequal sides that add up to a full turn, surface
  revolves short of a full turn, and an axis drawn in the profile's sketch (kept as that line).
  Ends at a face, vertex, or body and other face makers stop the history with a reason that
  names them.
- Patterns map named bodies, planar sketch paths, regular explicit offsets and angles, and two
  linear composite stages; unsupported pattern options receive specific refusals.
- Sketch history carries bounded conics, rational quadratic Béziers, equal-weight splines, and
  more geometric relations and dimensions; unsupported constraints retain counted, specific
  loss messages.
- Body mirror patterns and body booleans (join, cut, or intersect, optionally keeping the
  tools) map to SolidCraft. Shells and drafts, shape-replacing mirrors, and threads without a
  resolved standard and pitch say why they are not mapped.
- Flattened designs keep mapped sketches with resolved planes, hidden for reference after the
  imported geometry, and report how many were kept or left out.

### Assemblies

- A joint Dutchman does not convert is reported by the output that leaves it out. A FreeCAD
  macro (`--to freecad`, experimental) builds cylindrical, ball, distance, and other joints
  SolidCraft output leaves out, and no longer reports them as dropped.
- An assembly keeps the sketches of its own file, hidden for reference in the design itself,
  with the parameters their dimensions use; they were left out before without a note.
- A joint whose connector frames point the same way (its CADIR operands' `frame_sense:
  "aligned"`) is written as a flipped SolidCraft joint, and a part a joint places meets its
  other part z to z rather than face to face. A Fusion robot arm's gripper mounted on its wrist
  this way had been turned over into the wrist.
- When the source grounds none of the parts a set of joints ties together, Dutchman grounds the
  fixed side of that set's first joint, so SolidCraft's solver holds the set where the source
  places it, and the report names the occurrence as inferred (`Info`, `joints`).
- A Fusion design's root component is grounded where it is: the document's own, and the root of
  each design inserted into it, held by the occurrence that inserts it. Both were left free to
  move.
- Experimental FreeCAD macros rebuild assembly joints in the Assembly workbench (the fixed,
  revolute, and slider joints SolidCraft gets, through sub-assemblies, plus cylindrical, ball,
  distance, parallel, perpendicular, and angle joints between directly placed parts), with
  connector frames, limits, and grounding; if solving fails, the parts keep their stored
  placements.

### Desktop app

- When macOS blocks SolidCraft's first run, the CLI and desktop app explain how to allow it in
  Finder or System Settings and try the check again, without changing quarantine.
- The desktop app is named **Dutchman Convert**, part of Dutchman CAD: its window title and
  heading, and on macOS **Dutchman Convert.app** with a "Dutchman Convert" disk image volume.
  The commands (`dutchman`, `dutchman-gui`, `dutchman-mcp`), their subcommands, the settings
  and the download file names are unchanged. A Mac with 0.1.5 or earlier keeps
  `/Applications/Dutchman.app` until it is replaced.
- The macOS disk image opens on an installation window: Dutchman and Applications side by side
  with an arrow between them on Dutchman's own background, and the read-me below. If the window
  cannot be laid out while packaging, the image is built plain as before.
- A file that fails says what to do next: **Report this on GitHub** opens the **A file didn't
  convert** form filled in with Dutchman's version, the system, the error with its stage and
  upstream, and the file's anonymous report (copied to paste when it is too long for the link).
  The form had come back with its report and stage boxes empty.
- The line naming the SolidCraft that checks history stays on one line (the whole of it on
  hover): a long path to `solidcraft-cli` wrapped onto several lines and pushed the Convert
  button out of view at the window's default size.
- The desktop app prints `--version` and `--help` without opening a window, shows its version
  in the title, and has a **Report a problem** link to the **A file didn't convert** form. An
  unreadable-file error links to that form too.

### Command line and install

- Install with Homebrew (`brew install dutchman-cad/tap/dutchman`) or Scoop (`scoop install
  dutchman-cad/dutchman` after `scoop bucket add dutchman-cad
  https://github.com/dutchman-cad/scoop-bucket`); both follow each release.
- Convert with `dutchman part.f3d`, optionally followed by an output path. `convert` also
  accepts no output: it derives the path beside the input from `--to`. Existing derived outputs
  are kept (exit 2) unless `--force`; explicit outputs still replace by default. `--force`
  conflicts with `--no-clobber`, and a default cannot replace the input.
- Run installed Dutchman CAD tools with `dutchman <tool>`: executables named `dutchman-<tool>`
  next to the command take precedence over those on `PATH`. Help lists installed tools and
  versions. An installed Pilot takes precedence over the built-in Pilot and accepts its own
  flags. The std-only `dutchman-launcher` crate supplies a tiny `dutchman` front command for
  Pilot-only installations.
- `dutchman pilot` runs [Pilot](https://github.com/dutchman-cad/pilot) 0.1.0-beta.1: `dutchman
  pilot --help` lists its commands, and `dutchman pilot how "round all edges 3 mm"` shows
  example calls.
- Directory conversion keeps the newest numbered `.prt.N` save per part and reports older
  revisions as skipped. Numbered saves use the part's base output name and keep existing
  outputs.

### FreeCAD output (experimental)

- `dutchman convert --to freecad` (and `convert-all`, the MCP tools, and the desktop app's
  output choice) writes a FreeCAD rebuild macro (`.FCMacro`) instead of a SolidCraft design: it
  rebuilds the mapped history through FreeCAD's public scripting API and keeps the original
  geometry as embedded STEP, which it restores when the rebuild fails or differs. By default,
  Dutchman does not run FreeCAD, so these histories are unverified. Geometry whose STEP uses
  replicated trimming curves (`CURVE_REPLICA`), which crash FreeCAD 1.1's reader, is refused.
- Experimental FreeCAD macros keep a mapped history prefix and hidden reference sketches when a
  later feature cannot be rebuilt, retaining the final STEP shape.
- Write experimental native FreeCAD documents with `--to fcstd`, or check macros with `--to
  freecad --verify`, using bounded headless execution and per-part volume and bounding-box
  verification.

### cadmpeg

- The cadmpeg pin moves to `66550bb`. Fusion keeps parameters whose frames cannot be read as
  native-only instead of failing, reads more parameter expressions and every component context
  binding, places components bound through unregistered links, and reports ambiguous face
  appearances instead of failing. As-built joints carry slider limits, their limit references,
  and rest positions; rigid as-built joints and rigid groups of any size read, with each group
  member held to the first. Fillets with unit-bearing tangency weights stay native, extrude
  profiles resolve from circle-bounded and cap faces, rectangular face patterns keep both
  directions, feature patterns name the feature they repeat, and base flanges read thickness
  from their sheet-metal rule. Failed manifests say where framing stopped. Inventor 2019 and
  later parts carry history, newer assemblies place parts, and more sketches close through
  tangent-relation hints on either operand, other solver hints, and equal-length relations;
  thread annotations become cosmetic threads, and Thicken joins through selection lists.
  SolidWorks keeps native CAD state, as do Fusion, Creo, and NX. Creo 11–13 order tables read,
  and Creo View geometry becomes available. FreeCAD compounds of closed solids read as solids,
  fillets and chamfers keep edge ends on vertices, and document restoration improves. STEP/ACIS
  gains face boundary curves on their edges' ranges, more reversed ACIS curve families, fixes
  to ASM intcurve pcurves, and evaluable procedural faces (including a coil's swept face)
  written as sampled B-spline surfaces. Across formats, feature input edges carry their
  historical geometry, edges bounding selected faces keep their geometry, and joints carry
  connector frame sense on their operands.
- CADIR versions 8, 9, and 10 are read.

### Docs

- The README and guide link the Dutchman CAD Discord for questions and help.
- The issue forms' privacy box says that links to files that are already public are welcome; it
  asked for no file names at all, which kept a reporter from linking the public file that
  failed.
- A second issue form, **A file didn't convert**, reports one file: its format, the versions,
  the operating system, what was expected and what happened, and that file's anonymous report.
  Both forms list FreeCAD documents (`.FCStd`), and the conversion skill does too.

## 0.1.5 (2026-10-22)

### Inventor history, and FreeCAD in stable builds

- Dutchman builds against cadmpeg at one tested commit of its `main` branch (60b6065), the
  same for every cadmpeg crate, instead of waiting for cadmpeg releases. That brings Inventor
  extrude profiles resolved to sketch profiles and regions (trimmed curves included), with
  arcs, polygons and more sketch constraints, so Inventor part histories replay; FreeCAD
  bodies read as FreeCAD shows them; Fusion component names and joint motion; FreeCAD
  spreadsheet values; and fixes to how ASM pcurves, torus pcurves and tolerant edges are
  read. The nightly build still takes cadmpeg's latest `main`. Building from source works as
  before, with `cargo install --git`. Reports name the cadmpeg version with its commit
  (`0.6.3+60b6065`).
- FreeCAD documents (`.FCStd`) convert in stable builds: the `freecad` feature of `dutchman`
  and `dutchman-gui` is on by default. A build without it (`--no-default-features`) says it
  leaves FreeCAD files out and where the release builds are.

### Feature history

- Holes: counterbores and countersinks at a sketch's points, through all or to a
  flat-bottomed depth; holes at a directed position without a sketch, on an axis through the
  cap of the extrusion before them, at selected sketch points and centres, with a drill-point
  form, and ending at that earlier cap. Tapered holes, and counterbores or countersinks with a
  drill point, are not mapped yet.
- Extrudes that start or end at a planar face of an earlier body, one-sided extrudes with a
  taper, and start offsets against the sketch normal; two-sided revolves with equal angles.
- Sweeps carry an end scale and a fixed-binormal orientation, and sketch profiles can sweep
  into surfaces; taper, corner transitions and changing sections are not mapped yet.
- Lofts end at a point or vertex, and several sections may be profiles of one sketch.
- Coils map to SolidCraft coils: along any axis, up or down it, of round or square section,
  inside, centred on or outside their path, turning either way. The report says why a tapered
  coil, a flat spiral or a coil of triangular section is not mapped: SolidCraft's coil has no
  such option. When cadmpeg's STEP writer leaves out a helix surface (the swept face of a
  coil or thread), the report says so in plain words (`Degraded`, `geometry`).
- Circular feature patterns over a partial turn map, with copies at both ends of the span.
- Selected sketch profiles that cross themselves map through the regions they split into;
  a native profile pick resolves when even-odd nesting gives one region with its holes.
- A solid line drawn across a sketch profile from one point of its boundary to another, such
  as the diagonals of an Inventor centre-point rectangle, is written as construction when the
  sketch's profile table takes that profile whole, and the report counts them (`Info`).
- More holes map: at a directed position without a sketch, on an axis through the cap of the
  extrusion before them, at selected sketch points and centres, with a drill-point form, and
  ending at that earlier cap. A tapered hole is not mapped yet.

- The report says why a tapered coil, a flat spiral or a coil of triangular section is not
  mapped: SolidCraft's coil has no such option.

- Extrudes that start or end at a planar face of an earlier body, one-sided extrudes with a
  taper, start offsets against the sketch normal, and two-sided revolves with equal angles
  map to SolidCraft.

- Sweeps now carry end scale and fixed-binormal orientation, and sketch profiles can sweep into
  surfaces; taper, corner transitions and changing sections remain unmapped.

- Selected self-crossed straight sketch profiles map through their split regions; native sketch selections resolve when even-odd nesting identifies one region with its holes.

- Lofts map resolved point and vertex ends and retain separate profile selections for sections
  that share a sketch.

- Coils map to SolidCraft coils: along any axis, up or down it, of round or square section,
  inside, centred on or outside their path, turning either way, by turns and pitch, turns and
  height, or height and pitch. Tapered coils, flat spirals and triangular sections are not
  mapped yet.

- Circular feature patterns over a partial turn now map with copies at both ends of the span.
- Counterbore and countersink holes at a sketch's points map to SolidCraft holes, through all
  or to a flat-bottomed depth. One with a drill point at its bottom is not mapped yet.
- Coils map to SolidCraft coils: up the Z axis, of round or square section, inside, centred on
  or outside their path, turning either way, by turns and pitch, turns and height, or height and
  pitch. Tapered coils, flat spirals and triangular sections are not mapped yet.
- When cadmpeg's STEP writer leaves out a helix surface (the swept face of a coil or thread),
  the report says so in plain words (`Degraded`, `geometry`): the solid it bounds opens without
  it, with no volume.
- A solid line drawn across a sketch profile from one point of its boundary to another, such
  as the diagonals of an Inventor centre-point rectangle, is written as construction when the
  sketch's profile table takes that profile whole: solid, it would split the profile, and
  SolidCraft found no profile to match. cadmpeg does not yet say which Inventor sketch curves
  are construction. The report says how many lines were written so (`Info`).

### FreeCAD

- A grounded joint in a FreeCAD assembly becomes a rigid joint to the design, holding its part
  where it is stored, so the assembly's other joints keep their motion.
- Leaving out a FreeCAD document's hidden bodies no longer takes their sketches with them.

### Assemblies

- An Inventor assembly that places no parts, as Inventor's own assembly templates do,
  converts as an empty design with a note ("the assembly places no parts"), verification
  `not_applicable`, instead of failing with "none of the assembly's parts could be found or
  read".

### Parameters and reports

- A design holding parameters and nothing else converts as a SolidCraft design of those
  parameters (verification `not_applicable`). A design with no bodies and no parameters fails
  up front, saying it has no geometry.
- A design none of whose features cadmpeg decodes has no history to write: it converts as
  geometry with verification `not_applicable`, not as a history flattened.

### Getting started

- A file that cannot be read as its format (cut short, damaged or misnamed) says how to
  recover: copy it again or check its extension, and report it anonymously if the original
  opens. The CLI and the MCP tools give the same advice.
- The desktop app says what to check when its window cannot start (a desktop session and
  working graphics drivers), and exits with that advice when graphics start stalls for 30
  seconds instead of waiting.
- The guide and CONTRIBUTING start problem reports from an anonymous report, explain the
  sample cube's expected warnings, and the MCP tool descriptions say exactly what
  `convert_all` writes and that a successful `inspect` does not mean a file converts.

### Parameters

- A design holding parameters and nothing else (no features, no geometry), as a Fusion
  design other designs take their parameters from, converts as a SolidCraft design of those
  parameters, with verification `not_applicable` ("the design holds 11 parameters and no
  geometry") and an `Info` line, instead of failing for want of geometry. A design with no
  bodies at all and no parameters now fails up front, saying it has no geometry, rather
  than writing an empty import SolidCraft cannot open.

### Reports

- A design none of whose features cadmpeg decodes, such as an Inventor part made of a base
  solid, has no history to write: it converts as geometry with verification `not_applicable`
  ("no feature history cadmpeg decodes: its one feature is kept in its native form") and an
  `Info` line, not as a history flattened. A native feature beside ones that map still
  flattens the history, as before.

  recover, in the CLI and the MCP tools alike.
- The desktop app says what to check when its window cannot start, and exits with that advice
  when graphics start stalls for 30 seconds.
- The guide and CONTRIBUTING start problem reports from an anonymous report and explain the
  sample cube's expected warnings; the MCP tool descriptions say exactly what `convert_all`
  writes and that a successful `inspect` does not mean a file converts.

## 0.1.4 (2026-10-22)

### Privacy

- **The anonymous report (`--report-anon`) of 0.1.3 can hold names: re-run with 0.1.4 before
  sharing one.** Two causes, both fixed. Codes of Dutchman's own losses were made from the
  loss's subject, which is a file name as often as a kind (`dutchman/<letters of a file
  name>`); a code is now `dutchman/` and one of a fixed set of words, else `dutchman/other`,
  and a cadmpeg code that is not a plain code reads `cadmpeg/other`. And paths relative to an
  Inventor project or the folders its project file names, and part names the report was never
  told of, were left in messages. Each conversion now records every name it meets (the paths
  and folders it reads from, and every name the design gives anything; `names` in the convert
  report), and the anonymous report takes each one out, with any word holding a path
  separator. A `--journal` written before this records no names, so its files are converted
  again rather than reported from it, whatever version wrote it.
- The anonymous report of a large batch is made in one pass over each message: one of about
  7,000 files had not finished after an hour and a half; 8,000 files now take seconds.

### FreeCAD

- Bodies a FreeCAD document keeps hidden, such as the shape of each step inside a PartDesign
  Body, are left out with the geometry only they use, and the report says how many (`Info`);
  history and structure are kept. It takes effect once cadmpeg marks those bodies hidden.

### Checks

- Sheet bodies (surfaces, open by design) are no longer reported as bodies inside out or
  open in SolidCraft's check: SolidCraft gives an open sheet a volume, of either sign, and
  warns that its shell is not closed. As many such findings as the design has sheets are put
  down to them, with an `Info` line saying how many surface bodies were not checked; solids
  inside out or open beyond that are still `Degraded`.

- A `solidcraft-cli` older than SolidCraft 0.18.2, the oldest that opens and checks Dutchman's
  designs, is not used: the history is written unverified, saying so and naming the version
  found, and the desktop app says it too. CI opens converted designs in SolidCraft 0.18.2 and
  0.19.0, and the nightly checks with 0.19.0. The guide says which versions are supported.

### Desktop app

- The macOS app icon is a white rounded square with the mark, as macOS icons are, and the Dock
  shows it while the app runs too: it showed the bare mark there, dark on the Dock's dark
  glass. Elsewhere the window icon stays the mark.

### Reports

- A pattern cadmpeg does not decode says so ("cadmpeg does not decode what it repeats or how
  yet", "… its layout yet", "patterns of faces are not mapped yet: cadmpeg does not resolve
  which") instead of "it patterns nothing", which now means a pattern that really repeats
  nothing.

- A design SolidCraft opens with faces missing says so plainly: "SolidCraft opens 52 of the 53
  faces converted: 1 face is missing there", in place of "SolidCraft shows 52 of its 53
  faces". The guide says what to do about it.

- An assembly says what it brought across: the JSON reports give the joints written by kind
  (`joints`), and the desktop app's card reads `41 components, 6 joints (5 revolute, 1 rigid)`,
  or `18 of 20 components placed, …` when some were left out.

### Feature history

- A profile cadmpeg keeps as a native selection maps when its sketch has one profile, which
  the selection can only be; in a sketch of several it says it is not mapped yet.

- Sheet-metal base flanges map onto SolidCraft base flanges, under a sheet rule of their
  thickness, on the sketch's side the source puts them. A flange thick on both sides of its
  sketch is not mapped yet.

- Fusion base features (bodies a timeline holds as they are, modelled directly or imported)
  map onto SolidCraft Import features at their place in the timeline, each holding its own
  bodies as STEP, so the features after them keep their history instead of the whole design
  being flattened. A base body that later features change is caught by the check against
  the original geometry.

- A component inserted, or a joint made, in a Fusion timeline is counted as not needed in the
  part's history, since the assembly carries the component and the joint, instead of
  stopping the history.

- A sketch profile with curves drawn across it, such as a rectangle with its diagonals, maps
  when the feature takes every profile of the sketch. Fusion walks such a profile along each
  inner curve both ways; Dutchman selected it by those curves and SolidCraft found no profile
  bounded by them, so the replay failed and the history was flattened. Any other such selection
  is reported as not mapped yet.

### Assemblies and components

- A joint to a component inside a design inserted into a Fusion archive takes its frame in
  that component's own space, as Fusion stores it, not in the inserted design's: the robot
  arm's gripper sat 52 mm off its wrist flange.

- A fixed joint whose parts' stored placements already meet, with its frames opposed or (as
  Fusion mates a flipped joint) aligned, holds them where they are, as revolute and slider
  joints already did, instead of turning one part over to mate the frames opposed. Fusion's
  files do not yet say which joints are flipped; a part only a flipped joint places, such as
  a design inserted into an archive, can still come out turned over.

## 0.1.3 (2026-10-21)

### Formats

- **FreeCAD** documents (`.FCStd`) convert in the nightly builds, through cadmpeg's FreeCAD
  codec: their geometry, and their components when cadmpeg says which bodies each owns.
  Stable builds say so instead, until a cadmpeg release reads FreeCAD solids right. FreeCAD
  support is the `freecad` cargo feature, which the nightly build turns on. See
  [docs/freecad.md](docs/freecad.md).

### Feature history

- Datum planes the source places (FreeCAD's PartDesign Plane) are counted as not needed:
  what is sketched on them keeps the same plane of its own.

- Nodes that only order the feature tree (folders, FreeCAD's PartDesign Body, comment and
  annotation containers) are counted as not needed instead of stopping the history.

- Simple holes drilled at a sketch's circle centres, arc centres or points map onto
  SolidCraft holes: through all, or to a depth with a flat end or a drill point. FreeCAD's
  Hole is one; counterbores, countersinks and holes placed on faces are not mapped yet.

### Desktop app

- After **Convert**, the window scrolls to the results as the run starts, and to the first
  file that failed or converted incomplete when it ends, so the first problem is on screen at
  any window size.

### Reading

- A part whose cadmpeg model fails cadmpeg's checks (a parameter name repeated in one scope,
  a parameter used before it is defined) is no longer refused. Its geometry is read again
  without the design's history, sketches and parameters, where such failures lie, and written
  as geometry, with a `Degraded` loss saying why. In an assembly, the part is placed instead of
  left out. When the geometry fails cadmpeg's checks too, the error says what each
  attempt found.

### Checks

- A history whose original geometry SolidCraft opens unsoundly (solids as empty shells,
  bodies inside out) is kept, unverified, saying so, rather than flattened onto that geometry:
  there is nothing sound to compare its replay with. A history that does not replay is still
  flattened.
- The check of geometry written without history also reads each body's volume in SolidCraft:
  a solid it gives no volume (an empty shell) or a body it reads inside out is reported as
  `Degraded: geometry`, as missing faces already were.
- Installing the SolidCraft app is enough to check histories: without `SOLIDCRAFT_CLI` or
  `solidcraft-cli` on `PATH`, Dutchman uses the CLI inside the newest installed SolidCraft app
  (macOS `SolidCraft*.app`, Windows `Program Files\SolidCraft`). The desktop app says which CLI
  it checks with. `dutchman_solidcraft::find_cli` is public.

### Releases

- macOS: each release has a disk image, `dutchman-VERSION-macos-universal.dmg`, holding
  **Dutchman.app** with its own icon, for Apple silicon and Intel, with the `dutchman` and
  `dutchman-mcp` commands inside. It is signed ad hoc, not with a Developer ID; the guide shows
  how to open it the first time. The archives stay as they were.

- The README, and each release's notes, start with a direct download for each platform, with
  its checksum, and how to tell which Mac you have.
- [Testing Dutchman on your library](docs/testing-your-library.md) is updated for this
  release: the macOS app, SolidCraft found in its app, Inventor project search, the incomplete
  count and `dutchman.log`, and when to try the nightly.
- A nightly build: Dutchman's main built against cadmpeg's main every day, when either has
  changed, and published as the `nightly` pre-release once every test and the SolidCraft round
  trip pass. It brings cadmpeg fixes that are merged but not yet released; stable releases keep
  using released cadmpeg only. See [Channels](docs/guide.md#channels-stable-and-nightly).

### Reports

- A file with more than 12 losses is summarised by component in the text reports: the design
  first, then each part with its counts and, when its history was flattened, why. `--verbose`
  prints every line. `convert --log FILE` writes the whole report, every loss in full, to a
  file, and `convert-all` writes one for the run to `dutchman.log` in its output folder (or
  `--log FILE`). The desktop app folds each part's losses under its own heading and saves the
  full log with **Save the full log…**.

### Assemblies and components

- An assembly's conversion says how complete it is, before you open it: `assembly: 18 of 20
  placements placed; 2 left out:`, then each cause with its file and count, counting through
  sub-assemblies as often as they are placed. `convert`, `convert-all` and `inspect` print it;
  the JSON reports carry it as `completeness`. The desktop app marks such a file
  **Incomplete**, shows the count and the causes on its card, and counts incomplete files in
  the run's summary. The anonymous report counts them as `incomplete`.
- Inventor assemblies find their parts as Inventor does: at the stored path, relative or with
  the saving machine's drive or root moved onto this one, else by name anywhere under the
  project folder (the nearest above holding an `.ipj`, the folder given to `convert-all`, or
  `--project DIR`), preferring the file in the stored path's folders. A project whose parts
  are in sibling folders (`../Parts`, `../SubAssembly`) converts where it lies. `--search DIR`
  adds folders searched after the project's, and files found are named as their folders list
  them. `dutchman_core::read_in` and `convert_traced` take a `PartSearch`.
- The desktop app names the part files an assembly could not find, and converts it again
  searching a folder, or a found file's folder, chosen there. Its **Inventor project folder**
  setting gives the project folder for a run.
- Revolute and slider joints become SolidCraft revolute and slider joints, revolutes with their
  limits, so the parts move as in the source; fixed joints keep their offset. Joint frames meet
  face to face, as in Fusion.
- Joined parts are placed where their joints put them, as Fusion can store a joined part where
  it was before its joints were solved.
- An Inventor assembly's part files are also searched for in the folders its Inventor project
  file (`.ipj`) names: the workspace, workgroup and library folders, and the
  imported-components folder. The project file is the nearest one in or above the project
  folder, so `--project` given a folder inside a larger project still finds the files
  elsewhere in it. The report lists the folders searched, and a part not found says how many
  were. Folders below the project folder that hold a project file of their own (Pack and Go
  copies) are no longer searched.
- A design inserted into a Fusion archive (`.f3z`) is placed as a sub-assembly, named as the
  archive names it, and a joint to a component inside it joins that design as a whole (its own
  joints are not carried yet). Such an archive converted as geometry alone before, without its
  joints.
- A revolute or slider joint whose parts already turn or slide on its axis where they are
  placed holds them there, at the angle or distance their placements give, instead of moving
  them to the value the source stores. Revolute limits that this angle lies outside are left
  out, with a loss saying so.
- A revolute joint's angle in the source is taken as the angular offset that aligns its frames,
  as a fixed joint's is, not as where it has turned to: Fusion stores no current angle, only
  the placements. Its limits hold from the aligned frames.

## 0.1.2 (2026-10-21)

### Testing on your own library

- `convert-all` converts each file in a process of its own: a file that runs past `--timeout`
  (default 900 s) or holds more than `--max-memory` (default 8192 MB) is stopped and fails
  alone, and one that crashes the converter no longer takes the batch down.
- `convert-all --journal FILE` records each file's result as it finishes, so a stopped run
  carries on where it stopped without converting files again.
- `convert-all --report-anon FILE` writes an anonymous report, safe to share: counts by format,
  every failure and loss by cause and stage, blocked features, timings and versions, with each
  file named only by a salted hash of its path. `--report-salt` keeps the ids from one run to
  the next and `--report-key` writes the private key from ids to paths. The MCP `convert_all`
  tool takes the same options.
- Every loss and failure in the JSON reports says its `stage` (`input`, `decode`, `project`,
  `map`, `write`, `solidcraft`) and, when it arose in cadmpeg or SolidCraft, its `upstream`.
  Each `convert-all` result gives its `elapsed_ms`, and the report counts skipped files by
  extension.
- Formats Dutchman does not convert yet point to Dutchman's issues rather than cadmpeg's.
- New guide, [Testing Dutchman on your library](docs/testing-your-library.md), with a
  library test report issue form; [docs/triage.md](docs/triage.md) says how reports are sorted
  and taken upstream.

### Feature history

- Lofts guided by one or two rails of sketch curves keep their rails. A loft along a centreline
  still flattens, with the reason.
- Mirrors of earlier features across a plane map onto SolidCraft mirrors.
- Sphere and torus primitives map onto SolidCraft's (a torus about the Z axis). Boxes and
  cylinder primitives still flatten, with the reason: cadmpeg does not yet say where they sit.

## 0.1.1 (2026-10-21)

### Desktop app

- `dutchman-gui`: drop design files or folders, choose an output folder and convert. Each file
  shows what was written and its losses, and opens in SolidCraft or in its folder. Day and Night
  themes, following the system's setting. It ships in the release archives.

### Feature history

- Lofts through profiles of two or more sketches map onto SolidCraft lofts, when they have no
  guide rails or centreline and are open, smooth and solid.
- Sweeps of a sketch profile along sketch curves map onto SolidCraft sweeps: along the whole
  path, square to it or held as drawn, optionally twisted or guided by one rail.

### Assemblies and components

- Fusion joints between components become SolidCraft rigid joints, written as built, so the
  parts they join stay together. Joints of other kinds are reported as Dropped.
- A component holding other components converts when it is placed more than once at one level;
  such designs were wrongly refused before.

### Checks

- When `solidcraft-cli` is found, geometry written without history is opened in SolidCraft too,
  and the report says `Degraded: geometry: …` when SolidCraft shows fewer faces than the source
  model has or warns about it.

### Command line

- `convert` first prints what it wrote where, `converted  IN -> OUT (geometry without
  history)`, as `convert-all` does for each file, now in plain words for both.
- Long loss notes are cut to their first sentence in the text reports, marked `…`; `--verbose`
  prints them whole. `--json` is unchanged.
- `convert-all` lists each converted file's losses beneath it, as `convert` does, and counts the
  files it skipped as not in a format it knows (`skipped` in `--json`).
- `inspect` says when converting a file would fail and why (`would write: nothing; …`, and
  `conversion` in `--json`), instead of predicting geometry it cannot write.
- `--no-clobber` (`overwrite: false` in the MCP tools, and a checkbox in the desktop app) keeps an
  existing `.solidcraft` file; that file's conversion fails instead. `--help` and the MCP tool
  descriptions now say that an existing file is otherwise replaced.

### MCP server

- A tool call with arguments the tool cannot take returns a failure report as structured
  content too, the same shape as a failed conversion, not only text.

### Reading and documentation

- Reads native files with cadmpeg 0.6.3.
- The release archive's README links to the docs of its own release; the getting-started guide
  has a complete install and sample download.

## 0.1.0 (2026-10-21)

The first release that converts designs. Native CAD files are read by
[cadmpeg](https://github.com/cadmpeg/cadmpeg) 0.6 and written as
[SolidCraft](https://github.com/tangency-labs/solidcraft) designs (`.solidcraft`, format
`solidcraft/23`).

### Formats

- **Fusion** archives (`.f3d`) and multi-document archives (`.f3z`, with the designs they
  reference merged in by cadmpeg).
- **Inventor** parts (`.ipt`) and assemblies (`.iam`), with their sub-assemblies. Part files
  are found at the stored path, else by name next to the assembly, ignoring case and either
  path separator.
- **SolidWorks** parts (`.sldprt`). SolidWorks assemblies are refused with a message.

### Feature history

- When every active feature maps, the design is written as editable SolidCraft history, with
  the original geometry kept beside it as a suppressed import. Otherwise it is written as the
  geometry, and the report says which feature stopped the history and why.
- Mapped:
  - planar sketches: points, lines, circles, arcs, whole ellipses, clamped uniform splines,
    with their constraints and dimensions (angles too), linked to parameters;
  - parameters, keeping their expressions where SolidCraft reads them the same way;
  - extrudes of whole sketches, profiles, profile regions or selected curves: one-sided,
    symmetric or two-sided, blind or through all, as new body, join, cut or intersect;
  - revolves that are symmetric or a full turn;
  - patterns of features, linear in one or two directions, or circular all the way round.
- A written history is replayed in `solidcraft-cli` and compared with the original geometry
  (body count and volumes). A history that differs is flattened. Without a CLI, or with
  `--no-verify`, it is written and reported unverified. `--import-only` never writes history.

### Assemblies and components

- Assemblies become SolidCraft components and occurrences: one component per part or
  sub-assembly, one occurrence per placement. Each part keeps its own feature history when it
  maps. Its parameters, and its body names, are renamed with its component's name where
  another part already uses them.
- Designs that define and place their own components (Fusion) convert as SolidCraft
  components when cadmpeg says which bodies each owns. Components inside components become
  sub-assemblies.

### Appearances

- Body colours, and planar face colours, carry across: through STEP for imports, and as
  SolidCraft looks for replayed bodies, including bodies of parts inside assemblies.

### Command line

- `dutchman convert`, `dutchman inspect` (what a conversion would carry across, writing
  nothing), `dutchman formats`, and `dutchman convert-all` (many files and folders into one
  output folder, several at a time).
- `--json` on every command prints one report with a stable schema (`docs/json.md`). Exit
  codes: 0 converted, 1 could not convert, 2 usage error.

### MCP server

- `dutchman-mcp` serves the same four commands as Model Context Protocol tools over stdio, for
  AI agents. Each tool returns the matching `--json` report. The README shows how to add it to
  a client.

### Robustness

- Hostile input comes back as an error, never a crash. Inputs over 1 GiB are refused before
  they are read, cadmpeg decodes under its resource limits, and a panic inside cadmpeg is
  caught wherever Dutchman calls it. A deterministic test runs every fixture, truncated and
  corrupted, through the whole conversion.
- Output is written only when the whole conversion succeeds.

### Known limits

- Fillets, chamfers, holes and partial one-sided revolves are not mapped yet: they wait on
  cadmpeg giving typed edge and face references and a revolve direction. Inventor and
  SolidWorks extrudes wait on cadmpeg resolving their profiles. See `docs/fusion.md`,
  `docs/inventor.md` and `docs/sldprt.md`.
- Assembly joints wait on cadmpeg's next IR version.

## 0.0.1

Name reservation on crates.io. No functionality. Later releases ship as GitHub release
archives; the crates.io crate stays this placeholder until the core API settles.
