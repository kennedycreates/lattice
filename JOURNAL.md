# Lattice — Journal

Chronological development journal for the active project state.

## 2026-05-16 — Configurable shortcuts and custom context actions

Added config-driven action customization for keyboard shortcuts, right-click menu order, and safe custom command actions.

Changed:
- Replaced the theme-only config parser in [`src/config.rs`](src/config.rs) with a small parser for `theme`, `[shortcuts]`, `[context_menu]`, and `[[custom_actions]]`.
- Added first-run example config content with commented examples for `Open in GIMP` and `Compress Here`.
- Threaded [`AppConfig`](src/config.rs) into [`src/ui/main_window.rs`](src/ui/main_window.rs) so shortcut dispatch and context menus use the loaded config.
- Made built-in shortcuts configurable by action ID, including disabling a shortcut with an empty string.
- Added `custom.<id>` shortcut dispatch and custom context menu entries.
- Added configurable right-click menu order for normal file, folder, and current-folder/background menus.
- Added safe custom action launching through `gio::SubprocessLauncher` with argv-array execution, not shell interpolation.
- Added `{paths}`, `{path}`, and `{cwd}` placeholder expansion, with `{paths}` passed as separate process arguments.
- Updated [`README.md`](README.md) and [`docs/roadmap.md`](docs/roadmap.md) with the new config feature and examples.
- Added tests for config parsing, default shortcut dispatch, shortcut overrides, and custom shortcut dispatch.

Why:
- Users need daily-driver hooks such as `Open in GIMP` and `Compress Here` without Lattice hardcoding every external workflow.
- Passing selected file paths as argv entries avoids shell quoting bugs and command-injection risks from interpolating paths into a shell string.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test config::tests` — succeeded.
- `cargo test ui::main_window::tests::window_shortcuts_dispatch_standard_commands` — succeeded.
- `cargo test ui::main_window::tests::configured_shortcuts_override_builtin_and_dispatch_custom_actions` — succeeded.
- Initial combined `cargo test config::tests ui::main_window::tests::window_shortcuts_dispatch_standard_commands ui::main_window::tests::configured_shortcuts_override_builtin_and_dispatch_custom_actions` — skipped/invalid because Cargo accepts only one positional test filter.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` with the configurable action model, built-in shortcut IDs, and commented examples equivalent to the generated config template.

Known gaps:
- Live desktop validation is still needed for the actual right-click menu feel and launching configured external apps.
- The generated config file could not be observed in this headless run because GTK failed before activation completed; the template is implemented in code and will be written when `AppConfig::load()` runs in a normal session with no existing config.

## 2026-05-16 — Toolbar panel-toggle and shortcut alignment

Moved the two panel visibility controls together and filled in missing keyboard affordances for the compact toolbar.

Changed:
- Reordered [`src/ui/toolbar.rs`](src/ui/toolbar.rs) so the sidebar and preview visibility toggles sit next to each other, with split view following them.
- Added `Ctrl+B` for toggling the sidebar and `Ctrl+Shift+N` for creating a new text document in [`src/ui/main_window.rs`](src/ui/main_window.rs).
- Updated toolbar tooltips so the sidebar and new text document buttons list their shortcuts.
- Extended the focused shortcut test to cover `Ctrl+B` and `Ctrl+Shift+N`.
- Updated [`README.md`](README.md) and [`docs/theming.md`](docs/theming.md) to keep shortcut and toolbar descriptions accurate.

Why:
- The sidebar and preview controls are both panel hide/show controls and should be visually grouped.
- Icon-only controls need discoverable shortcut text in their hover tooltips, and every advertised toolbar action should dispatch through the same shortcut path where practical.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test ui::main_window::tests::window_shortcuts_dispatch_standard_commands` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` so the keyboard-support summary includes new folder, new text document, and sidebar toggling.

Known gap:
- Live desktop validation is still needed for final toolbar spacing and tooltip feel because this environment cannot open a GTK display.

## 2026-05-16 — New Text Document action

Added first-class empty text-document creation so Lattice can create quick notes without forcing a file extension.

Changed:
- Added a dedicated `New Text Document` toolbar button in [`src/ui/toolbar.rs`](src/ui/toolbar.rs) beside the other file-creation actions.
- Wired the button in [`src/ui/main_window.rs`](src/ui/main_window.rs) to a new controller flow that mirrors `New Folder` but creates an empty file instead of a directory.
- Added `New Text Document` to the current-folder right-click menu and broadened that background menu to work in both normal directory views and Downloads Triage, matching the existing writable-view rules.
- Added extensionless default name suggestion with `Untitled`, `Untitled 2`, `Untitled 3`, … fallback naming.
- After successful creation, Lattice now refreshes, reveals the new file when visible, and attempts to open it through the existing default-app launch path.
- If the file is created but the desktop cannot open an extensionless file with a default app, Lattice now shows a clear error explaining that creation succeeded but launch failed.
- Updated [`README.md`](README.md) and [`docs/roadmap.md`](docs/roadmap.md) so the shipped file-creation feature set is documented accurately.
- Added focused tests for extensionless name collision handling and writable-view availability in [`src/ui/main_window.rs`](src/ui/main_window.rs).

Why:
- Quick empty text files are a real daily-driver file-manager action, and Lattice already had the right interaction patterns for folder creation that could be reused cleanly.
- Keeping the default name extensionless preserves the lightweight “scratch document” workflow the user asked for, while still handling default-editor launch failures safely.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with one pre-existing warning: `show_confirm` in `src/ui/modal_host.rs` is currently unused.
- `cargo test ui::main_window::tests` — succeeded (`8` tests passed).
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` to include `New Text Document` in the shipped file-action set.

Known gaps:
- Live desktop validation is still needed for the final toolbar spacing and the real-world default-editor behavior of blank extensionless files, because this environment cannot open a GTK display.

## 2026-05-16 — In-window modal system (ModalHost)

Replaced all `gtk::Dialog` / `gtk::AlertDialog` popup windows with a centralized
in-window modal overlay (`ModalHost`) to eliminate the first-frame squash bug
that appeared on GTK4/Wayland.

Root cause of the bug: separate `GtkWindow` toplevels do not have a stable layout
until the window is realized and mapped, so dialog content appeared squashed for
~100 ms after opening.  Previous patches using `measure()` / `set_default_size()`
could not fix this structurally.

Architecture:
- New module: [`src/ui/modal_host.rs`](src/ui/modal_host.rs) — `ModalHost` wraps
  the entire app UI in a `GtkOverlay`.  A hidden `GtkOverlay` (`.modal-layer`)
  containing a scrim (`.modal-scrim`) and a centered panel (`.modal-panel`) is
  added as an overlay child.  When a dialog opens, the layer becomes visible.
  The content is already part of the widget tree and lays out correctly on the
  first frame.
- `MainWindow::new` now sets `modal_host.overlay` as the window child.
- `BrowserController` holds a `ModalHost` and uses it for every internal dialog.
- [`src/ui/bulk_rename.rs`](src/ui/bulk_rename.rs) refactored to accept
  `&ModalHost` instead of `&ApplicationWindow`.

Migrated dialogs (all formerly `gtk::Dialog` or `gtk::AlertDialog`):
- Single-file Rename
- Bulk Rename
- New Folder
- Pin as Project
- Add Tag
- Remove Tag
- Send to Project
- File Conflict (project copy/move)
- All error notifications (~25 sites)

CSS: Added `.modal-layer`, `.modal-scrim`, `.modal-panel`, `.modal-title`,
`.modal-content`, `.modal-actions`, `.modal-primary-button`,
`.modal-danger-button`, `.modal-secondary-button` to
[`themes/default.css`](themes/default.css).

Safety preserved:
- Dangerous actions still use `ButtonKind::Danger` (distinct crimson styling).
- Conflict dialog uses `scrim_dismisses: false` — cannot be dismissed without
  an explicit choice.
- All file operations still require user confirmation.

Docs: Added [`docs/modal_architecture.md`](docs/modal_architecture.md) with the
architecture explanation and rules for future contributors.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded (1 expected warning: `show_confirm` unused, kept for
  future use as a public API).

Known gap:
- Live desktop validation still needed because this environment is headless.

## 2026-05-16 — Toolbar focus-mode removal

Removed the extra focus-mode toolbar button after the panel toggles proved sufficient on their own.

Changed:
- Removed the focus-mode button from [`src/ui/toolbar.rs`](src/ui/toolbar.rs) so the top bar keeps only the direct sidebar and preview visibility toggles.
- Deleted the now-unused focus-mode controller state and helper methods from [`src/ui/main_window.rs`](src/ui/main_window.rs) while preserving the session-persistent sidebar and preview toggle behavior.
- Dropped the dedicated focus-mode active styling from [`themes/default.css`](themes/default.css) and [`themes/high-contrast.css`](themes/high-contrast.css).
- Updated [`README.md`](README.md) so it no longer advertises a focus-mode control that was intentionally removed.

Why:
- The explicit sidebar and preview toggles already cover the useful panel-space cases.
- Keeping a third button for “hide both” was redundant and consumed space in the exact toolbar area this pass was trying to simplify.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` to remove the focus-mode mention and keep the toolbar description accurate.

Known gap:
- Live desktop validation is still needed for final toolbar spacing feel because this environment cannot open a GTK display.

## 2026-05-16 — Toolbar declutter and panel toggles

Reduced top-toolbar crowding by converting the main browser actions to icon-only controls and adding direct panel visibility controls without hiding existing actions behind menus.

Changed:
- Rebuilt [`src/ui/toolbar.rs`](src/ui/toolbar.rs) around symbolic icon buttons/toggles for navigation, file actions, search/filter controls, split view, preview, and path editing, while keeping hover tooltips on every icon-only control.
- Added a manual sidebar toggle and a simple focus-mode action in [`src/ui/main_window.rs`](src/ui/main_window.rs), with shared controller methods that keep sidebar/preview visibility state session-persistent until the user changes it again.
- Kept preview toggling on the same controller path, but split internal visibility application from user-triggered toggles so focus mode can hide panels without fighting the normal toolbar handlers.
- Removed the old filter button text mutation so the toolbar stays consistently icon-only and uses active styling instead of label growth.
- Tightened toolbar chrome in [`themes/default.css`](themes/default.css) and [`themes/high-contrast.css`](themes/high-contrast.css), including compact icon-button sizing and a persistent focus-mode active treatment.
- Updated [`README.md`](README.md) so the shipped toolbar/panel controls are documented accurately.

Why:
- The existing toolbar buttons were useful, but their text labels were taking too much horizontal space and crowding the path area.
- Lattice still needed explicit mouse-first controls for reclaiming space when the sidebar or preview pane is not needed, without pushing actions into menus or changing the default icon-grid-first workflow.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` to mention the compact icon-only toolbar and the manual sidebar / preview / focus-mode controls.

Known gaps:
- Live desktop validation is still needed for final icon recognizability and panel-resize feel because this environment cannot open a GTK display.
- The current `gtk4-rs` code style in this repo does not expose a simple accessible-name helper alongside the existing tooltip helper, so this pass relies on comprehensive tooltips rather than adding parallel accessible labels.

## 2026-05-16 — Large icon fill correction

Corrected the remaining mismatch between emoji file/folder icons and image thumbnails in the browser grid by making non-image icons occupy much more of the available media slot.

Changed:
- Increased the rendered icon glyph size substantially in [`themes/default.css`](themes/default.css).
- Mirrored the same icon-fill change in [`themes/high-contrast.css`](themes/high-contrast.css).
- Added explicit icon minimum width/height and tight line-height so the larger glyphs actually use the available icon buffer cleanly.

Why:
- Even after the earlier density/readability passes, folder/file glyphs were still visually tiny next to image thumbnails because the image previews were already using most of the media slot while the emoji icons were not.
- This pass fixes that specific imbalance without reopening the broader grid sizing again.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a focused icon-scale correction only.

Known gap:
- The final icon balance still needs a real GTK desktop eyeball pass on a live display.

## 2026-05-16 — Icon-dominant readability follow-up

Raised the icon grid again with a stronger emphasis on icon/media scale and only a mild text increase so the floating-icon view feels more natural next to the larger preview surfaces.

Changed:
- Increased the icon-item footprint and media/thumbnail sizing in [`src/ui/file_grid.rs`](src/ui/file_grid.rs) so icons occupy more of the available item buffer instead of feeling undersized.
- Bumped icon scale substantially and filename scale slightly in [`themes/default.css`](themes/default.css).
- Mirrored the same icon-heavy scale adjustment in [`themes/high-contrast.css`](themes/high-contrast.css).

Why:
- The prior readability bump improved legibility, but the icons still felt too small and visually timid compared with adjacent UI elements like image previews.
- This pass specifically corrects icon presence first while keeping the text change modest enough to preserve grid density.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was another focused visual sizing tweak, not a feature or workflow change.

Known gap:
- The final icon-to-density balance still needs a live GTK desktop review on a real display.

## 2026-05-16 — Icon/text readability bump

Raised the dense icon-grid sizing one notch after the floating-icon redesign so filenames and icons read more comfortably without reverting to bulky cards.

Changed:
- Increased the core icon-item footprint and media sizing in [`src/ui/file_grid.rs`](src/ui/file_grid.rs) so each grid item has a bit more room for the icon and two-line label.
- Bumped icon, filename, and tag-chip sizing slightly in [`themes/default.css`](themes/default.css).
- Mirrored the same scale increase in [`themes/high-contrast.css`](themes/high-contrast.css) so both bundled themes stay aligned.

Why:
- The denser floating-icon pass improved fit and spacing, but the resulting icon/text scale landed a bit too small for comfortable scanning.
- This follow-up restores readability with a minimal density tradeoff instead of reopening the broader layout pass.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a small readability-only tweak, not a workflow or feature-boundary change.

Known gap:
- The final balance between density and readability still needs a real GTK desktop eyeball pass on a live display.

## 2026-05-16 — Dense floating-icon grid redesign

Redesigned the default icon grid away from dashboard-like cards and toward a denser floating-icon file-manager view, then tightened the surrounding pane chrome to match.

Changed:
- Reworked [`src/ui/file_grid.rs`](src/ui/file_grid.rs) so icon items use a smaller square footprint, tighter grid spacing, two-line filenames, and simplified tag chip handling instead of the older card-style layout.
- Reduced pane header and view-strip spacing in [`src/ui/main_window.rs`](src/ui/main_window.rs), plus tightened tab-strip and status-bar widget spacing in [`src/ui/tab_strip.rs`](src/ui/tab_strip.rs) and [`src/ui/status_bar.rs`](src/ui/status_bar.rs).
- Replaced the old permanent card backgrounds in [`themes/default.css`](themes/default.css) with a transparent floating-icon treatment, lighter hover/selection glows, tighter sidebar/tab/status spacing, and smaller overall chrome around the file area.
- Mirrored the same density/layout updates in [`themes/high-contrast.css`](themes/high-contrast.css) so the alternate bundled theme stays aligned with the new grid structure.
- Updated [`README.md`](README.md) so the current visual-polish description no longer talks about bulky file cards.

Why:
- The existing file grid still looked like a dashboard of mini tiles, with too much permanent chrome, too much padding, and not enough files visible at normal window sizes.
- The new layout keeps the icon-grid-first, mouse-friendly direction intact while making the browser feel closer to a real desktop file manager.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` to describe the dense floating-icon grid and compact tag-chip presentation more accurately.

Known gaps:
- The final density, hover feel, and filename readability still need a manual GTK desktop acceptance pass on a real display session.
- This pass intentionally stayed within the icon-grid view; it did not add or redesign any separate list view.

## 2026-05-16 — Keyboard-first secondary support pass

Added a centralized keyboard action layer that keeps Lattice mouse-first while making standard desktop shortcuts and grid navigation work across the main browser surfaces.

Changed:
- Replaced the old one-off window search shortcut with a window-level keyboard dispatcher in [`src/ui/main_window.rs`](src/ui/main_window.rs) that routes standard file-manager commands, tab/pane shortcuts, path/search focus, sidebar navigation, and safe `Escape` behavior while leaving text-entry shortcuts alone.
- Added internal file clipboard state for copy/cut/paste, including paste-target rejection for non-folder views, cut-state updates after completed moves, and keyboard paste reuse of the existing conflict-dialog + queued copy/move pipeline.
- Extended the copy/move batch runner so move operations update tag metadata and directory copies use the recursive copy path instead of failing on folders.
- Added keyboard-aware grid state for current index / anchor tracking, arrow navigation, range selection, `Space`, `Enter`, `Ctrl+A`, and `Escape`, plus focus cycling between split panes with `F6`.
- Updated toolbar and tab-strip tooltips for the new standard shortcuts, and corrected `README.md` so it reflects the shipped keyboard and drag/drop behavior.
- Added controller-level tests for shortcut decoding, clipboard state transitions, and paste availability rules.

Why:
- The app already had mouse-first file actions and drag/drop plumbing, but normal desktop shortcuts were inconsistent and path/search entries were the only places with reliable keyboard handling.
- This pass makes keyboard support feel conventional without turning the UI into a keyboard-first workflow or splitting file operations into a second implementation path.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test ui::main_window::tests` — succeeded (`7` tests passed).
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` in this headless environment.

README reviewed:
- Updated `README.md` to document the shipped keyboard shortcut coverage and the existing drag/drop support, and to replace stale “not implemented” notes for current-folder search/drag-drop.

Known gaps:
- This environment cannot perform the manual GTK desktop acceptance pass for real focus behavior, text-entry interaction, pane switching, or live grid navigation feel.
- The app still has no shortcut remapping layer or permanent-delete shortcut, which remains intentionally out of scope for this pass.

## 2026-05-16 — File-type classification consistency fix

Improved file-type detection so common local files stop falling back to `Unknown`, and search results classify more like the normal browser.

Changed:
- Expanded the shared `FileKind` classifier in `src/ui/file_grid.rs` to recognize common audio and document formats, plus extension-based fallbacks for common image, video, document, audio, and text files when MIME metadata is sparse.
- Updated search result classification in `src/ui/main_window.rs` to retry unknown matches through GIO `standard::content-type` metadata so search labels stay aligned with the main browser more often.
- Switched the Downloads Triage documents filter to rely on the shared classifier instead of a separate path-extension helper.
- Added bundled theme colors for the new audio/document file-type classes in `themes/default.css` and `themes/high-contrast.css`.
- Updated the Milestone 6 roadmap line that lists the shipped file-type badge coverage.

Why:
- Common files like PDFs and some audio/media assets were being shown as unknown because the shared classifier only understood a narrow set of types.
- Search was less reliable than the browser because it guessed types from filenames instead of falling back to the same GIO metadata path used by normal browsing.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test` — failed due an unrelated existing assertion failure in `metadata::tests::recent_locations_are_capped_and_sorted` at `src/metadata.rs:646` (`left: "/tmp/two"`, `right: "/tmp/one"`).
- `cargo test ui::file_grid::tests` — succeeded.
- `cargo run` — succeeded.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a focused file-type classification fix, not a workflow or feature-boundary change.

Known gap:
- Search kind chips still expose the existing filter set only; audio and document files now classify correctly in the browser, preview, Downloads Triage, and search results, but there are not yet dedicated search filter chips for those categories.

## 2026-05-15 — Icon-grid reflow follow-up

Made a final follow-up tweak to smooth out icon-grid resizing after the first compact-card pass.

Changed:
- Switched the `FlowBox` in `src/ui/file_grid.rs` back to homogeneous child sizing now that each card has an explicit compact footprint.
- Kept the smaller card dimensions, but centered cards inside the grid cells so resize transitions stay visually uniform instead of wobbling between slightly different widths.

Why:
- The previous pass fixed oversized cards, but per-child width negotiation still made some intermediate resize states look uneven.
- Uniform FlowBox cells with fixed card dimensions give cleaner reflow without returning to the old oversized behavior.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a visual layout refinement, not a feature change.

Known gap:
- The final resize feel still needs live GTK desktop validation, since this environment cannot open a display.

## 2026-05-15 — Icon-grid card sizing pass

Tightened the icon-grid card layout so the grid fits more columns and stops ballooning awkwardly as the window changes width.

Changed:
- Switched the `FlowBox` in `src/ui/file_grid.rs` away from homogeneous sizing and reduced the outer grid margins while slightly increasing inter-card spacing.
- Added a fixed compact card footprint in Rust so file and folder cards keep a stable, square-ish size instead of stretching unpredictably.
- Reduced card padding, icon size, and text sizing in both bundled themes while keeping hover states, selection states, file-type styling, and filename readability intact.
- Limited card names to two centered lines within the smaller footprint so long names still read cleanly without forcing oversized cards.

Why:
- The previous combination of homogeneous FlowBox sizing and larger card minimums made the grid feel oversized and caused awkward reflow at common window widths.
- Stable compact card dimensions produce a cleaner icon-grid experience and let the column count respond naturally to the available width.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a focused layout tuning pass, not a feature change.

Known gap:
- The final column density and filename readability still need a live desktop visual pass in a real GTK session, since this environment cannot open a display.

## 2026-05-15 — Titlebar truncation fix

Fixed the window-title truncation issue where `Lattice` could collapse to `LATTI...` even at normal window sizes.

Changed:
- Added an explicit `GtkHeaderBar` title widget in `src/ui/main_window.rs` instead of relying on the implicit default title widget.
- Configured the title label as a single-line `Lattice` label with ellipsizing disabled so the normal titlebar no longer truncates the app name unnecessarily.

Why:
- The implicit headerbar title widget was being ellipsized too aggressively despite enough available space.
- An explicit title widget is the smallest clean way to control the title label without redesigning the rest of the header.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a focused UI bug fix, not a feature or workflow change.

Known gap:
- The titlebar still needs a live desktop visual confirmation in a real GTK session, since this environment cannot open a display.

## 2026-05-15 — Final stabilization pass

Ran a final stabilization pass focused on compile/runtime safety, broken interactions, obvious doc drift, and incomplete UI surfaces.

Changed:
- Removed the fake `Open With` context-menu action that only proxied to the normal open path and advertised an unimplemented chooser.
- Replaced the hard display `expect()` in CSS setup with a graceful early return plus stderr message so theme loading does not introduce an avoidable panic path.
- Updated theming docs to include the current tooltip and preview-host CSS hooks.
- Updated roadmap/product brief copy so they no longer claim the removed `Open With` item or stale milestone statuses.

Why:
- The exposed `Open With` action was misleading because no chooser existed yet.
- The display `expect()` violated the repo rule against runtime `unwrap`/`expect` on GTK state that can legitimately fail.
- Several docs still lagged behind the current shipped surface.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test` — succeeded (`11` tests passed).
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed for this pass.

Known gaps:
- Desktop validation is still missing for resize behavior, split-pane ergonomics, hover polish, theme switching, and launcher workflows on a real GTK/Wayland session.
- Large `Send to Project` folder copies still use synchronous recursive GIO calls, so big transfers may freeze the UI.
- Tags are still path-based and can be orphaned by renames or moves performed outside Lattice.

## 2026-05-15 — Restore intended default theme

Reverted the mistaken audit-driven theme rollback and put the intended visual direction back in place.

Changed:
- Restored `themes/default.css` to the newer Victorian Gothic dark styling instead of the older cyberpunk palette that was pulled in during the audit pass.
- Updated docs that described the default bundled theme as "dark cyberpunk" so they now match the actual shipped default theme.
- Left the CLI launch fallback fixes and tests from the Milestone 6 audit intact.

Why:
- The previous audit pass incorrectly treated a docs/theme mismatch as a code regression and overwrote desired visual work.
- The right fix was to preserve the intended styling and correct the stale documentation, not force the UI back to an older palette.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` so the default theme description matches the current UI.

Known gap:
- Live GTK validation is still blocked in this environment, so the restored theme still needs a real desktop visual pass.

## 2026-05-15 — Milestone 6 audit fixes

Audited the current Milestone 6 pass and corrected the main launch/theme regressions found during review.

Fixed:
- **CLI launch fallback behavior**: startup path resolution now validates `--path`, `--project`, and `--split` targets before the UI boots, falls back to Home when a requested folder is missing/unreadable, and surfaces a startup status message instead of silently opening a broken view.
- **Project launch polish**: `--project` now resolves project names case-insensitively and distinguishes between missing project names and projects whose saved root folder no longer exists.
- **Default theme/docs mismatch**: identified that the default theme description in the docs was stale relative to the intended visual work in the tree.
- **Launch regression tests**: added parser tests in `src/launch.rs` and launch-resolution tests in `src/ui/main_window.rs` so the non-GUI CLI behavior is covered without a display server.

Why:
- The Milestone 6 docs already claimed invalid `--path` and `--project` inputs would fall back to Home, but the implementation was still trusting raw CLI paths and could start in an unreadable/broken directory state.
- The launch-mode behavior was objectively incorrect; the theme issue should have been handled as a documentation mismatch rather than a forced visual rollback.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test` — succeeded (`11` tests passed).
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

Docs reviewed:
- Updated `README.md` to clarify that invalid launch paths and missing projects fall back to Home with a status message.
- Updated `docs/roadmap.md` so the Downloads Triage filter checklist matches the current `This Month` and `Older Than 1 Month` UI.

Known gap:
- Live Milestone 6 desktop validation is still blocked in this environment, so launch modes, theme switching, resize behavior, hover polish, split-pane ergonomics, and labwc integration still need a real GTK desktop pass.

## 2026-05-15 — Milestone 6: config, CLI, themes, desktop entry, docs, polish

### Config and theme system
- Added `src/config.rs` — loads `~/.config/lattice/config.toml` (hand-rolled key=value parser, no new deps).
- `AppConfig` exposes `theme` (default: `"default"`), `config_dir()`, and `themes_dir()`.
- Updated `src/app.rs` to load the config and resolve the theme CSS via `find_theme()`.
- Theme resolution order: user `~/.config/lattice/themes/<name>.css` → CWD `themes/<name>.css` → cargo-build relative → installed `<prefix>/share/lattice/themes/<name>.css`.
- Falls back to `"default"` theme if the configured theme file is not found.

### CLI launch modes
- Added `src/launch.rs` — `LaunchConfig` struct with `path`, `downloads`, `project`, `split` fields.
- `LaunchConfig::from_env()` parses `std::env::args()` manually; no new deps.
- Supported flags: `--path <dir>`, `--downloads`, `--project <name>`, `--split <left> <right>`. Bare positional arg treated as `--path`.
- Updated `src/main.rs` to parse launch config before connecting activate, and call `app.run_with_args(&[prog])` so GTK doesn't see the custom flags.
- `on_activate` in `app.rs` now accepts `&LaunchConfig` and threads it through.
- `MainWindow::new` and `BrowserController::new` both accept `&LaunchConfig`.
- Added `TabState::for_launch()` in `main_window.rs`: resolves the initial tab's path and view from the launch config, using the metadata store for `--project` lookups, and sets `split_enabled = true` for `--split`. The existing `reload_active_tab()` in `bootstrap()` picks up the tab state correctly.

### Two bundled themes
- `themes/default.css` — existing dark cyberpunk theme (no changes this session).
- `themes/high-contrast.css` — new high-contrast dark theme: black backgrounds, full-saturation `#00ffff` accent, `#ffffff` text, `#ff3355` danger, thick borders.

### Desktop entry
- Added `lattice.desktop` at repo root.
- `Exec=lattice %F` lets the desktop pass a directory path (used by `xdg-mime`).
- `MimeType=inode/directory` for default-opener registration.

### Documentation
- `docs/theming.md` — full CSS class reference and theme authoring guide.
- `docs/labwc.md` — install instructions, labwc keybindings (Super+E, Super+Shift+E, Super+Alt+E), xdg-mime setup, Waybar launcher snippet.
- Updated `README.md`: status, CLI usage, theme config, install instructions, project structure.
- Updated `docs/roadmap.md`: detailed M6 checklist reflecting what is and isn't done.

### Checks
- `cargo build`: succeeded.
- `cargo test`: 3 tests pass (metadata store).
- Known gap: desktop validation still requires a real GTK display session.

### Known gaps remaining for M6
- No drag-and-drop.
- No undo for destructive operations.
- No cross-folder search.
- Desktop acceptance pass still pending.

## 2026-05-15 — Downloads Triage title removal

Adjusted the Downloads Triage strip again after follow-up UI feedback.

Changed:
- Removed the Downloads Triage title line entirely from the triage strip instead of merely shortening its text.
- Kept the shared strip title visible for tag views, but hide it for triage views so only the filter buttons remain.

Why:
- The Downloads Triage title in the strip was redundant with the surrounding navigation and still wasted vertical space.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a layout refinement, not a feature change.

Known gap:
- Live GTK validation is still blocked in this headless environment, so the final triage strip spacing still needs a desktop check.

## 2026-05-15 — Downloads Triage filter cleanup

Tightened the Downloads Triage header and expanded its time filters after UI feedback.

Changed:
- Removed the extra slogan text from the Downloads Triage view strip so it only shows the section title.
- Added `This Month` and `Older Than 1 Month` triage filters alongside the existing `Today` and `This Week` filters.
- Updated `README.md` so the current Downloads Triage filter set is accurate.

Why:
- The extra line in the triage strip was wasting vertical space without helping navigation.
- The existing time filters were too narrow for real Downloads cleanup; month-level buckets make the view more practical.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` for the revised Downloads Triage filter set.

Known gap:
- Live GTK validation is still blocked in this headless environment, so the new filter buttons still need a desktop click-through.

## 2026-05-15 — Milestone 5 audit fixes

Audited the Milestone 5 Projects/Tags/Downloads Triage pass and fixed the main correctness gaps found during review.

Fixed:
- **Tagged-path persistence on rename/move/trash**: added metadata prefix migration/cleanup helpers so file-tag rows now follow items when they are renamed in Lattice, moved to a project from Lattice, or moved to trash from Lattice.
- **Recursive project copy support**: `Send to Project` copy now handles folders recursively instead of relying only on the simple file copy path.
- **Known limitation docs**: documented that tags are still keyed by full path, so external renames/moves outside Lattice can still orphan tag associations even though in-app rename/move/trash now update them.
- **Metadata regression tests**: added in-memory SQLite tests covering project creation, tagged-path remapping, and tagged-path cleanup.

Audit notes:
- `cargo run` still cannot be fully desktop-verified in this environment because GTK fails to open a display in this headless session.
- SQLite is present intentionally through `rusqlite` with the bundled SQLite feature and the metadata database remains under the user data directory, not in the repo.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test` — succeeded (`3` metadata tests passed).
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` and `docs/metadata.md` to document the path-based tagging limitation more clearly.

Known gap:
- Manual desktop acceptance is still required for the full Milestone 5 UI flows, especially project send dialogs and Downloads Triage interaction on a live GTK display.

## 2026-05-15 — Milestone 5 projects, tags, and downloads triage

Implemented the first app-local workflow features that make Lattice more than a plain folder browser.

Changed:
- Added `rusqlite` with a bundled SQLite build and created `src/metadata.rs` for metadata storage.
- Added automatic metadata database initialization under the user data directory and documented it in `docs/metadata.md`.
- Added SQLite tables for `projects`, `tags`, `file_tags`, and `project_destinations`.
- Rebuilt the sidebar to support dynamic project and tag sections plus a dedicated Downloads Triage entry.
- Added project pinning for the current folder and folder cards.
- Added `Send to Project` with copy/move choice and conflict handling (`Cancel`, `Rename Copy`, `Replace`).
- Added tag creation, tag assignment, tag removal, tag chips on file cards, and tag-filtered virtual views.
- Added pane-local virtual view state so tabs/split panes can show normal folders, tag views, or Downloads Triage without breaking the existing browser model.
- Added Downloads Triage filters for `All`, `Today`, `This Week`, `Images`, `Videos`, `Archives`, `Documents`, and `Large Files`.
- Updated `README.md` and `docs/roadmap.md` so they describe the Milestone 5 state instead of the old roadmap.

Why:
- Milestone 5 is the first point where Lattice needs app-local metadata and workspace-specific views instead of behaving like a generic file manager.
- SQLite-backed metadata keeps tags and projects transparent and simple without introducing xattrs or sync concerns yet.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated `README.md` for the new Milestone 5 workflows and metadata overview.

Known gaps:
- Milestone 5 still needs a manual desktop acceptance pass on a real GTK display session.
- The new project-transfer flows compile and are wired, but they have not been manually exercised in a live desktop session from this environment.

## 2026-05-15 — Tooltip polish and disabled-hover restore

Adjusted the new popover-based tooltip system after desktop feedback.

Changed:
- Removed the heavy default popover shell styling from custom tooltips and restyled the tooltip surface with a thinner border and lighter padding.
- Stopped marking tooltip popovers as insensitive so they do not inherit disabled-looking GTK chrome.
- Added a shared tooltip host wrapper for controls that can become insensitive, so hover tooltips still work when the child button itself is disabled.
- Applied the disabled-hover wrapper to Back, Up, Rename, Trash, and tab close controls.

Why:
- The first popover pass fixed the random sizing bug but the outer tooltip chrome looked too heavy.
- Disabled buttons no longer received hover events directly, so their tooltips needed a live wrapper target.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a visual/interaction polish pass, not a documented workflow change.

Known gap:
- The tooltip visuals and disabled-hover behavior still need manual validation in a real GTK desktop session.

## 2026-05-15 — Tooltip hover popover rewrite

Replaced the shared tooltip helper again after the first fix did not resolve the unstable sizing behavior in live hover use.

Changed:
- Removed reliance on GTK's built-in tooltip widget for the shared `attach_tooltip()` path.
- Rebuilt tooltips as explicit hover-triggered `GtkPopover` instances with a fixed delay, single-line label, and max-width cap.
- Added dedicated tooltip popover CSS classes so the custom hover surface stays styled through `themes/default.css`.

Why:
- The previous switch back to `set_tooltip_text()` still left random size changes and clipping.
- A popover-based tooltip path gives Lattice direct control over sizing and avoids GTK's reused tooltip widget behavior entirely.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was an internal UI rendering fix, not a feature or workflow change.

Known gap:
- The new popover tooltip behavior still needs manual hover validation in a real GTK desktop session.

## 2026-05-15 — Tooltip sizing fix

Replaced the custom tooltip `query-tooltip` workaround with GTK's built-in plain-text tooltip handling.

Changed:
- Removed the custom `attach_tooltip` path that built a new label widget for every hover.
- Switched tooltip wiring back to `set_tooltip_text()` through the shared helper.
- Shortened verbose toolbar and tab-strip tooltip strings so the common hover state stays compact.
- Changed tab button tooltips to use the existing home-abbreviated display path instead of the raw full filesystem path.
- Simplified tooltip CSS by removing the now-unused custom tooltip label styling and restoring text styling on the `tooltip` selector.

Why:
- The custom tooltip widget path was producing inconsistent popup sizing and clipping.
- Standard GTK text tooltips are a better fit for these simple one-line hints and avoid the extra custom sizing path.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Reviewed `README.md`; no update was needed because this was a tooltip rendering fix and did not change documented feature boundaries or workflow.

Known gap:
- Tooltip behavior still needs manual verification in a real GTK desktop session because hover UI cannot be validated from this headless environment.

## 2026-05-15 — Milestone 4 audit

Audited Milestone 4 (tabs and split-pane browsing) for shared-state bugs, wrong-pane action routing, crashes, and layout issues.

Confirmed working:
- Tab close guard (`len <= 1` prevents closing the last tab)
- Pane activation on left-click (both `file_grid.flow` and `pane.root` have click controllers)
- Context menus capture slot in closure — right-click always acts on the correct pane
- All toolbar actions (`rename`, `trash`, `new folder`, etc.) route through `active_slot()`
- `show_hidden` toggle reloads both panes in split mode via `reload_active_tab()`
- Generation-based async cancellation is per-slot — stale loads cannot overwrite fresh ones

Fixed:
- **Split divider reset bug**: `sync_split_visibility()` was calling `split_host.set_position(540)` every time split was enabled (including on every tab switch that restored a split tab). This reset any user-adjusted divider position. Removed the `set_position` call; the initial `540` in `build_center()` handles first-construction only.
- **`split_host` dead field**: After removing the `set_position` call, `split_host` was unused everywhere. Removed it from `BrowserController`, `BrowserController::new()`, `BodyLayout`, `CenterLayout`, and both builder functions.
- **CSS violation — preview host**: `build_body()` called `preview_host.set_size_request(320, -1)`. The CSS already had `.preview-host { min-width: 320px; }`. Removed the duplicate Rust call.
- **CSS violation — input dialog**: `build_input_dialog()` had hardcoded `column.set_size_request(420, -1)` and `entry.set_size_request(420, -1)`. Changed `column` to use a `dialog-column` CSS class; added `.dialog-column { min-width: 420px; }` and `min-width: 420px` to `.dialog-entry` in CSS. Removed both Rust calls.
- **Malformed status message**: `finish_batch()` formatted as `"{count} {message} completed."` with `message = "Moved item(s) to trash."` — produced `"3 Moved item(s) to trash. completed."`. Changed format to `"{count} item(s) {message}."` and updated call site to `"moved to trash"` → produces `"3 item(s) moved to trash."`.

Ran `cargo fmt`: succeeded.
Ran `cargo check`: succeeded — zero warnings.
`cargo run` skipped — environment is headless. No runtime behavior other than the fixes above was changed.

Known gap: Milestone 4 still requires a manual desktop acceptance pass. Tabs and split panes are implemented and compile clean but have not been validated in a live GTK session.

## 2026-05-15 — Remove permanent delete

Removed permanent delete entirely at user direction. Move to Trash is and remains the only destructive action in Lattice.

Removed:
- "Permanent Delete" menu item from the file right-click context menu
- `confirm_permanent_delete` method (dialog + confirmation flow)
- `delete_paths_permanently` method
- `run_delete_batch` method (called `gio::File::delete_async`)

`finish_batch` was retained — it is shared with the trash flow.

Updated `README.md` and `docs/roadmap.md` to remove all mentions of permanent delete.
`docs/agent_rules.md` retains the sentence "trash instead of permanent delete" as policy guidance — no change needed there.

Ran `cargo fmt`: succeeded.
Ran `cargo check`: succeeded.
`cargo run` skipped — this environment is headless (no display). No Rust behavior other than the removal was changed.

## 2026-05-15

### Project foundation

- Created the Rust GTK4 application shell for Lattice.
- Established the main visual structure: toolbar, sidebar, tab strip, icon grid, preview pane, and status bar.
- Added the dark cyberpunk CSS theme and stable CSS class structure.
- Added initial product docs: `README.md`, `docs/product_brief.md`, `docs/roadmap.md`, and `docs/agent_rules.md`.

### Milestone 1 completed

- Replaced placeholder file cards with real local folder browsing using GTK4 + GIO/GLib.
- Started the app in the user home directory.
- Implemented sidebar navigation for Home, Downloads, Documents, and conditional Projects.
- Added folder-first alphabetical sorting and hidden-file filtering with a toolbar toggle.
- Implemented double-click folder navigation and external file opening through GIO default-app launching.
- Wired Back, Up, Refresh, current path display, status bar updates, and preview-pane metadata updates.
- Kept folder enumeration asynchronous to avoid blocking the GTK main thread.

### Milestone 2 completed

- Added right-click context menus on file and folder cards.
- Implemented New Folder, Rename, Move to Trash, Permanent Delete, Copy Path, and Open Terminal Here.
- Added toolbar actions for New Folder, Rename, and Trash.
- Added confirmation/error dialogs and status feedback for destructive or failed operations.
- Refined terminal launching with terminal-command fallback behavior.
- Added empty-space folder context actions for the current directory.

### Bug-fix passes

- Fixed GTK CSS parser issues caused by unsupported CSS properties/selectors.
- Fixed a `RefCell already mutably borrowed` panic in navigation history flow.
- Fixed repeated New Folder dialog sizing and layout issues.
- Changed New Folder to prompt for the name before creating the folder, with `Cancel` and `Create` actions.

### Repository and release prep

- Initialized the repository on the `main` branch.
- Added `.gitignore` for build output and local agent/editor state.
- Added GitHub Actions CI to run `cargo fmt --check` and `cargo check`.
- Updated `README.md`, `docs/roadmap.md`, `docs/product_brief.md`, and `Cargo.toml` so the repo state matches the actual Milestone 2 implementation.
- Added an MIT `LICENSE`.

### Process scaffolding

- Added root-level `AGENTS.md` to hard-require reading the journal first, writing changelog entries to the journal, following project standards, and keeping `README.md` current after major steps.
- Added this `JOURNAL.md` as the active project journal and changelog.

### AGENTS contract audit

- Audited and strengthened `AGENTS.md` as the repo-level operating contract for coding agents.
- Added explicit rules for `cargo fmt` and `cargo check` after Rust changes, and `cargo run` after UI/startup/theme/config/runtime behavior changes when practical.
- Added no-fake-completion rules: do not claim features work without implementation plus manual checking, do not mark placeholders as done, and do not mark roadmap items complete without real acceptance.
- Added do-not-silently-remove-functionality rules.
- Strengthened visual, file-safety, styling, audit/fix, and final-report requirements.
- Reviewed `README.md` during this pass; no README content change was needed.
- No Rust code changed in this session, so `cargo fmt`, `cargo check`, and `cargo run` were not required and were intentionally not run.

### Milestone 3 preview pane pass

- Rebuilt the preview pane into a real widget with a toolbar show/hide toggle, preview actions, richer metadata rows, image display, and safe text previews.
- Added async selection-driven preview loading in `main_window.rs` so preview metadata and text reads do not block the GTK main thread.
- Added preview support for folders, images, text/config/code files, and metadata-only handling for video/audio/unknown files.
- Added preview actions for Open, Copy Path, and Open Parent Folder.
- Updated `themes/default.css` with the new preview classes and updated `README.md` and `docs/roadmap.md` to match the new milestone state.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: the binary built and launched, but runtime verification stopped immediately with `Gtk-WARNING **: Failed to open display` in this headless environment, so desktop/manual preview behavior could not be fully checked here.
- Known gap: the preview implementation is compiled and wired, but it has not been manually validated in a real desktop session from this environment.

### Layout sizing fix

- Adjusted the nested `GtkPaned` sizing rules so the preview pane behaves like a stable side panel and the center area takes the resize pressure first.
- Kept the preview host at a deliberate width instead of letting it collapse into a clipped state.
- Updated the sidebar scrolled window and label behavior so the left rail degrades with ellipsis instead of cutting off awkwardly when the window gets small.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: the binary built and started, but runtime verification again stopped at `Gtk-WARNING **: Failed to open display` because this environment does not have a usable GTK display.

### Toolbar path entry

- Changed the toolbar path field from display-only into a live navigation entry.
- Typing a folder path and pressing Enter now navigates directly there.
- Supports absolute paths, `~`, `~/...`, relative paths from the current folder, and `file://` input.
- Typing a file path opens its parent folder and selects the file when it is visible in the current listing.
- Added clear error dialogs for invalid, unsupported, or missing paths.
- Reviewed `README.md` and updated the current feature list to mention direct path entry.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: the binary built and launched, but runtime verification stopped at `Gtk-WARNING **: Failed to open display` because this environment is headless.

### Path bar polish

- Made the toolbar path field behave more like a real file-manager location bar.
- Focusing the field now swaps to the absolute filesystem path and selects the full value for quick replacement.
- Pressing `Escape` cancels editing and restores the normal display path.
- Leaving the field without committing restores the normal display path instead of leaving half-edited text behind.
- Added optional `Ctrl+L` focus/select behavior as a secondary accelerator without changing the mouse-first model.
- Reviewed `README.md` and updated the wording to describe the toolbar field as an editable location bar.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: the binary built and launched, but runtime verification again stopped at `Gtk-WARNING **: Failed to open display` because this environment is headless.

### Idle breadcrumb location bar

- Replaced the always-visible plain path field with a stacked location bar:
  - idle state shows breadcrumb-style path segments
  - click switches into full-path editing mode
- Kept the existing typed-path navigation behavior underneath the new breadcrumb presentation.
- Styled the breadcrumb view so it still reads as the same toolbar path control, not a generic button.
- Updated `README.md` to describe the location bar as breadcrumb-style with click-to-edit behavior.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.

### Milestone 4 implementation pass

- Implemented a real tab model with per-tab primary/secondary folder state, back history, split-mode state, and active-pane tracking in `src/ui/main_window.rs`.
- Replaced the placeholder tab-strip behavior with live new-tab, switch-tab, and close-tab flows.
- Added split-pane browsing with two independent icon-grid panes, active-pane focus routing, pane-local path headers, and toolbar/preview/state updates driven from the active pane.
- Added folder context actions for `Open Folder in New Tab`, `Open Folder in Split Pane`, and `Open Folder in Other Pane`.
- Preserved the existing browsing, preview, rename, trash, delete-confirmation, new-folder, copy-path, and terminal flows while routing them through the active pane.
- Added styling for live tabs, close buttons, split-pane headers, and active-pane state in `themes/default.css`.
- Reviewed and updated `README.md` and `docs/roadmap.md` so they describe the new Milestone 4 state honestly without claiming acceptance is complete.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: the binary built and started, but runtime verification stopped immediately at `Gtk-WARNING **: Failed to open display` because this environment is headless and has no usable GTK display.
- Known gap: Milestone 4 has not been manually validated on a real desktop session yet, so tabs/split panes are implemented and compiling but not claimed complete.

### Default theme color enrichment

- Increased the default theme's colorfulness without changing layout or interaction structure.
- Kept the warm gothic / floral dark direction, but introduced richer rose, violet, blue, and botanical accents across the header, toolbar, sidebar, tabs, pane strips, file cards, tag chips, preview pane, status bar, menus, dialogs, and tooltips.
- Preserved the existing typography, component hierarchy, and stable CSS selectors so the pass stays visual rather than structural.
- Reviewed `README.md` for this pass and left it unchanged because the behavior and feature surface did not change.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: succeeded in building and launching the app process; runtime emitted GTK label measurement warnings, and visual validation is still limited from this terminal-only session.

### Card sizing consistency pass

- Normalized icon-grid card media sizing so folders, files, and image thumbnails all sit inside the same square media slot instead of negotiating slightly different shapes.
- Kept the existing compact card footprint, but made the card surface subtler with flatter shading, lower-contrast borders, and softer hover/selected emphasis.
- This pass was limited to card geometry and card styling; no layout model or workflow behavior changed.
- Reviewed `README.md` and left it unchanged because this is a visual refinement rather than a feature or workflow update.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: succeeded in building and launching the app process from this environment.

### Card shell follow-up and tab warning cleanup

- Reworked the icon-grid cards again so GTK measures a fixed outer card shell rather than the inner content directly; this is intended to stop folder cards, image cards, and tagged cards from ending up with slightly different outlines.
- Restored a restrained hover/selection glow after the previous pass flattened the cards too much.
- Replaced the tab strip `+` and `×` text-label buttons with symbolic icon buttons, which removed the GTK label measurement warnings shown during startup.
- Fixed unrelated stale compile breakage that surfaced during verification: missing `PaneView::Trash` match arms and one `FileItem` initializer missing `original_path`.
- Reviewed `README.md` and left it unchanged because this session only refined visuals and repaired internal consistency.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded with one remaining warning about an unused `restore_items_from_trash` helper in `src/ui/main_window.rs`.
- Ran `cargo run`: succeeded and no longer emitted the earlier GTK label measurement warnings or theme parser warnings in captured startup output.

### Trash and System Drives sidebar views

- Kept the existing Trash virtual view and surfaced it clearly as a first-class sidebar destination backed by `trash:///`.
- Preserved the existing safety model: Trash remains the default destructive path, and the Trash view exposes only a basic `Restore from Trash` action when the original path is available instead of making permanent deletion easier.
- Replaced the dead sidebar Drives placeholder with a functional `System Drives` view built from mounted local volumes reported by GIO `VolumeMonitor`.
- The System Drives view now shows mounted volumes as browsable folder cards, uses a clear empty state when no mounted local volumes are available, and avoids risky rename/trash/drop behavior on mount roots.
- Fixed the relevant docs so `README.md`, `docs/product_brief.md`, and `docs/roadmap.md` reflect the real state: basic Trash restore exists, while deeper undo-style recovery still does not.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: succeeded in building and launching the app process from this environment.

### Recent sidebar view

- Replaced the dead `Recent` sidebar placeholder with a functional Recent view.
- Implemented an app-local recent folder history in the SQLite metadata database and used it to populate the Recent view with recently visited folders inside Lattice.
- Kept the Recent view read-only as a virtual location: folders can be reopened, opened in a new tab or split pane, pinned as projects, copied, or opened in a terminal, but rename/trash/drop behavior is intentionally disabled there.
- Missing or deleted recent folders are handled gracefully: they are skipped from the view and pruned from the stored recent-location history.
- Updated `README.md`, `docs/metadata.md`, and `docs/roadmap.md` so the persisted recent-folder behavior is documented accurately.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: succeeded in building and launching the app process from this environment.

### Popup dialog sizing stabilization

- Fixed the popup sizing regression where `Pin as Project` could appear at the wrong width first and then resize after opening.
- Reused the older dialog-sizing approach consistently: keep popup sizing on the shared dialog shell/CSS path instead of letting ad hoc dialog content or initial entry text determine the width after the dialog is shown.
- Tightened the shared input dialog builder by constraining entry natural width with both `width_chars` and `max_width_chars`, and added shared prompt/dialog-shell helpers so the hand-built tag/project/conflict dialogs use the same stable layout rules.
- Updated `AGENTS.md` to require shared dialog helpers and shared dialog CSS for popup sizing so future dialogs do not reintroduce this behavior.
- Reviewed `README.md` and left it unchanged because this was a UI polish fix rather than a feature or workflow change.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: succeeded in building and launching the app process from this environment.

### Popup dialog sizing follow-up

- Strengthened the popup sizing fix after `Pin as Project` was still reproducing the initial wrong-width resize behavior.
- The remaining issue was the dialog shell itself, not only the entry sizing, so the shared standard-dialog helper now applies an explicit GTK width request to the dialog, content area, and shared column in addition to the shared CSS/min-width path.
- Updated `AGENTS.md` again to make the rule concrete: popup dialogs must use the shared helper's width-stabilized GTK shell path, not just the shared CSS classes by name.
- Reviewed `README.md` and left it unchanged because this was still a UI polish fix rather than a user-facing feature change.
- Ran `cargo fmt`: succeeded.
- Ran `cargo check`: succeeded.
- Ran `cargo run`: succeeded in building and launching the app process from this environment.
