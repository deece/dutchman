# Choosing what to keep

`convert`, `convert-all`, and `watch` accept these presets and narrowing switches:

| Preset | SolidCraft output |
|---|---|
| `fast` | Geometry at the stored placements; no history, reference sketches, or joints. No memory refusal. |
| `standard` (default) | History, sketches, and joints. History is left out when what the read really took leaves no room for it. |
| `full` | History, sketches, and joints whenever they map; history is never left out to fit memory. |

Parameters that map are retained with history or reference sketches; Fast leaves them out.

`--no-history` (also `--import-only`), `--no-sketches`, and `--no-joints` only remove
choices from the preset. `--no-sketches` removes reference sketches, including those
kept beside an imported shape or after a blocked history; sketches needed to replay
retained history stay. `--no-joints` omits assembly joints and restores the source's
stored placements. An Info line under `joints` records that choice.
FreeCAD output (`--to freecad` or `--to fcstd`) honours the history choice only;
the sketch and joint choices do not control that writer.

```sh
dutchman convert assembly.f3z assembly.sc --preset fast
dutchman convert part.ipt part.sc --preset full --no-sketches
dutchman convert-all project/ --out-dir converted/ --preset standard --yes
```

Text conversions show `Estimate: about N GB (±R %), about M min (±T %)` before converting,
unless `--quiet` is set. Estimates below 0.1 GB read `under 0.1 GB`, and times below
60 seconds read `under a minute`; longer times round up to whole minutes. Where measurements
don't yet support a time band for the format, the time reads `roughly M min`.
Estimates use file size and format. A bounded ZIP pre-read counts members and total
uncompressed content for `.f3d`, `.f3z` and `.FCStd`. Fusion archives also read each
embedded design’s directory, one design at a time, without decoding it. Separate assembly
part files are not pre-read. Unknown formats, unreadable metadata, ZIP directories over
10,000 members or 16 MiB, and unreadable archives have no estimate.

## Memory and time model

Fusion, Inventor parts and SolidWorks parts have three separately measured costs:
reading through native decode, conversion after decode, and checking. Conversion includes
validation, visibility filtering, component extraction, mapping and writing. The read
figure is its peak; conversion and checking figures are extra memory above the preceding
peak. Their memory and time add to the total shown in the desktop box and file row.
History on/off have independent conversion and checking fits. For each stage, history-on
memory and time use the greater of the history-on and history-off fits: retaining history
cannot reduce either estimate. Combined models for other formats enforce the same minimum,
including platform-specific timing. Tiny parts can consequently have equal history-on/off
memory estimates. Estimates never stop a conversion or change what it keeps: an estimate comes
from a file's size, which can't reveal the geometry, and can be far off in either direction.
Memory is measured instead. A running conversion stops with a message when the computer has
less than 1 GB free while the conversion holds at least 1 GB, including what it started.
The SolidCraft check stops earlier, under 1.5 GB free, and the design is kept, marked unverified with the reason, so a built design isn't lost to its check. `--no-memory-check` turns that off.

macOS compresses and swaps rather than running out, so on a Mac a large design can use far more than the computer's memory and convert slowly with heavy swapping: a 12.5 GB conversion finishes on an 8 GB Mac. A Mac also stops when the system reports critical memory pressure or the boot volume has under 2 GB free, again only when the conversion holds at least 1 GB. Once a design is read, Standard leaves history out if what
the read really took leaves no room for it, and the other presets stop. Geometry-only conversion also checks its converted shape
unless `--no-verify` is selected.

Memory is `fixed MB + content bytes × B/B + member count × MB/member` after converting
MB to bytes. A fitted content plateau, where listed, limits only the variable memory
cost. It is an empirical estimate, not a ceiling on allocations. Time is `fixed seconds +
content MiB × s/MiB + member count × s/member`. All coefficients live in `COEFFICIENTS`
in `crates/dutchman/src/choices.rs`. Decimal MB/GB are used for memory; MiB is 1,048,576
bytes. Fractional phase times add before `seconds` rounds up. Desktop labels instead add
the displayed stage durations, preserving sub-minute bounds (see [the desktop guide](desktop.md)).

| Format | Stage | Fixed MB | B/B | MB/member | Content plateau MB | Fixed s | s/MiB | s/member |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| .f3d | Read | 15 | 5.7 | 1.7 | — | 0 | 0.1 | 0.061 |
| .f3d | Convert, history off | 12 | 15 | 0 | — | 0 | 0.11 | 0.062 |
| .f3d | Convert, history on | 9 | 26 | 0 | — | 0 | 0.35 | 0 |
| .f3d | Check, history on | 23 | 260 | 0 | 1191 | 0.032 | 0.38 | 0 |
| .f3d | Check, history off | 25 | 230 | 0 | 740 | 0.03 | 0.46 | 0 |
| .f3z | Read | 29 | 20 | 0 | — | 0.011 | 1.2 | 0 |
| .f3z | Convert, history off | 24 | 16 | 0 | — | 0.02 | 0.32 | 0 |
| .f3z | Convert, history on | 12 | 19 | 0 | — | 0.019 | 0.38 | 0 |
| .f3z | Check, history on | 23 | 25 | 0 | — | 0.032 | 1.8 | 0 |
| .f3z | Check, history off | 25 | 230 | 0 | 2055 | 0.03 | 0.69 | 0 |
| .ipt | Read | 15 | 20 | 0 | — | 0.003 | 1 | 0 |
| .ipt | Convert, history off | 18 | 5 | 0 | — | 0.014 | 1 | 0 |
| .ipt | Convert, history on | 17 | 30 | 0 | — | 0.042 | 3 | 0 |
| .ipt | Check, history on | 20 | 55 | 0 | — | 0.002 | 0 | 0 |
| .ipt | Check, history off | 30 | 27.5 | 0 | — | 0.046 | 0 | 0 |
| .sldprt | Read | 22 | 20 | 0 | — | 0.006 | 2.5 | 0 |
| .sldprt | Convert, history off | 11 | 5 | 0 | — | 0.016 | 2.5 | 0 |
| .sldprt | Convert, history on | 20 | 22.5 | 0 | — | 0.017 | 6 | 0 |
| .sldprt | Check, history on | 26 | 46.75 | 0 | — | 0.023 | 0 | 0 |
| .sldprt | Check, history off | 20 | 27.5 | 0 | — | 0.025 | 0 | 0 |

## Calibration and misses

The current shared-index main build was measured on six public Fusion assemblies
(Voron V1.8, Voron V2.4, Cascade, v02, RailCore and Trident) and five smaller public or
synthetic Fusion, Inventor and SolidWorks parts. The 44 successful observations pair
Full with checking on/off and Standard without history with checking on/off. Runs used
Linux, four requested workers, SolidCraft 0.30.1, the default 120-second per-check timeout,
`nice -n 10`, and a 24 GiB per-process address-space limit, one conversion at a time.
The recorded costs include checks that reached that timeout. Whole-tree peaks use
simultaneous resident memory, bounded below by the largest individual high-water mark;
individual high-water marks are never summed. Read boundaries come from the decoder’s
end marker. Sampling was 200 ms for assemblies and 2 ms for small parts, with diagnostic
clock alignment on the repeated small cases. Signed checked-minus-unchecked differences
are preserved; negative differences contribute zero added memory.

Single-design Fusion adds member count only where leaving one file out improves weighted
fit error by at least 10%. Read and conversion fits use relative errors with 20 MB /
5-second denominator floors. The two multipart assemblies support one content slope per
metric and the measured small Fusion floor; their time fit minimizes absolute seconds.
Checking uses a measured small-file floor and absolute errors, with a capped memory ramp
where the assembly observations support a plateau.

Inventor and SolidWorks each have one measured small shape. Those measurements calibrate
fixed costs only. Their old total content growth remains a heuristic for larger parts:
25 B/B without history, 50 B/B for Inventor with history, and 42.5 B/B for SolidWorks with
history, partitioned across reading and conversion. Larger foreign-part check times have
no measured content growth. FreeCAD retains its combined model: 29 MB plus 150 B/B, with
a 1.04 history multiplier on content and its previous startup terms. Other unmeasured
formats retain combined heuristics.

The largest absolute misses in the independent fits after rounding the coefficients,
before applying the history-off minimum, are:

| Fit | Peak-memory miss | Time miss |
|---|---:|---:|
| Read | 0.806 GB (v18) | 61.2 s (v18) |
| Convert, history off | 2.980 GB (trident) | 24.7 s (cascade) |
| Convert, history on | 2.288 GB (v18) | 83.6 s (v24) |
| Check, history on | 1.810 GB (v24) | 131.7 s (railcore) |
| Check, history off | 1.857 GB (railcore) | 95.5 s (cascade) |

Total-peak memory envelopes for this Fusion corpus are ±45% for `.f3d` and ±30% for
`.f3z`, adding one percentage point and rounding up to 5%. These bands describe total
peak memory, not each individual stage. Inventor and SolidWorks use a ±100% heuristic
outside their measured small-file floors; FreeCAD keeps ±50%. These are not guarantees
for unseen designs. Checking depends on the geometry and applicable history, which ZIP
sizes cannot reveal. Time bands remain uncalibrated (`time_band: null`), so longer times
read as rough. Other platforms, worker counts and timeout settings have no fresh timing
calibration. The old multipart serial-fraction speedup is not reused after the performance
change; changing threads does not invent a measured speedup. Actual worker limits and
buffer budgets are unchanged.

`peak_bytes`, `seconds`, `seconds_exact`, `band`, `memory_band` and `time_band` keep their
existing JSON fields. No split fields are added. The estimator never runs SolidCraft or
decodes a design. Version discovery before conversion can still run with `--no-verify`;
a possible probe timeout is not normal conversion work.

`threads.used` is the largest bounded Dutchman part-worker limit selected across conversion
stages and embedded members. It excludes threads inside SolidCraft child processes and does
not measure simultaneously busy CPU cores. Failed worker tasks can retry serially.

Available memory comes from Linux `MemAvailable` bounded by remaining cgroup v2
headroom (including ancestor limits), or safe system APIs through sysinfo on macOS and
Windows. Memory probes do not launch subprocesses. Unknown memory or an unknown estimate
does not block a run. Estimates use decimal GB and are approximate, not
reservations; other processes and simultaneous conversions can change available memory.

The memory check runs before decoding, using only the pre-read estimate. Standard
leaves history out for any design whose history estimate exceeds free memory when
its history-free estimate fits. That retention decision stays fixed through conversion.
After reading, a conversion can also stop when the resident design leaves too little
memory for mapping and writing. When Standard leaves history out, one Info line under `history`
says so, and the other presets stop with a message saying what was measured. An estimate above
free memory never stops a conversion: an Info line under `memory` records it, the command line
prints it, and the desktop shows it before starting.

If memory really runs low, the conversion stops with exit 2 and this message:

```text
stopped before this computer ran out of memory: N GB left free while this conversion held M GB. Close other programs and try again, or use --preset fast
```

`--no-memory-check` turns off that stop, Standard's automatic history budget, and
history-check skipping.
Existing decoder input limits and batch worker memory limits still apply. Batch
conversion checks each input; changing presets or narrowing switches also prevents a
journal from reusing a result made with different choices.

The desktop preset selector uses the same preset descriptions. See [the desktop guide](desktop.md).

## Checking after reading

The presets, narrowing switches, and memory guard are described above; `dutchman convert --help` lists them too.

Checking stays enabled by default, independently of retention. It checks imported geometry
when feature history is off. The peak estimate includes the requested check. The note about an
estimate above free memory compares conversion-only memory with free memory.

The runtime guard uses the same calibrated extra check figure shown by the desktop. It
does not multiply the memory already held by the decoded model. Free memory is measured
after reading and checked again before replay. If the allowance will not fit, checking
is skipped, the original shape remains visible, and retained history stays unverified.
The report says `History check skipped: needs about N GB, M GB free`. Unknown metadata or
memory measurements allow checking. These figures are estimates, not reservations;
simultaneous work can change what is free.

Automatic workers remain capped at two, with lower limits for detailed parts and retained
preparations. Explicit requests stay bounded by part counts and free-memory worker budgets.
Small imported parts can still use serial preparation.

Component extraction shares one geometry index per design, across workers and serial
retries. Geometry stays in compact JSON until a component reaches it; each component
copies only its selected rows. Name collection also streams the design, and Fusion skips
visibility expansion when there are no consumed or removed bodies. The decoded design
and extracted parts still remain in memory for mapping. Placement admission copies each model's own data without first cloning
its entire child tree. Sketch ownership reads only its feature and definition records;
reference sketches expand the neutral model once, without native payloads or a second
geometry copy. Checking adds its own allowance as described above.

`--no-verify` keeps checking off. `--no-memory-check` forces requested checking even if
memory is low. Desktop Advanced offers the same override as **Convert and check history
even if memory is low**. CLI, desktop, web conversions and MCP use the same policy.
