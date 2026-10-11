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

Application tests use in-memory storage and dialog doubles for open/save,
cancellation, failure and dirty-buffer workflows, plus an egui pointer test
for the explorer resize handle. Windows compilation can also be checked with
`cargo check --target x86_64-pc-windows-gnu` after installing that target.
