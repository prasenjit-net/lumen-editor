# Lumen development guidelines

These instructions apply across this repository. Lumen is a native Rust/egui
file editor inspired by VS Code's visual language and practical workbench
behavior. Preserve existing functionality unless a task explicitly changes it.

## UI principles

- Follow VS Code's compact, restrained desktop-editor aesthetic: efficient use
  of space, clear hierarchy, subtle rounding, and consistent interactions.
  Avoid oversized padding, floating-card layouts, heavy shadows, or decorative
  gradients. If an exact new VS Code behavior is requested, inspect a current
  reference instead of inventing it.
- Reuse `src/ui/theme.rs` for colors, typography, and interaction states. Do not
  introduce isolated button/input colors or unrelated palettes in components.
  Current defaults: editor `#1e1e1e`, panels `#252526`, activity bar `#333333`,
  inactive tabs `#2d2d2d`, macOS title strip `#3c3c3c`, status/accent `#007acc`,
  text `#cccccc`, muted text `#9d9d9d`, inputs `#313131`.
- Keep rounding subtle and visible: shared pane surfaces use 5px corners with
  a 2px gutter; tabs use 3px upper corners; inputs/buttons use 3px corners.
  Do not let adjacent opaque surfaces hide pane corners. Reuse `surface()`
  and `canvas()` rather than approximating them per panel.
- Panel headers must keep titles left-aligned and command groups anchored to
  the right at every supported width. Use `panel_header()` in
  `src/ui/widgets.rs` for titled workbench panels, including headers without
  actions. Reserve space for commands before sizing the title; truncate long
  titles rather than wrapping, overlapping controls, or changing row height.
  Keep action order stable and supply descriptive tooltips.
- Header alignment also applies to new titled panels. Editor tab strips,
  status bars, dialogs, and native window chrome have distinct semantics;
  preserve their established layout rather than forcing panel headers on them.
- Use the bundled Codicons via `src/icon.rs`; reuse existing glyphs instead
  of emoji, Unicode approximations, or an unrelated icon set. Workbench action
  buttons are compact and unobtrusive; large activity icons are a separate
  component and should not dictate toolbar sizing.
- Keep system UI fonts with existing embedded fallbacks, and monospace fonts
  for text editing. Match established text sizes and row heights; don't enlarge
  an entire panel to make one control easier to lay out.
- Panes intended for resizing must retain functioning drag handles, sane
  minimum sizes, and persistent widths. Test narrow, default, and wide sizes;
  content and headers must respond without moving fixed-edge controls.
- Respect native platforms: macOS menus live in the desktop menu bar with real
  traffic-light buttons; Windows uses its native menu/title bar and caption
  controls. Use Command on macOS and Ctrl on Windows. Do not add simulated
  native controls or an in-window macOS menu. Keep the themed macOS title strip.
- Keep focus, keyboard navigation, hover/selection/disabled states, clipping,
  tooltips, and file operation feedback usable and consistent.

## Architecture and clean code

- Keep `main.rs` small; `bootstrap.rs` wires startup and platform integrations.
- `model/` contains document/editor state and rules, with no egui or I/O.
- `app/` owns orchestration, shared typed commands, input routing, and view
  state. File workflows belong in `app/documents.rs`.
- `services/` owns filesystem/dialog interfaces and local/native adapters.
  Use these boundaries so workflows can be tested without native dialogs.
- `ui/` contains focused workbench components, shared widgets, and the theme.
  Components borrow state through `Workbench` and issue typed commands; they
  must not open native dialogs or write files directly. Explorer reads use
  the filesystem interface. Share drawing/layout primitives before duplicating
  a pattern across panels.
- `platform/` isolates native menus and font setup. `xtask/` handles packaging;
  do not invoke recursive Cargo builds from `build.rs`.
- Menus, shortcuts, palettes, and buttons must reuse the same command behavior.
  Buffer-only commands apply locally to preserve same-frame tab behavior;
  I/O commands dispatch through the controller after each component.
- Prefer descriptive types and names, small cohesive functions, and explicit
  dependencies. Use enums instead of magic action indices. Avoid speculative
  frameworks, unnecessary dependencies, or broad abstractions for one widget.
- Preserve egui widget IDs and rendering order when refactoring so focus,
  scroll positions, resize state, and same-frame actions remain stable.
- Preserve document content, dirty flags, open/save/save-as behavior, duplicate
  tab handling, close/quit prompts, cancellation and error feedback. Do not
  silently bundle unrelated behavioral fixes into layout or refactor work.

## Verification and handoff

- Run `cargo fmt --all --check`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets -- -D warnings`. Add regression tests
  for changed layouts and workflows; test errors/cancellation as well as success.
- For platform-sensitive changes, check Windows compilation with
  `cargo check --target x86_64-pc-windows-gnu` when the target is installed,
  and verify macOS packaging with `cargo xtask bundle`.
- Visually inspect UI changes at representative widths using screenshots or
  a live app. For a refactor, compare a stable fixture before/after. Do not
  claim native runtime testing on a platform only checked by compilation.
- Preserve unrelated user changes and unsaved buffers during verification.
  Summarize the result and checks. Commit/push only when asked.
