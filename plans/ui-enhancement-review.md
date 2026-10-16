# Lumen UI review and enhancement plan

Reviewed: 2026-10-16. Scope: source-code review; no application behavior changed.

## Recommendation

Keep Lumen's compact desktop-editor identity. Prioritize dependable interactions, honest state, keyboard access, and readable feedback before cosmetic redesign. Retain the current dark defaults, Codicons, native window controls, system UI fonts, and monospace editing. Avoid larger padding, decorative cards, gradients, or additional permanent toolbars.

The biggest immediate opportunities are a functional command palette, coherent editor color schemes, truthful status information, visible focus/hover states, and predictable contextual actions.

## Implementation status and remaining gaps

Reviewed 2026-10-16 against the working tree that became branch `f-ui-polish` (35 modified files, +1449/-554; not yet committed). Local checks on that tree: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` (473 app tests plus crate tests) all pass. No screenshots, live-window or Windows runtime checks have been done for it.

| # | Finding | Status | Remaining gap |
|---|---------|--------|---------------|
| 1 | Command palette | **Done** (see item 1) | Windows runtime unverified. |
| 2 | Themes / scheme mismatch | **Mostly done.** `workbench.theme` (dark, light, high-contrast, system) in settings-core; `Palette`/`Theme` in `theme.rs`; `editor.color_scheme` and `terminal.color_scheme` default to `workbench`; project-scoped editor scheme; apply-failure, cancel and system-change tests; contrast test (>= 4.5) for syntax colors. | Screenshot review of every pane in light and high-contrast; confirm diffs, popups and the native window appearance follow the theme; the `Palette` accessor still reads visuals per widget, so audit remaining hard-coded colors (e.g. `Color32::from_gray`, local warning colors in Problems). |
| 3 | Status bar | **Done** (2026-10-16). Line ending is detected from the active text (LF, CRLF, CR, Mixed, No EOL); `Ln, Col` and selection length (shared editor) click through to Go to Line; no encoding/EOL claim without a document; right-hand facts are measured and reserved first, the transient message truncates (full text on hover). Pure logic in `model/status_info.rs`. | Encoding is still a fixed `UTF-8` label (not interactive); conversion not implemented; no test for the live position text. |
| 4 | Misleading controls | **Done.** Run and Debug and Extensions show an explanation; the Run button is disabled with a reason; the dead Extensions search and its `view.search` field are removed; non-native-platform menu names other than File are muted with an explanatory tooltip. | File is the only functional menu on those platforms. |
| 5 | Hover/focus/disabled states | **Mostly done** (2026-10-16). `explorer_row` draws a 1px accent keyboard-focus ring independent of selection and exposes name plus selected state to AccessKit; glyph-only Find close, tab close and signature-help close are named. Regression test covers ring and accessible state. | No separate inactive-selection role; disabled explanations exist for the palette only; remaining icon-only controls not audited. |
| 6 | Explorer | **Partly done.** One `explorer_entry_hidden` rule now drives drawing and arrow-key traversal (tested); rows show the full path on hover. | Reserved badge area at narrow widths, git-letter explanations and a blank-area context menu remain. |
| 7 | Context menus | **Mostly done.** Document tabs have a context menu (Close, Close Others, Close to the Right, Close All, Copy Path, Copy Relative Path, Reveal) with disabled reasons; right-click does not activate the tab, and middle-click closes. Bulk close targets stable document IDs, closes clean tabs first, then prompts for each dirty one through the existing dialog; Cancel or an incomplete Save abandons the rest, the active tab is preserved and the diff-only hidden tab is skipped. Editor menu language/agent entries now use the palette's availability rules with reasons; Problems rows offer Copy Message / Copy Location; Explorer menu shows Cut/Copy/Paste shortcuts and disabled explanations. | Source-control row menus, diagnostics tests for the copy actions, keyboard-invoked context menu verification, and live visual check. |
| 8 | Hover/completion anchoring | **Mostly done.** Hover is anchored to the caret it was requested at (not a fixed editor offset), is dropped if the caret moved before the reply, and dismisses when the caret moves, the tab changes or its close button is used. Hover, completion and signature popups share `ui/popup_place.rs`: they open on the preferred side (hover/signature above, completion below), flip when the other side has more room, and clamp horizontally to the editor so an open agent panel is not covered. The active signature parameter is underlined as well as accent-coloured. Tests: placement at every edge, hover reply anchoring/stale drop, render-level on-screen check at 500/900/1400 px. | Delayed pointer hover over symbols (needs pointer-to-byte mapping and request cancellation), selected-completion detail/documentation pane, high-UI-scale check, Escape focus-return audit, live visual check. |
| 9 | Typography / shortcut labels | **Shortcuts done.** `ui/shortcut.rs::primary` is the single formatter (⌘/⇧/⌥ glyphs on macOS, `Ctrl+` elsewhere), used by the palette, editor context menu, fallback File menu, welcome hint, activity tooltips and the Welcome status message. | Typography role centralisation and a Linux font fallback strategy remain open. |
| 10 | Pane geometry | **Mostly done.** Terminal, Problems and Agent panels now use `canvas(ctx)` + `surface(panel)` like the sidebar, so 5px corners and the 2px gutter show. | Visual check at several widths still owed. A reset-layout command is not feasible yet: egui's persisted panel state type is private, so it needs its own stored sizes first. |
| 11 | Problems / source control | **Mostly done.** Problems: filters wrap instead of overflowing; each row reserves its agent action and truncates the message with the full text, location and source on hover; Copy Message/Location; a semantic `error` palette color (contrast-tested in all themes) replaces the diff token for errors across Problems, Output, terminal, agent, search, navigation and editor underlines; the empty state says why when no server is running, starting, missing or failed, and a filtered-empty list is worded differently. Tests cover truncation at 320/600/1200 px and every empty state. Source control already reserved its action areas. | Branch/ahead-behind summary (only if the snapshot supplies it), a live visual check, and the other `diff_removed_text` uses that are genuinely diff colors were left alone. |
| 12 | Terminal | **Mostly done.** PTY columns use the font's measured advance (`glyph_width`) instead of `font_size * 0.58` (test scales columns with font size); right-click menu offers Copy, Paste (via the platform's own clipboard events, same path as the shortcuts) and Clear. | Terminal selection/copy across rows is still egui label selection (not audited); wide/combining characters unverified; output jump-to-latest not added. |
| 13 | Agent attention | **Done.** The badge is drawn on the real (lower) agent button, stays visible with the panel closed, and the accessible name adds "waiting for your approval"; regression test covers appear/clear. | - |
| 14 | Dialogs / settings | **Mostly done.** Unsaved-changes, quit, save-conflict and unapplied-settings prompts: Escape cancels (and is consumed), focus starts on the safe primary button (Save / Save All / Keep Editing), Tab stays inside the dialog, long names wrap, focus returns to the editor on cancel; tests cover Escape and that Enter cannot discard. | Background input blocking across every route not re-audited; settings pending-count/scope hints and font/theme previews not added. |
| 15 | Breadcrumbs / welcome | **Partly done.** The editor header is now clickable relative-path breadcrumbs (`model/breadcrumbs.rs`, `ui/breadcrumbs.rs`): directory crumbs reveal that folder in the Explorer (new `RevealInExplorer`, expands ancestors and selects, refuses paths outside the workspace); the file crumb opens the symbol picker when a server provides it; leading segments collapse into a hover-able ellipsis before the file name ever clips; untitled and outside-workspace files show just their name. Welcome view gains New File and up to five recent files, which fail safely when missing. Tests cover crumb building, collapsing, reveal, click-through and the missing-recent path. | Recent folders (needs a persistence format beyond today's file list), tab reordering/pinning/preview/split editors, and navigation history remain separate projects. |

### Verification note

`screencapture` is blocked in the agent environment, but the app's own `ViewportCommand::Screenshot` works, so a pass was done that way (light, dark and high-contrast; palette, terminal, agent, problems, settings, source control, search, dialogs, welcome, breadcrumbs). It found and fixed: missing glyph boxes for the Shift/Option shortcut symbols, and Problems/Settings/Search/Source Control bodies touching the pane edge. Still not covered: Windows/Linux rendering, high UI scale, populated Problems rows, and the tab context menu.

### Suggested next order

1. Commit the current theming and palette work in its own branch (it is on `main` today) after a visual pass at 800x600 and 1280x800 in dark, light and high-contrast.
2. Fix the correctness gaps that are small: #13 agent badge, #6 hidden-entry parity, #3 truthful status, #4 disabled placeholders.
3. Add the remaining shared-state work from #5 (focus ring, accessible names), then #7, #8, #11.
4. Leave #12 metrics, #14 and #15 for separate plans.

## Review coverage and confidence

Reviewed the workbench theme and shared widgets; activity/sidebar layout; Explorer and its keyboard/context actions; editor tabs, editor surface integration, find and context menu; command/navigation pickers; settings; source control/diffs; Problems; terminal/output; agent panel; dialogs; platform font setup; and supporting settings schemas. Inspected existing UI test coverage and the default `shared-editor` feature boundary.

- **Confirmed** means directly visible in the inspected implementation. It does not mean reproduced in a live window.
- **Enhancement** means an intentional product addition, not a claimed regression.
- **Validate** means a code-level risk requiring interactive or screenshot verification before calling it a visual defect.
- This review did not launch the app, inspect screenshots, or test native screen readers. Native font rendering, popup clipping, actual contrast in rendered glyphs, and small-window interaction remain visual QA tasks.
- Source references below use paths and function names so they remain useful as lines move. Paths are relative to the repository root.

## Existing strengths to preserve

- `ui/widgets.rs::panel_header` reserves command width before truncating titles; regression tests exercise narrow/default/wide headers.
- `surface()` / `canvas()` establish subtle pane corners and gutters. Tab overflow already has a menu, path tooltips, active-tab reveal, and width tests.
- Explorer already supports inline create/rename and contextual filesystem operations. Extend these rather than replacing their controller semantics.
- Quick Open already has keyboard navigation and focus-return tests. Settings already has search, scope, modified/pending filters, responsive categories, and an anchored footer.
- Workspace Search already has cancellation, result virtualization, previewed replacement, full-row clickable results, and narrow-width tests. Source control has stage/unstage, discard confirmation, and synchronized diffs.
- Editor mutation, selection, clipboard validation, gutter mapping, and shared viewport behavior have substantial safeguards. UI polish must preserve them.
- Agent UI already has named controls, permission handling, attachments, history, streamed content, and virtualized transcripts. Build on these working features.

## Prioritized findings

Priority: **P1** = address first for trust/usability; **P2** = consistent daily-work polish; **P3** = optional scope expansion. Effort: S = focused component, M = coordinated components/controllers, L = wider architecture/workflow work.

### 1. Make the command palette searchable and keyboard-operable — P1 / M

**Implemented (2026-10-16):** Dedicated palette query/selection/prior-focus state; case-insensitive all-word filtering; Up/Down selection; Enter execution; Escape/title-close focus restoration; bounded, responsive scrolling; empty-result messaging; platform shortcut labels; and disabled-command explanations. Quick Open remains available without a workspace for recent files. Language actions use ready-state and advertised capabilities. Search is independent of Extensions. Existing typed command dispatch is preserved.

**Verification:** Nine palette regression tests cover filtering, keyboard execution, disabled/empty results, focus restoration, independent search state, recent-file access, language availability, and final-result visibility at 360×300, 800×600, and 1400×900. Workspace tests, formatting, Clippy, and macOS bundling pass. Live macOS checks exercised filtering, disabled Save, New Text File, empty results, and Escape restoring editor focus. Windows cross-check was attempted but blocked by missing `x86_64-w64-mingw32-gcc`; no Windows runtime claim.

**Confirmed:** `src/ui/palette.rs::palette` writes the query to `view.search` but renders the entire fixed command list without applying the query. It has Escape handling, but no explicit selected-result Up/Down/Enter flow or bounded scrolling. `src/ui/explorer.rs::sidebar` also uses `view.search` for the Extensions field.

**Plan:** Give the palette its own query and selected-result state. Filter immediately, initially with case-insensitive matching; add fuzzy ranking only if useful. Support Up/Down, Enter, Escape, visible selected-row scrolling, empty results, and focus restoration. Display platform shortcuts and disable unavailable commands with explanations. Share a small typed command metadata list with menus/tooltips where practical; keep execution in existing controllers. Do not conflate command search with extension search.

**Acceptance:** Searching “save” removes unrelated commands; Enter executes the selected enabled command once; Escape restores prior focus; all results remain reachable in a short window; unrelated search fields retain their values.

### 2. Fix code-scheme/background mismatch, then add complete themes — P1 / M–L

**Confirmed:** `src/ui/theme.rs::configure_ui` forces dark visuals and a dark native theme. `syntax_palette("lumen-light")` supplies colors including black properties and blue keywords; `editor_shared.rs` applies that syntax palette while `editor.rs` retains `surface(BG)`. Appearance settings currently expose UI scale, not a workbench theme (`crates/settings-core/src/appearance.rs`).

**Plan:** First ensure every offered editor scheme pairs foreground/syntax colors with a compatible editor background, selection, gutter, caret, diagnostics, and find colors. Then add coordinated Dark, Light, High Contrast, and Follow System workbench choices. Keep current dark defaults. Implement schemas in settings-core and resolve palettes through `theme.rs`; pass semantic colors to the shared editor instead of creating independent UI palettes. Respect project/document precedence for editor overrides without accidentally retheming unrelated UI.

**Acceptance:** No offered scheme produces black code on the current dark canvas. Theme switching updates panes, popups, selection, diffs, terminal defaults, and native appearance coherently without losing focus, documents, or pending settings drafts.

### 3. Make status information accurate, useful, and width-safe — P1 / M

**Confirmed:** `src/ui/chrome.rs::status_bar` displays literal `UTF-8   LF`; it does not derive line endings from the active buffer. The right side includes language/LSP state and multicursor count, but no primary line/column. The left status message is an unconstrained label preceding other items.

**Plan:** Derive supported document metadata, show mixed line endings honestly, and add primary line/column and selection length. Clicking position can reuse Go to Line. Only make encoding/language/EOL fields interactive when backed by real commands. Reserve fixed right-side items first; truncate transient status text and expose the full message via tooltip/details. Let long LSP names degrade gracefully.

**Acceptance:** CRLF, LF, mixed-ending, untitled, no-document, and multicursor fixtures show truthful information. Long failures cannot push diagnostic counts or position offscreen. Conversion, if later implemented, must be explicit and undoable.

### 4. Remove misleading enabled controls — P1 / S

**Confirmed:** `src/ui/explorer.rs::sidebar` draws a “Run project” button without using its response and an Extensions search field without results. These activities look available in `activity_bar.rs`. Non-macOS/non-Windows chrome draws several menu names as plain labels (`chrome.rs::title_bar`).

**Plan:** Mark unfinished areas clearly with a concise explanation, disable nonfunctional actions, or hide them until supported. Keep navigation to explanatory panels if that helps discoverability. Do not introduce a run/debug engine or extension marketplace as part of visual polish. Make fallback-platform menus functional or visibly noninteractive.

**Acceptance:** Every enabled button/search/menu produces the advertised result or an explicit supported-state explanation.

### 5. Standardize hover, focus, selection, and disabled states — P1 / M

**Confirmed:** `theme.rs` uses `HOVER = #2b2b2b` and `ACTIVE = #04395e`; several custom rows and activity icons paint their own states. `widgets.rs::explorer_row` paints hover/selection but no explicit keyboard-focus treatment or accessible tree metadata. `widgets::named` is used in shared headers and agents, but icon-only tab/find/completion controls do not consistently use it.

**Plan:** Add shared semantic roles for row hover, focused selection, inactive selection, keyboard focus, warning, error, success, and popup borders. Retain current base colors. A slightly lighter dark row hover such as `#323232` is a candidate to compare visually, not an automatic global replacement. Use a thin focus outline independent of selection, and pair errors/warnings with text or glyphs. Audit painted rows for accessible names, roles, selected/expanded state, keyboard activation, and visible focus. Give disabled actions an explanatory disabled tooltip.

**Acceptance:** Mouse hover, active selection, inactive selection, focus, and disabled state are distinguishable. Every glyph-only action has a spoken name. Keyboard users can find and activate controls without relying on color or a tooltip.

### 6. Improve Explorer readability and navigation parity — P1 / M

**Confirmed:** `explorer_row` paints a combined icon/name/badge string clipped to the row. Explorer rows have no full-path tooltip. Git badges are appended to the name and can disappear when clipped. `explorer_entry` excludes hidden names when configured, but `explorer_actions.rs::visible_explorer_entries` does not apply that same filter, so keyboard traversal can select hidden entries.

**Plan:** Use one visible-entry model for rendering and keyboard traversal. Keep the focused entry in view. Add full-path/status tooltips, truncate only the name, and reserve a trailing badge area. Explain Git letters on hover and retain non-color identifiers. Separate disclosure hit targets from row selection if adopting that behavior; document it and test it. Add a blank-area context menu for valid workspace-level operations.

**Acceptance:** Hidden rows are never selected by arrow navigation; long paths expose their full value on hover; selected rows scroll into view; badges remain visible at 180px sidebar width; inline editing retains Enter/Escape and failure recovery.

### 7. Make context menus consistent and target-safe — P2 / M

**Confirmed:** `editor_menu.rs` groups editing and language actions and displays shortcuts. `explorer_actions.rs` has useful grouped actions but no shortcut labels. Tab rendering in `editor_tabs.rs::tab` has no context menu or middle-click close handling. Editor language/agent menu actions are presented without the capability enablement used for undo/redo.

**Plan:** Standardize label wording, separators, shortcut columns, disabled explanations, and dismissal/focus return. Add tab menus starting with Close, Copy Path, and Reveal; add Close Others/Right/All only with dirty-buffer cancellation tests. Middle-click close is an optional preference-compatible enhancement. Route delayed document actions by stable document identity. Check language capabilities and agent availability before presenting enabled actions. Add Copy Message/Location context actions to diagnostics and existing file operations to source-control rows where useful.

**Acceptance:** Right-click never clears editor selection, opens the wrong file, or changes a delayed action's target. Destructive and bulk close operations retain existing prompts; cancelling stops the remaining operation safely. Keyboard context-menu invocation is verified on supported platforms.

### 8. Anchor hover and completion to the code being inspected — P2 / M

**Confirmed:** `editor.rs` places language hover at a fixed offset from the editor UI origin. `app/language_services.rs::request_language_hover` requests at the caret. Completion/signature UI uses caret geometry but fixed position/size bounds and small 11–12px fonts (`completion.rs`).

**Plan:** Preserve the explicit Show Hover command and add optional delayed pointer hover using the existing viewport hit mapping. Attach requests to document/revision/position, cancel stale results, avoid moving the caret, and keep the popup open while the pointer enters it. Clamp/flip hover, completion, and signature popups within available editor/window bounds; measure behavior with egui's own area constraints before adding redundant logic. Improve signature active-parameter emphasis, readable documentation, and selected completion detail without increasing every UI font.

**Acceptance:** Hover targets the symbol under the pointer, never edits or relocates selection, and ignores stale responses. Popups remain reachable at every viewport edge, under high UI scale, and beside an open agent panel. Escape dismisses and restores appropriate focus.

### 9. Unify typography and platform shortcut labels — P2 / M

**Confirmed:** Theme text roles coexist with many local 11/12/13px overrides. `platform/fonts.rs` uses Arial/Menlo paths for all non-Windows targets and Segoe UI/Consolas on Windows. The welcome screen hard-codes Command glyph shortcuts; activity tooltips display “Cmd/Ctrl”; fallback File menus also use Command glyphs.

**Plan:** Retain compact sizes (body about 13px, buttons 12.5px, metadata 11px) but centralize recurring roles and avoid making essential error/documentation text tiny. Resolve the actual platform UI font through platform code with existing embedded fallbacks; give Linux its own fallback strategy. Preserve monospace code and selectable editor/terminal fonts. Use one platform shortcut formatter for menus, hints, and tooltips. Audit shared-editor shaping and egui text together before changing font registration.

**Acceptance:** macOS shows Command labels; Windows/Linux show Ctrl labels. Test glyph coverage, CJK, combining text, mixed-direction text, Codicons, and 100/125/150/200% scale. Typography changes cannot desynchronize gutter, caret, wrapping, or terminal cell placement.

### 10. Make pane geometry and bottom-panel behavior consistent — P2 / M

**Confirmed:** Explorer/editor/activity panes use shared `canvas()`/`surface()` primitives; terminal, Problems, and agent outer panels use direct frames. Terminal/Output share tabbed headers while Problems has its own titled bottom panel.

**Plan:** Apply shared pane surfaces where their workbench semantics match. Visually verify exposed 5px corners and 2px gutters. Decide whether Problems should join a single Terminal/Output/Problems panel; treat that as an explicit layout change, preserving each tab's scroll/focus and existing commands. Add a reset-layout command and verify persisted sizes before promising restart persistence.

**Acceptance:** Resize handles remain usable, pane widths/heights remain stable as content arrives, fixed-edge actions do not drift, and short windows preserve enough editor space. Avoid opaque nested frames hiding the intended corners.

### 11. Improve diagnostic and source-control scanning — P2 / M

**Confirmed:** `problems.rs` uses an unwrapped horizontal filter row and a message button followed by an agent action; lengthy messages can compete for space. Source-control rows already reserve action areas and have narrow-width tests. Warning colors are repeated locally; diagnostic errors reuse a diff text color.

**Plan:** Reuse the source-control width discipline in Problems: reserve trailing actions, truncate primary text, and offer full message/details and Copy actions. Make filters responsive. Use semantic diagnostic colors rather than diff tokens. Expose loading/error/stale states clearly; preserve the existing “open files” diagnostic scope wording. Consider branch/ahead-behind summary only when the snapshot actually supplies it.

**Acceptance:** Long messages and paths cannot displace actions; filtered-empty and genuinely-empty states remain different; staging/discard controls and confirmation retain their existing behavior.

### 12. Tighten terminal and output interaction — P2 / M–L

**Confirmed:** `terminal.rs` renders emulator cells into row labels, overlays a focusable canvas, and calculates PTY columns with `font_size * 0.58`. It has no terminal context menu in that component. Output already offers copy/clear and empty-state handling.

**Plan:** Measure cell advance from the selected font instead of assuming a width factor; validate wide/combining characters against terminal-core semantics. Audit selection/copy across rows before claiming full terminal selection support. Add a deliberate terminal menu for Copy Selection, Paste, Select All where supported, and Clear; distinguish Ctrl+C process interruption from copy shortcuts. Provide visible terminal focus and an actionable startup failure. Keep output autoscroll from pulling users away while they inspect older entries; expose a jump-to-latest action if needed.

**Acceptance:** Changing fonts or scale keeps the prompt/cursor aligned with PTY columns. Selection and clipboard behavior do not steal editor shortcuts. Closing a panel and terminating a process have clear, tested semantics. Use harmless terminal fixtures for verification.

### 13. Make agent attention visible and composer layout resilient — P2 / S–M

**Confirmed:** `activity_bar.rs` tests `glyph == icon::AGENT` for its attention dot inside the top activity loop, which never includes the agent glyph. The actual agent button is in the lower loop without that badge. Agent composer, permissions, unsaved-file card, and change summary share the bottom area (`agent_panel.rs`).

**Plan:** Move attention rendering to the actual agent action and name the waiting state accessibly. Verify short-window overflow with multiple cards; bound/scroll details while preserving primary permission and cancel actions. Offer a visible return-to-latest affordance when the user scrolls away during streaming. Preserve the existing permission choices and review workflow.

**Acceptance:** A pending permission is discoverable with the panel closed; the badge clears when resolved; no request is approved by layout changes; the composer and current permission remain operable in a short window.

### 14. Improve dialog and settings confidence — P2 / M

**Confirmed:** Save/quit dialogs in `dialogs.rs` use ordinary windows with horizontal button rows. Settings has an intentional Apply/draft model and retains drafts when its tab closes.

**Plan:** Audit dialog focus containment, initial safe focus, Enter/Escape behavior, long filename wrapping, and background input blocking across every input route. Do not assume ordinary windows provide modal behavior. Visually distinguish primary save and destructive discard without relying on red alone. In Settings, keep Apply semantics clear, show pending counts/scope, explain disabled actions, and give theme/font choices a compact preview if implemented.

**Acceptance:** Escape cancels safely, Enter cannot accidentally discard, focus returns to the invoking editor, and cancelling a settings/file operation preserves drafts and dirty buffers. Existing controller blocking must be tested before proposing a replacement.

### 15. Add optional orientation and tab workflow enhancements — P3 / M–L

**Confirmed:** `editor.rs` shows a simple workspace-name/file-name breadcrumb with no navigation. The welcome view provides Open File/Folder and shortcut text. Tabs already handle overflow and path hints.

**Plan:** Add clickable relative-path breadcrumbs and optional symbol navigation using existing providers; add recent files/folders and a clear New File action to the welcome view. Consider tab reordering, pinned tabs, preview tabs, split editors, and navigation history as separately scoped projects. These require explicit state/command design and cancellation tests, not decorative UI changes. Avoid adding a minimap or permanent extra chrome until user need is demonstrated.

**Acceptance:** Duplicate filenames remain distinguishable, deep paths truncate predictably, unavailable recent paths report recoverable errors, and optional preview behavior never replaces a dirty buffer.

## Suggested delivery order

1. **Interaction correctness:** palette filtering/keyboard flow; Explorer hidden-row parity; truthful status; inactive feature messaging; agent attention badge; platform shortcut labels.
2. **Shared visual language:** semantic colors and text roles; focus/accessibility helpers; scheme/background correction; consistent tooltips and pane surfaces. Implement full theme selection after these foundations.
3. **Daily editing polish:** tabs/context menus, Explorer badge layout, anchored hover/completion, diagnostic responsiveness, dialogs, terminal metrics and selection audit.
4. **Optional workflows:** full light/high-contrast/system theme support, navigable breadcrumbs, recent-item welcome view, tab/split editor extensions. Break large items into separate plans.

Each change should preserve widget IDs and render order where possible. Keep rendering in `ui/`, typed dispatch/orchestration in `app/`, document rules in `model/` or the editor crates, settings schemas in settings-core, and OS integration in `services/` / `platform/`.

## Verification plan for implementation

- Capture before/after screenshots using stable fixtures at 800×600, 1280×800, and 1920×1080, plus the actual supported minimum window size. Exercise sidebar widths 180/240/600px and agent panel min/default/max widths. Include 100/150/200% UI scale and short-window layouts.
- Fixtures: empty workbench; long/duplicate filenames; hidden files; dirty tabs; LF/CRLF/mixed text; long unwrapped code; many tabs; long diagnostics; unavailable LSP; staged/unstaged diff; pending settings; terminal failure; streaming agent with permission and change cards.
- Exercise mouse hover, right-click, secondary-click selection preservation, full-row hit targets, middle-click if added, scrollbars, resize cursors, keyboard-only navigation, focus return, and screen-reader labels. Verify Windows/macOS runtime behavior separately; compilation is not native testing.
- Add regression tests for each changed behavior, including disabled/empty/error/cancel paths. Extend existing header, tab, sidebar, settings, clipboard, viewport, and command tests instead of duplicating implementation details.
- Run `cargo fmt --all --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`. For platform-sensitive implementation, also run Windows target check when installed and `cargo xtask bundle` for macOS packaging.
- Preserve unsaved buffers and unrelated changes. Do not replace text mutation/history ownership as a side effect of UI work.

## Reference baseline

Consulted official documentation on 2026-10-16 for current product conventions, not as a promise of feature parity:

- [VS Code user interface](https://code.visualstudio.com/docs/editing/getting-started/userinterface): workbench orientation and discoverable commands.
- [VS Code themes](https://code.visualstudio.com/docs/configure/themes): a coordinated theme covers UI and code highlighting.
- [VS Code accessibility](https://code.visualstudio.com/docs/configure/accessibility/accessibility): keyboard command access and color/accessibility considerations.
- [VS Code terminal appearance](https://code.visualstudio.com/docs/terminal/appearance): terminal palettes need contrast considered alongside the theme.

Before implementing an exact VS Code interaction, inspect that specific current behavior and adapt it to Lumen's native architecture. The recommendations above are Lumen design proposals unless explicitly marked as confirmed code findings.

## Audit checks

- Documentation-only change; no runtime or platform code modified.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace`: passed. Cargo reported an existing output-name collision between the two `profile` examples in editor-core and editor-surface.
- Visual/native runtime testing and packaging were not performed for this plan-only audit.
