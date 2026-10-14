# Native editor validation

## 2026-10-13 partial native pass (default shared editor)

Environment: macOS 26.5.2 (25F84), debug `target/Lumen.app`, default shared
editor. Input method was not recorded. The automated live-app check on the
preceding build confirmed gutter folding, original source line numbers and
per-tab fold persistence. A false "Composition cancelled" status on an ordinary
tab switch was reproduced, fixed with a regression test, and bundled again.
Computer Use timed out in the native file chooser on the first rebuilt app, but
the separate rebuilt experimental bundle was then inspected successfully. In
that bundle, switching between two clean untitled tabs did not show the false
notice. A disposable buffer passed typed-text undo, Cmd+D creating two
selections, two-location replacement and undo, plus external Unicode paste
(`漢字 👩‍💻`) into both selections and undo. The Source Control panel and a
working-tree diff also rendered in the live app. Unit tests confirm ordinary
identity changes have no notice and an active preedit still rejects a late
commit. Do not count this as a completed full-workbench acceptance pass.

Checklist status: IME/dead keys, VoiceOver, exact native clipboard/CRLF, bidi,
the broader multi-cursor/history matrix, and file lifecycle are **not tested**
on this build. The
user could not perform the manual portion at this time. Windows live acceptance
also remains pending; Windows cross-compilation is not runtime validation.

## Current workbench acceptance (2026-10-12)

Default promotion was explicitly requested after this checklist was prepared.
Normal `cargo run` / `cargo xtask bundle` now use the shared editor. Native results
are still pending; either the normal or separate validation bundle can run these
checks. Legacy launch: `cargo run --no-default-features`.

The user can test macOS; Windows runtime remains pending. This acceptance applies
to the newly cached/shaped, affinity-aware **full workbench**, not the older
in-memory prototype. Use the separate `target/Lumen Experimental.app` bundle
(built with `cargo xtask bundle --offline --shared-editor`) or
`cargo run --features shared-editor`. Use a new untitled buffer/disposable file.
Save any clipboard content you need before testing. No input methods or
accessibility settings were changed by the agent.

Report these as pass/fail/not tested, with macOS version and input method:

1. **IME/dead keys:** start, update, commit and cancel Japanese/Chinese preedit;
   check candidate placement after scrolling/resizing. Option+E then `e` inserts
   `é` once. Repeat with multiple cursors; commit is one undo step, cancellation
   does not mutate. Switch tabs/apps during preedit and confirm no late insertion.
2. **Clipboard:** selected and empty-selection Copy/Cut/Paste; external Unicode
   payload at multiple cursors; exact CRLF round-trip in a disposable saved file.
   Immediately type after paste and undo paste/typing separately. Context-menu
   paste after changing the caret/selection must cancel safely.
3. **VoiceOver:** editable role, line reading, primary selection, selection changes
   and focus return. Check after CRLF, during preedit and on mixed-direction text.
   No duplicate tree rows or focus trap; accessible selection collapses secondary
   cursors without editing the buffer.
4. **Bidi/geometry:** paste `abc אבג xyz` and `abc العربية xyz` on separate lines.
   Left/Right should move visually, without skipping inside combining characters
   or emoji. Shift-arrows, click placement, multi-cursor Up/Down and undo must keep
   the correct boundary side. Select `c א` in the Hebrew line: separate visible
   highlights should not fill the intervening unselected Hebrew characters.
   Repeat at narrow/wide widths and after horizontal scrolling.
5. **Workbench/history:** Cmd+D, Shift+Cmd+L, Option+Cmd+Up/Down, Tab/Shift+Tab,
   Escape, native/menu/popup undo. Consecutive typing groups; save creates an undo
   boundary. Tab return restores scroll/selections. Right-click preserves selection.
   Text/blank editor space uses an I-beam; gutter/scrollbar do not.
6. **File lifecycle:** open duplicate file, save/save-as, cancel save-as, dirty
   close/quit cancellation and a failing save to a disposable read-only target.
   Unsaved content and history must survive failures/cancellation.

See [the promotion gates](editor-promotion-checklist.md) before enabling the new
backend by default. Screenshots/tree tests are not evidence that these native
checks passed. Earlier prototype results below remain historical only.

Updated 2026-10-11. The user reported **all five manual macOS checks passed** for
the preceding default (egui-galley) prototype. Input-method/OS-version details
were not supplied. This is user-reported validation, not an automated runtime
result, and does not automatically validate the new cosmic shaped renderer.
Windows runtime remains pending by user direction. Headless events, source probes and screenshots are not native
IME, clipboard or VoiceOver validation. Automation can inspect the isolated app,
but clipboard delivery still times out (-10005).

## Safe fixture

For the newly integrated shaped workbench, use a disposable file or new untitled
buffer, not an existing unsaved document:

```sh
cargo run --features shared-editor
```

On macOS, a separate app bundle preserves the default app:

```sh
cargo xtask bundle --offline --shared-editor
open "target/Lumen Experimental.app"
```

Repeat the checklist below for this new renderer/controller combination before
default promotion. The experimental workbench supports real saves: write only
your disposable test buffer. The shaping fixture remains memory-only:

```sh
cargo run -p lumen-editor-surface --example spike --features shaped -- 900
```

From the project root, run:

```sh
cargo run -p lumen-editor-surface --example spike -- 900
```

Do **not** add `--ime-preview`: that injects synthetic composition/focus. This
example edits memory only, cannot save files, and is not the production editor.
Close and relaunch to reset it. Test copying changes your clipboard; preserve any
clipboard content you need first. Use an already enabled IME, or enable one
yourself if desired; no system settings were changed by the agent.

## macOS checklist

1. **IME composition/undo.** The fixture starts with `first` and `second`
   selected. Focus the editor without clicking away from those selections (Tab
   to the editor if necessary). With a Japanese/Chinese input method, start a
   composition. Only the primary preview should appear inline, push ` = 1;`
   right, and leave revision/Modified unchanged. The candidate window should
   follow the primary caret. Commit once: both names change once, revision rises
   once; Cmd+Z restores both names and both selections; Shift+Cmd+Z restores the
   commit. If clicking is needed to focus, add the other cursor with Option-click
   and test insertion instead of replacement.
2. **Cancellation/geometry.** Start preedit, then Escape, click after the preview,
   or switch to another app. Uncommitted text must disappear without a text undo
   entry or delayed insertion on return. Repeat after horizontal/vertical scroll
   and at narrow/wide widths. Following text, gutter and candidate positioning
   should stay aligned. Also test the macOS Option+E dead-key sequence then `e`:
   it should insert `é` once, not its intermediate accent separately.
3. **Clipboard/ordering.** Reset, focus the editor, Cmd+C the two selected names,
   Cmd+X, then Cmd+V: names should return to their respective locations. Undo
   paste and cut separately. Paste `漢字 👩‍💻` copied from another app into two
   cursors: the entire external payload should be inserted at each cursor;
   type immediately afterward and undo. Confirm no lost/repeated input. The
   example still uses eframe's clipboard transport (which normalizes CRLF):
   this test does **not** approve exact native CRLF transport. The live editor's
   injected clipboard service requires a separate disposable-buffer round trip.
4. **Accessibility.** If you use VoiceOver, enable it yourself and focus the
   editor. Check its editable-text role, line reading, primary selected text and
   caret navigation. An accessible selection action should collapse secondary
   cursors without editing text. Check rows after CRLF, then during preedit.
   Preedit-interior positions must not become stored source selections. Record
   any missing role, incorrect speech, unsupported action or focus trap. AccessKit
   tree tests alone do not prove these behaviors.
5. **Commands.** Cmd+A, paste `name name\nname`, then click inside the first
   `name`. Cmd+D seeds it; repeat to add the next occurrence. Shift+Cmd+L selects
   all. Type once, then undo once to restore all selections. Option+Cmd+Down/Up
   adds a cursor on the adjacent line (short lines clamp but retain the desired
   horizontal position). Tab/Shift+Tab indent/outdent each touched line once.
   Escape removes only secondary cursors. Also verify empty-selection logical-line
   Copy/Cut and line paste at EOF/CRLF; they now share the original live rules.

Report each item as **pass / fail / not tested**, with macOS version, input method,
width and reproduction steps. A screenshot is helpful for candidate placement.
Successful native checks will not by themselves approve migration: the final
shaping renderer/accessibility adapter and regression matrix still need completion.

## Migration order after the backend gates pass

1. Supply a shaping-capable geometry/rendering adapter and platform font coverage;
   test mixed-direction affinity, ligatures, emoji and selection rectangles.
2. Move document text, saved checkpoint, revision, selections and history under
   the shared core in one change. No simultaneous TextEdit history for a document.
3. Route typing, native/popup commands, clipboard, Find/Replace, IME and accessible
   edits through that owner; preserve typed targets and sticky paste cancellation.
4. Keep stable document/scroll identities and file workflows (save failure,
   rename/delete, duplicate tabs and dirty close/quit). Re-run all existing tests.
5. Verify narrow/default/wide workbench layout, native focus/menu behavior,
   Windows compilation and macOS packaging. Record platform runtime gaps honestly.
