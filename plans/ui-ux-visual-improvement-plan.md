# Lumen visual style and UI/UX improvement plan

Reviewed: 2026-10-21  
Baseline: `1ef372f`  
Scope: presentation, layout, readability, interaction feedback, accessibility, and discoverability of existing functionality. This document proposes work; it does not implement UI changes.  
Concept mockup: [mockups/lumen-before-after.html](mockups/lumen-before-after.html). Open it in a browser; it shows current and proposed screens side by side, with numbered callouts that name the plan item behind each change. It is an HTML approximation for discussion, not a pixel specification.

## Assessment

Lumen already has a coherent, compact desktop-editor foundation. Keep its dark palette, native window controls, system UI fonts, monospace editing, restrained pane separation, and bundled icons. The largest gains will come from making newer surfaces follow the same rules, fixing specific contrast problems, and giving content a predictable layout when panes become narrow.

Prioritize readable controls and dependable geometry, followed by clearer information hierarchy. A new visual identity, larger default spacing, decorative cards, or additional permanent toolbars would work against the existing product.

### Evidence and limits

- Visually inspected the checked-in [workbench](../assets/screenshots/workbench.jpg), [Settings](../assets/screenshots/settings.jpg), and [Terminal](../assets/screenshots/terminal.jpg) screenshots. These show the established appearance, not a verified rendering of the current commit. For example, the current title strip has task/panel controls, and current agent/task surfaces are not represented there.
- Reviewed the UI module inventory, shared theme/widgets, and presentation paths for the major surfaces listed below. Inspected relevant editor-surface drawing, font setup, and existing layout tests. Recommendations cite source files and functions so they can be checked against implementation.
- Checked the earlier [UI enhancement review](ui-enhancement-review.md) as historical context. Its feature-completion assessments are not used as current evidence. Palette navigation, themes, tabs/splits, tasks, Git UI, and native agent chats already exist in the current source.
- No Lumen window was available in the running-app inventory. This review did not launch an app, alter user settings, or exercise native interactions. Light/high-contrast appearance, resize behavior, and keyboard/screen-reader behavior still need live validation.
- **Visual** below means visible in the repository screenshots. **Source** means directly supported by current rendering code. **Validate** means a plausible usability concern requiring a fixture or live check, not a reproduced defect.

## What to preserve

- Current dark defaults: editor `#1e1e1e`, panel `#252526`, activity bar `#333333`, inactive tab `#2d2d2d`, macOS title strip `#3c3c3c`, status/accent `#007acc`, text `#cccccc`, muted text `#9d9d9d`, input `#313131`.
- Shared 5 px pane corners and 2 px canvas gutters; 3 px upper tab corners and 3 px input/button corners. Keep opaque child backgrounds from covering those corners.
- Fixed-edge header actions through `panel_header()` and related shared header primitives. Editor tabs, status bars, and native chrome retain their own layouts.
- Existing 22 px tree/change rows, roughly 34–35 px tab/header strips, compact toolbar controls, and the separate 48 px activity-button scale.
- Working tab overflow/reordering/splitting, breadcrumb collapse, inline Explorer inputs, scroll persistence, keyboard navigation, and existing context menus.
- Existing state distinctions and protections: dirty documents, unsaved Settings drafts, task-running state, permission choices, cancellation, errors, and destructive-operation confirmation.

## Surface coverage

| Area | Current strengths | Improvement direction | Evidence |
| --- | --- | --- | --- |
| Theme and shared widgets | Central palette, semantic colors, shared surfaces and headers | Complete foreground/background pairs, typography roles, selection/focus states | Source: `theme.rs`, `widgets.rs` |
| Native chrome and activity bar | Real platform chrome; compact activity icons and active markers | Keep document identity visible beside task controls; consistent toggle feedback and names | Visual + Source: `chrome.rs`, `activity_bar.rs`, `task_launcher.rs` |
| Status bar | Compact document facts; message truncation already reserves right-hand space | Validate crowded states and use a defined priority for secondary facts | Source: `chrome.rs::status_bar` |
| Explorer and inline operations | File icons, 22 px rows, inline creation/rename, focus ring | Separate Git badge from filename; ellipsize long names; retain readable inline errors | Visual + Source: `explorer.rs`, `explorer_edit.rs`, `widgets.rs` |
| Editor, gutter and splits | Dominant editing surface; shared text/gutter geometry; configurable typography | Clarify focused group and inactive selection; refine visual layering without changing editor behavior | Visual + Source: `editor_shared.rs`, `editor_split.rs`, `crates/editor-surface/src/` |
| Editor tabs and breadcrumbs | Overflow handling, fixed close targets, active stripe, path collapse | Keep dirty/close affordances clear; validate duplicate names and very long final breadcrumbs | Source: `editor_tabs.rs`, `breadcrumbs.rs` |
| Find and workspace search | Existing match navigation, options, progress and replacement preview | Stable compact controls, shared option styling, stronger distinction between results/errors/partial results | Source: `editor_find.rs`, `workspace_search.rs`, `workspace_search_diff.rs` |
| Source Control | Reserved action space in file/repository rows; semantic markers; operation banners | Carry good row geometry into shared disclosures and make repository/operation context easy to scan | Source: `source_control.rs`, `source_control_rows.rs`, `source_control_menu.rs` |
| Git history, diff and blame | History already changes layout below 640 px; distinct diff colors | Align metadata hierarchy, long-path treatment, selection and change cues across review surfaces | Source: `git_history_tab.rs`, `source_control.rs`, `workspace_search_diff.rs` |
| Terminal, Output, Problems and task output | Shared bottom-panel identity and tab header; persistent height | Use one outer pane treatment; reduce task header crowding; preserve readable output sizing | Visual + Source: `terminal.rs`, `output.rs`, `problems.rs`, `task_output.rs`, `bottom_tabs.rs` |
| Tasks and task editor | Running/scope sections, validation, existing Run/Stop actions | Responsive form labels and consistent status/action hierarchy | Source: `tasks.rs`, `task_editor.rs`, `task_launcher.rs`, `task_dialogs.rs` |
| Settings | Search, scope, category navigation, pinned footer, narrow category picker | Reduce repeated labels/metadata; distinguish scopes, filters and pending changes | Visual + Source: `settings.rs` |
| Native agent session list and chat | Provider identity, session status, capped reading width, pinned composer | Adopt workbench header/control geometry; avoid minimum-width overflow; clarify queued sends | Source: `session_list.rs`, `chat.rs` |
| Legacy agent panel | Existing transcript, approvals, changes and composer | Match native chat's visual vocabulary where appropriate while preserving different capabilities | Source: `agent_panel.rs`, `transcript_layout.rs` |
| Palette, Quick Open and language popups | Shared floating frame; keyboard navigation; bounded caret popups | Consistent search/result hierarchy, restrained elevation, complete focus-return checks | Source: `palette.rs`, `navigation.rs`, `completion.rs`, `popup_place.rs`, `language_navigation.rs` |
| Dialogs, menus and review prompts | Safe defaults in document dialogs; descriptive actions and disabled reasons in several menus | Consistent action emphasis, bounded content and visible errors | Source: `dialogs.rs`, `task_dialogs.rs`, `language_edit.rs`, `editor_menu.rs`, `explorer_actions.rs` |
| Welcome and empty states | Existing open/new/recent actions and several useful panel messages | Shared spacing and vocabulary; fit short editor groups; show an existing next action when useful | Source: `editor.rs`, `explorer.rs`, `tasks.rs`, `session_list.rs` |

Paths in the table are relative to `src/ui/` unless otherwise specified. Language-service UI is conditional: `language-server` is off by default in `Cargo.toml`. Review that UI with an enabled build or fixture; do not present it as default runtime behavior.

## Prioritized improvements

Priority meanings: **P0** = a concrete readability failure; **P1** = important consistency or usability work; **P2** = refinement after the main issues. Effort is relative: **S** = localized presentation change; **M** = several related components; **L** = broad state/viewport validation. These are not calendar estimates.

### 1. Give filled controls explicit foreground/background pairs

**P0 · S/M · Source**

`dialogs.rs::primary_button` always uses white text on `colors.accent`. High Contrast makes that accent yellow. Native chat approval buttons repeat the pairing; `chat.rs::composer` uses white on provider colors for Send.

Calculated from the opaque palette RGB values using sRGB relative luminance:

| Pair | Contrast ratio |
| --- | ---: |
| White on high-contrast yellow accent | 1.07:1 |
| White on dark-theme Codex green | 2.28:1 |
| White on dark-theme Claude orange | 2.67:1 |
| White on existing blue status background | 4.51:1 |
| Muted text `#9d9d9d` on macOS title strip `#3c3c3c` | 4.07:1 |

These are color-pair measurements, not screenshot measurements or a whole-app accessibility certification.

The title-strip pair affects the inactive panel toggles and the task launcher's "Select task" label (`task_launcher.rs`). Muted text passes on ordinary panels (5.65:1 on `#252526`), so this needs a title-strip secondary-text role rather than a global change to `muted`; `#a8a8a8` measures 4.64:1 there. The launcher's run and stop glyphs (5.55:1 and 4.49:1) meet the 3:1 non-text target.

**Plan:** Add semantic filled-action background/foreground roles to `theme.rs`; apply them through shared button presentation. Keep provider color as a secondary identity cue, or provide a tested foreground when it is used as a fill. Audit hover, pressed and disabled variants, links, warning text, diff highlights, and file-icon colors against their actual surfaces. Preserve the established status-bar blue, whose measured white-text pairing is already suitable for the proposed 4.5:1 normal-text target.

**Acceptance:** Every enabled small-text action meets a 4.5:1 project contrast target in Dark, Light and High Contrast, including secondary text on the title strip. Primary actions remain legible when hovered or focused. High Contrast never renders white action text on yellow.

### 2. Finish the shared visual vocabulary

**P1 · M · Source + Visual**

The theme defines text styles, but components frequently bypass them with local sizes from 10 through 16 px. `Palette` has one `active` fill and no explicit inactive-selection or focus-border role. Some controls use `ui.visuals().error_fg_color` while others use `colors.error`. Chat uses 4, 5, 6 and 8 px corners for its inputs, messages and buttons, while the workbench convention is 3 px. Components use nine distinct local text sizes between 10 and 16 px, and nine call sites use `ui.visuals().error_fg_color` alongside `colors.error`.

Floating surfaces (palette, Quick Open, menus, dialogs) deliberately use `floating_frame()` with a lighter `float_border` and an 8 px offset, 24 px blur shadow so they read as raised above the editor. That elevation was a recent, explicit design decision; keep it.

**Plan:** Name a small set of roles for panel captions, body/list text, metadata, headings, toolbar glyphs, focus, inactive selection and semantic feedback. Keep the current scale as the baseline; avoid globally enlarging the interface. Standardize input/action radii and insets; retain deliberate differences for activity icons and useful conversation grouping. Map egui semantic error/warning visuals to the same palette. Keep the floating elevation; only verify that Light's lighter shadow still separates popups from the editor and that High Contrast relies on the white border, since a shadow is invisible on black.

**Acceptance:** Equivalent controls have matching text size, baseline, hit area, corner treatment and state feedback. A focused control is identifiable independently of hover or selection. No component invents a new action palette.

### 3. Make all pane boundaries agree

**P1 · S/M · Source**

Terminal and Problems use `canvas()` plus `surface()`. Output and task output instead use a flat filled outer frame. Switching the same bottom slot therefore changes the surface construction even though the panel ID and saved height are shared.

**Plan:** Use the shared outer surface treatment for every bottom-panel mode, preserving the panel ID, height, resize handle, inner scroll IDs and terminal viewport calculations. Align header/body insets and separators without forcing terminal cells or log lines into ordinary list-row heights. Verify agent-panel child backgrounds and split-editor boundaries preserve visible corners too.

**Acceptance:** Switching Terminal → Output → Problems → task output leaves the outer silhouette, header baseline and divider position stable. Resizing persists, pane corners remain visible, and text/carets do not disappear at panel edges.

### 4. Budget space for labels, badges and actions before drawing rows

**P1 · M · Source**

Explorer appends Git letters directly to filenames; `explorer_row_with_icon()` paints the combined text with clipping rather than ellipsis or a reserved badge area. Long names can obscure the marker. `disclosure_row()` places its title/count before placing hover actions, without reserving their width. Source Control's dedicated rows already demonstrate a better pattern with fixed-edge controls and fitted text.

**Plan:** Give Explorer rows separate caret/icon, flexible filename, and fixed status-badge regions. Reuse fitted text and preserve full path/status explanations on hover and in accessible descriptions. Reserve disclosure actions first; fit title/count into the remaining region. Keep row height, indentation, IDs, selection and inline-edit placement stable. Use semantic status colors with letters or icons so status never depends on hue alone.

**Acceptance:** At sidebar widths of 180, 240 and 600 px, long nested names ellipsize, badges remain visible, and actions never overlap labels/counts. Hovering a row does not move its text. Enter/Escape and error retention in inline inputs remain unchanged.

### 5. Define narrow-layout behavior for controls and forms

**P1 · M/L · Source + Validate**

Several surfaces calculate space independently: Find reserves `width - 340` for its query then wraps the controls; task forms retain a 130 px label column at every width; task output reserves seven header action slots; native chat forces its content column to at least 240 px even when its editor group is narrower. These are code-visible constraints; specific overlaps need rendered reproduction.

**Plan:** Define per-surface collapse rules based on available pane width. Keep the most frequent actions at the fixed edge; put secondary actions in the existing overflow-menu pattern in a stable order. Give Find a deliberate second row at narrow widths instead of incidental wrapping. Stack task form labels above inputs when the control column would be too small. Let chat fit its group without a width floor larger than available space. Keep important buttons visible before allocating flexible text and scroll content.

**Acceptance:** A 900×560 workbench with side panels, splits and a bottom panel remains usable. In 220–300 px editor groups, Find, task forms and chat do not create offscreen controls or negative content widths. A narrow task-output header still provides access to every action and its active tab. Layout changes preserve drafts and focus.

### 6. Reduce Settings repetition and clarify the draft lifecycle

**P1 · M · Visual + Source**

The screenshot and `setting_row()` show a setting title, technical ID, help, checkbox repeating the title, effective-value metadata, a gear, and a separator for each boolean. The toolbar also places scope choices and query filters in one sequence. This gives metadata similar visual weight to the actual setting. `settings.jpg` predates the current source, so confirm the row anatomy against `setting_row()` in a current build before redesigning it.

Settings also lists options for features that are not compiled in. Since `language-server` became an off-by-default Cargo feature, the default build still shows the language-server enable, server-path and startup-timeout settings, and changing them has no effect.

**Plan:** Use one visible label per boolean while retaining a complete accessible name. Make the setting name/control the primary row, help the secondary line, and ID/origin/default information a quieter details treatment. Keep details discoverable and copyable. Visually separate User/Project scope from Modified/Pending filters. Make Pending clear when a draft differs from the live value; retain the pinned Apply/Cancel footer and inline error feedback. Keep the existing narrow category dropdown.

For compiled-out features, let the app tell the settings presentation which features are available, and hide those sections or show them disabled with "Not available in this build". Keep `settings-core` free of Cargo feature checks: availability is supplied by the caller, not discovered inside the crate. Preserve values already saved in user/project TOML so they apply again in an enabled build.

**Acceptance:** More settings fit in the same viewport without reducing body-text size or hiding essential help. The user can identify the active scope, unsaved draft count, effective value and Apply action without relying on color. Unapplied changes never look as though they have already taken effect. In the default build no language-server setting appears editable, and existing saved values survive a round trip.

### 7. Bring agent chat into the workbench's visual system

**P1 · M · Source**

Native chat has a custom two-line header, provider pills, rounded message/approval blocks, and a larger-radius composer. `chat_header()` allocates the title before the right-hand status; its folder line is an ordinary label. Session rows paint their own contents. This area needs the same title/action budgeting and interaction-state discipline as the rest of the editor.

**Plan:** Use `panel_header()` or its shared header machinery for the titled chat panel, reserving status space before fitting the title; put folder context in a clipped secondary row. Use provider identity sparingly and reserve warning/error emphasis for session state. Keep readable conversation width, selectable text, code formatting, and compact expandable tool details. Standardize composer corners/buttons; visibly label the action **Queue** while a message will be queued, instead of explaining that distinction only in a tooltip. Ensure Stop remains accessible alongside it. Apply common status wording and focus treatment across session rows and the legacy agent panel without redesigning their underlying lifecycles.

**Acceptance:** Long session titles, folder paths, waiting reasons and commands fit narrow groups. Queued, working, failed and approval-needed states remain distinguishable without provider colors. Approval text, scope and every allowed choice remain readable; no presentation change alters permission semantics or defaults.

### 8. Clarify editor focus, tabs and search feedback

**P1 · M · Source + Validate**

Selected tabs use a top accent line; dirty tabs replace the close glyph with a dot. Tree focus already has a visible ring. Comparable focus and inactive-selection distinctions should be checked across editor groups, session lists, task lists and history. Find uses text buttons and wrapped controls while workspace search has a different arrangement for related options.

**Plan:** Add a restrained focused-group cue and a separate inactive-selection treatment. Keep the dirty marker visible, and reveal the close glyph on hover/focus within the same hit target. Give Find and workspace search common option-button states and match-count hierarchy. Distinguish empty query, no matches, searching, partial/cancelled results and errors with concise text. Preserve current caret, match, bracket and selection geometry; check that their colors/layers remain distinguishable when they coincide. Validate final breadcrumb truncation and duplicate tab-name context before changing either.

**Acceptance:** The typing destination is obvious with two groups open. Dirty state is understandable before hovering. Search controls have stable positions at defined widths. Selection, current match, other matches and diff additions remain distinguishable without obscuring code. No editor history/input behavior changes are bundled into the work.

### 9. Complete keyboard focus and icon-only control feedback

**P1 · M · Source + Validate**

The project already has `widgets::named()` and several custom focus rings. Adoption is incomplete: the title-strip run and stop buttons and panel toggles in `task_launcher.rs` (which never calls `named()`), Settings' gear, and several search controls use tooltips without the shared accessible naming helper. Shared header buttons use normal hover text even when disabled, unlike menu entries that supply disabled-hover explanations. Session-list painting needs the same audit as other custom rows.

**Plan:** Name remaining icon controls, expose selected/expanded state for custom rows, and show visible keyboard focus. Supply disabled reasons where context makes availability unclear, including shared headers. Make row actions discoverable on keyboard focus as well as hover. Audit Tab/Shift+Tab order, menu dismissal, Escape and focus return across palettes, dialogs, composer controls, task forms and inline inputs.

**Acceptance:** A keyboard-only pass can reach and identify the controls for each existing workflow. Focus never becomes visually indistinguishable from ordinary selection. Screen-reader inspection exposes meaningful names instead of private-use glyphs. Do not claim that naming alone establishes full assistive-technology support.

### 10. Simplify chrome and protect status information under pressure

**P2 · M · Source + Validate**

The macOS title strip reserves up to 350 px for task/panel controls and omits the centered title when it collides. Task names are shortened by character count. The status bar already reserves facts before the transient message, but can still accumulate long language/server/position/selection text. Activity icons and title-strip toggles offer multiple entry points to the same panels.

**Plan:** Fit the document title into the space between real native controls and right-edge commands, truncating before hiding it. Measure task labels in rendered width. Give toggles consistent active/hover/focus states and explanatory names; preserve both entry points unless usability testing supports removing one. Define which status metadata shortens first under pressure, keeping position, problem access and meaningful operation feedback accessible with full text on hover. Present actual shortcut conventions per platform, including the commit-message hint that currently says `Cmd/Ctrl+Enter` on all platforms.

**Acceptance:** Long names do not erase all document context or crowd native controls. At narrow width, status facts stay inside the bar with a stable order and understandable truncation. Windows keeps native caption/menu controls; macOS keeps real traffic lights and desktop menus.

### 11. Align Git, diff and diagnostic information hierarchy

**P2 · M · Source + Validate**

Source Control already has repository accordions, semantic change markers, fixed-edge actions and operation banners. Git History already has a responsive details pane and column reduction. Avoid rebuilding those completed features. The remaining opportunity is consistency across dense review views and related errors.

**Plan:** Use the same priority order for filename/subject, secondary path/author, state/count, and actions. Keep the active repository and staged/working context visible when acting. Align change colors and non-color markers across Git diff, workspace replace preview, agent changes and language-edit review. Give conflicts/errors stronger hierarchy than routine metadata; keep full messages selectable or inspectable. Verify row actions, counts, history ref chips and long paths at minimum widths.

**Acceptance:** A user can tell which repository, file/version and change group an action affects before clicking. Color is supplemented by labels, signs or icons. Long diagnostics can be inspected without expanding every row or widening the pane. Existing commit, staging and review behavior remains unchanged.

### 12. Unify prompts, empty states and output readability

**P2 · M · Visual + Source + Validate**

Empty-state placement varies from a 28 px Explorer offset to a fixed 130 px welcome offset and 48 px chat offset. Welcome calls its untitled-buffer action “New File,” which overlaps Explorer's filesystem terminology. Dialogs use a mix of fixed/default widths; the save-conflict comparison requires at least 620 px. Output and task logs cap the configured terminal font size at 13 px.

Dialogs are built at 25 separate call sites, each with its own frame, title and button arrangement: `dialogs.rs` (7 `egui::Window`s), `source_control.rs` (7 `egui::Modal`s), `task_dialogs.rs` (5 via its local `centered()` helper), `workspace_search.rs` (2), `language_edit.rs` (2), `settings.rs` (1) and `explorer_edit.rs` (1). Windows have egui's title bar; Modals have none, so similar confirmations look different. Quick inputs and pickers (`palette.rs`, `navigation.rs`, `language_navigation.rs`) already share `floating_frame()` and are out of scope here.

**Plan:** Use a compact empty-state pattern with a clear title, one explanatory line and relevant existing actions. Name the welcome action “New Text File” to match its actual command. Make welcome/recent content fit short split groups through bounded spacing and scrolling.

For dialogs, add one shared `dialog()` primitive in `widgets.rs`: a 30 px title strip with the title and a close button, a padded body, and a right-aligned footer with Cancel and one emphasized action (accent for safe actions, a tested danger pair for destructive ones). Use it as a modal with the existing scrim for blocking confirmations, and non-modal for reviews that can stay open. Migrate the 25 call sites in reviewable groups by file, preserving each dialog's widget IDs, Escape handling (`escape_pressed`), safe default focus (`keep_focus`) and command semantics. Bound dialogs to the available viewport and stack comparison panes when needed. Remove the unexplained log font-size cap or define a coherent existing-setting mapping so increased reading size applies predictably; do not shrink text merely to fit more output. Keep output follow/scroll behavior stable and show whether content is live or completed.

**Acceptance:** Empty states remain useful in short/narrow panes. Every dialog has the same title, close, body and footer placement; destructive actions are distinguishable without color alone. Dialog actions and errors stay visible with long paths/messages and increased text scale. Enlarging reading text does not silently leave logs capped. Cancel, task input secrecy, save conflict review and error recovery preserve their current semantics.

## Delivery sequence

| Phase | Work | Exit condition |
| --- | --- | --- |
| 0. Capture tooling | Add a repeatable way to render fixtures to images (see below) | A test or command writes the baseline matrix for the current build without a person taking screenshots |
| 1. Baseline and readability | Record baselines; item 1; item 9's naming and focus work; smallest necessary shared roles from item 2 | Confirmed contrast failures fixed, icon-only controls named, baselines recorded |
| 2. Shared geometry and access | Items 3–5; remaining item 9 keyboard audit; shared state/spacing work from item 2 | Panels, headers, rows and forms remain usable at minimum widths with visible focus |
| 3. Main content hierarchy | Items 6–8 | Settings, chat, editor and search follow the same workbench conventions |
| 4. Remaining polish | Items 10–12; floating-surface/font/icon review | Chrome, review views, prompts, empty states and logs pass the visual matrix |

Item 9's accessible naming moves into Phase 1 because it is small, low risk, and touches the same controls as item 1. Within item 1, fix the Codex/Claude Send buttons first: they fail in the default Dark theme, so they are the most visible failure.

Keep changes reviewable by surface or shared primitive. Update all consumers of a changed primitive together, but avoid a broad rewrite of unrelated rendering code. Treat items marked Validate as experiments: reproduce the problem before implementing a remedy, and retain the current design if it already meets the acceptance condition.

## Visual and UX verification plan

### Representative layouts

| Dimension | Cases |
| --- | --- |
| Window | 900×560 minimum, 1280×760 default, 1920×1080 wide; dimensions in logical UI units |
| Sidebar | 180, 240 and 600 px; long filenames, deep nesting, badges and inline errors |
| Agents | Native session list at 240/300/600 px; legacy panel at 280/380/760 px; chats in narrow editor groups |
| Bottom panel | Minimum 120 px, default about 220 px, expanded; every bottom-tab mode and many task tabs |
| Editor groups | Single, side-by-side, stacked; 220–300 px narrow groups; active group switched both ways |
| Theme | Dark, Light, High Contrast; system following; editor/terminal overrides independent of workbench |
| Scaling | 100%, 150%, 200% effective UI scale where available; minimum/default/large configured reading fonts |
| Content | Long paths/titles, duplicate basenames, Unicode, many tabs, large counts, multiline errors, long commands and code lines |
| State | Idle, hover, focus, active/inactive selection, disabled, loading, empty, no matches, error, dirty, pending, approval and queued |

Capture before/after screenshots from deterministic fixtures with identical content, pane sizes, theme and font settings. Use a disposable workspace/profile for runtime checks so the user's documents, Settings drafts and session history are preserved. Screenshots should show the full workbench and relevant close-ups; keep a note of logical size and display scale.

The full table above multiplies to far more combinations than anyone will check by hand, so treat it as the menu, not the requirement. For each item, record the minimum matrix that exercises its change: for example, item 4 needs sidebar widths × Dark/Light/High Contrast with one long-name fixture, not every window size and scale.

#### Capture mechanism (Phase 0)

The repository has no image capture today, and OS-level screen capture is not reliably available to automated sessions. Choose one before Phase 1:

- **Snapshot tests with `egui_kittest`** (version matched to `egui` 0.33): render `LumenApp` fixtures headlessly at fixed sizes and themes and compare against stored images. Best for regression coverage in CI; requires a wgpu-capable runner or a software renderer, and stored images per platform.
- **A debug-only in-app capture**: behind an environment variable, load a fixture, request `egui::ViewportCommand::Screenshot` and write the returned image to disk. Exercises the real native window and fonts, so it suits macOS/Windows visual checks, but it needs a display and is not a CI gate.

Either way, keep fixture content in the repository (a small workspace with long names, Git changes, a Settings draft and a chat transcript) and never point capture at a user's real workspace or profile.

### Required acceptance checks when implementing

- Compare header action rectangles, title clipping, row badges, footer visibility and pane boundaries across representative widths. Extend existing render-level tests instead of recreating coverage that is already present.
- Add regression cases for actual changed behavior: very long Explorer names, disclosure actions, chat content narrower than 240 px, narrow task forms, crowded bottom headers, and primary-button color pairs.
- Exercise success, errors and cancellation for affected inline inputs, Settings drafts, task prompts, save/close dialogs and agent approvals. Verify that new layouts preserve stable widget IDs, keyboard focus and scroll positions.
- Run `cargo fmt --all --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- For platform-sensitive implementation changes, run `cargo check --target x86_64-pc-windows-gnu` when installed and `cargo xtask bundle` on macOS. Verify native runtime separately; a compilation or bundle result does not prove visual behavior.
- Include an enabled language-server build/fixture when changing conditional completion, diagnostic or language-edit presentation.

### Checks performed for this planning change

- `cargo fmt --all --check`: passed.
- `cargo test --workspace`: passed; existing ignored tests remained ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Repository screenshots inspected; theme/button contrast pairs calculated from source colors.
- Review pass on the same baseline: re-checked the cited constants and call sites against source (Find `width - 340`, task form 130 px labels, chat 240 px floor, 350 px launcher reserve, 13 px log cap, 620 px save-conflict width, dialog call-site counts); added the title-strip contrast pair, run/stop naming, compiled-out settings, dialog inventory and capture tooling; rebuilt the concept mockup.
- No application source, assets, settings or native behavior changed. No Windows runtime, fresh live screenshots, packaging or feature-enabled language-server validation is claimed.

## Implementation status

Branches are stacked: `f-git-ui-redesign` (#29) ← `f-ui-ux-visual-improvements` (#31) ← `f-ui-dialogs`.

| Item | Status |
| --- | --- |
| 1 Filled-control contrast | Done: `on_accent`, `title_muted`, `danger`/`on_danger` roles with a contrast test in all themes |
| 3 Bottom-pane boundaries | Done for Output, task output, Problems (Terminal already matched) |
| 4 Explorer badges | Done: badge column, ellipsized names, spoken status |
| 7 Chat | Partly: Send/Queue wording, accent fill, 3 px radii. Header budgeting and approval card pending |
| 9 Naming and focus | Partly: task launcher, Settings gear, disabled header tooltips. Keyboard-order audit pending |
| 10/12 small fixes | Done: platform commit hint, "New Text File", log font cap removed |
| 12 Shared dialogs | In progress: `ui/dialog.rs` (title strip, close, footer, scrim, danger button). Migrated: document, quit, task, conflict, language-server error, safety, Explorer delete, unapplied settings, workspace replace confirmation, Git abort and revert. Pending: Git branch/clone/tags/stash/history forms, rename and language-edit review, workspace replace preview (resizable review), empty-state pattern |
| 2, 5, 6, 8, 11 and Phase 0 | Not started |

## Scope boundaries

This plan does not add new editor engines, Git operations, debug support, language servers, agent integrations, search algorithms or filesystem features. It does not change permission policy, command semantics, data persistence or document mutation rules. Preserve the app/model/service boundaries and typed commands while changing presentation. Any newly discovered functional defect belongs in a separate issue rather than being silently included in visual cleanup.
