# FreeCAD documents (`.FCStd`)

FreeCAD documents convert through cadmpeg's `fcstd` codec. Checked in SolidCraft, all 19
of the test documents (pads, pockets, revolves, holes, fillets, chamfers, shells, and a
three-part assembly) come out at FreeCAD's volume. Before 0.1.5 FreeCAD documents converted
in the nightly builds only: cadmpeg 0.6.3 stored some faces as replicas of curves, kept each
feature's intermediate shape as a body of its own and could turn revolved faces inwards.

When a feature blocks SolidCraft history conversion, earlier editable features and resolved
sketches are kept in a hidden **Editable history (partial)** component. The original final
shape remains visible as an Import after the kept history. Show the hidden component to
continue editing it. The report names the blocker and counts retained source features;
`mode: partial_history` and verification `partial` distinguish this from full verified
history. The reason says whether retained features replayed or remain unverified. Later
sketches with unresolved planes or external geometry are reported and left out. A sketch
that fails replay is retried without constraints, with the loss reported. If there is
no body-building prefix, resolved sketches remain hidden beside the imported shape for
reference. A final shape mismatch preserves the mapped timeline for correction when it
replays; it does not identify which feature needs correction. SolidCraft's numerical eval
includes hidden bodies as well as the visible final shape.

## What converts

- **Geometry**, written as a SolidCraft import and checked in SolidCraft like any other.
  Only the bodies FreeCAD shows convert: a body cadmpeg marks hidden (`visible: false`, as it
  marks the shape of each step inside a PartDesign Body) is left out, with the geometry only
  it uses, and the report says how many were (`Info`). A curve or surface the shown bodies use
  keeps its construction, such as the full ellipse a trimmed elliptical edge is cut from, even
  when that construction is stored with a hidden step. A document that hides every body
  converts all of them, as stored.
- **Components**, when cadmpeg says which bodies each owns, as for a Fusion design.
  External `App::Link` references to Bodies and Parts in other `.FCStd` documents convert
  as geometry in component occurrences at their stored placements, including assemblies with
  no geometry in the assembly document itself. Link arrays place each element once.
  Dutchman looks for the referenced file beside the assembly first, then uses the shared
  [project and search folders](inventor.md#finding-the-part-files) (`--project`, `--search`).
  A missing or unreadable linked file is left out and reported by name; the other components
  still convert. Cyclic links are left out and reported, with nesting limited to 32 levels.
  Links that name only a document identifier, or select subelements, are not supported yet.
- **Grounded joints** hold their part fixed to the design where it is stored, as a rigid
  joint to the design; the assembly's revolute and slider joints keep their motion.
  Assembly-workbench joints use the same mapping as Fusion joints when cadmpeg resolves
  their operands and frames. Joints that cannot be carried are reported; the parts keep
  their stored placements.
- **Feature history**, when every feature maps: sketches, pads, simple, counterbored and
  countersunk holes, constant-radius fillets, equal-distance chamfers, and revolutions,
  replayed and checked in SolidCraft like a Fusion history. One-sided partial solid
  revolutions keep their sweep side, including `Reversed`, which cadmpeg folds into the
  axis direction, when the selected profile bounds prove one side of the axis (see
  [revolve coverage](guide.md)). Thicknesses (shells), counterbores or
  countersinks with a drill point, and unproven revolution sweep sides stop the history;
  the original geometry stays visible, and the report names the first blocker.

Hidden container visibility takes precedence over a child's own shown flag, including
nested Parts, groups, and PartDesign Bodies. A body hidden only by its source container can still supply a separately shown occurrence.

Unused binders and datum helpers do not block feature history. They are reported as
`not_needed`, with an Info note: `helper not converted: nothing uses it`. Suppressed later
features do not count as consumers. A used datum point or coordinate system needs no
separate feature when the dependent sketch carries its resolved plane or a pattern carries
resolved axes. Resolved datum constructions can carry earlier helpers in the same way.
Other uses keep the history flattened with a specific placement reason.
A simple binder over a resolved construction plane can likewise be carried in its attached
sketches. A binder used by a later feature remains a blocker when its source geometry
reference is not resolved; the report says `a binder used by a later feature is not mapped yet`.

MultiTransforms containing resolved mirrors require a target of SolidCraft 0.25 patch 1 or newer
when a stage patterns or mirrors an earlier stage. Older targets report
`needs SolidCraft 0.25 patch 1 or newer` and keep the original geometry. Single-stage mirrors
and patterns still map on every supported target. These MultiTransforms keep their stage
order as a chain of SolidCraft
mirror and pattern features. Each stage repeats the complete instance set from the previous
stage. Full-turn polar patterns with a repeated seed endpoint keep every distinct location
once. Replay checks body count, volume, and area against the stored original; a chain that
cannot reproduce it falls back to geometry with the replay reason. Unresolved mirror planes,
body-seeded composite mirrors, and progressively scaled stages remain unsupported.
An attempted replay that cannot finish also keeps the original geometry for these chains;
with verification off or no usable CLI, history remains unverified.

Some polar patterns referring to the document's Z axis currently receive another axis or
an incorrect endpoint from the decoder. These do not have verified editable history yet.
Replay differences and unfinished replay checks of repeated full-turn endpoints fall back
to geometry, as reported. With verification off or no usable CLI, history remains unverified.
The endpoint mapping does not repair a misdecoded axis or instance set.

Whole-sketch hole placements exclude construction geometry, including reference origin
points. Explicit entity selections can still use construction points or circle centres.

Fillets and chamfers need resolved named edges of their base feature's stored shape,
before the blend.
Dutchman preserves those edges when it removes the hidden intermediate shapes. It accepts
straight edges and open circular arcs only when their midpoint identifies exactly one edge
on SolidCraft's replay at that step, with matching ends and length. An ambiguous pick, a
missing base shape, a closed edge, or another curve stops the editable prefix, with the
reason reported. This requires a SolidCraft CLI to replay the history; `--no-verify` cannot
prove blend edges. Final-body edge numbers are never substituted for base-feature edges.

FreeCAD signed horizontal and vertical sketch distances convert to positive dimension
magnitudes at their solved positions. Their signed parameters and expressions remain
available; a dependent magnitude parameter drives each negative distance in SolidCraft.

## Spreadsheet parameters and expressions

Numeric spreadsheet cells become editable SolidCraft parameters. Aliases keep their names;
unaliased cells include their sheet name, with punctuation replaced by underscores
(`Sheet.A1` becomes `Sheet_A1`). Formula cells can remain live without a saved evaluated
value. Text and empty cells are omitted silently. An unused numeric cell is retained with
an `Info` note; the spreadsheet grid and formatting are not reproduced.

Expressions such as `Parameters.plate_w / 2` bind to the parameter owned by the named
object, including cell addresses and aliases. Renamed parameters remain linked through
formulas. Length units such as `mm` and `in`, angles in `deg`, arithmetic, `pi`, and supported
functions such as `min`, `max`, `abs`, `sqrt`, and degree-based trigonometry remain live.
Unsupported functions, conditionals (`? :`), unresolved or ambiguous references, and cycles
use the saved scalar value when available, with a specific reason. A formula with neither
a supported expression nor a scalar value is dropped with that reason.

Only bindings supplied by the decoder can be restored. Named sketch constraint bindings
saved as `.Constraints.width` are not projected by the current decoder; their dimensions
retain their evaluated values. Non-sketch feature dimensions such as pad depth are fixed
scalars in the current neutral model, without parameter links to their geometry inputs;
a live formula in the parameter table alone cannot drive those dimensions. Object-label
references also need an unambiguous decoded label identity. A verified initial volume checks the saved shape; it does not prove that
all source dependencies remain editable. Check the resulting geometry after editing a
parameter. A flattened design retains only parameters needed by its reference sketches.

Constant-radius AdditiveHelix and SubtractiveHelix features keep their sketch profile,
axis, pitch/height/turn law, handedness, reversed direction, and join/cut operation as
sketch-profile coils when the target is SolidCraft 0.25 patch 1 or newer. Choose a target with
`--solidcraft-version x.y.z` on `convert`, `convert-all`, or `watch` (at least 0.18.2).
The default is the found CLI's version, even with `--no-verify`, else the newest release
known at build time (currently 0.40). The rolling 14-day window is currently
0.18.2–0.40. Older targets block the helix with
`needs SolidCraft 0.25 patch 1 or newer` and the target version, preserving the original
geometry. Resolved feature parameter
bindings drive the coil dimensions; dimensions without bindings keep their values.
Spreadsheet parameters follow the usual expression/value fallback. Conical helices and
flat spirals are not supported yet: SolidCraft's sketch-profile coil has no cone angle or
radial growth. Replay verification still applies, including fallback when a profile or
Boolean operation cannot rebuild.

## Periodic sketch B-splines

Uniform periodic (closed) sketch B-splines map exactly when CADIR supplies the full knot
vector and its corresponding poles. Dutchman inserts endpoint knots to clamp the active
parameter interval without changing the curve, and shares the seam point in SolidCraft.
Equal-weight rational splines map the same way. Clamped splines with nonuniform spacing
or repeated interior knots become chains of exact spline pieces, up to 200 pieces, with
shared endpoints. Uniform splines stay one spline. Unequal weights remain unsupported
because SolidCraft control splines have no weights.

The CC0 FreeCAD 1.1.4 recipe fixtures N04a and N04b currently stop before this mapping:
cadmpeg's sketch reader passes FreeCAD's compact periodic knots and poles directly to CADIR,
which requires a full knot vector. Native periodic sketch decoding needs normalization in
cadmpeg; Dutchman does not decode or repair native spline data. Synthetic CADIR tests cover
the mapping and a closed-profile extrusion in SolidCraft independently of that reader gap.

## Building with FreeCAD

FreeCAD support is the `freecad` cargo feature of `dutchman` and `dutchman-gui`, on by
default. A build without it says it leaves FreeCAD files out:

```sh
cargo build --release -p dutchman -p dutchman-gui --no-default-features
```

## Testing without FreeCAD

The FreeCAD fixtures in `crates/dutchman-core/tests/fixtures/` are cadmpeg's corpus
documents, authored in FreeCAD 1.1.1 by the cadmpeg project and dedicated to the public domain
(CC0-1.0). See that folder's README for where each came from.

## Writing rebuild macros

`--to freecad` writes a `.FCMacro` independently of `.FCStd` input support.
`--to fcstd` runs FreeCAD to save and verify a native document; `--to freecad --verify`
checks a macro without saving a native document. See
[FreeCAD output](#freecad-output-experimental) for coverage, fallback, and execution.

The writer is Rust and emits calls to FreeCAD as an outside Python program. It does not
link or vendor FreeCAD. Its interface references are the public
[Sketcher scripting](https://wiki.freecad.org/Sketcher_scripting),
[Part scripting](https://wiki.freecad.org/Part_scripting),
[Pad](https://wiki.freecad.org/PartDesign_Pad),
[Revolution](https://wiki.freecad.org/PartDesign_Revolution),
[Linear pattern](https://wiki.freecad.org/PartDesign_LinearPattern),
[Group](https://wiki.freecad.org/Std_Group) documentation, and the
[public API reference](https://freecad.github.io/API/). No FreeCAD implementation source is
needed to generate or test the macro text.

Partial rebuilds retain the longest mapped prefix and every mapped sketch in a hidden
**Rebuilt history (partial)** group. When that prefix differs from the final body, the
embedded STEP is the visible result and the report says `rebuilt N of M features; kept
as reference`. Only the final shape participates in verification; reference features do
not add bodies or volumes. This output remains experimental; it is verified only when FreeCAD runs it headlessly
(`--to fcstd` or `--verify`).

The synthetic macro regression tests can also run in a prebuilt FreeCAD command-line
binary when `FREECAD_CMD` names that executable, including a final-volume check beside a
hidden partial rebuild.

Assembly output also follows the public [Assembly workbench](https://wiki.freecad.org/Assembly_Workbench),
[joint properties](https://wiki.freecad.org/Assembly_CreateJointFixed), and
[grounding properties](https://wiki.freecad.org/Assembly_ToggleGrounded) documentation.
The Assembly object, the `JointObject.Joint` and `JointObject.GroundedJoint` constructors, and
`solve()` were exercised through the installed FreeCAD 1.1.4 runtime on synthetic parts.
Only runtime calls, properties, and signatures were inspected; no FreeCAD source was read.
The unit tests include joint data assertions and a hinge golden. Set `FREECAD_CMD` to a
FreeCAD command-line executable (or a wrapper accepting `-c CODE`) to run the synthetic
macro checks for every mapped joint family, including hinge and slider motion about their
own axes, a sub-assembly solved inside its assembly, grounding, limit clamping, and volume
preservation. Joint coverage and specific fallback cases are listed in the guide.

Headless measurement and saving use the public [topological data scripting](https://wiki.freecad.org/Topological_data_scripting)
and [FreeCAD scripting basics](https://wiki.freecad.org/FreeCAD_Scripting_Basics) APIs:
`Shape.Solids`, `Volume`, `BoundBox`, document `recompute()` and `saveAs()`.

## TechDraw to DXF

```
dutchman convert part.FCStd --to dxf
dutchman convert part.FCStd --to dxf --page Page -o front.dxf --json
dutchman convert part.FCStd --to dxf -o existing-directory
```

One ASCII DXF R2000 (`AC1015`) is written per page with `$INSUNITS=4` (mm) and
metric measurement. Names are `part-<object name>.dxf`, in source page order;
`--page` uses the source object name, not its displayed label. Names are limited
to 100 ASCII letters, digits, underscores, and hyphens; other characters become
underscores and collisions get numeric suffixes, also on case-insensitive systems.
A selected page with an explicit file path uses that exact path. An unselected
file path supplies a base: `-o drawing.dxf` writes `drawing-Page.dxf`, etc.
An output directory must already exist. Defaults need `--force` to overwrite;
explicit outputs replace unless `--no-clobber` is set. All pages are encoded and
existing outputs checked before publication; an I/O failure while publishing
multiple files can leave earlier pages written. A document without a TechDraw
page returns a clear error. Pages without a 3D body can be exported.

| Source IR | DXF and report |
|---|---|
| Page sheet | Millimetre page limits; page coordinates are written at 1:1. |
| Page `Views`, collection `Group`, and template links | Determine page membership. |
| View curves | Exact lines, circles, and arcs, scaled and rotated about the view origin. Each view has a layer; hidden curves use a HIDDEN line type, and center lines use CENTER. Cosmetic curves have separate layers. |
| Dimension measurement, kind, replacement text, tolerances, and reference anchors | Labels on DIMENSIONS, with verified measurements in mm or degrees. Linear and radial anchor pairs generate conventional lines and open arrows; these are not the saved source layout. Angular dimensions and missing anchors keep text only, with a loss; dimensions are not associative DXF DIMENSION entities. |
| Template primitives and resolved text | FRAME geometry and TITLE_BLOCK text, directly in page coordinates. Unpositioned editable fields are reported. |
| Annotation text, position, and text style | One TEXT or MTEXT entity per nonempty line on ANNOTATIONS; blank line spacing is preserved. Neutral physical height is kept, with 2.5 mm as fallback and line spacing at 1.4 times height. Font family, alignment and rich styling are reported as losses. |
| Drawing visibility, with agreed `ViewPresentation.visible` as fallback | Hidden objects are left out with Info notes; conflicting flags are Degraded; page UI visibility does not filter selected pages. Curve display visibility is separate from its hidden-line class. |
| Unsupported objects, resources, parameters, and relationships | Specific losses; unassigned drawing objects are reported at conversion level when exporting every page. |

The encoder supports LINE, ARC, CIRCLE, LWPOLYLINE, exact polynomial/rational
SPLINE (degree 1–10 with explicit knots/control points/weights), and TEXT/MTEXT.
Unicode text is ASCII escaped. No model projection or native/SVG parsing is done
in Dutchman. View edges absent from the neutral model, unsupported text styling,
and dimension decoration that cannot be carried are reported explicitly.
Layer entity counts and losses appear in `pages` in the JSON report; decode
losses remain at conversion level. Declared layers can be empty.

The CC0 fixtures include the original synthetic pages made with
`crates/dutchman-core/tests/fixtures/techdraw.py` and cadmpeg's documented
TechDraw corpus. Tests read committed files and require neither FreeCAD nor a
Python DXF reader. No CAD implementation source is used for drawing mapping.

## FreeCAD output (experimental)

**Experimental.** The macro, its options and what it rebuilds may change between releases,
including the headless verification result. Keep the original design for comparison.

Write a self-contained FreeCAD rebuild macro instead of a SolidCraft design:

```sh
dutchman convert in.f3d --to freecad -o out.FCMacro
```

The positional output path still works. `convert-all --to freecad --out-dir DIR` writes
`.FCMacro` files; the desktop app's **Output format** choice offers **FreeCAD macro** too.
This output works independently of the `freecad` feature used to read `.FCStd` inputs.

The macro makes a Body for each placed part, preserving nested assembly placements. It
uses Dutchman's existing history mapping and timeline order to rebuild planar sketches
(lines, circles, arcs, ellipses, and supported B-splines), constraints, pads and pockets,
full-turn or symmetric revolves, unguided lofts, sweeps, simple holes, linear and circular
feature patterns, spheres, tori, and coils. Numeric feature values are retained. A parameter
object keeps evaluated scalar values and source expressions as text; source expressions
are never executed. Coils are rebuilt with Part geometry and stored as a feature, with
numeric properties; editing those properties does not recompute the coil.

When a later feature cannot be rebuilt, the macro keeps the longest mapped prefix and
all mapped sketches in a hidden group named **Rebuilt history (partial)**. The embedded
STEP remains the visible final shape when the prefix differs; reference features are not
included in the final shape's measurements. The report says `rebuilt N of M features;
kept as reference` (sketches count as features; suppressed and organizational features do
not). Later body-building features are blocked, with their reasons; later mapped sketches
remain hidden references. Partial histories are still experimental and unverified.

A part whose history cannot be rebuilt is kept as an embedded STEP solid or surface in a
Body. Examples include unresolved source selections, intersect operations, fixed-orientation,
guided, twisted, scaled, or binormal sweeps and lofts, lofts ending at a point, extrudes from
or to a point or face or with taper, counterbored, countersunk, or face-ended holes, coils
about an axis other than Z, two-direction patterns, and histories spanning multiple bodies. The report gives one result per source feature, including the reason each feature
was not rebuilt. `--import-only` keeps every part as STEP and leaves out joints.
Assemblies retain placements and rebuild supported joints as described below.
Pattern counts are limited to 10,000. Solid lines drawn corner to
corner across a profile taken whole (a centre rectangle's diagonals, when the source does
not mark them construction) are left out of the padded profile, which is the same region.
Geometry whose STEP uses replicated trimming curves (`CURVE_REPLICA`) is refused, because
FreeCAD 1.1's STEP reader crashes on it.

Assembly workbench joints are experimental and exercised with FreeCAD 1.1.4. The fixed,
revolute, and slider joints the SolidCraft output writes are written to FreeCAD too, with the
same resolved parts, connector frames, frame sense, angles, and limits, through
sub-assemblies as well. Between parts placed directly in the assembly, cylindrical, ball,
distance, parallel, perpendicular, and angle joints are also written; axis coincidence uses
a cylindrical joint, and an insert uses a revolute joint with its plane offset. Their
attachment offsets are kept, angular limits use degrees and linear limits millimetres, and
either bound may be enabled alone. FreeCAD's static solve preserves free coordinates; limit
properties are kept for the workbench rather than used to drive a pose during conversion.
For a single revolute joint with one of its parts grounded, the macro also provides
`drive_joint_angle(assembly, joint, degrees)`, which clamps the requested angle to the
stored bounds and solves the other part's new pose. Use the workbench's simulation tools
for chains and loops.

A sub-assembly whose parts are joined gets its own Assembly, solved before the one that
holds it; FreeCAD moves a sub-assembly as one body in its parent, so a moving joint from a
part outside it to a part inside it turns the whole sub-assembly, with a note. Explicit
grounding is retained. In each assembly, a joint chain nothing grounds gets one inferred
grounded part (the second side of its first joint, as in the SolidCraft output), named in
the report. Suppressed joints remain suppressed. External CADIR operands, subelement
operands, joints between a sub-assembly and a part inside it, suppressed grounding, limits
on fixed joints, driving parameter expressions, rack/pinion, screw, gear, and belt
couplings, plane coincidence, point-on-axis, tangent, and native joint families get
specific report notes. Their stored placements are kept.

The macro checks the solver result and every part's stored pose. If the Assembly API is
unavailable, solving fails, or a part moves during reconstruction, it removes the new
joints and puts each part back where it was placed, as the macro does without joints.
FreeCAD's report view gives the reason. Conversion reports remain unverified until the
macro runs.

Open the macro in FreeCAD with **Macro > Macros… > Execute**, or run:

```sh
freecadcmd out.FCMacro
```

The macro creates a new document. Save it as `.FCStd` from FreeCAD, or have Dutchman
run FreeCAD headlessly and write the document directly:

```sh
dutchman convert in.f3d --to fcstd -o out.FCStd
dutchman convert in.f3d --to freecad --verify -o out.FCMacro
```

`--to fcstd` always checks the result. `--to freecad --verify` checks the macro without
saving a native document; the macro remains self-contained. Both compare each part's
resulting solids with its embedded source STEP: solid count, volume (relative tolerance
0.01%, the same as SolidCraft), and bounding-box coordinates (0.01% of the part's span,
with a 1 mm minimum scale). Assemblies are compared in each part's local coordinates.
The report says `verified` when every part matches, or `differs` and identifies the part.
A runtime rebuild failure restores embedded STEP and reports the specific reason as a
loss and a `differs` result, even when the restored solid matches. Sheets, wires, and other
parts without solids are reported as not checked rather than verified.

Discovery uses `FREECAD_CMD` (one executable path, or the Flatpak form
`flatpak run --command=FreeCADCmd org.freecad.FreeCAD`; no shell is involved),
then `freecadcmd` or `FreeCADCmd` on `PATH`, the standard `/Applications/FreeCAD.app` or
`~/Applications/FreeCAD.app` bundle's `Contents/Resources/bin`, then an installed
`org.freecad.FreeCAD` Flatpak. Flatpak receives access only to the temporary execution
folder in addition to its existing permissions. `--freecad-timeout SECONDS` defaults to
300; the process is killed on timeout. Native output fails when FreeCAD cannot run or
produce measurements. A macro still writes successfully with verification `unverified`
and reason `not checked: FreeCAD not found` when discovery finds no installation.
`convert-all` forwards these options to each conversion worker.

No FreeCAD installation is needed to generate a macro without `--verify`. Original STEP
geometry remains in the document, hidden after a successful rebuild. The macro compares
rebuilt geometry with STEP; a rebuild failure or difference restores the original and
prints a warning in FreeCAD's report view. Temporary execution and STEP files are removed.

