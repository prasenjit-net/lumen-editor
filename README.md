# Lumen

Native Rust/egui file editor with native desktop menus.

On macOS, menus live in the desktop menu bar. The title strip uses the
editor theme with real macOS traffic-light buttons. On Windows, the app
uses a native Win32 menu and the native Windows title bar and caption buttons.
File, Edit and View actions share the editor's commands; shortcuts use
Command on macOS and Ctrl on Windows.

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
