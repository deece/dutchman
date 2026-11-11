# Feature history

## What maps

A part that holds an imported solid becomes a base-body Import feature at its place in the
timeline. Later features keep editable history when they map. Stored geometry has no recipe
to lose: the report says, at Info level, “This part holds an imported solid; there is no
feature history to bring over”. A part made only of stored geometry is geometry without
history, with no flattened-history warning.

- When every feature in a design's history maps onto SolidCraft, the SolidCraft design keeps
  that history, editable, with the original geometry kept beside it. If `solidcraft-cli` is
  installed, Dutchman replays the history in it and compares each body's position, box, area,
  and volume with the original before keeping it. A face-count difference (SolidCraft merges faces that share a
  surface) or an open-shell warning alone does not fail that comparison. What maps, most of it from Fusion:
  - sketches: lines, arcs, circles, conics, splines, and many constraints and dimensions;
    3D sketches of lines, arcs, circles, splines and points;
  - extrudes and revolves, with their end forms;
  - holes: counterbores, countersinks, chamfered entries, and thread labels on tapped holes;
    a hole placed at several sketch points, drilled at each when its placements are exactly
    those points and all drill the same way;
    drill points on plain drilled holes, and on counterbores and countersinks for SolidCraft
    0.22.1 and newer;
  - sweeps, lofts, and coils;
  - linear, circular, and path patterns of features or bodies, body mirrors, and body
    booleans;
  - sheet-metal base flanges, edge flanges and square attached plates with their bends;
  - delete face, for faces cadmpeg resolves in the feature's own input topology: planar faces and
    cylindrical faces, each written as a point inside the face, with the heal setting kept. A
    delete-face selection cadmpeg leaves native is reported as such and stops the history there;
  - move face, as SolidCraft's offset face: planar faces cadmpeg resolves in the feature's own
    input topology and moves along their normal, each written as a point inside the face with
    the signed distance. Translations along a direction, rotations, non-planar faces and native
    selections are reported with their reason and stop the history there;
  - constant-radius fillets and equal-setback chamfers, on edges proven on SolidCraft's own
    replay of the history before them (so they need `solidcraft-cli`); a Fusion fillet with a
    tangency weight of exactly 1 maps as a circular fillet.

  The sections below have the detail, and what stops a history.
- A blocker keeps the editable prefix and resolved sketches hidden beside the visible
  original shape. Without a body-building prefix, the design keeps imported geometry and
  hidden reference sketches.
- Assemblies keep their components and placements. Fusion and FreeCAD assemblies keep their
  joints too (revolute, slider, and rigid), with their grounded parts held where they are.
  Fusion components keep their current poses, including child placements used to resolve an
  inserted design's pose. A joint that conflicts with the placements
  is kept suppressed, so no part moves when SolidCraft opens the design. Inventor assemblies
  place their parts, without joints. Sketches in the assembly's own file are kept, hidden, for
  reference.
- FreeCAD output is experimental: `--to freecad` writes a self-contained rebuild macro,
  `--to fcstd` runs FreeCAD headlessly to save a native `.FCStd` document and checks its
  solids against the original, and `--to freecad --verify` runs the same check on a macro.
- Every conversion lists what it could not carry across, as Info, Degraded, or Dropped.

## How a history is checked

A converted history is checked by replaying it in `solidcraft-cli` and comparing the bodies
and each body's position, box, area, and volume with the original. Position and box checks
allow the larger of 0.01 mm and 0.02% of the larger box diagonal, for the centre distance and
each minimum/maximum coordinate. The size term is four times the measuring mesh chord
allowance, with margin above the measured 0.0022 mm difference on a 38 mm curved body.
Volume and area keep their measuring mesh tolerances. Bodies are paired by nearest box
centre, with volume breaking ties. When SolidCraft gives no box for a pair, only area and
volume are compared. The result is in the
`history:` line of the report and in `verification.status` in `--json`:

| Status | Meaning |
|---|---|
| `verified` | The history replays in SolidCraft and matches the original. |
| `unverified` | The history was written without the check: no `solidcraft-cli` was found, it timed out, or `--no-verify`. The reason is given. |
| `partial` | An editable prefix and resolved sketches are retained in a hidden component. The imported final shape is visible. The reason names the blocker, counts kept features, and says whether retained features replayed; this does not verify the full history. |
| `flattened` | No body-building prefix could be retained. The design keeps the imported geometry and mapped sketches with resolved planes, hidden for reference. |
| `not_applicable` | There was no history to write, or `--import-only`. |


Assembly visibility reports count source-hidden placements separately from retained
editable history. An Info `visibility` note, `N editable-history component placement(s)
are placed hidden`, counts the latter across repeated and nested placements. Both use
the same bounded hidden-occurrence paths; a Degraded note reports paths left shown
past the 4096-path limit.

For a flattened design, one Info line reports `sketches: kept N sketches for reference`,
with `; M left out` when any sketches did not map or their planes did not resolve.
`--import-only` leaves out history while retaining reference sketches when the preset
keeps sketches. Use `--no-sketches` as well, or `--preset fast`, for geometry alone.

An assembly keeps the sketches of its own file the same way, hidden, in the design itself
(SolidCraft's root component), with the parameters their dimensions use; one Info line
reports `sketches: kept N sketches in the design itself, for reference`. cadmpeg does not say
which component a sketch belongs to, so a component's sketch stays where it was drawn and
does not move with the component.

Assemblies with multiple top-level designs put each design's features in a uniquely named
timeline folder, including the root's own features when present. Feature order stays the
same, and joints stay outside folders. Each folder added produces an Info note with subject
`timeline folders`: `"Name": N feature(s) grouped`. Parts and assemblies with one top-level
design get no folders.
Root-owned bodies and adjacent root reference sketches share a folder. If another design's
features separate them, they use numbered folders so timeline order stays unchanged.

When the design is written as geometry, the same `solidcraft-cli` opens it too. Faces it
cannot mesh, bodies inside out, solids without volume, and timeline errors still produce a
`Degraded: geometry: …` loss, such as
`Degraded: geometry: solid 'unit cube': 3 face(s) could not be meshed and are not shown`.
Look over those bodies in SolidCraft before relying on them. SolidCraft can merge faces that
share a surface, so a face-count difference alone produces no loss. The history comparison
uses position, box, area, and volume, so an open-shell warning there produces no loss when
those measurements agree.
Geometry-only output has no source measurements to compare against, so a solid it opens as
an open shell is still reported. Sheets remain exempt from volume and closed-shell checks.
`--no-verify` skips these checks too.

## Feature by feature

Sketch entity distances carry line lengths, point pairs, point-to-line distances, and
spacing between parallel lines (a parallel relation plus a point-to-line dimension).
Point-on-curve coincidences carry across alongside point coincidences. Resolved construction
and projected entities use the same mapping as drawn entities; unresolved external entities
cannot supply operands. Dimensions link to named parameters, whose expressions (for example
`Length` or `Head_Diameter / 2`) remain live. The stored numeric value initializes the solved
sketch; the parameter supplies the driving value on recompute. Driven dimensions remain
reference dimensions. Nonparallel line spacing and curve-only coincidences without identified
point loci remain reported losses. A point beyond a finite line's end uses an endpoint
point-pair dimension when the source value matches that distance instead of the perpendicular
distance to the line. A value matching neither measurement is reported, without reshaping
the sketch to force it. Disjoint parallel segments can likewise retain a uniquely nearest
endpoint pair when their source value identifies it. Infinite reference axes never gain
artificial endpoint dimensions.

A feature keeps the parameter that sets it: a blind extrude distance and a partial revolve
angle take the parameter the source feature owns for that value (as `abs()` of it when the
source signs it) when exactly one matches. A sketch lying parallel to an origin plane at the
height of a point of an earlier sketch, which that sketch's dimensions place, is written as
that origin plane offset by those dimensions, so a cut on the end of a screw follows its
length; the sketch's axes are turned back when the source drew them turned by a half or
quarter turn, and a sketch with other curves than points, lines, circles and arcs keeps a fixed
plane. When a selection holds loops that lie inside one another, each inner loop is a hole of the loop around it (an annulus is its outer and inner circle). A sketch
with a parameter and at most four loose points (nothing in the source fixes them) keeps each
where it was drawn, tied to the origin when it sits there and otherwise at its drawn distance
from it, so one edit cannot move it across the profile.

Cosmetic thread features carry an exact ISO metric diameter and pitch, with a blind length
or the whole cylindrical face. A missing standard or designation does not block a known
metric size: `3 mm` and `0.5 mm` become `M3x0.5` without rounding. The face must resolve to a
complete cylindrical band between two circular rims. Historical face IDs without input
surface geometry remain unresolved; Dutchman does not infer faces from opaque ID text.
Modeled threads, conflicting designations, nonmetric standards, and unresolved extents or
face geometry remain reported losses. When the source does not resolve whether the thread is
modeled, the annotation is kept with a Degraded note; helical geometry is not reconstructed.
Cosmetic annotations do not change the body's geometry.

FreeCAD signed horizontal and vertical sketch distances convert to positive dimension
magnitudes at their solved positions. Their signed parameters and expressions remain
available; a dependent magnitude parameter drives each negative distance in SolidCraft.

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
chain direction. A fillet with several radius groups becomes one SolidCraft blend per group,
in the source's order; a later group's straight edge may be shorter than the source edge where
an earlier group rounded its end. Final-body edge selections never stand in for input geometry.

Unused FreeCAD binders and datum helpers are `not_needed`, with the Info note
`helper not converted: nothing uses it`. Used datum points and coordinate systems can be
carried by resolved sketch placements, pattern axes, or another resolved datum. A used helper without the required
resolved placement or binder source geometry remains a blocker, with its reason in the report.
These Info notes remain when another feature or a replay mismatch flattens the history.
A body inside a hidden container is hidden even when its own flag says it is shown.

FreeCAD fillets and chamfers can supply input edges from their base feature's stored shape.
Those edges must belong to one hidden result of an earlier dependency, and still pass the
same replay proof. A resolved selection without that proven earlier result is refused.

Hole history includes chamfered entries and tapped holes: the hole keeps its bore and an ISO
metric thread label such as `M6x1`. A thread's hand, tolerance class, and length don't carry
across; the report lists them as Degraded. Taper, exit treatments, modeled threads, and entry
recesses combined with drill points are not supported yet, and the report names the option.
A counterbore, countersink, or chamfered entry with a drill point keeps both for SolidCraft
0.22.1 and newer. For an older target the reason is "SolidCraft holes take a drill point only
on a plain drilled hole": SolidCraft 0.21.0 ignores `tip_angle` on counterbores and
countersinks. Conversion keeps the original geometry.


3D sketches (Fusion spatial sketches) become SolidCraft sketches with 3D Sketch on, written at
their solved positions: lines, exact arcs, full circles (as two half arcs), NURBS splines and
points. A spline is written as a dense curve through the source curve with fit points along
it, so it keeps its shape but a moved fit point redraws it as a SolidCraft spline. Constraints,
links to model edges and vertices, construction styling, surfaces and source-native geometry
are not carried; each is reported with the sketch, and a curve that cannot be written leaves
out only that curve. A sweep path or guide rail picked from the sketch's curves resolves to
its wires; one given by native selection records is reported and keeps the original geometry.

Sweeps accept ordered sketch curves (planar, or curves picked in a 3D sketch), or a whole
sketch containing a single non-construction curve, for their path and optional guide rail. Taper, explicit right or round corners,
changing sections, tangent-path extension, and post-sweep simplification keep the original
geometry and report the unsupported option. Twist, end scale, and the supported orientations
remain editable; a guide rail requires the default orientation without twist or scale.

Revolves keep one-sided partial solid sweeps about the oriented source axis when the selected
profile curves can be bounded on one side of it: lines, circles, ellipses, and arcs whose
whole carrier circle lies on that side. Reversing the source axis reverses the sweep.
They also keep symmetric angles, equal angles on two sides, and full turns, including unequal
sides that add up to a full turn. An axis that is a line of the profile's sketch stays that
line; an edge or datum axis becomes a fixed line in the sketch plane. Surface revolves keep
their symmetric history short of a full turn. Unproven one-sided sweep sides, one-sided
partial surface sweeps, unequal partial two-sided angles, ends at a face, vertex, or body,
full-turn surface revolves, and faces built other than
the way FreeCAD's default (bullseye) face maker builds them stop the history, and the report
names which.

Whole-sketch hole placements exclude construction entities; explicit entity selections
can still use construction points or circle centres.

When every feature using a sketch selects regions (whole loops or trimmed) or a list of its
profile loops, curves that bound none of the selected regions or loops are written as
construction, so they do not divide them; the report counts them. That covers open segments and
lines laid across a region, and also closed loops no feature takes: a tab along part of a
selected region's edge, or a ring around a hole the region keeps. A feature that takes the whole
sketch takes every region its curves bound, which the profile table may not list in full, so it
keeps every curve. A trimmed region counts only when its boundary closes into one shape;
otherwise nothing is inferred. A piece of a line in a trimmed boundary is read as cadmpeg gives
it: the fraction of the way from the line's start to its end. A curve a feature names, and any
sketch another kind of feature uses, keep their curves as real geometry.

Selected loops that share part of an edge with another solid line (a tab taken together with
the plate it sits on) are named by a point inside each region, since SolidCraft splits the
shared edge and the loop's own curves no longer bound one of its profiles. So is a region two of
whose loops cross (a nut pocket drawn over a bolt hole, both holes of the region): its hole is
their union, and the point lies outside both.

A feature that takes a whole sketch, or a list of its profile loops, treats loops inside loops
as holes (even-odd nesting), as Fusion does: two concentric circles are a ring, not a disc. The
selection is written as whole boundaries (outer loop and its holes). Loops that touch or cross
keep the previous selection.

A selection that takes several separate regions of a sketch can arrive as one loop that crosses
itself (four openings around two crossing ribs given as one loop that follows both diagonals).
That is not mapped yet, and the report says its profile is several selected regions merged into
one self-crossing loop, to tell it from a profile whose loop is crossed by other curves of the
sketch ("a profile crossed by its own sketch's curves is not mapped yet"). The self-crossing
test covers lines, arcs and circles; a loop with other curve types gets the second message.

A selection cadmpeg keeps native (it names no profile Dutchman can read) maps only when the
region is certain. In a sketch with one profile loop that is the whole sketch, unless other solid
curves of the sketch divide that loop into several regions: the source picked one of them, and
cutting or adding the whole sketch would take them all. That case stops the history there ("a
profile cadmpeg does not resolve, in a sketch whose curves divide its one profile into several
regions, is not mapped yet").

Patterns can repeat earlier features or named bodies in one or two linear directions,
around an axis, or at fixed spacing along curves of an earlier planar sketch. Explicit
linear offsets and circular angles map when their intervals are equal; two linear composite
stages map to a rectangular grid. Composite feature patterns containing resolved mirrors
map to an ordered chain: each stage repeats the preceding stage's entire instance set.
Chained stages require a target of SolidCraft 0.25 patch 1 or newer; older targets report
`needs SolidCraft 0.25 patch 1 or newer` and keep the original geometry. Single-stage mirrors
and patterns still map on every supported target.
Full-turn circular angles that repeat the seed endpoint keep only the distinct locations.
Composite mirror chains allow up to 32 stages; patterns are limited to 10,000 instances,
including the product of the counts in a chain. For mirror chains and repeated full-turn
endpoints, an attempted replay that cannot finish keeps the original geometry and reports
why; turning verification off, or having no usable CLI, still writes unverified history. Nonuniform or
skipped instances, progressive scaling, face seeds, and unresolved body or path references
stop editable body-building history with a specific reason. This CADIR version has no solid-pattern symmetry
or extent fields, so Dutchman cannot recover those settings independently of the decoded
spacing and locations.

Sketches retain bounded parabolas and hyperbolas, elliptical arcs up to one turn,
and rational quadratic Béziers as editable conics. Equal-weight rational splines become
polynomial control splines. Uniform periodic (closed) B-splines keep their exact shape
through endpoint knot insertion, becoming clamped control splines with a shared seam point.
Weighted periodic splines with unequal weights are not supported yet because SolidCraft
control splines have no weights. Clamped splines with nonuniform spacing or repeated interior knots become chains of exact
Bézier spline pieces, up to 200 pieces, sharing their endpoints. Uniform splines stay one
spline. Profiles and sweep paths select the entire chain; constraints that cannot apply to
a chain are left out with an Info message. A loft whose section or guide rail uses such a
chain is not mapped yet: more edges in a section would make a different lofted surface.
A loft section that takes several loops of one sketch (the whole sketch, or a list of its
profiles) is not mapped yet either: the source can loft the filled outer loop where the loops
read as a ring, so whether an inner loop is a hole is not known.
Selected multi-piece elliptical arcs, unbounded conics, other weighted splines,
and text without solved outlines still prevent history replay.
Additional coordinate, intersection, tangency, curvature, symmetry, and repeated-dimension
relations carry across. Static sketch patterns retain point and radius associations;
driving count or span parameters are not supported yet. Constraints that cannot carry
across are counted in a `Degraded` sketch loss with a specific reason; they never by
themselves flatten a sketch whose solved geometry maps.

FreeCAD numeric spreadsheet cells and supported formulas become parameters, with aliases
or names such as `Sheet_A1`. Qualified object properties bind to their resolved parameters;
renaming keeps those links. Text cells are omitted silently, and unused numeric cells get
an `Info` note. Unsupported functions, conditionals, ambiguous references, and cycles retain
available scalar values with a specific reason. See [FreeCAD expressions](freecad.md#spreadsheet-parameters-and-expressions)
for decoder limitations, including named sketch constraint bindings. Verification checks
the initial geometry; check geometry again after changing a parameter.

## Sheet metal

Sheet-metal history includes base flanges (not of symmetric thickness) and full-edge flanges
on an unchanged rectangular base sheet. An edge flange needs its own CADIR input topology
with edge midpoint, tangent, and end vertices; each midpoint must identify exactly one edge
of the rebuilt base sheet. Fixed outer-face heights, angles between zero and 180 degrees,
inside/outside bend placement, and positive bend radii map. Inner-face heights, partial widths,
object-ended heights, adjacent/tangent bend placement, and picks on already modified sheets
keep the geometry and report the reason. Final-body edges alone never select a flange.
An attached plate with its paired bend maps to one SolidCraft edge flange (the bend is reported
as mapped with its plate) when the plate is one rectangle that spans the whole proven input edge,
stands square to the sheet at 90 degrees, is flush with the edge's end face, has the sheet's
thickness and starts where the bend ends; the flange height is the plate's far end measured
from the sheet's outer face. Any other plate, or a bend without its plate, keeps the geometry
and reports the unsupported shape. Hems keep geometry: CADIR specifies developed length while SolidCraft requires length to the
outer face, and rolled, teardrop, and unresolved hem forms are not supported yet. CADIR has no
contour-flange, bend, unfold/refold, or corner-relief operations, nor K-factor or relief
settings; sheet rules keep SolidCraft's defaults for those settings. The report names the
first feature that stops the history; the original geometry is still converted.

## Partial history

Show **Editable history (partial)** in the browser to keep editing its body, and hide the imported final shape while you work on it.

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

When a partial history is checked against the source's stored bodies, each kept body must
sit where the source has it: within the larger of 1 mm and 2% of its box diagonal when the
source's position is approximate, or 0.5 mm and 1% when it is exact.

For targets of SolidCraft 0.30 or newer, partial and unverified editable-history copies
use **Hide its History**: their features stay in the Browser and keep rebuilding, but
are left out of the History bar. Use **Show its History** on the component to restore
them. Older targets keep these features in History. A kept history of base features
alone has nothing to edit, so in an assembly no hidden component is written for it.

A writer that can never run the check (the SolidCraft importer plugin, which runs Dutchman
in a sandbox with no `solidcraft-cli`) keeps a whole unchecked history the same way, in a
hidden **Editable history (unverified)** component beside the visible original, with
`mode: partial_history` and verification `unverified`. In an assembly each part with a mapped
history is written that way, inside the part's own component.
