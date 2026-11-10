### History

- Write a spline SolidCraft cannot take directly as a chain of exact spline pieces instead of
  leaving out the sketch, including its profiles and sweep paths.
- Keep shared endpoints and smooth joins between spline pieces, and report constraints that
  cannot apply to the chain.
- A loft whose section or guide rail would use such a chain is not mapped yet, since more edges
  in a section would change the lofted surface.
