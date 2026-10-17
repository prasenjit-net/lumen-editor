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

### Long-line and gutter follow-up

The probe now reports fixture names/byte counts and includes a 13,000-byte single
line with ligatures, combining marks, CJK and emoji. Offscreen gutter labels no
longer produce paint commands; empty visible rows still keep their numbers.
The full document geometry and accessibility rows remain unchanged.

| Fixture | Bytes | AccessKit | Cold frame ms | Mean idle frame ms |
| --- | ---: | --- | ---: | ---: |
| 1,000 rows | 37,000 | Disabled | 14.042 | 8.673 |
| 1,000 rows | 37,000 | Enabled | 10.670 | 8.754 |
| Long Unicode line | 13,000 | Disabled | 6.864 | 5.620 |
| Long Unicode line | 13,000 | Enabled | 21.484 | 20.492 |

The Unicode sample uses embedded fallback fonts, not a native font-coverage check.
Its enabled cost confirms that indexing document rows does not remove repeated
glyph scans within a long line. A cluster/glyph lookup optimization needs the
same coordinate-equivalence checks before replacing the current public run
lookup; do not infer bidi caret-affinity support from matching positions.

## Remaining performance work

### Completion-batch sample (2026-10-12)

The shape cache now reuses an immutable layout when egui's galley identity is
unchanged. A glyph/cluster index preserves the public run cursor-position policy
without rescanning all glyphs for every character; positions and visual arrow
stops are lazy per-layout caches. Text/source projection, font/style/scale changes
and document identity invalidate the corresponding cached shape. Source mappings
are not shared across differing LF/CRLF content. Cache equivalence/invalidation
and affinity/history tests accompany this change.

The expanded release probe measures cold + three idle frames:

| Fixture | Bytes | Cursors | AccessKit | Cold ms | Idle mean ms |
| --- | ---: | ---: | --- | ---: | ---: |
| 1,000 rows | 37,000 | 1 | Disabled | 14.256 | 0.995 |
| 1,000 rows | 37,000 | 1 | Enabled | 14.463 | 1.298 |
| 10,000 rows | 370,000 | 1 | Disabled | 81.267 | 4.756 |
| 10,000 rows | 370,000 | 1 | Enabled | 105.680 | 9.381 |
| 1,000 cursors | 37,000 | 1,000 | Disabled | 10.437 | 2.622 |
| 1,000 cursors | 37,000 | 1,000 | Enabled | 12.128 | 2.976 |
| Long Unicode line | 13,000 | 1 | Disabled | 6.934 | 0.131 |
| Long Unicode line | 13,000 | 1 | Enabled | 7.217 | 0.174 |

A quadratic root-row membership check was also replaced with a set lookup;
the preceding 10,000-row enabled sample averaged 20.792 ms idle. Root child order
and unrelated children are preserved. No document/accessibility rows are removed.

These samples exclude native input, GPU presentation, typing, allocator/RSS and
native screen-reader consumers. Cold layout still scales with file size. The core
1 MB typing/history figures above remain relevant; idle layout caching does not
eliminate per-edit buffer rebuilding, source-map construction or full snapshots.

- Unchanged shaping and character geometry are now cached across idle frames.
  Further reduce per-edit buffer/source-map work, then measure native presentation.
- Reduce enabled-accessibility character positioning costs without dropping
  document rows or changing source/selection semantics. Verify tree equivalence
  and native VoiceOver behavior before adopting incremental publication.
- Offscreen gutter labels are culled and a long-line fixture is measured. Extend
  coverage to many cursors and large-file edits separately from cold setup.
- Replace full history snapshots only with an independently tested transaction
  representation; preserve checkpoints, original cursor sets and atomic commands.
- Extend probes to native release builds and larger Unicode fixtures. Do not
  promote the experimental editor based on these small local CPU samples.

### Persistent geometry and tab-return follow-up (2026-10-16)

The surface now retains up to 20 recently displayed layouts. Geometry tables
are shared through `Arc`, and cache keys include widget and buffer identity,
revision, typography, language/palette, semantic token identity, folds, scale,
text color and font definitions. Buffer identity prevents a disk reload with the
same revision/length from displaying old text. Composition bypasses this cache.
Only one revision per widget is retained; eviction bounds layout count, not bytes.
Large documents can still retain substantial layout memory until eviction or
surface destruction. No syntax, shaping or accessibility features are disabled.

The workbench also retains its text snapshot on unchanged revisions, and folding
skips line indexing when there are no collapsed headers. These changes do not
remove the core's full-buffer edit/history costs.

The release surface probe now includes alternating large/small tabs, five return
frames, accessibility disabled, embedded fonts, no syntax coloring or disk I/O:

| Large document | First frame ms | Mean return frame ms |
| --- | ---: | ---: |
| 10,000 rows / 370 KB | 87.218 | 0.399 |
| 50,000 rows / 1.85 MB | 437.304 | 1.298 |

These are same-run cold/return measurements, not before/after measurements of the
whole application. CPU results exclude workbench controllers, native fonts,
renderer/GPU submission and disk reads. Returning to an evicted layout remains a
cold operation. First opening still performs whole-document projection,
highlighting (when configured), font layout and shaping synchronously. Visible-row
or incremental layout remains necessary to reduce that cold latency materially.

Regression coverage verifies recent-tab reuse, bounded retention, same-size
buffer replacement, LF/CRLF source mapping, and snapshot synchronization through
editing, undo and save. Persistent geometry can outlive egui's own galley cache;
source-projection tests therefore assert mapping/size equivalence rather than
requiring shape allocation identity across different revisions.

## Opening and editing large files (2026-10-17)

Release build, macOS, headless 1200x800 egui frames with the shared editor and the
shaped renderer; real layout, shaping, highlighting and status bar, no GPU.
Reproduce with `cargo test --release --bin lumen-editor perf_probe:: -- --ignored --nocapture`
(`big_file_open`, `big_file_typing`; the `*_loop` probes feed `sample`/Instruments).
File contents are synthetic Rust-like text of about 30 bytes per line.

**Cause.** Reading a file and creating its document takes 1-2 ms. Everything else
happened in the first frame and was linear in file size: cosmic-text shaped the
*entire* document up front (about 70% of the open time), egui laid out a whole-document
galley the shaped renderer never draws (about 50% of what remained), and
`EditorBuffer`/selection validation built a grapheme table of the whole text, as did
every arrow key and delete. Each keystroke repeated the shaping, the galley and the
tables.

**Changes.**
- The shaped layout shapes one source line at a time, on demand: the visible rows plus a
  margin when painting, the caret line, the clicked row. Documents up to 256 KB still
  shape everything immediately so their widths stay exact; larger ones estimate the width
  of unshaped lines from the monospace advance until they scroll into view. After an
  edit every shaped line whose text is unchanged is carried over, so only the edited line
  is re-shaped. A test-only copy of the old whole-document layout is compared with the
  lazy one on randomized mixed-direction, emoji, tab and empty-line text (carets,
  bidirectional arrow stepping, hit testing, selection, row geometry).
- egui's galley is not built when a shaper renders the text.
- Grapheme boundary questions (selection validation, arrow, delete, caret snapping, word
  and literal matching) look only at the offset in question instead of tabulating the
  document; a randomized test checks them against the full table.

| First frame after opening | before | after |
| --- | ---: | ---: |
| 600 KB Rust (20k lines) | 268 ms | 117 ms |
| 3 MB Rust (100k lines) | 1,220-1,400 ms | 458 ms |
| 9 MB Rust (300k lines) | 4,200-5,070 ms | 1,247 ms |
| 9 MB plain text | 3,626 ms | 184 ms |
| 9 MB log | 2,890 ms | 183 ms |

| One typed character | before | after |
| --- | ---: | ---: |
| Rust, 10k lines (300 KB) | 120 ms | 38 ms |
| Rust, 50k lines (1.5 MB) | 620 ms | 192 ms |
| Rust, 200k lines (6 MB) | 4,000 ms | 780 ms |
| Plain text, 10k lines | 80 ms | 5 ms |
| Plain text, 50k lines | 420 ms | 29 ms |
| Plain text, 200k lines | 3,000 ms | 120 ms |

Idle frames (scrolling, caret blink, hover) were already cheap from the retained
geometry cache: about 2 ms at 50k lines and 8 ms at 200k.

**What is left.** For highlighted languages the remainder is tree-sitter: a full
re-parse plus a query over the whole tree on every edit (about 80% of a Rust keystroke
at 30k lines) and one full parse when opening. Incremental re-parsing with the previous
tree, and running the highlight query only over the visible byte range, would take it to
the plain-text numbers. Other whole-document passes still run per edit: the geometry
tables (byte/visual offsets, line starts), fold discovery, whole-document undo snapshots
and the status-bar facts. `row_rects()` allocates one rectangle per line for the gutter
and sizing, and accessibility (screen readers only) still shapes every line.
