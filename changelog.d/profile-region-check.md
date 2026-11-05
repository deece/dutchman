### History

- An extrude whose profile region does not match the source is no longer counted as kept. When
  the design's only geometry-building feature is a blind one-sided extrude whose end face
  survives in the stored model, the area of the swept region (outer loops minus holes) must
  match that face's area to within 1 mm² or 3%. A mismatch blocks the extrude with the
  swept and source areas in its reason, records the loss code
  `history/profile-region-mismatch`, and stops the editable history there.
- A kept extrude whose region could not be checked this way carries `profile_unverified: true`
  in the JSON report's `history.features`, so counts do not claim more than was proven.
