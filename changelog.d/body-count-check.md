### Reports

- A partial history that replays but whose bodies differ from what the source stores at the same
  point, fewer of them or one away from where the source has it, is now reported, with the loss code
  `history/body-mismatch`, rather than counted as replaying cleanly. Fusion files store this.
  In a design with components, a stored list that features of more than one component carry is
  not used, since it isn't any one component's state.
