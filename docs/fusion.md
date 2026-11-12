# Fusion: what converts today, and what stands in the way

Dutchman reads Fusion archives (`.f3d`) through cadmpeg's Fusion codec,
cadmpeg's most complete native reader. This page says what Dutchman makes of what that codec
gives and which upstream gaps remain. Improvements land in cadmpeg and reach Dutchman when the
pin is bumped.

State at the cadmpeg commit pinned in `Cargo.toml` (`6c775550`, cadmpeg 0.6.11).
More thread and hole forms, healed Delete Face operations, shell wall selections, loft
sections and rails, circular pipes, and pattern placements decode for history mapping.
More extrude profiles and face ends, fillet edge selections, body moves, and component
placements resolve. Mapping still depends on the resolved operands each feature needs.
Sketch names follow their timeline sketch features.

Historical body centres use bounded samples along curved edges as well as vertices and
proven circular extrema. cadmpeg identifies incomplete edge or topology evidence separately;
only bodies certified as exact have exact bounding-box centres.

## Parts and history

As-built joints build no geometry and do not stop the part's feature history.

Bodies a Combine used up as a tool, or that a Remove step took out of the design, are left out when cadmpeg marks them hidden and no component owns them; an Info `bodies` note gives the count. A body an active Remove deletes is left out too, even when cadmpeg lists it as shown, as long as cadmpeg resolves which body the Remove names; an Info note on the Remove gives the count. Bodies you hid yourself stay available, and a part with both shown and hidden bodies is shown whole, with an Info note.

| What the codec gives | What Dutchman does |
|---|---|
| The design's B-rep, with body transforms applied. | Converts it: restated as STEP, written as SolidCraft's import. |
| A design saved while Fusion was offline, which records no cloud lineage. | Reads it like any other design. |
| A Base Feature that holds a stored solid (no recipe), typed as a base body. | Writes it as a SolidCraft Import at its place in the timeline; later features keep their history. cadmpeg gives a base body only as the design ends. Later joins and cuts on it re-apply without changing the shape, so they are kept but counted as re-applied, not rebuilt; a Base Feature whose body any other later feature changes (a Move, a blend, a deleted face) is refused, naming that feature. A Base Feature proven to hold no bodies is not needed. |
| A Canvas: a reference image placed in the design. | Needs no feature: it builds no geometry, so it is counted as not needed and doesn't stop the history. |
| Stored geometry from any format (`stored_geometry`) with output bodies, solids and sheets. | The same Import, holding exactly the bodies it outputs. |
| A mesh body whose container cadmpeg cannot decode. | Leaves it out and reports it as an undecoded mesh body. |
| Parameters, planar sketches with typed and native constraints. | Mapped, including point-on-curve coincidences, point-to-line distances, resolved finite-endpoint distances, and parallel-line spacing. Dimensions keep their parameter expressions, so user parameters drive the editable geometry. Native constraints without neutral operands are reported as not carried. |
| Spatial (3D) sketches. | Mapped as SolidCraft 3D sketches at their solved positions (lines, arcs, circles, splines, points); constraints and links to model edges are reported as not carried, and a later sweep path or rail picked from their curves follows them. |
| Cosmetic thread annotations with diameter, pitch, extent, and a resolved cylindrical face. | Writes a cosmetic ISO metric thread, including a blind length or the whole face. An absent designation is derived exactly from diameter and pitch; conflicting or nonmetric designations, modeled threads, and unresolved faces remain reported losses. Unspecified construction keeps the annotation with a Degraded note. Historical face IDs without input surface geometry remain reported losses. |
| Typed extrudes, revolves, fillets, chamfers and more, with selections often native. | Extrudes, revolves, lofts, sweeps, spheres, tori, coils and whole cylinders map, and whole cones on SolidCraft 0.24.1 and newer (see ARCHITECTURE.md for the cases); boxes wait on cadmpeg saying where a box's placement is; simple, counterbore and countersink holes drilled at a sketch's points map; constant-radius fillets and equal-setback chamfers map from proven feature input edge geometry, matched on SolidCraft's own replay of the history before them (a fillet with a tangency weight of exactly 1 maps as a circular fillet, with an Info note; any other weight is refused); other blends and holes placed on unresolved faces wait on input geometry (see guide.md). |

Fusion stores consumed Combine tools (with keep-tools off) and bodies removed by the
timeline alongside its live bodies. For `.f3d`, designs inside `.f3z`, and component parts,
Dutchman leaves out bodies cadmpeg marks hidden only when no CADIR product definition lists
them in its `bodies`. Their exclusive geometry is left out too, with one Info note giving
the count and reason. History remains available; a reference that can no longer map is
reported as a history blocker.

User-hidden bodies that still belong to a component are kept, with an Info note. Hidden
occurrences remain hidden; the current writer shows a part with both shown and hidden bodies
whole, so its hidden members can appear visible in SolidCraft.

When every active feature using a sketch selects explicit whole-loop regions, unused lines
crossing selected whole circles are kept as construction geometry. This preserves
layout lines through hole centres without splitting the circular holes during replay. If
another active consumer needs the whole sketch or a different selection form, those lines
stay solid.

Resolved `to_plane` extrude ends map before any body-building feature when the terminating
plane is parallel to the sketch. The plane origin becomes a fixed SolidCraft To Object point;
a construction plane or body face need not exist there. After earlier geometry, `to_plane`
ends are not supported yet: SolidCraft can reinterpret a point on an existing face as a
different end, and CADIR supplies no input face geometry to prove which face exists then.
Final-body faces cannot establish that timeline state. Tilted plane ends and native,
generated, or historical vertex selections are not supported yet. Symmetric To Object ends
and non-blind ends against the sketch normal on two-sided extrudes remain unsupported.

An extrude whose profile is a face of the model as it was earlier in the timeline
(`historical_faces`, "extrude this face") is rebuilt from that face's boundary. CADIR carries
no surface for such a face, but it keeps the ends, midpoint and tangent of every edge around a
selected face in the feature's own input topology. Dutchman reads those edges as straight lines,
circular arcs and full circles in one plane, chains them into closed loops (a loop inside a loop
is a hole) and writes a sketch of them on that plane, ahead of the extrude, taking every
profile. The extrude runs along the face's outward normal, which only the body knows: after
SolidCraft has replayed the history before the extrude, Dutchman finds the face among the
rebuilt faces (same plane, same area, every boundary edge on it and named for it) and mirrors the
sketch if it was written on the inner side. A face it cannot find exactly once, an edge that is
not a line or circular arc, a boundary that does not close or is not flat, an input topology
without edge geometry for the face, a direction of its own, and ends to or from an object
leave the extrude unmapped with the reason. Faces that share an edge are refused. Without a
SolidCraft CLI to replay the history, the extrude waits for one like a blend does.

## Appearances

Body and face appearance bindings follow the component definition's own BREP ids into its
written geometry, including imported and stored solids. Every placement inherits that
geometry's looks. Complete xref and instance identities are kept distinct through the shared
component index. Saved wire edges beside a solid do not consume its imported body name or
discard its appearance. When STEP omits an unwritable sibling body, Dutchman checks each
body separately, for groups of up to 32 bodies, before keeping the remaining body names
and colours. These solids keep their existing STEP import names, including numbered
siblings; matching their colours does not rename them or change other components' names.
A body binding supplies
the body colour, and face bindings override it per face.
When a body has no body binding and all its faces have the same resolved appearance,
Dutchman writes one body colour, which SolidCraft shows in its body appearance controls.
Mixed appearances remain face looks, selected by interior points in the component's
coordinates. Resolved analytic and NURBS faces use bounded trim-curve sampling in surface
coordinates, with spatial edge evaluation when parameter-space curves are unavailable.
Planes, cylinders and cones also have analytic picks when a trim chart is unavailable.
A projected trim must stay within
0.002 mm of its support, and its pick must clear the sampled boundary by eight times that
deviation. Cylindrical and conical trim charts account for periodic seams and holes;
narrow torus bands also check their trim orientation and retry across the minor-angle seam.
Sampling preserves exact knot endpoints. Spherical corners bounded by three minor
great-circle arcs use an oriented interior pick across the pole. Disconnected charts and
carriers that cannot be evaluated retain a Dropped note.
Resolved trim charts take precedence over vertex-only picks, so a cylindrical face keeps
its own long arc and a planar appearance cannot pick through a hole in its outer boundary.
Imported body names are made unique across the design so an unpainted namesake in another
component cannot receive a painted body's appearance.
When a partial or unverified editable history is hidden beside the final shape, the
visible final-shape Import receives its own names and colours. Names remain distinct
from those of the editable copy.
When a partial history is kept hidden, appearance losses refer to the visible final
shape rather than to final bodies absent from that partial copy.
The look keeps the resolved colour, opacity, and appearance name; it does not infer a
physical material from a visual preset.

A binding whose target or appearance cannot be resolved is reported as Dropped under
`appearance`. An unplaced binding identifies its body or face target and distinguishes a
target absent from the decoded model from existing geometry that is not placed. Missing
decoded targets need a cadmpeg decoding fix; Dutchman does not guess another instance by
matching an id suffix. Bindings to consumed or removed Fusion geometry are reported
separately when that geometry is filtered out. Faces without a resolved interior point
cannot receive SolidCraft's point-selected face looks yet; their bindings are also reported
as Dropped. A decoded body without geometry that STEP can write retains a separate note;
its writable siblings can still receive their colours.
Uniformly painted curved bodies can still use a body colour. Conflicting assignments to
one target are left out and reported. Textures and shader properties are not mapped by this
look mapping.

The pinned cadmpeg CADIR supports body and face appearance targets, but has no component or
occurrence appearance target kind. cadmpeg must project those assignments onto body/face
targets, or represent them through an IR extension, before Dutchman can map them. Source
assignments absent from CADIR cannot be reconstructed; remaining grey bodies may therefore
still need additional decoding in cadmpeg.

## Assembly memory

Reference sketches are mapped from the sketch, feature and parameter data alone, built once per
conversion: on the Voron V1.8 that takes the peak from 11.0 GB to 8.7 GB, with the same output.

## Regions of crossing circles

Two crossing circles bound several regions with the same curves, so naming the curves can't tell
them apart. A profile region that uses part of a circle is selected by a point inside it, taken
from its stored boundary: its uses' parameters must close into one ring. If they don't, or if the
readings that close disagree on the shape, the region stays selected by its curves, and SolidCraft
refuses what stays ambiguous.

## Assembly placements

**Part positions** defaults to **Smart**. For each occurrence, Smart prefers a displayed
pose bound to it by the decoder's `native.f3d.display_poses` arena. Only rows with
`binding.state: "bound"` select a pose; ambiguous and unbound rows are skipped. These
root-design world matrices are already in millimetres and bypass the current-pose guards.
Missing holder worlds are derived bottom-up as child displayed world × inverse of child
local placement, only when all available children agree within 0.001 mm and 1e-6 rotation
entries and reproduce each of them. A holder whose children conflict keeps its first
placement, even inside a holder that moves, and is reported. Admitted float32 scene rotations
are normalized before inversion; translations are unchanged.
Smart also checks every displayed holder (bound or derived): when a part below it that follows
its pose, or a part with a displayed pose that is known to lie outside, would leave the
first-placement assembly bounds grown by 10%, the holder and everything inside it keep their
first placement. A local placement that matches the stored one within 0.001 mm and 1e-6
rotation entries is written unchanged, so components whose copies keep the same arrangement
stay shared.
An absent arena supplies no displayed poses; unbound scene matrices do not choose a
placement. Otherwise it accepts a native
current pose only when touching joints close (or there are none), the placed geometry fits
inside the assembly's first-placement bounds grown by 10%, and copies of the same inserted
design keep their relative placement. Other occurrences retain their first placement.

Bounds checks use conservative geometry envelopes. Unsupported or missing bounds keep the
first placement, as do unsupported touching joints and ambiguous pose carriers. Rejections
are deterministic. Joints then work from the selected placements as before.

The report names the parts whose position nothing confirms. A part whose placement could not be
read sits at its design's origin, and the sentence always shown on the result card says so,
naming the first few, with all of them in a warning below. A part with only a
stored pose and no joint, or with a later pose that was not used, is named in a note, because
those positions are usually right: when one looks misplaced, it's the place to look. The full list is
`part_positions.assumed` in the [JSON report](json.md).

**As Fusion last showed them** prefers the displayed-pose source, then uses current poses
where available without Smart's checks, and first placements otherwise. Until displayed
poses are bound by the decoder, this option relies on current poses, which can be stale.
**As first placed** ignores later poses throughout. Native current-pose matrices use centimetres and are
converted to millimetres. A pose in an inserted design is composed with that document's
insertion. Definition-local operation poses are not used as world poses. First placements retain
the decoder's existing path and component-operation placements; native current-pose records
remain a separate selectable source.

Select with CLI `--part-positions smart|last-shown|first-placed`, desktop **Advanced →
Part positions**, or MCP `part_positions`. The desktop remembers the choice. To restore
the old placement choice, use `--part-positions first-placed`; `--stored-placements` remains
an alias. Reports include an Info summary of the rule counts, `choices.part_positions`,
and `part_positions` counts and fallback reasons. Counts describe source occurrences,
including assembly holders, rather than additional history representations in the output.

## Components

Assemblies with multiple top-level designs group each design's features into a uniquely
named timeline folder, including the root design's own features when present, while keeping
feature order and leaving joints outside folders; a single part or one top-level design
gets no folders.
Features of one design separated by another design's features use numbered folders to
preserve timeline order.

A Fusion design can define components and place them, several times over (a copy and paste
of a component places it twice). cadmpeg gives each placement as a local occurrence with its
transform, and each component as a product definition.

- **When a component lists its bodies,** Dutchman writes the design as SolidCraft components:
  one per component, holding only that component's bodies as an import, one occurrence per
  placement, and the design's own loose bodies as one more component at the origin. This works
  for any format whose CADIR says which bodies a component owns.
- **Each component keeps its own history:** all features and parameters cadmpeg identifies as
  belonging to its definition, including features with no output-body links, plus the features
  that make its bodies and the sketches it owns with their building features. Referenced
  features come along, but timeline predecessors alone do not. A feature explicitly owned by
  another component stays out; without ownership, the existing body and sketch links still
  select history. A sketch another component builds that a feature of this one uses (designed
  in the context of the assembly) is the exception: when each of the two components has a single
  instance, its sketch feature comes in as this component's, with the sketch's plane moved into
  this component's coordinates. A component placed more than once also takes the sketch when
  exactly one of its instances puts the selected profile on the part. If none or several do,
  the history stops at the feature that uses it.
  Root-owned history goes to the design's loose-body component when it exists.
  Each sketch is written under the part that owns it, once per part however often it is
  placed, and a sketch owned by a component that holds no bodies goes under that component;
  only sketches with no owner, or whose owner component is not written, stay in the assembly
  root (counted in the report).
  A sketch the part's history builds nothing on, or that the source hides, starts hidden.
  The component's history is then mapped and checked like a single part's; when it does not
  map, the component keeps its bodies as an import and the report names its first blocker.
- **When it does not,** the design converts as stored and the report says so (Degraded,
  `components`): the geometry is written once, so a placement whose geometry the file does
  not store separately is not shown.
- **Instances that hold different layouts** are written as one definition per layout. When
  two instances of a component hold different components, or the same ones at different local
  placements (compared deepest first, within 1e-7 of each cell, so a difference anywhere below
  makes every component above it differ), the component becomes `<component>` for the first
  layout met and `<component> (2)`, `<component> (3)` for the others, in order of first
  appearance; every instance sits at its resolved pose, and instances with the same layout
  share one definition. The report notes how many components were written as how many
  definitions. A component's own bodies are the same in every layout, but each definition
  carries its own copy of them (the geometry and its STEP data are written once per
  definition). Mirrored placements combine with this: a mirrored instance of a layout uses
  that layout's `(mirrored)` copy.
- **Mirrored placements** use a shared component named `<component> (mirrored)` with
  reflected geometry and outward-facing solids. Ordinary copies retain the original
  component and its history. Nested reflections compose, including pairs that cancel.
  Mirrored definitions keep imported geometry only; the report notes any history left out.
  Joints touching a mirrored component are left out with a note and the placed pose retained.
  Tiny numeric drift in placements is corrected. Non-finite, scaled, or sheared placements
  beyond tolerance are left out with a Dropped note naming the occurrence index and kind.
- **Components inside components** become sub-assemblies: a component that holds others
  becomes a SolidCraft component with them beneath it, placed by occurrences inside it, and its
  own bodies as one more component at its origin. cadmpeg places the components a design's root
  component contains inside one occurrence of that root component, so such a design converts
  as the root component with its components beneath it. A component placed several times that
  holds different components in different instances is written once per layout (above); when
  a placement is inside an occurrence the design does not place, the design converts as stored
  and the report says why.
- **Hidden components and bodies** stay hidden: a placement whose occurrence the design hides,
  or whose part holds only bodies the design hides (a design's own body hidden beside its
  components, say), is written as a SolidCraft occurrence hidden in every placement of what
  holds it. Its geometry is still converted and can be shown again in SolidCraft. A part with
  shown and hidden bodies is shown whole.
- **Joints** between components become SolidCraft joints: fixed ones rigid (their offset kept),
  revolute ones turning about the connector's z axis with their limits, slider ones along it.
  Each connector frame is in its own part. Offsets apply to the second connector:
  `C0 = C1 * T(offset) * Rz(angle) * F`, where `C0` and `C1` are the stored connector
  world frames. `F` is a half turn about x for `opposed` frame sense (also the default),
  or identity for `aligned`. Fixed offsets translate in the second connector's axes before
  turning about its z axis; a revolute's angular offset turns that same connector.
  Dutchman folds these offsets into SolidCraft's second snap, so placement and the written
  joint use the same relation. Slider translation offsets also apply to the second snap;
  their distance is expressed along the first snap's z, with its sign reversed for aligned
  frames. With no offsets, opposed frames meet face to face: the same origin and opposite
  z axes; aligned frames meet with their axes pointing the same way. Fusion can store a joined
  part where it was before its joints were solved, so Dutchman places joined parts where their
  joints put them: from the design's own root, each joint places the part at its other end; a
  joint neither of whose parts is placed yet normally moves its first part to its second,
  as Fusion moves the first part picked for a joint. When starting a group of joints that
  reaches nothing placed, a non-identity stored world pose takes precedence over an identity
  pose on the other side; otherwise the outermost occurrence stays put. Joints that already
  close at the stored poses hold their parts in their common component before any open joint
  moves a part. That component can still travel with an outer joint.
  A joint to a part inside an identity-placed component carries the component when child
  poses describe its relative layout, or all its children belong to the joined group, and
  the joints reaching into it agree on its place. An unrelated internal joint is insufficient. If that outer component is only an organiser,
  Dutchman tries the deepest smaller holder whose children are connected, then the named part.
  A body's identity placement at a holder's origin can also identify the holder as the moving
  assembly, including a holder with a non-identity placement, provided every outside joint
  reaches it through that same body. A joint that names the holder itself prevents this
  promotion. Children independently reached by other joints stay with their own placements.
  Internal joints solve first, so the outer joint uses the inner part's
  solved connector position. A file pose, grounding joint, or earlier joint holds that component in
  place instead; a conflicting outer joint is suppressed. Of a joint between a component and a
  part inside it, as a rigid group holds its members to its first, the inner part moves and the
  component keeps its own placement; a component holding a part already placed by an outer
  joint is never moved to meet that part, as that would move the part too. A component a joint
  moves leaves the parts inside it that outer joints placed already where they are, and carries the
  parts the file poses inside it, whose poses are relative to it. Internal
  joints alone do not fix their holder in the assembly. A component the file stores at the pose it shows
  it in starts placed: its joints are checked there instead of moving it back to where it was
  inserted. Within an already posed component, an isolated leaf pose marker yields to a
  partner held by joints that already close; the design root alone does not override that
  child pose. Explicit holder poses and child poses already
  confirmed by a solved joint stay protected. This also preserves a child component named by
  a contained pose inside an inserted design when that design has no current pose of its own. An inserted design's own current
  pose holds the design; its children still use their own poses or joints. A contained pose is
  matched within that design instance and its stored owner chain, so an
  unrelated component or mirrored sibling does not inherit the pose. A part two joints would
  place differently is reported, and the joint that cannot hold it is written suppressed:
  SolidCraft solves the joints when it opens a design, and the part stays where the file places
  it. Earlier internal joints are checked again after outer joints place their holders,
  so an active joint cannot move those parts again on open. A joint to the design's root component holds its occurrence to the design. Fusion joint motion converts from Dutchman 0.1.5 on (before it, every Fusion joint was
  fixed). A revolute limit on one side only is left out, and the report says so. A component
  placed more than once is held once in the SolidCraft design, so a joint between two parts
  inside one copy of it is that component's own: it is written once, through the first copy, as
  SolidCraft solves joints between repeated parts in the first placement's frame, and every copy
  follows; so is a joint between a copy and a part inside that copy. A joint from a part inside
  one copy to something outside that copy cannot hold the part alone, so it holds the whole copy
  where the part is, and the report says so; when another joint already holds that copy itself
  to the same occurrence, as a rigid group holding both does, the part's joint is left out
  instead. A part inside a design inserted into a component, which Fusion names by its key
  alone, is found inside that component. A component empty in its source becomes an empty
  SolidCraft component, so the joints to it are kept. A joint is left out, and reported as
  Dropped (`joints`), when it is of another kind, joins a component to itself or to one in
  another design, or grounds a part inside a component placed more than once (grounding it would
  ground every copy).

### Upstream gaps

1. **Component bodies.** cadmpeg says which bodies a component owns when the file binds the
   component to its entities (its naming space): the bodies whose body-map entries lie in the
   component's entity range. A component placed by an operation the file does not bind (as in
   some copy-and-paste designs) has no bodies, so that design converts as stored.
2. **Joint motion.** cadmpeg reads revolute and slider joints, and revolute limits; other
   motion types, slider limits and one-sided limits stay fixed with its
   `assembly.joint-motion-undecoded` loss, which the report carries.
3. **Joints between referenced designs.** In an `.f3z`, a joint names each part by its design's
   Fusion document id and occurrence id, which cadmpeg does not yet tie to the occurrences it
   merges, and the merged occurrences do not say which bodies they hold; such a design converts
   as stored.

`.f3z` multi-document archives (a design with the designs it references) convert like `.f3d`:
cadmpeg merges the referenced designs into the root design with their placements, as
components.

While reading an `.f3z`, Dutchman shows the current stage and elapsed time with a five-second
still-working heartbeat. cadmpeg owns member discovery and composition; decode attempts
can repeat, so the progress display does not infer unique design counts or a determinate
reading bar for `.f3z`.
A single `.f3d` reports one design when its read succeeds.

Optional [diagnostic logs](troubleshooting.md#diagnostic-logs-for-unfinished-runs) record salted `.f3z`
member ids, sizes, actual decode starts/finishes/failures, and re-decode counts. Per-member
records appear when the decoder reports them; they do not establish a unique design total
for the progress bar. The pinned decoder's observer is installed on the decoding thread.
