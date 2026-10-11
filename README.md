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
editor for Undo/Redo, selected-text Cut/Copy, Paste, Select All, Find and Replace.
Right-click preserves the selection. Undoing to saved content clears the dirty
marker; edits and replacements do not save automatically.

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
The gutter and text still scroll independently; multi-cursor editing and
empty-selection line copy/cut remain planned. See
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
