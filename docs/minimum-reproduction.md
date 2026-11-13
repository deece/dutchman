# Make a minimum reproduction

When a CAD file fails to convert and must stay private, keep it local and rebuild a small
example of the failure. Dutchman makes a kit to guide you or your own AI. It uploads nothing.

## Make the kit

```sh
dutchman repro part.f3d --out reproduction
```

The MCP equivalent is `repro` with `input` and `out`. Use a fresh output folder for every
run. The command converts using the default SolidCraft settings with diagnostics enabled.
It writes three files under `reproduction/kit`:

| File | How to use it |
|---|---|
| `anonymous.json` | Safe to share: versions, format, coarse size class, stage/category, finding codes and kind counts, anonymous narrowing path and limits. No source names, paths, ids, geometry or parameter values. |
| `local-key.json` | **Do not share.** Look up anonymous ordinals here to find your own names in your CAD program. Missing names require manual lookup; CADIR order may differ from the program's order. |
| `next-steps.md` | Follow the recipe to build a small candidate in a new design. It contains only anonymous ordinals and fixed instructions. |

If conversion succeeds, the kit says `no_failure_to_reproduce`. A successful kit command
exits 0 even when conversion failed; `--json` and MCP return the same summary, paths and
anonymous content. Keep that response envelope local because it contains output paths.
MCP returns key content only with explicit `include_local_key: true`, intended for a
client keeping the response local. A remote chat assistant should coach from anonymous.json
while you look up private names yourself.

For a model check failure, Dutchman rechecks the retained decoded model on independent
subsets. Multi-document Fusion archives first use cadmpeg's member-session API. It then
tries components/occurrences, bodies, features and sketches as available. Each confirmed
step keeps the same stage, category and complete check-code set; counts may decrease.
No native file is edited, and stored final geometry does not identify the operation that
created it. Dependencies may prevent a unit standing alone. The kit records those limits
and its bounded trial/entity budget.

Writer failures are retried on subsets through the writer interface, which combines mapping,
restatement and any replay. Without structured codes, matching a stage/category produces
candidates with isolation false. Decode failures can report independent member outcomes;
member-composition failures may need several documents. These limits are part of the result.
Temporary output and diagnostics are removed after normal completion, including errors.
A killed process can leave a temporary directory under the output folder; keep it private.

## Rebuild and compare

1. Open `next-steps.md`. Look up its anonymous ordinal in the local key without sharing
   the key or names. Start a new design in your CAD program and follow the suggested recipe.
2. Save the small candidate and run `dutchman repro small-file --out next-reproduction`.
   Compare its failure stage and **complete set of finding codes** with the original
   anonymous report. Counts may shrink from 88 findings to one.
3. Remove unrelated features, sketches and components, one change at a time. Keep a change
   only if the same stage and codes remain. If no recipe matches, the generic instructions
   suggest copying the failing body into a new design locally, then rebuilding the small
   shape from scratch with simple dimensions you are free to share.
4. Review an issue draft with anonymous.json's `versions` block copied verbatim and its
   format, stage, category, code counts, kinds, anonymous ordinals and limits. An agent runs
   `dutchman --version` and `solidcraft-cli --version` if installed (using its full path or
   `SOLIDCRAFT_CLI` if needed), adding the outputs separately, or "none" if no SolidCraft
   was installed. State the source program/version from metadata already reported by
   Dutchman or the program's About screen; the agent asks once if missing. If exact builds
   are unknown, give the program, desktop or web, and the years the files are from.
   Post it on Dutchman's issues yourself. If rebuilding did not reproduce, say so.

Share the reviewed anonymous.json; include a synthetic file rebuilt from scratch only
if it reproduces and you are free to share it, with its STEP export and a public licence
such as CC0. Never send the original file. Never send local-key.json. The
[paste-ready prompt](testing-your-library.md#let-your-own-ai-make-a-minimum-reproduction)
coaches this procedure without posting or uploading anything for you.

Want to go further? The next tiers are [finding the cause in cadmpeg, fixing it, and finishing it in Dutchman](testing-your-library.md#want-to-go-further);
an agent asks once how far to go, then does every tier up to that answer.

## Worked example: 88 use-curve range findings

A model check can report 88 coedge use-curve ranges outside their carrier domains. The
pinned cadmpeg check code is `parameter_domain`; it also covers other parameter-domain
failures. The structured coedge and NURBS counts, together with the anonymous structural
`use_curve_outside_carrier` count, describe this class without parsing the error message.

This example comes entirely from synthetic CADIR. It combines public generated cube
geometry into three bodies and adds closed periodic NURBS use-curves crossing their stored
parameter seam in body #2. The ranges' endpoints agree with their traversal vertices so
only the intended domain check fails. Several disconnected regions in that body yield
exactly 88 findings. There are no native feature/sketch recipes to isolate, and the kit says
so. `size_class` is null because this case is generated in memory rather than read from a
native file. Versions are those used to generate the example.

The following [anonymous.json](examples/repro/anonymous.json) and
[next-steps.md](examples/repro/next-steps.md) are generated by the reproduction formatter
from that synthetic case. The suggested cylinder/spline construction is a candidate recipe:
it does not guarantee that a particular CAD program's saved file will trigger a decoder
fault. Re-run repro and compare the stage and full code set before sharing a rebuilt file.

### Example anonymous.json

```json
{
  "failure": {
    "curve_kinds": {
      "nurbs": 88
    },
    "findings": {
      "parameter_domain": 88
    },
    "kind": "cadmpeg_checks",
    "stage": "project"
  },
  "input": {
    "format": "fusion_archive",
    "size_class": null
  },
  "narrowing": {
    "limits": [
      "Component or occurrence structure is unavailable in the retained unit.",
      "Feature or sketch history is unavailable in the retained unit; stored geometry cannot identify the operation that made it."
    ],
    "members": [],
    "path": [
      {
        "ordinal": 2,
        "total": 3,
        "unit": "body"
      }
    ],
    "smallest_failing_unit_isolated": true
  },
  "retained_unit_kinds": {
    "feature_kinds": {},
    "finding_entity_kinds": {
      "coedges": 88
    },
    "surface_kinds": {
      "plane": 24
    },
    "use_curve_outside_carrier": 88
  },
  "schema": "dutchman.repro/1",
  "status": "failure",
  "versions": {
    "cadmpeg": "0.6.7+9bc00b9",
    "dutchman": "0.2.5"
  }
}
```

### Example next-steps.md

```markdown
# Make a minimum reproduction

The conversion stopped. cadmpeg's checks rejected its decoded model. The finding codes and counts are in anonymous.json.

Look up body #2 of 3 in local-key.json, following the preceding step's scope. Keep that name local.

Recipe for parameter_domain with a NURBS carrier (including use-curve ranges outside their carrier domain): In a new design, make one body with a cylindrical or spline face whose edge crosses the face's seam. Extrude a full circle, then split the body along a plane through the axis. Save it and run `dutchman repro small-file --out new-results`. This is a candidate recipe, not a guarantee: this check code also covers other parameter-domain failures. Compare the stage and complete set of finding codes, not the counts.

Limit: Component or occurrence structure is unavailable in the retained unit.

Limit: Feature or sketch history is unavailable in the retained unit; stored geometry cannot identify the operation that made it.

Send only anonymous.json, and the small rebuilt file if it reproduces and you are free to share it. Never send the original file. Never send local-key.json.
```
