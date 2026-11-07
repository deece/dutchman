### Reports

- An estimate from a file's size no longer refuses a conversion or leaves out feature history.
  The conversion runs, watching memory, and stops with a clear message if the computer really
  gets down to 1 GB free while the conversion holds at least 1 GB.
- Standard leaves feature history out only when what the read really took leaves no room for it.
- `convert-all`, `watch`, and the MCP batch tool no longer stop a file at 8 GB by default.
- The desktop no longer stops to warn about an estimate before converting.
- A file that runs out of memory alongside others in a batch is converted again alone at the end,
  once, and the report says so.
- The desktop converts each file in its own process, so a stop or a crash ends that file, not the
  app.
