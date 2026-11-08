### History

- cadmpeg gives a Fusion Base Feature's body only as the design ends. Joins and cuts after it
  re-apply to that body without changing the shape, so they are kept but no longer counted as
  rebuilt: the report's `history.reapplied` counts them, and the result card says editing them
  may not change the shape. A Base Feature whose body any other later feature changes, such as a
  Move, is refused, naming that feature; it had replayed the body where the later features leave
  it, then applied them again.
