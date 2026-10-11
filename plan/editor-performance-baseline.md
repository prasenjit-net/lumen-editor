# Experimental editor performance baseline

Measured locally on macOS, 2026-10-11. Diagnostic samples, not CI timing limits or
native runtime/GPU benchmarks. Both probes use embedded fonts, no disk/clipboard
access and deterministic fixtures. Rerun on the target machine before comparing.

```sh
cargo run -p lumen-editor-core --example profile --release --offline
cargo run -p lumen-editor-surface --example profile --features shaped --release --offline
```

The core probe performs 30 insertions at one caret, either independent atomic
edits or a continuous typing group, then undoes all changes. Retained history
text measures string lengths in undo/redo snapshots only, not total memory,
allocation capacity, current/saved contents or selection vectors.

| Initial bytes | Atomic edit total ms | Grouped edit total ms | Atomic history bytes | Grouped history bytes |
| --- | ---: | ---: | ---: | ---: |
| 10,000 | 15.228 | 9.883 | 300,435 | 10,000 |
| 100,000 | 60.799 | 48.217 | 3,000,435 | 100,000 |
| 1,000,000 | 508.106 | 513.818 | 30,000,435 | 1,000,000 |

Grouping significantly reduces retained snapshots but does not remove linear
text rebuilding/grapheme validation on every input. A 1 MB typing sample still
averages about 17 ms per edit before UI layout. History remains capped at 100
groups; repeated atomic edits can still retain roughly 100 document copies.

The surface probe measures a cold frame and three idle frames in a headless
900×600 egui context. It includes CPU layout, paint-command generation and,
when enabled, accessibility tree construction. It excludes renderer submission,
GPU presentation, platform fonts, native IME and OS accessibility consumers.

An initial **debug** probe found roughly 1,941 ms/idle frame for 1,000 rows even
with AccessKit disabled. The shaped accessibility publisher computed every
character's geometry before checking whether a node was requested. Gating the
row work through egui's public node builder reduced the same disabled debug
sample to 154 ms. Enabled accessibility remained roughly 1,971 ms. Debug timings
are evidence of the redundant path, not production frame-rate estimates.

After also reusing the sizing layout for unchanged painting, the **release** probe
reported:

| Rows | AccessKit | Cold frame ms | Mean idle frame ms |
| --- | --- | ---: | ---: |
| 100 | Disabled | 6.019 | 2.191 |
| 100 | Enabled | 6.154 | 4.019 |
| 1,000 | Disabled | 12.930 | 8.383 |
| 1,000 | Enabled | 155.765 | 154.006 |

The intermediate release sample, before sizing-layout reuse, was 15.031 ms for
1,000 rows with AccessKit disabled. These are single local runs; font setup and
system load affect timings. The reuse is limited to one unchanged frame's sizing
and painting passes, not a persistent cross-frame layout cache.

### Accessibility row-index follow-up

The previous enabled path called the buffer-wide cursor lookup for both ends of
every character. Each lookup scanned the document's layout runs. The publisher
now indexes those runs once, then uses the same public run cursor-position method
to build character coordinates. It publishes every row as before, including
offscreen rows, and preserves node IDs, values, bounds and primary selection.

The subsequent release sample was:

| Rows | AccessKit | Cold frame ms | Mean idle frame ms |
| --- | --- | ---: | ---: |
| 100 | Disabled | 6.081 | 1.360 |
| 100 | Enabled | 3.576 | 1.941 |
| 1,000 | Disabled | 15.114 | 8.854 |
| 1,000 | Enabled | 11.787 | 8.588 |

Enabled idle time in this fixture fell from about 154 ms to 8.6 ms. Small
differences between enabled/disabled samples are run noise, not evidence that
accessibility makes rendering faster. Coordinate equivalence tests cover empty
lines, newline boundaries, ligatures, combining marks, tabs, mixed-direction text,
CRLF projection and transient preedit. This does not add bidi caret affinity or
prove native VoiceOver behavior. Long-line glyph scans remain a potential cost.

## Remaining performance work

- Cache/reuse unchanged shaping/layout across idle frames. Sizing/painting now
  reuse a layout when source/preedit are unchanged, but each frame still rebuilds.
- Reduce enabled-accessibility character positioning costs without dropping
  document rows or changing source/selection semantics. Verify tree equivalence
  and native VoiceOver behavior before adopting incremental publication.
- Cull offscreen gutter labels; measure long lines, many cursors and large-file
  edits separately from cold font/glyph setup.
- Replace full history snapshots only with an independently tested transaction
  representation; preserve checkpoints, original cursor sets and atomic commands.
- Extend probes to native release builds and larger Unicode fixtures. Do not
  promote the experimental editor based on these small local CPU samples.
