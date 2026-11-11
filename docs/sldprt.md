# SolidWorks: what converts today, and what stands in the way

Dutchman reads SolidWorks parts and assemblies through cadmpeg's SolidWorks codec (cadmpeg-registry's
`sldprt` feature, at the same pin as f3d and Inventor). Improvements to SolidWorks
decoding land in cadmpeg and reach Dutchman when the pin is bumped. This page says what
Dutchman makes of what that codec gives, and which upstream gaps keep SolidWorks history from
replaying. It drives what is asked of cadmpeg next.

State at the cadmpeg commit pinned in `Cargo.toml` (`6c775550`, cadmpeg 0.6.11).
`dutchman formats` reports the reader's current reader status alongside what Dutchman converts.

## Parts (`.sldprt`)

| What the codec gives | What Dutchman does |
|---|---|
| The part B-rep, from the file's Parasolid body streams: analytic and NURBS carriers, offset and blend surfaces where they resolve, the active configuration's bodies. | Converts it: restated as STEP, written as SolidCraft's import container, like Fusion and Inventor. The body is named after the file. |
| No geometry (no Parasolid stream yields a body). | Stops: "no geometry could be read". |
| Bodies with no face whose bounds can be written (for example a lone face whose edges have no curve). | Stops: "the model has no solids or surfaces SolidCraft can import". Faces that can be bounded convert as surfaces, with the rest reported. |
| Losses: an unverified SolidWorks version, configurations inferred from geometry, opaque carriers, derived body hierarchy. | Printed as Degraded, with cadmpeg's loss code. |
| Parameters, with their expressions (`Depth = 12.5mm`). | Mapped to SolidCraft parameters when the history maps. |
| Feature history from the Keywords XML: extrusions with their operation, extent and depth; sketches; suppression state; many features only as `native` records. | Keeps editable features before the first blocker and resolved sketches, hidden beside the imported final shape; the report names the blocker (below). Without a body-building prefix, keeps reference sketches beside the import. |
| Sketch geometry from the ResolvedFeatures records: lines, arcs, circles, ellipses, splines, coincidences, with a resolved plane. | Read only through a sketch feature that binds it (below). |

A part saved with several configurations converts in the configuration it was saved in.
Bodies belonging only to the other configurations are left out, and the report says so.
Source bodies retained for derived or mirrored geometry are also left out of the current
body set. Unresolved body, shell, or face ownership is reported.

Closed faces without boundary edges and closed ring edges without vertices are retained
when their surfaces and curves resolve. More offset surfaces decode over analytic, NURBS,
and swept supports. Offsets whose support orientation, distance, or trim boundaries cannot
be resolved remain reported losses.

Parts read by 0.1.5 that 0.2.0 to 0.2.2 read empty, with fewer bodies, or failing cadmpeg's
edge checks read again since 0.2.3: cadmpeg no longer lets bytes inside other
Parasolid records hide or refuse a part's bodies, and keeps the range of arcs stored past the
seam.

Stored base features name only regions that STEP can emit: a region with no shells
contributes no shape and consumes no appearance target. Empty exterior or void regions
therefore do not discard the editable history. Other shape-count mismatches leave the explicit appearance targets unbound while
keeping the base geometry and history; colours must never shift onto different bodies.

Offset datum planes retain their resolved reference frame and signed distance for later
plane consumers. A missing support does not block history when every consumer is a sketch
with its own resolved placement. A consumer needing a missing frame still stops history.

Shells map a resolved thickness and inward or outward direction with faces from their
own historical input topology and one earlier named target body. Native selections,
final-body faces, unsupported offset modes and unresolved historical boundaries remain
specific blockers. The selected input faces must exist before shelling removes them.

A through-all extrusion cut needs an earlier named body. Features replay in neutral
ordinal order even when stored in a different order. When no earlier producer supplies a
body, the cut stops mapping with a body-producing-feature explanation before replay.

## Upstream gaps, in the order they block history

Found by running Dutchman's history mapper on what the codec decodes from cadmpeg's published
SolidWorks fixtures (`dutchman inspect --json`).

1. **Sketch features do not bind their sketch.** A `Sketch` feature in the history arrives
   without the sketch it owns ("the sketch it binds is not resolved"), and the sketch geometry
   the codec does recover arrives as a separate feature-input sketch that no feature points
   at. Dutchman cannot place the sketch in the timeline, so the first sketch stops the history.
   Needed: the sketch feature's binding set to the CADIR sketch it owns.
2. **Extrusion profiles are unresolved.** An extrusion's profile arrives as `unresolved` (the
   extrusion's own native id), or, when the history places a sketch next to it, as `feature`
   (the sketch feature). Dutchman maps profiles given as a sketch, sketch profiles, sketch
   regions or sketch curves, so every SolidWorks extrusion stops the history ("unresolved
   profiles are not mapped yet"). Needed: the profile as `sketch` or `sketch_profiles` of the
   CADIR sketch. Once (1) lands, Dutchman can also read a `feature` profile as the whole sketch
   that feature binds.
3. **Many features are native.** Features the codec does not type (for example
   `EquationDrivenCurve`) arrive as `native` and stop the history where they come.
4. **Edge and face selections are native.** Fillet, chamfer and hole carry mostly native edge
   or face selections. SolidCraft finds edges by points on them and faces by persistent names,
   so these need typed historical or generated references first. The same gap holds for
   Fusion and Inventor.

Resolved planar sketches are kept for reference even without matching timeline rows or
feature history. The original geometry remains visible; unavailable sketch geometry or
placement is reported.

## Assemblies (`.sldasm`)

Assemblies convert as placed parts and nested components. cadmpeg supplies product definitions,
occurrences and placements; Dutchman resolves external `.sldprt` and `.sldasm` references
through the shared assembly file finder. Absolute Windows paths are matched by file name,
case-insensitively, beside the assembly first, then through the project and `--search` folders.
See [finding part files](inventor.md#finding-the-part-files) for the shared search rules.

Local sub-assemblies keep their component tree. Referenced assembly files are read recursively,
with cycle protection and a depth limit of 32. Repeated references to the saved active
configuration share one read across the whole assembly tree, including sibling sub-assemblies.
Saved display tessellations are released before building that tree; exact geometry and feature
history are kept.

Assemblies with many coloured faces retain their body and face colours without long pauses
while mapping those colours, including with the fast preset.

Parts that cannot be found or read are left out and reported for every affected occurrence;
the remaining parts still convert. Virtual components without readable geometry are reported.

The part reader supplies its saved active configuration. A reference to that configuration
uses it; when the requested configuration cannot be confirmed or selected, Dutchman places
the available active configuration and reports a Degraded loss. Mates are not converted yet.
Suppressed components and mirrored placements follow cadmpeg's projection and loss notes:
Dutchman does not reconstruct components that the reader leaves out. Reader resource limits
still apply, so an assembly can be refused before any parts are resolved.

Folder and ZIP projects read assembly references to select top-level `.sldasm` files.
Local sub-assembly holders are matched to saved `.sldasm` files by their source document
names. Each converts with its nested assemblies and parts resolved inside the project; unused
parts convert afterwards. The plan counts the full inventory before grouping references.

## Testing without real SolidWorks files

Dutchman never uses customer files. The fixtures (provenance in
`crates/dutchman-core/tests/fixtures/README.md`):

- `body_history.sldprt`, copied unchanged from cadmpeg's `authored_complete_history.sldprt`
  golden fixture: a one-triangle body and a history with an extrusion (blocked by gap 2).
  Inspect reports the history blocker; the triangle cannot convert to an importable solid
  or surface.
- `container_metadata.sldprt`, generated with cadmpeg's authored native envelope test helper:
  a readable container with no geometry.
- `solidworks_cube.sldprt`, generated with cadmpeg's own SolidWorks encoder from its 10 mm
  example cube and authored native envelope helper. It converts end to end against a golden
  document and opens in SolidCraft. All three fixtures follow the conforming envelope layout
  introduced by cadmpeg PR #286; the fixtures README records the exact commit and recipes.
