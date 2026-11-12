# Dutchman architecture

Dutchman converts native CAD design files into SolidCraft designs (`.sc`). It starts
as a command-line tool and becomes a SolidCraft plugin once SolidCraft has plugins. The
design rule is low coupling, high cohesion: one self-contained conversion core, and a thin
SolidCraft adapter that can later sit behind a plugin interface unchanged.

## Pipeline

```
native file ──cadmpeg decode──▶ CADIR (neutral model) ──Writer──▶ target file
  .f3d .ipt .sldprt …            exact B-rep, history,             .sc
                                 product structure
                    └──────────── Report (losses) ─────────────┘
```

Decoding belongs to [cadmpeg](https://github.com/cadmpeg/cadmpeg). Dutchman never parses a
native format itself; it drives cadmpeg's codecs and maps cadmpeg's neutral model, CADIR
(`cadmpeg_ir::CadIr`), onto a target. Format work lands in cadmpeg, so every cadmpeg
consumer benefits and Dutchman stays small.

## Crates

| Crate | Role | Depends on |
|---|---|---|
| `dutchman-launcher` | Discovers installed `dutchman-<tool>` commands and dispatches them; also supplies a small front command. | std only |
| `dutchman-core` | Format gate, input caps, the `Writer` seam, the `Report`, the `convert` pipeline. Owns the cadmpeg decode call. | `cadmpeg-ir`, `cadmpeg-core`, `cadmpeg-registry` |
| `dutchman-solidcraft` | Implements `Writer` for `.sc`. Knows the SolidCraft design format and nothing about native formats. | `dutchman-core`, cadmpeg's STEP encoder, `zip`, `sha2`, `serde_json` |
| `dutchman` | The CLI (`dutchman convert <in> <out.sc>`) and the MCP server (`dutchman-mcp`), sharing one small library that sets up a conversion and builds the JSON reports both return. Front-end orchestration, printing, the MCP protocol and plain presentation data. | both of the above, `clap` |
| `dutchman-gui` | The desktop app (eframe/egui). Plans and converts through the `dutchman` library's batch code on background threads; draws the Dutchman design system's Day and Night themes. | `dutchman`, `eframe`, `rfd` |

`dutchman-core` and `dutchman-solidcraft` are `publish = false` until their APIs settle. The
`dutchman` crate is the published CLI (`cargo install dutchman`); `dutchman-cad` stays a
reserved placeholder for a future library facade.

### Why CADIR is the model

CADIR is already a format-neutral, exact B-rep model with feature history, sketches,
parameters, appearances and assemblies, in millimetres and radians. A second Dutchman-owned
model would duplicate it and double the mapping work. `dutchman_core::Model` carries the
`CadIr` itself, cadmpeg's source-fidelity record (which its encoders take alongside the
model) and the design's name; if a target ever needs a different shape, the adapter
derives it.

### Decoding

`dutchman-core`'s `decode` module is the one place cadmpeg is called. It reads the input
(after the size cap) into memory, lets cadmpeg's catalog identify the codec from the
contents, falls back to the codec the extension names when the contents are not
conclusive, and refuses contents that clearly belong to another format. It decodes in
salvage mode under cadmpeg's desktop ceilings, folds cadmpeg's loss notes into the
`Report`, then validates the model as cadmpeg's own conversion does: a model with no
geometry or with validation errors is refused. A panic inside cadmpeg is caught and
returned as a decode error.

Shared `.prt` and numbered `.prt.N` suffixes use cadmpeg's bounded content detector before
choosing Creo or NX. Inconclusive contents remain recognised without choosing a reader.
Dutchman's `Format::reads_geometry` records whether the reader supplies geometry. Formats
without geometry support or a codec in this build are refused before decode. All cadmpeg
crates come from the same git revision; `scripts/use-cadmpeg-main.sh` updates those pins together.
IGES is also refused before opening the input: cadmpeg does not yet sew its faces into
solids, so Dutchman directs users to SolidCraft's own IGES importer (File > Import).

## The plugin seam

```rust
pub trait Writer {
    fn extension(&self) -> &'static str;
    fn accepts_extension(&self, extension: &str) -> bool;
    fn write(&self, model: &Model, out: &mut dyn Write, report: &mut Report) -> Result<(), Error>;
}
```

The default `accepts_extension` checks the writer's extension case-insensitively; the
SolidCraft adapter also accepts legacy `.solidcraft` and `.solvecraft` names. Its default
extension follows the same target version gates as feature mapping.

`Writer::write_with_progress` is an optional default method that delegates to `write`.
Writers that report named work receive the conversion's shared `Progress` explicitly,
including on scoped part workers. Running-step guards end on drop. A snapshot selects
the oldest innermost step per worker; names remain local presentation data. Anonymous
logging keeps its existing stage/count allowlist and never serializes these snapshots.

The SolidCraft adapter depends only on this trait, CADIR and the SolidCraft file format.
When SolidCraft grows a plugin interface, a small shim crate implements that interface by
calling `dutchman-core` to decode and `dutchman-solidcraft` to build the design (or hands the
design to SolidCraft in memory instead of as bytes). Neither crate changes.

### SolidCraft evaluation

`dutchman-solidcraft::SolidCraftEvaluator` is the object-safe host boundary for every
blend/face proof, partial-history replay, history comparison and final geometry check.
Pass an `Arc<dyn SolidCraftEvaluator>` to `SolidCraftWriter::evaluator` and set
`target_version` to the host's release. Native callers that supply nothing retain CLI
discovery and `CliEvaluator`, including launch retries, timeouts, memory monitoring and
platform launch diagnostics. `Verification::Off` still disables final verification;
blend/face proofs have their separate `replay` switch.

The two methods accept an in-memory design archive and a timeout. `evaluate` returns the
unchanged SolidCraft eval JSON (body volumes, areas, bounding boxes, timeline errors and
warnings). `run` executes an ordered engine-command list and returns the unchanged JSON
result for every command, including errors. Each call opens its own isolated design;
state persists between commands within a call. Labels identify requests for diagnostics.
An unavailable answer is `Err`; a design refusal uses `evaluation_failed` in the eval
JSON. Measurement parsing, matching, tolerances and fallback policy remain shared in
`verify.rs`, independent of the transport.

A plugin host must open a design archive with its STEP blobs, evaluate it with `inspect.measure` plus `document.inspect` (the same combined JSON
that CLI eval returns), and dispatch `timeline.roll_to`, `document.inspect`,
`model.edges` and `model.faces`. Edge results include names, endpoints, midpoints and
lengths; face results include names, areas, sample points and optional planar normals.
It must enforce deadlines, bound responses, isolate evaluations from the user's open
design and serialize engine access if needed. The `Send + Sync` contract also permits
native assembly workers to share an evaluator.

On `target_family = "wasm"`, CLI execution, registry/xattr probes, the memory watchdog,
worker threads and heartbeat threads are excluded. Preparation runs serially and progress
still reports synchronous steps; the host may drive heartbeats itself. Both library crates
build for `wasm32-wasip2` with `--no-default-features`; choose core input-format features
explicitly for a plugin. Native advisory snapshot rendering remains native-only.

## Depending on cadmpeg

- From cadmpeg's git repository, pinned to one tested commit of its `main` (`cadmpeg-ir`,
  `cadmpeg-core`, `cadmpeg-registry`, the same `rev` for each); Cargo fetches it, so CI and
  `cargo install --git` work without a cadmpeg checkout. docs/releasing.md says when the pin
  moves; the nightly build takes cadmpeg's `main` instead.
- `cadmpeg-registry` with `default-features = false`; each crate turns on the codecs it
  needs. `dutchman-core` has a feature per input format (`fusion`, `inventor`, `solidworks`,
  `catia`, `creo`, `nx`, `rhino`, `step`, `iges`, `sat`, `pvz`), all of them in its default
  `formats` feature, which `dutchman`, `dutchman-gui` and `dutchman-web` turn on; its
  `freecad` feature controls whether FreeCAD inputs are accepted. `dutchman-solidcraft` turns
  on the `step` codec for the STEP its Imports keep. A build that reads only some formats (the
  SolidCraft importer plugin) names just those and links only their codecs.
- Unmerged cadmpeg work is tried locally with a `[patch."https://github.com/cadmpeg/cadmpeg"]`
  path override that is never committed. When the checkout's crate version differs from the
  pinned one, Cargo keeps the locked version and lists the override as unused
  (`warning: Patch ... was not used`, `[[patch.unused]]` in `Cargo.lock`). Run
  `cargo update -p cadmpeg-core -p cadmpeg-ir -p cadmpeg-registry` to take it, and restore
  `Cargo.lock` along with `Cargo.toml` afterwards.
- Minimum Rust follows cadmpeg's (1.88 today).

## Writing SolidCraft designs

The adapter writes exactly what SolidCraft saves when it imports a STEP file: a design
container (a zip of `blobs/<sha256>.step`, `document.json` and `container.json`) whose
document holds one component named after the file, one occurrence of it, and a
non-parametric `import` feature. The feature's geometry is cadmpeg's AP214 restatement of
the model, kept in the blob; its `file` names the native source, as SolidCraft's own
FreeCAD and Rhino imports do. The adapter builds the container itself with `zip`, `sha2`
and `serde_json` rather than linking `solidcraft-doc`: that crate pulls in SolidCraft's
kernel and renderer, needs edition 2024, and would build against unpatched dependencies
outside SolidCraft's workspace.

SolidCraft reads the import's assembly tree from the STEP when it evaluates the design.
When an assembly carries appearance bindings, Dutchman names imported output bodies explicitly
so looks cannot attach to a different component's namesake. Stored-solid history imports also
name their bodies. The name sequence follows the restated regions and is checked against the
written shape count; unmatched bindings are reported rather than assigned by a guessed name.

The adapter pins format `solidcraft/23`, the first in which an Import may name a non-STEP
source. SolidCraft upgrades older formats on open and refuses newer ones, so a pinned older
format provides the 0.18.2 compatibility floor. This stays `solidcraft/23` for 0.28
targets too: its format 45 additions (nested Browser groups, point cloud properties and
filters, and print faces) are not used by Dutchman. Feature mapping also respects the writer's
target SolidCraft release: the found CLI's version capped at newest known, even with verification off, else
`NEWEST_SOLIDCRAFT` (currently 0.30), unless an explicit target is supplied.
`WINDOW_OLDEST` (currently 0.18.2) marks the oldest release in the rolling 14-day window;
older targets down to the floor remain accepted with the existing feature gates. The
CLI's `--solidcraft-version x.y.z` selects a target on `convert`, `convert-all`, and `watch`.
Sketch-profile helical sweeps require a target of 0.25 patch 1 or newer; older targets block them
and preserve the original geometry. Entry-recess holes write an explicit flat tip for
0.22 patch 1 and newer targets, where SolidCraft applies their drill-point angle.

SolidCraft status and presentation use major/minor pairs. `NEWEST_SOLIDCRAFT` is a pair;
its internal writer default is minor.0. `SolidCraftVersion` keeps all three numbers so
0.18.2, 0.22.1, and 0.25.1 gates still apply. CI resolves the latest stable patch of the
newest minor using authenticated release metadata and verifies its release checksum.

Checks: an end-to-end test converts a synthetic `.f3d` and compares the container layout and
`document.json` with a golden file; a non-blocking CI job opens the result in a released
SolidCraft CLI (`scripts/solidcraft-roundtrip.sh`).

After decode, the core indexes component geometry once per design. The index borrows rows
from one compact JSON document; only structure and history are expanded into JSON values.
A component follows references from its bodies and history, then emits just the reached
rows in their original arena order. Bounded scoped workers share the index. Tree building
consumes prepared slices in its original order and assigns the original part names;
identical empty assembly holders share an empty model. Failed preparations retry serially
through the same index where needed. Name collection streams compact JSON, and Fusion
visibility filtering avoids expanding the model when no consumed or removed body needs
to be omitted.

Placement admission copies each model's own data once and prepares its children recursively;
it does not keep a throwaway clone of the original child tree during that recursion.
Sketch ownership reads the admitted feature and definition records directly. Reference
sketch mapping expands only the neutral model, once, without native records or a second
copy of the geometry arenas.

Assembly leaf parts prepare on bounded scoped workers: STEP restatement, history mapping,
blend proving, and SolidCraft replay checks each own their reports and temporary data.
The builder consumes results in the original depth-first part order; only it allocates IDs,
relocates histories, renames bodies, appends reports, and writes blobs. Worker failures and
panics leave an empty result slot, retried serially by the builder before assembly.
`--threads 1` takes the serial path without creating workers. Native decoding is unchanged.

## Feature history

When every active feature of the model maps onto SolidCraft features, the design is that
history: SolidCraft parameters (`params`) and features at the root, then the original
geometry as an Import named "Original geometry (imported)", **suppressed**, at the end of the
timeline. The replayed history is the live result; the original is there to compare with and
to fall back on, so a history design is never worse than an import design. A history made
only of base features that hold every body already is the original, so it is not written twice.

In an assembly, a part that repeats one written earlier (same source, same layout below it) is
one definition placed wherever it repeats, at any level. Sub-assemblies follow the same rule
when everything inside them is named and no joint reaches inside them: SolidCraft joints name
an occurrence, so those stay apart.

Unused construction helpers need no target feature. Later active consumers carry resolved
datum frames in sketches, pattern axes, and later datum constructions; unresolved helper
uses remain blockers.

At the first blocking feature, the editable timeline stops. Earlier features and
independently resolved sketches are kept in a hidden component, followed by the original
final geometry as a visible Import. The original is a separate result, not an
intermediate body on which later features could safely continue: CADIR supplies final
geometry only. The report names the blocker and counts retained source features. If no
body-building prefix exists, the import keeps resolved sketches for reference as before.
Suppressed source features are left out and
reported. Mapped today: principal datum planes; planar sketches (points, lines, circles, arcs,
whole ellipses, elliptical arcs up to one turn (multi-piece curves require whole-sketch
selection), bounded parabolas and hyperbolas, rational quadratic Béziers, clamped B-splines
(including equal-weight rational curves and periodic curves clamped by exact knot insertion),
reference lines, at their solved positions) with their constraints and dimensions, each
dimension linked to its parameter (a constraint that does not map is dropped and reported,
since the geometry is already exact). A clamped uniform spline stays one spline; other knot
spacing becomes a chain of exact Bézier spline pieces, up to 200, with shared endpoints.
Also mapped: extrudes of a whole sketch, of selected sketch
profiles, of profile regions (whole loops with their holes, or regions bounded by pieces of
curves) or of selected curves, one-sided, symmetric or two-sided, blind or through all, from the sketch plane or an
offset, or a resolved parallel planar face on an earlier named body. A face of the
feature's own input topology (a face of an earlier history state) is rebuilt as its plane and
an interior point from that state's boundary samples; a face that is not parallel to the
sketch is refused. Curves that bound a selected trimmed sketch region stay solid (not
construction), so SolidCraft splits the sketch there and sweeps that region, not the merged
area around it. One-sided extrudes may
carry draft (positive grows outward), end on such a face with an offset, or end on a resolved
parallel plane. Before any body-building feature, a plane end uses its origin as a fixed
To Object point (SolidCraft accepts a free point without a construction plane or body face).
After earlier geometry, a `to_plane` end whose plane is a face's surface (`source:
face_surface`) uses the same point, since a pick of that face ends at the same plane; other
`to_plane` ends wait on input face geometry: SolidCraft may reinterpret a point as a body
face, and final-body faces cannot prove the timeline state. A parallel `from_plane` start
becomes a start offset along the sketch normal. Two-sided extrudes may use a non-blind positive-normal end when
the negative-normal end is blind.
Extrudes map as new body, join, cut or intersect (a new body keeps its source name and colour);
revolves about a line in the sketch plane (a line of the profile's sketch stays that line)
that are symmetric, equal-angle two-sided or a full turn, including unequal sides that add up
to one, as solids or, short of a full turn, as surfaces; one-sided partial solid revolves
follow the oriented CADIR axis when bounds on the selected lines, circles, ellipses, or whole
arc carriers prove the profile lies on one side of it. SolidCraft 0.18.2 and 0.26.0 both
sweep toward the sketch normal for a positive angle, independently of the axis direction;
the writer signs the angle to preserve the source's right-hand sweep. Unproven sides and
unequal partial two-sided revolves remain blockers;
lofts through two to fifty profile or terminal point sections,
open, smooth (or ruled through two sections without rails), solid or surface (a surface loft
becomes a new surface body, without point ends), guided by up to two rails of sketch curves or
one-curve whole sketches (sections can select separate regions of one sketch; resolved point and
vertex ends become sketch points; closed, centreline and degree-limited lofts, and rails of
edges or model-space curves, are not mapped); sweeps of one sketch profile along
sketch curves (whole path, solid or surface, square to it, held as drawn or with a fixed
binormal, optionally twisted up to four turns and scaled at the end by 0.01–100, or guided by
one rail without other orientation, twist, or scale options; a whole-sketch path or rail maps
when it has a single non-construction curve; taper, explicit corner transitions, changing
sections, tangent-path extension, and post-sweep simplification are not mapped); spheres, tori about the Z axis, and whole cylinders and cones placed by their feature frame (cones on SolidCraft 0.24.1 and newer; boxes wait on cadmpeg saying whether a box's placement is its corner or its centre); coils along any axis, travelling up or down it, of round or square section,
turning either way (tapered coils, flat spirals and triangular sections are not mapped);
simple, counterbore and countersink
holes drilled at the circle centres, arc centres or points of a sketch (including explicit
entity selections), or at a directed spatial placement, through all or to a depth, flat or,
for simple holes, with a drill point; axes placed on proven earlier extrusion caps and holes
ending at those caps (the cap must match the sole polygonal profile of the last earlier
solid feature's one-sided new-body extrusion; a face from final geometry alone is not enough);
chamfered entries map as countersinks; a tapped hole keeps its resolved bore and an ISO
metric thread label (`M6x1`, the pitch from the designation, the source, or the coarse
series), and the thread's hand, class, and length are reported as Degraded, not mapped;
entry recesses combined with drill points, exit treatments, taper, and modeled threads are not mapped;
patterns of earlier features or named bodies, linear in one or
two directions, circular over a partial span or all the way round, or at fixed spacing along
planar sketch curves; regular explicit offsets and angles, and pairs of linear composite
stages; ordered chains of feature-pattern stages containing mirrors (target SolidCraft
0.25 patch 1 or newer), each repeating the previous instance set (at most 32 stages), with at most
10,000 instances per pattern or chain;
mirrors of earlier features across a
plane; and body mirror patterns and body combines (join, cut or intersect, optionally keeping
the tools) of bodies named by earlier features. Shells and drafts wait on input face geometry
(their faces are known only on the final body, which the feature has changed);
modeled threads and threads without resolved metric size, extent, and cylindrical face
geometry, and shape-replacing mirrors are not supported yet. Features are
named uniquely on the timeline, since SolidCraft patterns refer to them by name. Parameters
keep their expressions where SolidCraft reads them the same way, else their evaluated values.

Constant-radius fillets and equal-setback chamfers keep editable history when their own
feature input topology supplies edge midpoints, tangents, and endpoints, and each midpoint
can be proven to pick exactly one edge of the pre-feature body, whatever history builds it.
SolidCraft itself rebuilds that body: Dutchman replays the mapped history in `solidcraft-cli`
once per design (two runs, however many blends, within the check's time limit), up to the last
feature a blend starts after, and reads the edges each blend starts from. Without `solidcraft-cli`, or when the replay fails or runs out
of time, blends are refused, saying why. SolidCraft reports each edge's ends, midpoint and
length but no tangent, so an input edge must be a line or a circular arc; its midpoint must
lie on exactly one rebuilt edge across every body, and that edge must have the same ends (in
either order) and length; a straight one must lie along the input tangent. Arcs SolidCraft
splits into pieces, closed edges and other curves are refused. Equal two-distance chamfers also map; distance-and-angle chamfers map at
45 degrees on perpendicular faces. Unequal distances and other angles lack a source
reference-face ordering and are refused specifically. Variable fillets lack a proven
chain direction. Final-body edge selections never stand in for input geometry.

Selected straight profile loops that cross or retrace their own curves are split at
intersections into bounded regions for selection. The sketch keeps its original curves and
constraints; SolidCraft matches the pieces on recompute. Selected subsets with overlapping
curves, ambiguous boundary matches or curved crossing arrangements still flatten the history.
Native profile picks within a known sketch also map when even-odd nesting identifies exactly one material
region, with its holes; multiple islands and touching loops remain ambiguous.
When every active consumer of a sketch selects explicit whole-loop regions, unused lines
crossing selected whole circles become construction geometry. This keeps layout
rectangles through hole centres from splitting those holes. Any other selection form keeps
the lines solid; a boundary needed by another region consumer stays solid too.

A written history is checked in SolidCraft itself, since Dutchman has no kernel. The writer
runs `solidcraft-cli eval` on the design as written and on a copy with the history suppressed
and the original live, and compares body count, volumes and areas. SolidCraft measures from
a mesh whose chords keep within 5e-5 of a body's size of its faces, so two meshes of one shape
may differ in volume by both their areas times that chord, and never less than 1e-4 of the
volume is allowed; areas must agree within 1e-3. A mismatch retains the editable
timeline hidden beside the visible original final shape (`partial_history`, verification
`partial`). A replay error that identifies a feature shortens the prefix before that
feature; a final volume or area difference alone cannot identify a bad step, so the
mapped timeline stays available for correction. Retained features are replayed
separately from the original, without claiming that the unfinished result matches the
source. If no editable prefix remains, the design falls back to the import with
reference sketches. When no CLI is found (`--solidcraft PATH`, else `$SOLIDCRAFT_CLI`,
else `PATH` or beside the program), when it times out (`--verify-timeout`, default 120 s), or with
`--no-verify`, the history is written and reported unverified. For ordered mirror chains
and repeated full-turn endpoints, an attempted replay that cannot finish instead keeps the
original geometry and reports why. `--import-only` always writes the import design.

Geometry written as an import is opened in the same CLI. A refusal, timeline errors, faces
it cannot mesh, bodies inside out, or solids without volume produce a `Degraded` `geometry`
loss, as does a solid it opens as an open shell (sheets aside). Face counts are not compared:
SolidCraft merges faces that share a surface. Area and volume comparisons use the history and
original evaluations above, where an open-shell warning alone produces no loss; geometry-only
output has no independent source measurements.
An assembly is checked for warnings and bodies inside out, with sheets exempt as for parts.

Sheet-metal edge flanges select only from their own input topology. Midpoints, tangents, and
end vertices must match unique perimeter edges of an unchanged rectangular base sheet,
reconstructed from the mapped sketch and thickness; other input bodies are refused. Fixed
outer-face heights, full-edge widths, inside/outside placement, angles, and bend radii map.
Hems are refused because their developed lengths cannot be passed as SolidCraft outer-face lengths,
and rolled, teardrop and unresolved hems have no SolidCraft form.

## Presentation

`dutchman::present` builds plain `FileView` and `RunView` data from the conversion JSON
reports. It owns desktop outcome wording, status classification, missing-part recovery,
loss groups, summary counts, first-issue selection and severity ranks. `RunView` keeps source
indices alongside its severity-sorted files so the desktop can retain input order.
Component ordering, assembly wording, invitations and issue-form encoding reuse the existing
CLI library helpers. Issue anonymization stays with the caller; presentation only fills the
form from an already anonymous report. No UI types or I/O enter this module.

## Errors and losses

- **`Error`** stops a conversion: I/O, input over the size cap, unknown or not-yet-supported
  format, wrong output extension, a decode or encode failure. Nothing is written when the
  input cannot be read.
- **`Report`** lists what a successful conversion could not carry across, each `Loss`
  tagged `Info` (no effect on geometry), `Degraded` (carried in weaker form, such as history
  flattened to a body) or `Dropped`. cadmpeg's own `DecodeReport` / `LossNote`s are folded
  into the same report, so the user sees one list.
- Every `Loss` and `Error` carries a **`Stage`**: `input`, `decode` and `project` (cadmpeg),
  `map` and `write` (Dutchman; cadmpeg's STEP restatement within writing), `solidcraft`. The
  reports name the upstream project from it, so a report reaches the code that can fix it.
- **`convert-all`** converts each file in a child process (the same program, started with a
  worker argument), which prints the file's report and sends progress JSON lines on stderr
  at each stage, document completion, and five-second heartbeat. The parent stops a child past its time or memory limit, and reports the stage it had
  reached. A journal of finished files lets a stopped run carry on, and `anon` turns the
  batch's reports into one with no names in it.
- The CLI prints losses to stderr and exits 0 on success, 1 on a conversion error, 2 on a
  usage error. With `--json`, `convert` and `inspect` print one report on stdout instead, with
  stable field names (`docs/json.md`). The writer records a structured outcome in the `Report`
  (mode written, the history's check, each feature's status) that the JSON is built from.

## Never crash

Hostile input comes back as an error, never a panic: workspace lints deny `unwrap`,
`expect`, `panic!`, `todo!`, `unreachable!` and slice indexing outside tests; `unsafe` is
forbidden. Inputs are size-checked before opening (`MAX_INPUT_BYTES`), cadmpeg decodes
run under its `DecodePolicy` resource limits (salvage mode, desktop ceilings), and a panic
inside cadmpeg is caught wherever Dutchman calls it (decoding, STEP restatement, cutting a
component's geometry out). A test feeds every fixture, cut short and corrupted in several
hundred deterministic ways, through the whole conversion and requires a design or an error
each time, never a crash and never a partial file. Release builds therefore keep `panic = "unwind"`.
Output is encoded in memory and written only when the whole conversion succeeds.

## v0.1: one format, end to end

Fusion `.f3d` parts convert to a SolidCraft design holding the model as one non-parametric
import, which behaves like any STEP import in SolidCraft. History, sketches, parameters and
appearances are reported as losses. Tests use synthetic fixtures only: CADIR built in code
and synthetic files from cadmpeg's public test suite (provenance in
`crates/dutchman-core/tests/fixtures/README.md`). No customer or downloaded models.

## After v0.1

- **History** (v0.2, in progress): partial revolves once CADIR states the
  turning direction; more blend inputs once their pre-feature topology can be proven.
- **Components**: designs that define and place their own components (Fusion) convert as
  SolidCraft components when cadmpeg says which bodies each owns, components inside components
  as sub-assemblies; see fusion.md. Each component's part model is cut from the decoded
  document without copying its model or native records, which hold most of a large archive.
- **Assemblies**: Inventor assemblies, with their sub-assemblies, convert as placed geometry
  (each file read once per assembly, one SolidCraft component per part or sub-assembly, one
  occurrence per placement; see inventor.md). Each part keeps its own feature history when it
  maps and checks out as a single part's would: its features are authored in its component,
  their ids moved into the design's, and parameters whose names another part already uses are
  renamed with the component's name (`Second_d1`) wherever they are used; so are bodies whose
  names another part already uses (`Body (Second)`), so each keeps its own colours. Fusion
  joints become SolidCraft rigid, revolute or slider joints, the parts they join placed where
  the joints put them (see fusion.md); Inventor joints wait on
  cadmpeg reading them.
- **Appearances**: body and face colours carry across: through STEP for imports; as looks for
  replayed bodies, with a planar or cylindrical face's own colour placed by a point on the face
  (other curved faces keep their body's colour). Inside an assembly the point is in the part's own frame,
  which SolidCraft applies to every occurrence of the part. Materials come next, once cadmpeg
  gives physical materials (CADIR 0.6 carries appearances only).
- More formats as cadmpeg's codecs mature. The state of each format and the upstream gaps that
  keep its history from replaying are tracked in [fusion.md](fusion.md),
  [inventor.md](inventor.md) and [sldprt.md](sldprt.md).
- The SolidCraft plugin shim.

## Decisions

Settled 2026-10-20:

1. First format: Fusion `.f3d`, cadmpeg's most complete native codec. Inventor parts (`.ipt`)
   follow through cadmpeg's Inventor codec, pinned like f3d; improvements land
   upstream and reach Dutchman by bumping the pin. Assemblies (`.iam`) come later. SolidWorks
   parts (`.sldprt`) follow the same way through cadmpeg's SolidWorks codec.
2. Dutchman ships cadmpeg's commercial-format codecs (f3d, ipt, sldprt) in its Apache-2.0
   binary; hosting them outside SolidCraft is its purpose.
3. Crates: the CLI is `dutchman`; `dutchman-cad` is reserved for a library facade.
4. Output checks: golden files block CI; the SolidCraft round-trip job does not.
5. Output form: the design container, as SolidCraft writes an import.

## FreeCAD macro writer

`dutchman-freecad` implements the same `Writer` seam for `.FCMacro`. It consumes the
SolidCraft adapter's mapped-history data through a small workspace interface, sharing
CADIR mapping and refusals without changing those history modules. It emits escaped data
and fixed Python API calls, with guarded STEP restatement reused for the original geometry.
Each placed part is rebuilt independently or kept as STEP. The macro keeps original geometry
for runtime recovery. CLI, batch, MCP and the GUI share the output adapter selector.

The experimental FreeCAD writer also retains the longest mapped history prefix and all
mapped sketches when a later feature cannot be rebuilt. They remain hidden references in
**Rebuilt history (partial)** beside the final embedded STEP body; only an exact prefix
match can replace that final shape. The SolidCraft writer still requires complete history.

## Conversion progress

`dutchman-core::Progress` accepts a `Send + Sync` callback, serializes coarse snapshots,
and holds document counts in atomics. `convert_with_progress` runs a five-second timer for
the conversion and joins it before returning. `read_with_progress` reports input, decode,
and model-check stages, and separate document completions; callers using it directly own
the timer (`Progress::run`). There is no per-entity reporting. Fusion `.f3z` member decoding
is internal to cadmpeg. Its optional observer reports actual member decode attempts,
which can repeat during composition and do not establish a unique design total.
The shared batch events forward progress to the desktop worker; isolated CLI workers use
JSON lines on stderr. The CLI formats these separately from its unchanged final reports.


Diagnostics attach a format-neutral `MemberObserver` to `Progress`. The guarded decoder
installs cadmpeg's thread-local F3Z observer around the decode call, measures each member,
and reduces errors to fixed categories. The front-end sink hashes member paths with the
anonymous report's salt before writing. Isolated workers forward private member activity
over their existing stderr pipe to that sink. Each conversion writes an append-only JSONL
stream with immediate flushes, stage/duration records, a 30-second resource heartbeat, and
an end summary; an independent timer syncs the file at least every five seconds. Default
paths, timestamp naming, retention, and unfinished-run discovery live in `dutchman` and are
shared with the desktop app. See [the diagnostic schema](json.md#dutchmandiagnostics1).



## Drawing output

`dutchman-dxf` maps the neutral drawing and semantic annotation arenas to one
ASCII R2000 DXF per page. It depends on `dutchman-core` and `cadmpeg-ir` only and
has no native-format parser. Its entity encoder accepts page-space geometry;
missing typed geometry is reported rather than projected from 3D bodies or read
from native caches. Sheet sizes, template primitives, resolved template text, view
curves, and verified dimension values come from neutral IR fields. See [freecad.md](freecad.md#techdraw-to-dxf).

`dutchman-core::read_drawings` uses the existing input limits, codec detector,
full decode, validation and panic guard. It requires no 3D body and does not
retry at geometry depth, which would remove the drawing arenas. The existing
model conversion geometry gate remains unchanged. `dutchman::dxf` plans and
encodes all selected pages, checks output paths, then writes them and builds
per-page results through `json.rs`. This multiple-file operation has its own
entry point because the existing `Writer` seam writes one byte stream. Batch
and MCP drawing conversion are not supported yet.

## Conversion choices and pre-read budgeting

`dutchman::choices` owns presets, the one coefficient table, metadata-only estimates,
and the shared memory check. Format detection uses `dutchman-core::Format`; ZIP reads
are bounded before directory allocation and do not inflate members. Front ends use
`ConvertOptions::writer_for_input` (or `convert_options_with_progress`) to prepare
Standard's history budget. Conversion checks the budget before decoding, using the
pre-read estimate and one available-memory snapshot. When only the history-free
estimate fits, it fixes the history-free writer for parts and assemblies and records
one Info reason when writing. There is no memory refusal at the map stage.
`ConvertOptions::writer()` retains options in an `OutputWriter::Configured` adapter;
`batch::run_each` prepares its inputs individually, including desktop runs and retries.
Direct low-level writer calls apply presets and switches without a source-specific budget.
Core assemblies retain transforms from before joint solving so a writer omitting
joints can restore stored placements without learning the source format. SolidCraft
controls reference sketches and joints independently; FreeCAD honours history only.

The history mapper reads CADIR wire versions 6 through 11. Version 11 includes the
fragment and reproduction contract; complete documents continue through the same
history mapping and feature checks. Unknown later versions remain refused.

Debug builds accept `DUTCHMAN_TEST_SLOW_PART=<design name>` to hold one named part's
preparation for six seconds, long enough for a real five-second heartbeat. Release builds
do not read it. The running-step GUI screenshot case uses injected snapshots at 0 s,
61 s, 120 s and 121 s through the normal card renderer in both themes.
