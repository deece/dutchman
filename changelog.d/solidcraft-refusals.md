### Reports

- A feature SolidCraft refuses when it replays the history is now reported as
  `SolidCraft refused feature "X": ...`, without SolidCraft's internal site number, instead of
  reading like a Dutchman mapping limit. The JSON feature entry gains `source`, `category`,
  `upstream` and `reason_raw`; a fillet larger than its neighbouring faces has its own category
  with a link to the SolidCraft issue. The anonymous report carries the category and link.
  The same wording is used in the history summary, the verification reason and per-part
  reasons.
