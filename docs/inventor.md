# Inventor: what converts today, and what stands in the way

Dutchman reads Inventor through cadmpeg's Inventor codec (cadmpeg-registry's
`inventor` feature). Improvements to Inventor decoding land in cadmpeg and reach Dutchman
when the pin is bumped. This page says what Dutchman makes of what that codec gives, and which
upstream gaps keep Inventor history from replaying. It drives what is asked of cadmpeg next.

State as of the cadmpeg commit Dutchman pins (`f9d0a14`, cadmpeg 0.6.11; see `Cargo.toml`). From that
commit more Inventor fillet forms decode instead of staying native.

## Parts (`.ipt`)

| What the codec gives | What Dutchman does |
|---|---|
| The part B-rep, from one typed ACIS/ASM kernel carrier (verified for ACIS majors 217 and 218). | Converts it: restated as STEP, written as SolidCraft's import container, like Fusion. |
| No geometry (empty or invalid carrier: blocking `geometry_not_transferred`). | Stops: "no geometry could be read". |
| Parameters (closed scalar graphs, including ones whose stored unit type cadmpeg doesn't recognise). | Mapped to SolidCraft parameters. |
| Planar sketches: points, lines, circles, ellipses, polygons, with typed and native constraints. | Mapped, ellipses as whole ellipses; typed constraints mapped, native ones reported as not carried. |
| Periodic (closed) sketch B-splines, with a full knot vector and polynomial or equal-weight poles. | Mapped exactly by endpoint knot insertion into a clamped SolidCraft control spline. Nonuniform spacing and repeated interior knots become chains of exact spline pieces with shared endpoints, up to 200 pieces; uniform splines stay one spline. Unequal weights remain unsupported because SolidCraft control splines have no weights. |
| Extrude, hole, constant-radius fillet, equal-distance chamfer, when every property, dimension and selection resolves. | Extrude: mapped when cadmpeg resolves its profile to sketch profiles or regions, blind, through all, or to a resolved plane parallel to the sketch before any body-building feature (including a placed work plane); resolved parallel planar faces on earlier named bodies also map. Native face selections, tilted terminating planes, vertex selections, and to-next-face ends are not mapped yet. Holes map where their placement resolves, drilled the way the hole points; constant-radius fillets and equal-setback chamfers map with proven feature input edge geometry (see guide.md). |
| Resolved revolve with a sketch profile, axis, and fixed angle. | Maps full turns and one-sided partial solid sweeps when selected profile bounds prove the sweep side about the oriented axis; see [revolve coverage](guide.md). |
| Mirror of features about a work plane. | Mapped as a SolidCraft mirror pattern of those features about that plane. |
| Rectangular pattern of features with fixed spacing, in one or two directions. | Mapped as a SolidCraft rectangular pattern of those features. |
| Rectangular pattern with suppressed instances. | Not mapped yet: SolidCraft's rectangular pattern fills the whole grid. The report names the pattern and how many positions it skips, and the part keeps its exact geometry. |
| Combine of bodies (cut, join, or intersect), with its target and tool bodies as native references. | Not mapped yet: reported as "its bodies are not resolved" until Dutchman can name the bodies earlier features made. |
| Move, Move Body, and Rotate of whole bodies (`move_body`). | Not mapped yet, and reported. |
| Split of a solid by a work plane or a surface (`split_body`). | A split of one body by a placed work plane maps to a SolidCraft split. The piece on the side the plane's normal points to keeps the body's name, and the other piece gets its own. When the source's two result bodies can be told apart by side, later features select the right piece; when they cannot, a later feature selecting one is refused. A split by a surface is reported as not supported yet. |
| Work planes in the browser, including the origin planes. | Read as datum planes; sketches on them keep their placement. |
| Stored bodies: a base feature, derived part or composite the codec emits as stored geometry (`stored_geometry`) with the bodies it outputs, solids and sheets. | Written as a SolidCraft Import of exactly those bodies at its place in the timeline; later features that map keep their history. One with no output bodies is reported, not guessed. Where the codec cannot name the bodies it keeps the feature native, which stays unmapped. |
| Features the codec keeps in native form (`native`). | Not mapped; a part whose every feature is native has no history Dutchman can write. |
| Suppression and dependency state: unresolved (`suppressed` absent). | Features are treated as active; verification catches a replay that differs and keeps a replayable editable prefix hidden beside the imported final shape when possible. |
| Product name: none for the root product definition. | cadmpeg's STEP writer then names the product by its id (`inventor:document:product#root`), which SolidCraft shows as the body name. |

Resolved `to_plane` extrude ends map before any body-building feature when the terminating
plane is parallel to the sketch. The plane origin becomes a fixed SolidCraft To Object point;
a construction plane or body face need not exist there. After earlier geometry, a `to_plane`
end maps when the plane is a face's surface (its `source` is `face_surface`): a SolidCraft
pick of that face ends at the same plane. Other plane ends after earlier geometry (a work
plane or point) are not supported yet: SolidCraft can reinterpret a point on an existing face
as a different end, and CADIR supplies no input face geometry to prove which face exists then.
Final-body faces cannot establish that timeline state. A `from_plane` start parallel to the
sketch maps as a start offset along the sketch normal. Tilted plane ends and native,
generated, or historical vertex selections are not supported yet. Symmetric To Object ends
and non-blind ends against the sketch normal on two-sided extrudes remain unsupported.

## Upstream gaps, in the order they block history

1. **Native features.** Most Inventor features still come from the codec in native form, so
   most part histories stop at their first such feature.
2. **Unresolved sketches and profiles.** Some sketches are not bound to their stored geometry
   ("the sketch it binds is not resolved"), and some extrude profiles do not resolve to the
   sketch's loops (profiles built from parts of sketch curves, among them).
3. **Edge and face selections are native.** Fillet, chamfer and hole carry `Native` edge or
   face selections, and an extrude to a face names its face natively. SolidCraft finds edges
   by points on them and faces by points or persistent names, so these need typed historical
   or generated references (cadmpeg's `Historical`/`Generated` selections) before Dutchman can
   place them. The same gap holds for Fusion.
4. **Unresolved revolve controls.** The codec projects its admitted fixed-angle direction
   with a resolved axis. Other direction controls or unresolved profile/axis operands remain
   native. Dutchman maps a partial solid sweep only when the selected profile curves can be
   bounded on one side of that oriented axis; it reports an unproven sweep side otherwise.
5. **Suppression and dependencies unresolved.** Suppressed Inventor features currently look
   active.
6. **STEP product name.** cadmpeg's STEP writer names a product with no name by its
   product-definition id.

## Assemblies (`.iam`)

Assemblies convert as placed parts, each with its own feature history when that history
maps (see the parts table above for what blocks Inventor history today):

- cadmpeg gives each occurrence its stored part path (often absolute on the machine that
  saved it), its rigid placement in millimetres, and its name. Dutchman finds each part as
  [below](#finding-the-part-files), reads each part file once, and writes one SolidCraft
  component per part, holding the part's geometry as an import, with one occurrence per
  placement.
- Sub-assemblies are read from their own `.iam` files the same way, recursively (up to 32
  deep; an assembly that places itself, directly or not, is refused). Each becomes a component
  whose parts are components beneath it, placed by occurrences inside it, so SolidCraft keeps
  the assembly's structure. The occurrences a parent file records inside its sub-assemblies
  are taken from the sub-assembly files instead.
- Left out and reported as Dropped: parts or sub-assemblies that cannot be found or read,
  scaled placements, and occurrences whose part is not a separate file.
- Upstream: cadmpeg leaves local prototypes untransferred. CADIR version 7 (cadmpeg 0.6.3) can
  carry assembly joints, but the Inventor codec does not read them yet.

### Finding the part files

An assembly stores where its parts were on the machine that saved it. Dutchman finds each one
as Inventor does, so a project can be converted where it lies:

1. **The stored path.** A relative path is taken from the assembly's folder. An absolute one
   is tried as it is, then with its drive or root (`C:\`, `\\server\share\`, `/Users/…/`)
   moved onto this machine: the longest end of the path found under a folder above the
   assembly, nearest first. Folder and file names may differ in case.
2. **The project folder.** Otherwise the file is searched for by name under the project
   folder, with all its subfolders. Of several files with that name, Dutchman takes the one in
   the most of the stored path's folders (`Parts\Brackets\pin.ipt` matches
   `…/Parts/Brackets/pin.ipt` before `…/Assemblies/pin.ipt`), then the one nearest the
   assembly; the report says which it used. Inventor's `OldVersions` backups, hidden folders
   and symbolic links are not searched, and the search stops after 200,000 files and folders.

If the file is not under the project folder, the folders its Inventor project file names are
searched the same way, in order: the workspace, workgroup and library folders in the `.ipj`,
then its imported-components folder in each workspace. The project file is the nearest `.ipj`
in or above the project folder, else above the assembly, so a project folder given inside a
larger project still reaches the rest of it. Its paths are taken from its own folder; a path
from another machine is found by its longest tail below that folder, and one that is not on
this machine is skipped. After those come the folders given with `--search DIR` (the MCP tools'
`search`), in order: a shared library of standard parts, say. The report says which folders
were searched (`parts search`).

A folder below the project folder that holds a project file of its own, such as a Pack and Go
copy, is another project, and its copies of the files are not searched.

The project folder is, first found:

- the folder given: `--project DIR` (the MCP tools' `project`); `convert-all` uses each folder
  it is given;
- the nearest folder above the assembly holding an Inventor project file (`.ipj`);
- two folders above the assembly's, but never your home folder or a whole drive.

A part found nowhere is left out and reported, `Dropped`, naming the project folder searched
and how many other folders were.
In the desktop app, the file's card names the parts not found: **Search a folder…** or **Find a
file…** (whose folder is then searched) converts it again with that folder searched too. The
app's **Inventor project folder** setting sets `--project` for a whole run.

Every file found is named as its folder lists it, whatever case the assembly stored it in.

## Testing without real Inventor files

cadmpeg publishes no Inventor part with geometry or history, and Dutchman never uses customer
files. `crates/dutchman-core/tests/fixtures/synthetic_cube.ipt` is generated from cadmpeg's own
synthetic sources (its README has the recipe). History behaviour is tested on CADIR built in
the tests in the shape the Inventor codec emits, until cadmpeg publishes generated Inventor
goldens with a design segment (sketch and extrude).
