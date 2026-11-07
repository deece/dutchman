### Reports

- A partial history that replays but whose bodies differ from what the source stores at the same
  point, in number or in where they are, is now reported, with the loss code
  `history/body-mismatch`, rather than counted as replaying cleanly. Fusion files store this.
