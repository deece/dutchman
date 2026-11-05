### Command line and install

- `convert-all` warns on stderr, before converting, when a missing positional argument looks
  like an output path, and its help says every positional argument is an input (use
  `--out-dir` for the output folder).
- `convert --report-anon FILE` writes the anonymous report of one file, with `--report-salt`
  and `--report-key`, as `convert-all` does.
- Progress lines on stderr are off when stderr is not a terminal; `--verbose` brings them back.
