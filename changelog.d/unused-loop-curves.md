### History

- A sketch curve that bounds no region any feature uses is written as construction, even when it
  belongs to one of the sketch's closed loops. A tab no feature takes, drawn along part of the
  edge of the region an extrude takes, or a ring around a hole that region keeps, used to stay
  solid and split the region in SolidCraft, which then refused the extrude ("unknown complete
  source profile"). Features that select trimmed regions now count the curves they use instead
  of keeping every curve of the sketch solid, when the region's boundary closes.
- Regions that share part of an edge and are taken together (a tab and the plate it sits on)
  are named by a point inside each, so SolidCraft finds them where it splits the shared edge.
