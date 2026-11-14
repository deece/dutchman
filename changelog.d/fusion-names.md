### Formats

- Solid and sheet shapes written to STEP now carry explicit body names from the decoded
  CAD model (`MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`, `SHELL_BASED_SURFACE_MODEL`) so
  imported designs preserve their original body names (such as "Bottom" and "Top")
  rather than falling back to generic numbers.
- Sketch history entries now fall back to the decoded sketch's `name` when the timeline
  feature itself carries no display name.
