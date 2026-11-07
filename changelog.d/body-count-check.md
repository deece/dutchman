### Reports

- A partial history that replays but whose bodies differ from what the source stores at the same
  point, fewer of them or one away from where the source has it, is now reported, with the loss code
  `history/body-mismatch`, rather than counted as replaying cleanly. Fusion files store this.
