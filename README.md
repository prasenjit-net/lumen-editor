# Lumen

Native Rust/egui file editor with native desktop menus.

On macOS, menus live in the desktop menu bar. The title strip uses the
editor theme with real macOS traffic-light buttons. On Windows, the app
uses a native Win32 menu and the native Windows title bar and caption buttons.
File, Edit and View actions share the editor's commands; shortcuts use
Command on macOS and Ctrl on Windows.

New File is workspace-aware. With a folder open, it asks for a file name and
creates an empty file in the selected explorer directory (or a selected file's
parent directory). With no selection, it uses the workspace root. The new file
is revealed in the explorer and opened in the editor. Enter confirms; Escape
or Cancel makes no changes. Existing files are never overwritten by creation,
and errors stay in the prompt so you can correct the name or retry.
Without an open folder, New File creates an untitled editor tab as before.

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
Keep file workflows in `app/documents.rs`, document rules in `model/`, and
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
