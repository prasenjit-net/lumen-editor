# Settings feature implementation plan

Prepared: 2026-10-12. Status: initial implementation available; broader catalog and
native acceptance remain in progress.
Applies to the Rust/egui Lumen editor and repository `AGENTS.md`.

## Implemented first batch

- Separate Editor, Appearance, Workbench and Explorer section schemas under
  `crates/settings-core/src/`. Defaults, labels, controls, search aliases, validation
  and cross-field rules are schema-owned; UI and TOML parsing reuse the catalog.
- User/project TOML paths and project priority; outside-project documents use
  user typography. No ancestor discovery or automatic config creation.
- Searchable Settings workbench view, category navigation (resizable on wide
  layouts), narrow category chooser, User/Project scopes, origins, persisted
  modified and pending filters, Apply/Cancel and override/default actions.
- Six working settings: platform/bundled Hack font family, editor font size,
  line height, UI scale, sidebar visibility and dotfile visibility. Unsupported
  features are not represented by placeholder toggles.
- Gear, native menus, palette and Cmd/Ctrl+, entry points. Save applies the
  selected draft while Settings is active; closing the view retains drafts.
  Quitting requires an explicit decision about unapplied drafts.
- Comment-preserving sparse writes, same-directory atomic replacement, absent-file
  no-clobber creation, conflict checks, validation and symlink refusal. These
  checks are not a transactional lock against hostile concurrent filesystem changes.
- Config reload on startup, project changes, saved config documents and explicit
  Reload. Invalid reloads keep the last valid state. Drafts are retained separately
  by scope/project; Reload refuses to discard them.

Remaining: background config watching; installed-font discovery/custom fallback
lists and previews; configurable tabs/indentation; runtime semantic themes and
additional existing preferences; keybindings, auto-save and later capabilities;
ranked search/highlighting and direct setting navigation. Form Apply deliberately
refuses while its config file is open in an editor tab, preserving that buffer's
text/history; edit/save it there or close it before using the form. Native
IME/VoiceOver and Windows runtime acceptance remain pending. This is not completion
of every catalog entry or of the entire plan.

## Objective

Make useful appearance, editor, file and workbench behavior configurable without
changing today's defaults or compromising document safety. Provide both a compact
settings UI and human-editable configuration files. Support user settings, an
opened directory's project settings. Show where an
effective value came from, rather than making inheritance invisible.

This is an incremental feature plan, not a promise that settings unlock features
the editor does not yet implement. Unsupported options must not appear functional.
Configuration must never create a second mutation/history owner for documents.

## Product decisions and scope

### Configuration files

Use TOML as the initial proposed format, with a schema version and typed fields.
It supports comments and readable nested sections without inventing a custom
parser. Evaluate `serde` plus a comment-preserving TOML editor during implementation;
do not add dependencies until the parsing/persistence phase requires them.

User locations (requested product convention, resolved by a platform home-path service):

| Platform | User settings |
| --- | --- |
| macOS | `~/.lumen/settings.toml` |
| Windows | `%USERPROFILE%\.lumen\settings.toml` (resolved user home) |
| Linux | `~/.lumen/settings.toml` |

Project settings live at `<opened-directory>/.lumen/settings.toml`. The opened
directory is the project root, whether or not it contains a Git repository.
There are two writable scopes: User and Project. Do not discover nested directory
or ancestor configs as additional override layers. Merely opening a folder must not create
a config file or modify the project. Create files only when the user saves a
setting to that scope. Do not read or write VS Code's `.vscode/settings.json`.

Keep settings separate from transient/personal session state: window position,
current tab, scroll offsets, selected files and recent history must not become
tracked project preferences. Store session state separately if that feature is
implemented; do not silently bundle it into settings persistence.

### Resolution and precedence

Resolve settings from lowest to highest priority:

1. Built-in defaults matching current behavior.
2. User settings.
3. Project settings in the opened directory's `.lumen/settings.toml`.

Project values override user values for project-allowed keys. Missing project
keys inherit user values, then built-in defaults. User-only keys retain their
scope restrictions and cannot be overridden by a project file.

Within each layer, future language-specific overrides apply after that layer's
base values; a higher-priority layer still wins. Arrays replace lower-priority
arrays. Typed sections merge by known leaf keys; do not concatenate lists or
deep-merge arbitrary unknown objects. Removing a key restores inheritance.
Reset-to-default is a separate explicit action that writes the default value.

Window/workbench settings also resolve defaults → user → project. They must
not change when switching into a child directory or language. Some settings are
user-only, according to the schema's allowed scopes.

- For a saved file inside the opened root, use that project's settings only;
  do not load child-directory configs.
- For a file outside the opened root, do not apply that project's document settings.
- With no folder open, use user settings and built-in defaults, even for a saved
  file. Do not infer a project from its parent directory or crawl ancestors.
- Untitled buffers use user settings and the current workspace-root defaults, not
  an arbitrary selected explorer child. Re-resolve when a path is assigned.
- Project scope requires an opened folder; otherwise disable that scope in the UI.
- Resolve only the selected project config path and prevent symlink escape outside
  the opened root when saving it.

Every effective value carries its origin: default/user/project/language
and config path. Project switching, Save As and rename/move update applicable
settings without clearing text, dirty state, selection, history or document IDs.

## Settings catalog

Use one typed catalog for defaults, parsing, validation, labels/help, allowed
scopes, UI controls, reset behavior and runtime application. Start with implemented
capabilities; add new useful settings through the same catalog later.

| Group | Configurable aspects | Notes |
| --- | --- | --- |
| Editor typography | Font-family/fallback list, size, line height, supported weight/style, tab display width | First implementation priority; renderer and geometry must agree |
| UI typography | System/UI font family, base font size, UI scale | User/workspace scope; preserve Codicons and embedded fallbacks |
| Appearance | Dark/light/system theme, semantic color-token overrides, selection/caret/match/gutter colors, subtle corner radii and density | Centralize in `ui/theme.rs`; retain current defaults |
| Editor display | Line-number mode/visibility, gutter width/padding, current-line highlight, caret width/blink, whitespace/control-character visibility, scrolling sensitivity and scrollbar visibility | Add rendering capabilities before exposing their settings |
| Editor navigation | Scroll margin, wheel behavior, cursor-add modifier when supported, word navigation policy | Preserve native Cmd/Ctrl conventions and focus rules |
| Editing | Tab size, spaces/tabs insertion, indentation width, new-document EOL, typing undo pause | EOL choices do not rewrite existing buffers implicitly |
| History/resources | Undo group limit, optional history byte budget, large-file thresholds | User-only limits; document trimming semantics and memory consequences |
| Files/save | Auto-save mode/delay, default new-file encoding/EOL, optional trailing-whitespace/final-newline rules | New workflows require dedicated tests; transformations are explicit undoable edits |
| Explorer | Sort order, folders-first, hidden-file visibility, excludes, compact-folder display, auto-reveal active file | Excludes hide entries; they do not delete files or bypass filesystem rules |
| Workbench | Initial sidebar/status/tab visibility, sidebar side/default width, tab sizing/labels, breadcrumbs when implemented | Layout preferences, not live tab/scroll session state |
| Search | Default case/whole-word options, search exclusions, match/resource limits | Do not imply regex or workspace search exists before implemented |
| Accessibility | High-contrast token set, reduced motion/blink and readable UI scaling | Accessibility/focus safety must not be disabled by project config |
| Keybindings | User-editable mappings to existing typed commands, conflict diagnostics, native shortcut defaults | No arbitrary executable commands; separate bindings file if warranted |
| Language/file overrides | Indentation, font/display and save policies per known language/file type | Later phase; bounded discovery and visible origin |

Also plan editor font ligature controls, letter spacing, word wrap, rulers and
minimap options as follow-up capabilities. Current long lines intentionally scroll
horizontally. Do not expose a wrap/minimap toggle until text, gutter, highlights,
pointer mapping and accessibility support that behavior consistently.

Not configurable away: Trash-only deletion, overwrite/self-recursion guards,
explicit destructive confirmation, dirty-close safety, stale-target validation,
native menu placement and single document history ownership. Project files cannot
inject commands, alter environment variables, read arbitrary font paths, access
the network or silently weaken confirmation/security rules.

## Example configuration

Illustrative schema proposal, not an already supported file:

```toml
schema_version = 1

[editor]
font_family = ["Menlo", "Consolas", "Hack"]
font_size = 14.0
line_height = 18.0
tab_size = 4
insert_spaces = true
line_numbers = "on"

[workbench]
theme = "dark"
sidebar_visible = true

[files]
auto_save = "off"
default_eol = "platform"
```

The existing renderer uses a 14-point font and 18-point rows; reuse those defaults.
Platform font order should match current native defaults rather than imposing the
example's cross-platform list everywhere. Existing UI typography and 5px panes,
2px gutters and 3px tab/input/button rounding remain the initial defaults.

## Architecture

- **Pure settings core:** typed settings, optional per-layer overrides, metadata,
  validation, resolution, search and TOML parsing/serialization live in the
  `lumen-settings-core` crate under `crates/settings-core/`. No egui,
  OS paths, filesystem access or global singleton.
- **Services:** injected config-path and config-store interfaces, font discovery,
  platform/local adapters, change monitoring and safe atomic persistence.
- **Application controller:** `app/settings.rs` loads layers, owns editable drafts,
  dispatches typed settings commands, resolves active-document/workbench settings
  and schedules runtime updates. I/O remains outside UI components.
- **UI:** focused `ui/settings/` components borrow through `Workbench`, show
  metadata-driven controls and issue commands. Reuse `panel_header()`, theme
  tokens, Codicons, compact layout and stable IDs.
- **Platform:** `platform/fonts.rs` and `platform/editor_fonts.rs` resolve supplied
  font names and fallbacks. Font scanning/loading does not happen during painting.
- **Editor crates:** receive explicit UI-independent typography/editing/history
  options. Do not import app settings files or filesystem concerns into the core.

Introduce runtime semantic theme tokens in the existing theme module, replacing
direct component reads of fixed color constants incrementally. Do not create a
second unrelated palette. Resolve UI and document typography separately and make
the selected font data available consistently to egui and cosmic-text.

## Runtime application and safety

Classify each key as live, deferred until an interaction finishes, new-document
only, or restart-required. Show that classification in the UI.

- Font size/family, line height and tab display changes invalidate all relevant
  shaped/galley/character/visual-stop caches through an explicit settings/font
  generation, not a coincidental text-color change.
- Remove hardcoded font/row dimensions across geometry, shaping, gutter, input,
  page navigation, IME candidate rectangles and accessibility together. One
  authoritative layout determines positions; no independently sized gutter rows.
- Preserve selection/affinity, focus, document identity, dirty flags, history and
  scroll anchors when relayout occurs. Defer font/geometry swaps during active
  preedit or pointer/gutter drag, then apply once the interaction ends.
- Keep line height finite and large enough for the selected font metrics. Reject
  impossible combinations instead of silently clipping text/candidates.
- Missing fonts use visible, documented fallbacks; do not silently accept a chosen
  family while rendering with an unrelated font. Font changes may rebuild font
  resources, but never recreate document buffers/history.
- Theme application changes all semantic consumers, including panes, buttons,
  inputs, tabs, status, highlights and supported native chrome. Platform-native
  menu/caption constraints remain explicit; no simulated native controls.
- Changing indentation/EOL defaults affects future edits/new documents. Converting
  an existing buffer requires an explicit command and an undoable transaction.
- Optional save-time transformations run through the core before I/O; saved
  checkpoints advance only after a successful write. Disabled defaults preserve
  current save behavior. Auto-save never chooses a path or dismisses dirty prompts
  for an untitled buffer, and pauses during composition or blocking dialogs.
- Retention settings disclose history trimming; do not silently clear history
  while applying unrelated appearance changes.

## Persistence, validation and reload

- Validate types, enum values, bounded finite numbers, colors and scope legality
  before applying. Establish explicit ranges for font sizes, line height, scale,
  tab sizes, resource budgets and timing values; reject NaN/infinite values.
- Missing files mean an empty override layer, not an error. An invalid layer at
  startup is ignored with diagnostics; a failed reload retains its last valid
  snapshot. Do not partially apply an invalid known-settings layer.
- Unknown keys produce warnings and are preserved during edits. Unsupported schema
  versions are not automatically rewritten or downgraded.
- Persist only overrides for the selected scope, not a dump of merged values.
  Preserve comments, unknown keys and unrelated sections where feasible.
- Apply a UI draft only after validation and a successful save; on failure retain
  the draft/error and previous effective settings. Optional temporary preview is
  separate, explicit and reversible by Cancel.
- Use same-directory staging and platform-appropriate atomic replacement. Detect
  external changes before overwriting; offer reload/reconcile rather than losing
  another editor's edits. Test permission, rename/replace and disk-write failures.
- Monitor only known config paths with debounced reload and self-write detection;
  keep watching off the rendering/input thread. Do not overwrite an unsaved config
  document tab when an external file changes.
- User settings must never be written by project actions. Project saves
  create `.lumen/` only after explicit selection/apply. File symlinks must not cause
  project-scope writes outside the permitted root; report conflicts safely.
- Do not change backend selection at runtime for an already core-owned document.
  The legacy build remains a build-time fallback, not a settings toggle.

## Settings UI and commands

### Visual and interaction reference

Use the current [VS Code Settings editor documentation](https://code.visualstudio.com/docs/configure/settings#settings-editor)
as the reference for discoverability: search, grouped navigation, scope tabs,
modified indicators and per-setting actions. Lumen keeps TOML and its requested
User/Project paths; it does not adopt VS Code's JSON files or extra scopes.
Lumen also retains explicit Apply/Cancel for drafts, rather than copying VS Code's
immediate-persistence behavior. Do not claim an exact replica of every interaction.

Make Settings a dedicated, reusable workbench tab/view, not a native-dialog form
or a collection of floating cards. Keep the compact VS Code-inspired palette,
subtle surfaces, restrained spacing and normal workbench typography. Settings is
not a text document: do not allocate document history or mark an editor buffer
dirty when a settings draft changes.

### Layout

- Header: left-aligned Settings title and right-aligned actions for Open Config
  File and other commands, using `panel_header()` and Codicons/tooltips.
- Search: a prominent full-width search field near the top, a compact clear
  action and filter menu. Keep search visible while scrolling results.
- Scope: User and Project tabs immediately below search; show the project name
  and resolved file path. Disable Project with an explanation when no directory
  is open. Switching scope preserves the query and reconciles any unsaved draft
  explicitly; never saves/discards a draft silently.
- Navigation: a compact left category tree with Commonly Used, Editor,
  Appearance, Files, Explorer, Workbench, Search, Accessibility and Keybindings.
  Category result counts reflect the current query/filter. Only implemented
  settings appear as editable options.
- Results: one scrollable column with category headings and compact setting rows,
  not a dense grid of unlabeled controls. Each row shows its readable name, stable
  setting ID, concise help, appropriate control, effective value/source and scope
  override/modified state. Descriptions may wrap; toolbar/header titles do not.
- Footer: Apply/Cancel and a pending-change count; errors stay beside affected
  settings, with an optional summary that can focus the first error.
- Narrow widths: collapse category navigation into a chooser/drawer, retain search
  and scopes, and stack controls below labels. Do not overlap fixed-edge actions
  or require horizontal scrolling to use settings. At wider widths, keep the
  navigation resizable with sane bounds rather than wasting space.

### Search and filters

Search must feel immediate and useful, without requiring users to know setting IDs.
Index the typed catalog's names, IDs, descriptions, categories and aliases such as
`text size` → editor font size. Use deterministic local, case-insensitive matching;
rank exact ID/name matches above prefixes and other token matches. Multiple plain
tokens narrow results together. Do not use a network or AI search service.

Initial filters:

- `@modified`: explicitly configured overrides in the selected persisted scope,
  including values explicitly equal to defaults. Keep this distinct from unsaved
  draft changes and inherited user values in Project scope.
- `@id:editor.font_size`: exact setting-ID lookup.
- Category chips/filter actions: accessible alternatives to typing query syntax.
- A separate Pending Changes view/filter for edited, unapplied draft values.

Provide a result count, highlighted matching text where practical and an empty
state with Clear Filters. Filtering combines with the selected category rather
than hiding which category is active. Debounce/index only if measurements justify
it; do not rerun expensive filesystem/font discovery for every keystroke.
Language filters can be added when language overrides actually exist.

### Controls, actions and keyboard use

Use checkboxes for booleans, dropdowns for enums, validated number inputs for
sizes/timing, font family selectors with preview, and bounded list editors for
fallbacks/excludes. A theme preview must use the same semantic tokens as the real
workbench. Preview changes do not imply successful persistence.

Each setting has a compact gear/context action menu: Remove Override (inherit),
Set Built-in Default explicitly, Copy Setting ID and Show in Config File. Show
the difference between removal and an explicit default. User-only settings in
Project scope show why editing there is unavailable.

Opening Settings focuses search. Tab/Shift+Tab traverse controls; Escape first
closes popups/choosers and never silently discards a dirty draft. Search results
must not steal focus while typing. Return visits to the Settings view restore
query, category, scope and scroll position. Add a typed focus-setting command for
opening Settings directly at a stable ID. Shortcuts must not intercept typing,
clipboard, IME or ordinary navigation in settings controls.

Acceptance includes searching `font`, `text size`, a full ID, `@modified`, combined
category/query filters and a no-results query; switching User/Project with pending
changes; reset/inheritance differences; keyboard-only use; screen-reader labels;
and narrow/default/wide screenshots. Project values must still take priority over
user values regardless of which scope is currently being edited.

Provide Settings from the existing native menu, command palette and a compact
settings action. Use Cmd+, on macOS and Ctrl+, on Windows if no existing conflict.
All entry points reuse the same typed command. Do not add an in-window macOS menu.

The page provides:

- Search/filter by name, description and modified values, with compact categories.
- User / Project scope selection; Project explains why it is unavailable when no
  directory is open.
- Effective value and origin, plus a separate editable override for the chosen scope.
- Validated controls, font chooser/preview and live/deferred/restart indicators.
- Apply, Cancel, Remove Override, Reset to Built-in Default and Open Config File.
- Inline parse/scope/write/conflict diagnostics; no silent save failure.
- Stable focus/tab order and responsive narrow/default/wide layout. Text fields
  own typing/clipboard input; editor/explorer shortcuts must not intercept them.

Opening the config file uses ordinary document commands, duplicate-tab handling
and dirty-close behavior. Saving that document triggers validated reload; typing
an invalid intermediate TOML buffer does not apply it before save.

## Implementation order and checkpoints

1. **Schema and resolution:** inventory existing constants/preferences; establish
   exact defaults, scope rules, immutable effective snapshots and origin tracking.
   Add table-driven validation/precedence tests before wiring UI or filesystem.
2. **Storage and scope lifecycle:** injected platform paths/config store; TOML
   parsing, comment-preserving writes, conflicts and safe reload. Wire startup,
   folder switching and document path changes without changing defaults.
3. **Typography end to end:** editor font family/size/line height and tab metrics;
   shared/legacy adapters, font fallbacks, cache generations, IME deferral and
   scroll preservation. Verify rendering and native candidate placement first.
4. **Settings page:** scope-aware controls, diagnostics, Apply/Cancel/reset, config
   file commands, native menu/palette/shortcut entry points and origin display.
5. **Theme and existing behavior:** runtime semantic tokens, UI font/scale,
   existing explorer/workbench/editing preferences and safe history/timing knobs.
6. **New capabilities:** implement auto-save/save policies, language overrides,
   keybinding editing, additional display options and wrap/ligature/ruler/minimap
   capabilities in separate tested increments. Expose only completed options.
7. **Acceptance/documentation:** validate persisted user/project layers,
   native font/IME/accessibility behavior and rollback; document schema, paths,
   inheritance and limitations. Commit/push only when requested.

Each checkpoint must leave the application usable with no configuration files and
the current visual/editor defaults intact. The first implementation batch should
cover phases 1–2; do not jump directly to a cosmetic settings window with no model.

## Verification and completion criteria

- Pure tests: missing/invalid/unknown settings, numeric/color validation, scope
  restrictions, precedence, removal vs explicit reset, language-layer ordering,
  outside-root files, untitled buffers, opened-root containment and origin reporting.
- Service tests: absent/unwritable directories, creation cancellation, preserved
  comments/unknown keys, atomic replacement, write failures, conflicts, symlinks,
  malformed reload and duplicate/self-generated notifications.
- Controller tests: startup, project switches, Save As/rename, settings drafts,
  config editor tabs, open/save/cancel/error paths and per-document resolution.
- Editor tests: multiple font sizes/line heights, missing/fallback fonts, CRLF,
  long lines, bidi affinity, selection rectangles, gutter/scrollbars, pointer
  mapping, page navigation, IME and accessibility; no text/history mutations from
  display-only changes. Test both default shared and legacy builds.
- UI/native checks: narrow/default/wide settings page; keyboard focus, commands,
  previews, consistent theme consumers, native menus, scaling and candidate windows.
- Required commands: formatting; default, legacy and all-feature workspace tests;
  strict all-target Clippy; installed Windows cross-target compilation; normal and
  legacy macOS bundles. Keep native runtime evidence separate from compilation.
- Completion requires working user/project persistence and project-priority inheritance,
  real font/size/line-height application, consistent UI, useful existing preferences,
  safe errors/reload and documentation. Follow-up capabilities remain clearly
  tracked rather than appearing as nonfunctional settings.

Related backlog: [pending activities](pending-activities.md). Native acceptance and
large-file/clipboard limitations remain independent of the settings feature.
