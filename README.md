# Lumen

Native Rust/egui file editor with native desktop menus.

On macOS, menus live in the desktop menu bar. The title strip uses the
editor theme with real macOS traffic-light buttons. On Windows, the app
uses a native Win32 menu and the native Windows title bar and caption buttons.
File, Edit and View actions share the editor's commands; shortcuts use
Command on macOS and Ctrl on Windows.

The explorer follows [VS Code's workbench conventions](https://code.visualstudio.com/docs/editing/getting-started/userinterface):
New File, New Folder and Rename use an inline tree input. Enter confirms;
Escape cancels. Creation uses the selected folder, a selected file's parent,
or the workspace root. Nested names (`src/components/button.rs`) create parent
folders; a trailing slash creates a folder. Errors remain inline for correction.
Cmd/Ctrl+N and the editor's plus button create an untitled tab, independently
of the workspace, matching VS Code's New Text File action.

Right-click a file/folder for open, new file/folder, cut/copy/paste, copy path,
copy relative path, reveal in the system file manager, rename and delete.
The header offers New File, New Folder, Refresh and Collapse All. Tree arrows
navigate/expand/collapse; F2 renames; Delete requests deletion. On macOS, Return
renames, Cmd+Down opens and Cmd+Backspace requests deletion. Text-editor
shortcuts remain separate from explorer operations.

Deletion requires confirmation and moves items to Trash/Recycle Bin, never
silently falling back to permanent deletion. Clean affected tabs close; dirty
buffers stay open without their old disk paths and use Save As next time.
Rename/move updates descendant tabs and preserves unsaved text. Creation,
rename and paste never overwrite an existing destination; copying into an
occupied folder generates a unique copy name. A failed recursive copy can
leave a partial destination, reported in the status bar for inspection.

Current limits: single-selection operations, in-app file clipboard, no drag
and drop or explorer operation undo. Symlink copying is rejected explicitly.

## Text editing

The Search activity panel searches an open folder in a cancellable background
scan, showing scanned-file and match counts while it runs. Press Cmd/Ctrl+Shift+F,
type a literal query, then Enter; Match Case,
Whole Word, include and exclude globs narrow results. The searchable
`search.exclude` setting accepts comma-separated globs at user or project scope;
the project value takes priority, and the panel's exclude field adds temporary
patterns. Click a result to open
its file at the match. Results form a virtualized, collapsible folder/file tree
that keeps all bounded matches available, with Replace
actions for all results or an individual folder or file. Replace offers a
paginated, color-coded diff preview with nearby edits shown together
(long groups use per-match excerpts), followed by explicit confirmation. An open
editor tab is skipped, and a file changed
since search is never silently overwritten; failures are reported in Search.
Scans skip links, `.git`, `target`, binary/non-UTF-8 files and files over 2 MiB.
Nested `.gitignore` files use Git-style pattern matching, including anchoring,
escaped names, character classes and negation. The result-count tooltip breaks
down skipped entries; descendants of a skipped directory are not counted.
Git's global excludes and `.git/info/exclude` are not read. See
[the roadmap](plan/vscode-core-feature-roadmap.md) for remaining validation.

The shared editor colors comments, strings, numbers, keywords, types, functions,
properties and markup using a compact dark syntax palette. Rust, TOML, Markdown,
JSON, YAML, JavaScript/TypeScript, HTML, CSS, Python, Go, C/C++ and shell files
receive lexical highlighting; other files remain plain text. Exact filenames
also select their syntax: `CMakeLists.txt`, Makefiles,
`Dockerfile`/`Containerfile`, ignore files, `.editorconfig`, `.env` variants,
`Cargo.lock`, and extensionless `README`/`CHANGELOG`. CMake, SQL, INI and
related file extensions are recognized too. The searchable
`editor.syntax_highlighting` setting can disable colors per user or project.
Highlighting is display-only and does not provide semantic language-server
classification or embedded-language parsing yet. The legacy TextEdit fallback
remains plain text.

Each tab owns its cursor, undo history and scroll state. Right-click in the
editor for Undo/Redo, Cut/Copy, Paste, Select All, Find and Replace.
Right-click preserves the selection. Undoing to saved content clears the dirty
marker; edits and replacements do not save automatically.

With no selection, Copy copies the current logical line without moving the
caret, and Cut removes that line in one undoable edit. Copied lines include a
newline, including the final or empty line. Existing LF/CRLF endings are retained;
cutting the last line removes its preceding terminator. Search inputs and Explorer
clipboard commands remain separate. Pasting a line copied/cut in this Lumen
session with no selection inserts above the current line and preserves the
caret's original column, in one undo/redo step. Selected-text and external pastes
remain ordinary text insertion. Session metadata is matched against exact pasted
text and cleared on other copies or mismatched pastes. egui supplies no custom
clipboard metadata: identical text copied in another app is indistinguishable,
and line provenance does not survive restarting Lumen. The editor adapter retains
received line endings without converting them to the destination convention.
Requested editor pastes now read through a background native clipboard service,
which does not normalize text and returns a request ID with text, empty, or error
results. Superseded/duplicate service results are ignored; failures finish the
request and report status without editing the document. This preserves CRLF line
metadata through that read path. Focused-editor keyboard paste now rereads text
through the same service, synchronously, keeping the event's position relative
to typing/navigation. This bypasses winit's CRLF normalization. Non-editor fields
and pointer-transition batches retain eframe's ordinary transport. The interim
reread cannot recover an earlier clipboard snapshot if another application changes
the clipboard between platform delivery and routing, and can block on the native
read; a future input backend should own transport directly. Live native clipboard
round trips still need verification: macOS automation captured the test app but
could not deliver input (clipboard timeout / noWindowsAvailable). Windows
compilation passes; Windows runtime testing remains pending, not a release gate
for this milestone by user direction.
Requested editor paste captures document ID, revision and selection. Changed text,
caret, focus, modal or input mode cancels delivery before other inputs can consume
it; returning to the original caret/focus does not reactivate a cancelled request.
Native Edit-menu paste shares this validation; other text fields remain ordinary
inputs. Service reads are independently correlated; untagged keyboard/platform
events are not service completions and retain the existing guarded input path.
If an accelerator already delivered a paste event, no additional asynchronous
request is made; editor routing resolves its payload once through the service.
Keyboard batches split at clipboard boundaries and continue in repaint frames:
Copy/Cut see preceding typing/navigation, and later typing follows restored paste
selection. New keyboard input stays behind an older deferred batch. Deferred input
is cancelled, with status feedback, if its document revision, caret, focus, or input
mode changes. This is a temporary TextEdit adapter, not a new editing/undo backend.
Pointer gestures and IME remain on the widget's existing path; mixed clipboard
gestures keep ordinary selected-text behavior. App-wide shortcuts still dispatch
before widget input, rather than participating in this document keyboard queue.

Find uses Cmd+F on macOS or Ctrl+F on Windows/Linux. Replace uses Cmd+Alt+F
on macOS or Ctrl+H on Windows/Linux. The native Edit menu, command palette and
editor context menu expose the same actions. Find seeds from selected text or
the word at the caret and selects the query for easy overwriting. Enter and
Shift+Enter navigate results, and Escape closes Find and returns editor focus.
Use Match Case (`Aa`) or Match Whole Word (`ab`), Replace, or All (Replace All).
Queries and options stay separate for each open document.

This milestone supports literal search, not regex, selection-scoped search,
preserve-case replacement, or multiline input controls. Case-insensitive matching
uses Unicode lowercasing with original-byte boundary mapping, not full Unicode
case folding. Replace All searches one snapshot and is one undoable widget edit.
The gutter and text share vertical scrolling; long lines scroll horizontally
without wrapping, while line numbers stay fixed at the left. Click a line number
to select its whole line; Shift-click extends by lines, and dragging selects a
line range. Holding a gutter drag above/below the viewport scrolls vertically
and extends selection at the visible edge. Speed increases with distance from
the edge; release or a blocking dialog stops the drag. Document bounds clamp
scrolling, and reversing direction responds immediately. Unicode and existing
line endings are preserved. Multi-cursor editing remains planned. See
[the editor plan](plan/text-editor-improvement-plan.md) for the remaining work.

Run from source:

```sh
cargo run -- . src/main.rs Cargo.toml
```

Build the macOS app bundle:

```sh
cargo xtask bundle
open target/Lumen.app
```

Use `cargo xtask bundle --release` for an optimized app, or add `--offline`
when dependencies are already downloaded. The Rust helper builds the editor
first, then packages it; no shell script or recursive `build.rs` is involved.
`CARGO_TARGET_DIR` is respected when set.

Build on Windows:

```powershell
cargo build --release
.\target\release\lumen-editor.exe
```

The bundled Codicons font is attributed in `assets/CODICON-LICENSE.txt`.

## Code layout

`main.rs` is the entry point; `bootstrap.rs` wires the native window, theme,
fonts, startup paths and platform menu. The application is split into layers:

| Module | Responsibility |
| --- | --- |
| `model/` | Documents, open buffers, active tab and workspace; no UI or I/O |
| `app/` | Typed commands, file-operation workflows, input routing and view state |
| `services/` | Filesystem and file-dialog interfaces with local/native adapters |
| `ui/` | Workbench components, shared widgets and theme |
| `platform/` | Native menus and system font setup |
| `xtask/` | Build-time app packaging, separate from runtime code |

The controller owns state and services. UI components borrow a `Workbench`
context and issue `Command` values rather than opening dialogs or saving files.
Buffer-only commands apply immediately to preserve same-frame tab behavior;
I/O commands are dispatched after each component. The explorer uses the read
side of the filesystem interface. Native menus, shortcuts and the command
palette share the same command dispatcher.

To add an action, define a `Command` and handle it in `app/dispatch.rs` (or
`apply_local` for buffer-only changes), then connect its menu or component.
Keep editor workflows in `app/documents.rs`, explorer workflows in
`app/explorer_ops.rs`, document rules in `model/`, and
reusable drawing helpers in `ui/widgets.rs`. Preserve widget IDs and render
order when moving components so focus, scroll and resize state stay stable.

Verify changes with:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

GitHub Actions runs these checks on every push and pull request. Workspace tests
run on Linux, macOS and Windows; the macOS job also tests the legacy editor and
builds the app bundle. CI checks the native builds but does not replace live UI
validation.

To publish a release, update the crate version in `Cargo.toml` and
`CFBundleShortVersionString` in `assets/macos/Info.plist`, then push a release
branch named `r<major>.<minor>.<patch>` (for example, `r0.1.0`). The Release
workflow can also be run manually with an existing release branch name. It
checks that versions agree, tests and packages macOS, Windows and Linux builds,
then creates a `v<version>` GitHub release with archives and SHA-256 checksums.
An existing release tag is never replaced. These builds are not signed or
notarized.

Application tests use in-memory storage and dialog doubles for open/save,
cancellation, failure and dirty-buffer workflows, plus an egui pointer test
for the explorer resize handle. Windows compilation can also be checked with
`cargo check --target x86_64-pc-windows-gnu` after installing that target.

### Default shared editor and legacy fallback

The default workbench uses the shared core for typing, clipboard, IME,
Find/Replace and complete-selection undo/redo, with a cosmic-text/Swash renderer:

```sh
cargo run
```

Option/Alt-click adds a cursor; Option/Alt-drag adds another selected range;
Shift+Option/Alt-drag makes a column selection. Shift+Option/Alt+I places a
cursor at the end of each selected line. Cmd/Ctrl+D selects the next occurrence
and Shift+Cmd/Ctrl+L selects all. Cursor-above/below uses Option+Cmd+Up/Down on macOS,
Ctrl+Alt+Up/Down on Windows, and Shift+Alt+Up/Down on Linux. Tab/Shift+Tab
indent/outdent touched lines once. Cut, copy and paste apply to every selection;
matching clipboard rows distribute in document order, while a different row
count pastes the full text at each cursor. Logical-line copy/cut, native/popup
commands, save checkpoints and stable tab IDs share the same editing owner.
The editor never instantiates TextEdit document history
in this mode; generic fields such as Find remain ordinary text widgets.

Build `cargo xtask bundle --offline` on macOS to produce `target/Lumen.app` with
the shared editor. The legacy single-selection TextEdit backend remains available:

```sh
cargo run --no-default-features
cargo xtask bundle --offline --legacy-editor
```

The latter produces a separate `target/Lumen Legacy.app`. The existing
`--shared-editor` bundler option still creates `Lumen Experimental.app` for
isolated validation, using the same shared backend as the normal build.

The shared editor retains core caret affinity, uses visual grapheme
arrows and caches unchanged shaped layouts/character geometry. Large-file,
many-cursor and long-line probes are recorded in the
[performance baseline](plan/editor-performance-baseline.md).
The user requested default promotion on 2026-10-12 with native acceptance still
pending. This is a default configuration change, not a claim of completed native
IME/VoiceOver validation. Earlier manual passes apply to the preceding prototype.
Clipboard metadata is session-only; the service reread still has its
documented latency/snapshot race. A current-match replacement inside a grapheme
is rejected with feedback; the user explicitly accepted that grapheme-safe rule.
Windows is compilation-
checked, not runtime-tested. See [native validation](plan/native-editor-validation.md).

### Settings

Open Settings with the activity-bar gear, the native Settings menu, the command
palette or Cmd/Ctrl+,. The searchable page is generated from section schemas.
Schemas, validation, precedence, search and TOML encoding live in the independent
[`lumen-settings-core`](crates/settings-core/README.md) crate; disk storage,
application drafts and egui components remain outside it.
Search names, descriptions or IDs; `@modified`, `@pending` and
`@id:editor.font_size` are supported filters.

User settings are stored at `~/.lumen/settings.toml` (Windows:
`%USERPROFILE%\.lumen\settings.toml`). Project settings are stored inside the opened
folder at `.lumen/settings.toml` and override user values. Apply saves only explicit
overrides; Remove Override inherits, while Set Built-in Default writes an explicit
default. Cancel discards the selected scope's draft. No files are created just by
opening Settings or a folder.

The initial controls cover font family (platform or bundled Hack), font size,
line height, UI scale, sidebar visibility and hidden files. Both editor backends
use the typography settings. Set line height at least as large as font size.
Settings preserve document text and history. Outside-project files use user
typography; untitled buffers inherit the current project.

### Unsaved-work protection

Explicit Save remains the default. The Files section in Settings offers
`files.auto_save` (`off`, `afterDelay`, `onFocusChange`, `onWindowChange`) and
`files.auto_save_delay` (100–60000 ms, default 1000). Project settings override
user settings for files in the opened directory. Auto Save does not name or save
untitled buffers, and it never resolves an external-change conflict on its own.

Dirty buffers are backed up about once per second in per-instance files under
`~/.lumen/recovery/` (`%USERPROFILE%\.lumen\recovery\` on Windows). Lumen
skips backups owned by live processes and restores interrupted sessions on the
next start, including untitled text. Divergent recovered copies of one path
stay as distinct buffers; later copies require Save As. The old single-file
`recovery.bin` is imported when present. Invalid backups are retained and
reported in the status-bar Safety details. Backups contain unencrypted document
text; they are not version history or cloud sync. A crash can lose edits since
the latest snapshot.

Lumen checks every open disk-backed tab for external changes and compares disk
contents again just before saving. A changed or removed disk file blocks Save
and Auto Save. The tab marker and active-editor banner open a side-by-side
comparison of disk and editor text (preview limited to 20,000 characters).
Keep Editing, Save As, explicit Overwrite, and Reload from Disk are available;
Reload discards that tab's unsaved edits. An explicit Overwrite rechecks the
disk version before replacing it. Save/Save As stage writes in the target
directory and replace the file only after the write succeeds. There is no
automatic merge or hunk-level diff yet; the remaining work and native
acceptance are tracked in the [core feature roadmap](plan/vscode-core-feature-roadmap.md).

External changes currently require Reload (config files saved in Lumen reload
automatically). Invalid files and disk conflicts are reported without replacing
valid settings or drafts. Close an open config tab before applying its form draft,
or edit/save that config directly. Broader settings capabilities and native
acceptance are tracked in [the settings plan](plan/settings-feature-plan.md).

### Standalone multi-cursor fixture

A separate in-memory prototype
exercises real multi-location input/history and shared text/selection geometry:

```sh
cargo run -p lumen-editor-surface --example spike
```

It reuses Lumen's theme/fonts but does not open files or alter workbench buffers.
Add `--features shaped` to exercise the new renderer. Native validation and
platform runtime checks remain open. See
[the spike evaluation](plan/multi-cursor-backend-spike.md) before backend migration.
