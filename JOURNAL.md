# Lattice — Journal

Chronological development journal for the active project state.

## 2026-05-17 — Context Menu Reliability Fix

Audited and fixed unreliable right-click context menu behavior.

Changed:
- `src/ui/main_window.rs`:
  - Item right-click gestures in icon and list views now run in capture phase and explicitly claim the event before opening the context menu.
  - Background right-click handlers now claim the event when opening the current-folder menu.
  - Sidebar Places right-click handlers now claim the event before opening their context menu.
  - Context-menu buttons now unparent their owning popover before running the action callback, so modal popups opened by menu actions visually win over old context menus.

Why:
- Right-clicking an unselected folder while another item was selected could let GTK selection handling consume/reorder the click, requiring a second right-click before the intended menu appeared.
- Context menus could remain visible when their action opened an in-window modal, making the menu appear over popup dialogs.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `cargo test places_can_be_created_and_removed` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

README reviewed:
- No README update needed; this fixes interaction reliability without changing documented features.

Known gaps:
- Live desktop validation is still needed to confirm single-right-click menu opening and modal-over-menu behavior on a real GTK display.

Files modified:
- `src/ui/main_window.rs`
- `JOURNAL.md`

Follow-up:
- Replaced per-card/per-row right-click controllers with one capture-phase FlowBox handler and one capture-phase ListBox handler.
- The container handlers now decide item vs. background from pointer coordinates and claim the event before GTK selection/focus handling can reinterpret it.
- This specifically targets the remaining soft-selected/focused item case where right-click could still be unreliable.
- Re-ran `cargo fmt`, `cargo fmt --check`, `cargo check`, `cargo test window_shortcuts_dispatch_standard_commands`, `cargo test places_can_be_created_and_removed`, `git diff --check`, and `timeout 3 cargo run`; all succeeded except `cargo run` again built then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

## 2026-05-17 — User-Managed Places

Made sidebar Places user-managed, separate from Projects.

Changed:
- `src/metadata.rs`:
  - Bumped the metadata schema to version 5.
  - Added a `places` table plus create/list/remove helpers.
  - Added a focused metadata test for creating and removing Places.
- `src/ui/sidebar.rs`:
  - Replaced fixed Downloads/Documents buttons with a dynamic Places list under fixed Home.
  - Added `SidebarTarget::Place` for active-state tracking.
- `src/ui/main_window.rs`:
  - Loaded pinned Places from metadata and connected each row to navigation, drag/drop, and a right-click menu.
  - Added Place row context actions: Open, Copy Path, Remove from Places.
  - Added `Pin to Places` to folder and background context menus.
  - Kept Places separate from Projects; Project pinning and Send to Project behavior are unchanged.
- `src/config.rs`, `README.md`, `docs/roadmap.md`, and `docs/product_brief.md`:
  - Documented the `pin_place` context action and the new Places behavior.

Why:
- Downloads and Documents should not be hardcoded Places. Home remains fixed, while every other sidebar Place is chosen and removable by the user.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test places_can_be_created_and_removed` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

Known gaps:
- Live desktop validation is still needed to right-click Places rows and verify add/remove/sidebar DnD behavior in a real GTK session.

Files modified:
- `src/metadata.rs`
- `src/ui/sidebar.rs`
- `src/ui/main_window.rs`
- `src/config.rs`
- `README.md`
- `docs/roadmap.md`
- `docs/product_brief.md`
- `JOURNAL.md`

## 2026-05-17 — Activity Log Row Actions and Undo History

Added actionable Activity Log rows backed by item-level operation history.

Changed:
- `src/metadata.rs`:
  - Bumped the metadata schema to version 4.
  - Added `activity_log_items` storage and `ActivityLogItem` loading so new receipts can preserve per-item source/destination paths.
  - Added a focused metadata test for item-history persistence.
- `src/ui/activity_log_panel.rs`:
  - Added compact row buttons for Undo, Repeat, Reveal, and Copy Path with disabled states when a row lacks enough history.
- `src/ui/main_window.rs`:
  - Wired Activity Log actions into existing file-operation flows.
  - Added guarded undo for copy, move, duplicate, rename, bulk rename, new file, new folder, and trash entries when new item-level history is available.
  - Added repeat handling for supported successful entries.
  - Began logging structured item history for copy/move/duplicate/trash/rename/bulk rename/new file/new folder/permanent delete operations.
- `themes/default.css` and `themes/high-contrast.css`:
  - Added Activity Log row action button styling.
- `README.md`, `docs/roadmap.md`, `docs/product_brief.md`, and `docs/theming.md`:
  - Documented Activity Log row actions, reversible undo boundaries, and new theme classes.
  - Corrected stale product-brief status for implemented folder search.

Why:
- Activity Log receipts were display-only. Row-local actions make the history useful for mouse-first recovery, repetition, navigation, and path copying while preserving the project rule that unsafe operations must not silently overwrite.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test activity_log_preserves_item_history` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

Known gaps:
- Existing Activity Log rows created before schema version 4 do not have item-level history, so they only support utility actions when enough row-level path data exists.
- Permanent delete, cancelled operations, and failed/partial operations remain non-undoable.
- Live desktop validation is still needed to confirm row button spacing and end-to-end undo/repeat behavior in a real GTK session.

Files modified:
- `src/metadata.rs`
- `src/ui/activity_log_panel.rs`
- `src/ui/main_window.rs`
- `themes/default.css`
- `themes/high-contrast.css`
- `README.md`
- `docs/roadmap.md`
- `docs/product_brief.md`
- `docs/theming.md`
- `JOURNAL.md`

Follow-up crash fix:
- Fixed a `RefCell already mutably borrowed` panic in `pin_project()` by ending the mutable metadata borrow before refreshing the sidebar.
- Removed unsupported GTK CSS properties reported at startup: `max-width` from sidebar CSS and `max-height` from conflict resolver CSS.
- Re-ran `cargo fmt`, `cargo fmt --check`, `cargo check`, `cargo test activity_log_preserves_item_history`, `cargo test window_shortcuts_dispatch_standard_commands`, and `git diff --check`; all succeeded.

## 2026-05-17 — Search/Triage Switch Responsiveness Fix

Fixed slow switching between Search and Triage.

Changed:
- `src/ui/search_panel.rs`:
  - Added `SearchQuery::is_unconstrained()` to identify the default empty search state.
- `src/ui/main_window.rs`:
  - Opening Search with no name, type/date/size filter, or tag filter now shows a prompt instead of immediately launching a recursive match-everything scan.
  - Added per-pane search cancellation flags so recursive blocking search checks can stop after leaving Search.
  - Pane load cancellation now marks an active search cancellation flag before invalidating the load generation.
  - Recursive search checks the cancellation flag while reading directories, scanning files, scanning subdirectories, and before descending further.

Why:
- Switching from Triage to Search launched a full recursive folder scan even before the user typed a query.
- Switching from Search to Triage invalidated stale results but did not stop the blocking filesystem traversal, so the old search could keep consuming disk/CPU while Triage was loading.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

README reviewed:
- No README update needed; documented search capabilities are unchanged.

Known gaps:
- Live desktop validation is still needed to confirm Search ⇄ Triage switching now feels immediate on large folders.

Files modified:
- `src/ui/main_window.rs`
- `src/ui/search_panel.rs`
- `JOURNAL.md`

## 2026-05-17 — Sidebar Tool Switch Guardrails

Stabilized switching between the Search, Triage, and Activity Log sidebar tools.

Changed:
- `src/ui/main_window.rs`:
  - Added a shared pane tool-scope resolver so Search and Triage can use the active pane's last real folder when switching from Search or Activity Log.
  - Search now opens from Activity Log using the pane's preserved folder context instead of reporting that search is unavailable.
  - Triage now opens from Search or Activity Log using the same preserved folder context.
  - Activity Log now clears hidden file-grid selection, resets keyboard state, updates status/preview/navigation/sidebar state, and preserves the pane folder context.
  - Cancelling a pane load now also advances that pane's load generation, preventing stale generation-checked async work such as search results from repainting after switching tools.

Why:
- The three sidebar tools are mode switches over a working folder. Treating Search and Triage as unavailable once the pane was already in Search or Activity Log made the TOOLS section feel inconsistent and left stale UI state behind.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

README reviewed:
- No README update needed; this fixes tool-switching behavior without changing documented capabilities or run instructions.

Known gaps:
- Live desktop validation is still needed to click through Search ⇄ Triage ⇄ Activity Log in a real GTK session and confirm the visual state feels right.

Files modified:
- `src/ui/main_window.rs`
- `JOURNAL.md`

## 2026-05-17 — Pane Header Tag and Hidden Controls

Moved Tag Filter and Hidden Files out of the top toolbar and into each pane header beside the existing icon/list view toggle.

Changed:
- `src/ui/toolbar.rs`:
  - Removed the toolbar Tag Filter and Hidden Files toggle buttons.
- `src/ui/main_window.rs`:
  - Added compact per-pane header buttons for Tag Filter and Hidden Files.
  - Kept `Ctrl+G` and `Ctrl+H` targeting the active pane.
  - Changed hidden-file visibility from one global flag to per-pane, per-tab state, matching the per-pane view mode behavior.
  - Routed directory, tag/search loading, and known-path item lookup through the active pane's hidden-file state.
  - Kept tag-filter panel open/active state reflected on the pane header button.
- `themes/default.css` and `themes/high-contrast.css`:
  - Added active styling for pane header controls.
  - Removed stale toolbar tag-filter styling.
- `README.md` and `docs/theming.md`:
  - Updated feature/status and theme class docs for the new per-pane control placement.

Why:
- Tag filtering and hidden-file visibility are pane-local browsing controls, like icon/list view mode, and should live in the pane-specific header instead of the global toolbar.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

Known gaps:
- Live desktop validation is still needed to confirm the new pane-header control spacing and active states in a real GTK session.

Files modified:
- `src/ui/toolbar.rs`
- `src/ui/main_window.rs`
- `themes/default.css`
- `themes/high-contrast.css`
- `README.md`
- `docs/theming.md`
- `JOURNAL.md`

## 2026-05-17 — Tag Filter Collapse Fix

Fixed the Tag Filter panel leaving dead vertical space when closed.

Changed:
- `src/ui/main_window.rs`:
  - Switched the tag-filter revealer from `Crossfade` to `SlideDown`, so closing the panel collapses vertically instead of fading while preserving allocation.
  - Initial tag-filter revealer state is now invisible as well as unrevealed.
  - `set_filter_panel_open(false)` now unreveals the panel and hides the revealer after the close animation, matching the sidebar/preview/tray/panel queue pattern.
  - Activity Log view also hides the tag-filter revealer directly when suppressing the filter panel.

Why:
- The previous visibility fix made the tag filter content visible to the revealer, but `Crossfade` plus an always-visible revealer could keep layout space reserved even while the panel was not visually open.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test tag_filter` — succeeded, but ran 0 tests because no focused tag-filter tests currently exist.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

README reviewed:
- No README update needed; this fixes closed-state layout behavior for an existing control.

Known gaps:
- Live desktop validation is still needed to confirm the closed Tag Filter panel no longer reserves space.

Files modified:
- `src/ui/main_window.rs`
- `JOURNAL.md`

## 2026-05-17 — Per-Pane View Toggle and List Row Layout Finish

Finished the incomplete icon/list view-mode relocation and list-view layout fix started by another agent.

Changed:
- `src/ui/toolbar.rs`:
  - Removed the global top-toolbar icon/list view toggle from the toolbar surface.
  - Left split-panel cycling in the top toolbar.
- `src/ui/main_window.rs`:
  - Added a per-pane icon/list toggle button in each pane header.
  - Wired each pane button to toggle only that pane's `ViewMode`, preserving independent view modes across split panes and tabs.
  - Added a concise tooltip for the pane button: `Toggle icon/list view (Ctrl+1/Ctrl+2)`.
- `src/ui/file_grid.rs`:
  - Removed the list-row kind text column; the file-type icon already conveys type.
  - Hardened list-row allocation so the name column keeps the left side visible and ellipsizes naturally, while size/date columns use less fixed width and ellipsize if needed.
- `themes/default.css` and `themes/high-contrast.css`:
  - Added/finished `.pane-view-btn` styling for both bundled themes.
  - Removed stale `.file-list-kind` styling.
- `README.md` and `docs/theming.md`:
  - Updated docs to reflect the per-pane icon/list toggle and the new `.pane-view-btn` theme class.

Why:
- The top toolbar was crowded, and view mode is a pane-specific preference in split layouts.
- In list view, the type/kind column consumed horizontal space and could make the useful left-side filename area collapse awkwardly in narrow split panes.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

Known gaps:
- Live desktop validation is still needed to confirm the list row truncation and pane-header toggle placement in real split-pane use.

Files modified:
- `src/ui/toolbar.rs`
- `src/ui/main_window.rs`
- `src/ui/file_grid.rs`
- `themes/default.css`
- `themes/high-contrast.css`
- `README.md`
- `docs/theming.md`
- `JOURNAL.md`

## 2026-05-17 — Toolbar Surface Toggles Regrouping

Moved the Tag Filter and Hidden Files buttons into the same toolbar group as the side-panel and workspace-surface toggles.

Changed:
- `src/ui/toolbar.rs`:
  - Moved `filter_toggle` and `show_hidden_toggle` from the file-actions group into Group 2 with Sidebar, Preview, Holding Tray, and Plan Actions.
  - Left button behavior, CSS classes, tooltips, and shortcuts unchanged.
  - File Actions now stays focused on creation/rename/trash actions plus the contextual Empty Trash button.

Why:
- Tag filtering and hidden-file visibility are view/surface controls, not file operations.
- Grouping them with Sidebar, Preview, Holding Tray, and Plan Actions makes the toolbar semantics clearer.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

README reviewed:
- No README update needed; the existing toolbar description remains accurate at a high level.

Known gaps:
- Live desktop validation is still needed to confirm the revised toolbar grouping feels right visually.

Files modified:
- `src/ui/toolbar.rs`
- `JOURNAL.md`

## 2026-05-17 — Tag Filter Panel Visibility Fix

Fixed the toolbar Tag Filter button appearing to do nothing.

Changed:
- `src/ui/tag_filter.rs`:
  - Removed the initial `root.set_visible(false)` from `TagFilterPanel::build()`.
  - The parent `Revealer` in `main_window.rs` remains the single show/hide controller for the panel.

Why:
- The toolbar button toggled the revealer correctly, but the tag filter panel root stayed hidden, so the revealed child could remain invisible.
- With the panel root visible, opening Tag Filter can show either tag chips or the existing empty-state hint.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test tag_filter` — succeeded, but ran 0 tests because no focused tag-filter tests currently exist.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

README reviewed:
- No README update needed; this fixes broken visibility for an existing UI control and does not change documented behavior.

Known gaps:
- Live desktop validation is still needed to confirm the panel visibly opens and shows the empty-state hint or tag chips.

Files modified:
- `src/ui/tag_filter.rs`
- `JOURNAL.md`

## 2026-05-17 — Concise Toolbar Tooltips and Missing Shortcuts

Updated the top toolbar buttons so every toolbar action has concise tooltip text with its keyboard shortcut included, and disabled-capable buttons keep showing tooltips while greyed out.

Changed:
- `src/ui/toolbar.rs`:
  - Shortened toolbar tooltip copy across navigation, view/layout, workspace, file-action, trash, and path controls.
  - Added shortcut text to Holding Tray and Empty Trash tooltips.
  - Wrapped disabled-capable file-action buttons (`New Folder`, `New Text File`, `Rename`, `Move to Trash`, `Empty Trash`) in tooltip hosts so the hover target remains active when the child button is insensitive.
  - Switched dynamic view tooltip updates to the custom tooltip label instead of GTK's built-in tooltip text.
  - Removed the dynamic tooltip sensitivity gate so custom tooltips can still appear for greyed-out buttons if those buttons later become disabled.
- `src/config.rs`:
  - Added default shortcuts for `toggle_holding_tray` (`Ctrl+Alt+H`) and `empty_trash` (`Ctrl+Shift+Delete`).
  - Added those shortcuts to the generated config comments.
- `src/ui/main_window.rs`:
  - Added `ToggleHoldingTray` and `EmptyTrash` window commands.
  - Routed the new shortcuts through the existing window shortcut dispatcher.
  - Added shortcut test coverage for the two new commands.
- `README.md`:
  - Updated the built-in shortcut ID list to include the new and recently added toolbar-related IDs.

Why:
- The toolbar had a few long or inconsistent tooltips.
- Holding Tray and Empty Trash were mouse-only toolbar actions.
- Disabled buttons such as Rename, Trash, New Folder in virtual views, and Empty Trash in an empty Trash view need explanatory hover text even when greyed out.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `cargo test window_shortcuts_dispatch_standard_commands` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built successfully, then failed with `Gtk-WARNING **: Failed to open display` because this environment has no active GTK display.

Known gaps:
- Live desktop validation is still needed to visually confirm the disabled-button tooltip behavior on a real GTK session.

Files modified:
- `src/ui/toolbar.rs`
- `src/ui/main_window.rs`
- `src/config.rs`
- `README.md`
- `JOURNAL.md`

## 2026-05-17 — Toolbar Reorganisation and Search as Sidebar Tool

Restructured the top toolbar into three logical groups separated by elegant dividers, and moved Search out of the toolbar into the sidebar TOOLS section.

Changed:
- `src/ui/toolbar.rs`:
  - Removed `search_button` from the struct and all construction code.
  - Reordered toolbar into three groups, left to right:
    - **Group 1 (Navigation / View Basics):** Back, Up, Refresh, View toggle (icon/list), Split/Panel cycling button.
    - **Divider 1.**
    - **Group 2 (Workspace Surfaces):** Left sidebar toggle, Preview panel toggle, Holding Tray toggle, Action Planning Mode toggle.
    - **Divider 2.**
    - **Group 3 (File Actions):** New Folder, New Text Doc, Rename, Trash, Tag Filter, Show Hidden, Empty Trash.
    - **Divider 3.**
    - **Path / breadcrumb** (expands to fill remaining space).
  - Improved sidebar toggle tooltip: "Show/hide left sidebar (Ctrl+B)".
  - Improved preview toggle tooltip: "Show/hide preview panel (Ctrl+P)".
  - View toggle now precedes split/panel toggle in Group 1 for cleaner visual flow.
- `src/ui/sidebar.rs`:
  - Added `Search` variant to `SidebarTarget` enum.
  - Added `pub search_button: Button` to `Sidebar` struct.
  - Search is now the **first item in the TOOLS section** (before Triage and Activity Log).
  - `set_active` now highlights the Search button when the active pane is in search view.
- `src/ui/main_window.rs`:
  - Removed `toolbar.search_button.connect_clicked` wiring from `connect_navigation`.
  - Added `sidebar.search_button.connect_clicked` wiring in `connect_sidebar`, calling the same `open_search_in_current_dir()`.
  - `update_sidebar_state` now maps `PaneView::Search(_)` to `SidebarTarget::Search` instead of `None`, so the Search sidebar item lights up when searching.

Why:
- The toolbar was getting crowded with actions of different categories mixed together.
- Search is a contextual tool that belongs with Triage and Activity Log, not as a trigger button next to navigation.
- The three-group divider layout gives visual breathing room and makes the toolbar semantically obvious at a glance.
- Functional search behaviour is unchanged — Search still opens the full search view with filename, kind, date, size, and tag filters, non-blocking async results, and normal file grid actions.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded, zero warnings.

Files modified:
- `src/ui/toolbar.rs`
- `src/ui/sidebar.rs`
- `src/ui/main_window.rs`
- `README.md`
- `JOURNAL.md`

## 2026-05-17 — Beautiful Conflict Resolver

Replaced all file conflict handling with a polished, batched, mouse-first Conflict Resolver dialog hosted inside ModalHost.

Changed:
- Added `src/ui/conflict_resolver.rs` (new module): `ConflictItem`, `Resolution`, `ConflictDecision` types; `collect_conflicts()`, `apply_decisions()`, `decisions_note()` helpers; `show()` UI function.
- Registered `pub mod conflict_resolver` in `src/ui/mod.rs`.
- Added ~140 lines of conflict resolver CSS classes to `themes/default.css`: conflict-item-row cards, name/type-chip/meta labels, three-way choice toggle buttons (keep/replace/skip), batch action buttons, cancel styling.
- Removed `show_conflict_dialog()` (gtk::AlertDialog–based, ugly, per-file) from `main_window.rs`.
- Removed `ConflictChoice` enum from `main_window.rs`.
- Removed private `free_name_in()` from `main_window.rs` (now lives in `conflict_resolver.rs`).
- Converted `handle_dnd_drop` from `async fn` to plain `fn` (no longer needs async since conflict resolution is now callback-based via ModalHost, not awaited AlertDialog).
- Updated all four DnD drop callers to call `handle_dnd_drop` directly instead of `glib::MainContext::spawn_local(async move { ... .await })`.
- Updated paste clipboard caller similarly.
- Added `start_copy_move_with_conflict_check()`: collects conflicts, shows resolver if any, passes decisions to `apply_decisions()`, then calls `start_copy_move_op()` with enriched label including conflict note.
- Updated `execute_plan_queue` Copy/Move arms to call `start_copy_move_with_conflict_check` instead of `plan_copy_move_items` → `start_copy_move_op` directly. This fixes a bug where queued Copy/Move with conflicts would fail per-file with opaque GIO errors.

Why:
- `gtk::AlertDialog` was the wrong widget for Lattice conflict UX: native OS appearance, no file metadata, ugly per-file prompts.
- Plan-mode queued Copy/Move with conflicts was silently failing per-file (GIO returned IOErrorEnum::Exists).
- Both paths are now unified under the same polished resolver.

Conflict Resolver features:
- Shows all conflicts batched before any operation starts (not per-file prompts).
- Per-conflict: file name, MIME type chip, existing size + age, incoming size + age.
- Per-conflict choices: Keep Both (default), Replace ⚠ (danger-tinted), Skip.
- Multi-conflict: "Keep Both for All" and "Skip All" batch buttons (no Replace All — dangerous choice must be made per file).
- "Cancel All" aborts the entire operation.
- "Apply Choices" starts the op; label includes conflict note, e.g. "(1 renamed, 2 replaced)".
- Logged to Activity Log via the existing `log_activity` path with enriched summary string.
- Never defaults to Replace.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded, zero warnings.
- `cargo run` — app launched (terminated by test timeout; launch confirmed clean).

Known gaps:
- `show_project_conflict_dialog` (project transfer path) still uses its own per-file ModalHost dialog. It already uses ModalHost so it's not ugly, but it does not use the new resolver. Could be unified in a future pass.
- Live desktop testing with actual file conflicts needed to validate the full interaction.

Files modified:
- `src/ui/conflict_resolver.rs` (NEW)
- `src/ui/mod.rs`
- `src/ui/main_window.rs`
- `themes/default.css`
- `JOURNAL.md`

## 2026-05-17 — Warning and GTK adjustment cleanup

Cleaned up the remaining startup warnings and backed out the adjustment override that likely caused the last pair of GTK startup criticals.

Changed:
- Removed unused `ModalHost::show_action_plan()`. Action-plan modals now use the active `BrowserController::show_action_plan()` path.
- Removed unused `ActionPlan::short_file_list()` and `ActionPlan::confirm_label()`, which were only referenced by the removed modal helper.
- Removed unused `MetadataStore::clear_activity_log()`.
- Removed explicit file-grid/list `ScrolledWindow` adjustment overrides added during earlier DnD debugging.

Why:
- `cargo run` was still reporting three Rust dead-code warnings.
- Live testing showed the GTK criticals had narrowed to two startup `gtk_adjustment_get_value` calls after tray drop-target removal. The remaining explicit adjustment override was no longer needed and was the most likely source of that startup noise.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with zero warnings.
- `cargo test holding_tray` — succeeded.
- `timeout 3 cargo run` — built with zero Rust warnings and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless. No adjustment criticals appeared before the display failure.

README reviewed:
- No README update needed; this is warning/error cleanup and does not change documented behavior.

Known gaps:
- Live desktop validation should confirm the startup adjustment criticals are gone on a real display session.

## 2026-05-17 — Source-side Holding Tray staging

Replaced tray-specific GTK drop targets with source-side tray staging after live testing showed persistent GTK adjustment criticals and unreliable tray drop resolution.

Changed:
- Removed active tray `DropTarget` use from the Holding Tray staging path.
- File-grid and list-row drag sources now remember the dragged paths during `prepare`.
- On drag end or drag cancel, the source checks the pointer location; if the pointer is over the Holding Tray or one of its children, the dragged paths are staged.
- The source explicitly marks the drag done after successful tray staging.
- Added one tray-wide visual affordance during file-card/list-row drags by toggling `.holding-tray-drop-active` from the drag source instead of from a tray drop target.

Why:
- The tray is a staging area, not a filesystem destination. It does not need GTK's full drop-target machinery, and that machinery was producing adjustment criticals in live desktop testing.
- Detecting tray staging from the source drag keeps normal GTK DnD available for real file operations while isolating the tray from GTK drop/autoscroll internals.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with the existing 3 warnings (`short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan` are unused public/future APIs).
- `cargo test holding_tray` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; documented tray drag-in behavior is unchanged.

Known gaps:
- Live desktop validation is still needed. If GTK adjustment criticals persist with no tray `DropTarget`, they are unrelated to tray staging and come from the remaining GTK DnD source or real file drop targets.

## 2026-05-17 — Holding Tray wide hit targets without duplicate visual zones

Adjusted the tray drop implementation after live testing showed the single-target version was unreliable, while the earlier multi-target version was reliable but visually confusing.

Changed:
- Restored multiple tray drop hit surfaces for reliability: panel, staged item strip, empty label, and action buttons.
- Kept a single visible tray drop style by applying only `.holding-tray-drop-active` to the tray panel.
- Removed the tray drop target motion handler; enter/drop/leave are sufficient for staging files and avoid extra GTK drag-motion work.
- Moved source grid/list scroller adjustment setup until after child widgets are installed.

Why:
- GTK drop targets are per-widget enough that one parent target missed practical tray areas.
- Multiple hit surfaces should not imply multiple visual drop zones.
- The persistent adjustment criticals happen during drag motion; reducing tray motion handling and ensuring source scrollers have adjustments after child setup is the least invasive next step.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with the existing 3 warnings (`short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan` are unused public/future APIs).
- `cargo test holding_tray` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; documented tray behavior is unchanged.

Known gaps:
- Live desktop validation is still needed for the drag/drop hit area and GTK critical output.

## 2026-05-17 — Holding Tray single drop highlight + source scroll adjustments

Followed up on live feedback that tray drag/drop still showed GTK adjustment criticals and two highlighted drop zones.

Changed:
- Reduced Holding Tray drop handling to one capture-phase drop target on the visible tray panel instead of separate drop targets on nested tray children.
- Switched the tray hover style from generic `.drop-active` to a dedicated `.holding-tray-drop-active` class in both bundled themes, so the tray has one visual drop zone and does not share pane drop styling.
- Added explicit horizontal and vertical adjustments to the main icon-grid and list-view `ScrolledWindow`s. The tray no longer has a scroller, so remaining adjustment criticals during drag are most likely from the source grid/list autoscroll path.

Why:
- Multiple nested tray drop targets could create competing enter/leave visual states.
- GTK's repeated `gtk_adjustment_get_value` criticals are still occurring after tray scroller removal, so the source file-grid scrollers need explicit adjustments too.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with the existing 3 warnings (`short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan` are unused public/future APIs).
- `cargo test holding_tray` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; documented tray behavior is unchanged.

Known gaps:
- Live desktop validation is still needed to confirm the adjustment criticals and duplicate drop-zone highlight are gone.

## 2026-05-17 — Holding Tray scroller removal for DnD criticals

Removed the Holding Tray's horizontal `ScrolledWindow` after live testing still showed repeated GTK adjustment criticals during drag attempts.

Changed:
- Replaced the tray item's horizontal `ScrolledWindow` with a direct compact horizontal item box.
- Left tray drop targets attached to the visible tray surfaces from the previous reliability fix.

Why:
- The tray is intentionally compact and should not behave like a second file browser.
- Removing the tray scroller takes tray-side GTK adjustment/autoscroll behavior out of the drag path entirely, which is the safest response to the repeated `gtk_adjustment_get_value` / `gtk_adjustment_set_value` criticals.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with the existing 3 warnings (`short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan` are unused public/future APIs).
- `cargo test holding_tray` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; documented tray behavior is unchanged.

Known gaps:
- Live desktop validation is still needed. If adjustment criticals continue after this change, they are likely coming from the source grid/list drag surface rather than the Holding Tray.

## 2026-05-17 — Holding Tray drop reliability fix

Tightened the Holding Tray drag target after live testing showed drops were detected intermittently and GTK emitted repeated adjustment criticals during tray drag attempts.

Changed:
- Exposed the tray's visible panel, header, body, item strip, and empty label from `HoldingTray` so the controller can attach drop handling to real visible surfaces instead of only the outer `Revealer`.
- Attached capture-phase drop targets to the tray's visible surfaces. This makes drops over the header, action area, empty state, staged item strip, and item area route to the tray staging path consistently.
- Added explicit horizontal and vertical adjustments to the tray `ScrolledWindow` so GTK's drag/autoscroll path is not handed a null adjustment while dragging over the tray.

Why:
- A single drop target on the `Revealer` was too indirect; child widgets could become the effective pointer target and miss the tray drop.
- The repeated `gtk_adjustment_*` criticals indicated GTK was trying to inspect or update a missing adjustment during drag motion.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with the existing 3 warnings (`short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan` are unused public/future APIs).
- `cargo test holding_tray` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; documented tray drag-in behavior is unchanged.

Known gaps:
- Live desktop validation is still needed to confirm this removes the intermittent drop misses and GTK adjustment criticals on a real display session.

## 2026-05-17 — GTK CSS parser warning cleanup

Removed web-only CSS from bundled themes after live `cargo run` reported GTK parser warnings.

Changed:
- Removed unsupported `pointer-events: none` from `.card-exit` in `themes/default.css`.
- Removed unsupported `@media (prefers-reduced-motion: reduce)` blocks from `themes/default.css` and `themes/high-contrast.css`.

Why:
- GTK CSS does not support those web CSS features, so the app launched with theme parser warnings even though Rust compilation succeeded.

Checks run:
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded with the existing 3 warnings (`short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan` are unused public/future APIs).
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless. No theme parser warnings appeared before the display failure.

README reviewed:
- No README update needed; this only removes invalid CSS declarations and does not change documented behavior.

Known gaps:
- Live desktop validation should confirm the CSS parser warnings are gone on a real display session.

## 2026-05-17 — Holding Tray audit, drag-in, and keyboard polish

Audited the Holding Tray against the current feedback and tightened it into a clearer staging surface.

Changed:
- Added a tray add-selection button so selected grid items can be staged directly from the tray header.
- Added drag-and-drop into the Holding Tray from grid/list drag sources. Drops stage local files/folders only; they do not move/copy files on disk.
- Added tray item selection, visible selected/focused styling, full-path item tooltips with open hints, and safer status messages for remove/clear actions.
- Added tray-focused keyboard support: `Delete` / `Backspace` removes selected staged items from the tray only, `Ctrl+C` copies selected staged paths, `Ctrl+V` stages the app-local file clipboard into the tray, `Escape` clears tray selection, and `Enter` opens the selected staged item.
- Clarified all tray button tooltips, including shortcuts where shortcuts exist, and kept the trash action visually distinct.
- Logged tray action receipts to the SQLite Activity Log in addition to the bottom operation panel receipts.
- Added tray drop/selection CSS for both bundled themes.
- Updated `README.md` with current add methods, shortcuts, safety behavior, receipts, and the remaining drag-out limitation.

Audit findings:
- Adding items worked through the context menu; it now also works through the tray add-selection button and drag-in.
- Tray project/tag/trash/path actions were wired and already used tray-scoped ActionPlan previews.
- Move/copy/trash tray actions left bottom-panel receipts; this pass also records tray receipt rows in the Activity Log.
- Drag/drop into the tray did not exist before this pass.
- Dragging out of the tray into folder views remains a known limitation because the existing folder drop path performs direct file operations and needs a broader ActionPlan-preview pass before tray drag-out can safely ship.
- Clipboard file integration remains app-local. `Ctrl+C` reliably copies staged paths as text; `Ctrl+V` into the focused tray stages paths from Lattice's own file clipboard.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded with the existing 3 warnings (`short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan` are unused public/future APIs).
- `cargo test holding_tray` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

Checks skipped:
- Live desktop validation was not possible because the environment cannot open a GTK display.

Known gaps:
- Dragging files out of the Holding Tray into folder views is not implemented in this pass.
- System file-manager clipboard formats are not integrated; tray copy is path text, and tray paste uses Lattice's app-local clipboard.
- Visual acceptance still needs a real desktop session to confirm drag hover, selected styling, and tooltip feel.

## 2026-05-16 — Action Planning Mode + queue panel

Replaced the intrusive modal pop-up approach with a proper Action Planning Mode. Normal file operations are now uninterrupted. When Plan Mode is toggled on, operations go to a visible queue instead of executing.

Changed:
- `src/action_plan.rs`: Added `for_rename(path, new_name)` factory method.
- `src/config.rs`: Added `"toggle_plan_mode": "Ctrl+Shift+P"` to default shortcuts.
- `src/ui/plan_queue_panel.rs` (NEW): Bottom-mounted `Revealer` panel. Shows queued `ActionPlan` items as rows with ↑/↓ reorder buttons and ✕ remove button. "Execute All" and "Clear" header buttons. Appears when plan mode is on or queue is non-empty; hides with `SlideUp` animation when empty and mode is off.
- `src/ui/toolbar.rs`: Added `plan_mode_toggle: ToggleButton` after `holding_tray_toggle`.
- `src/ui/mod.rs`: Added `pub mod plan_queue_panel;`.
- `src/ui/main_window.rs`:
  - Removed `modal_host.show_action_plan()` intercepts from `trash_selected()` and `paste_file_clipboard_into_active_pane()` — operations execute directly in normal mode.
  - Added plan mode check to `trash_selected()`, `paste_file_clipboard_into_active_pane()`, and `rename_path()` — when plan mode is on, these queue an `ActionPlan` and return early.
  - Added `PlanQueuePanel` to window layout (above ops_panel), `BrowserController` struct, and `BrowserController::new()`.
  - Added `plan_mode_active: Cell<bool>` and `action_queue: RefCell<Vec<ActionPlan>>` to `BrowserController`.
  - Added `set_plan_mode()`, `queue_plan()`, `refresh_plan_queue_panel()`, `execute_plan_queue()` methods.
  - Wired `plan_mode_toggle`, `execute_btn`, and `clear_btn` in bootstrap.
  - Added `WindowCommand::TogglePlanMode` and `"toggle_plan_mode"` to builtin_command lookup.
  - Added `plan_copy_move_items()` free function for reconstructing copy/move items from a queued plan.
- `themes/default.css` + `themes/high-contrast.css`: Added `.plan-queue*`, `.pq-reorder-btn`, `.pq-remove-btn`, `.toolbar-plan-btn:checked` rules in violet accent.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — 3 warnings (public API methods for future use: `short_file_list`, `confirm_label`, `clear_activity_log`, `show_action_plan`). Zero errors.
- `cargo run` — not run; headless environment.

Known gaps:
- `execute_plan_queue()` starts all operations simultaneously rather than sequentially. A sequential executor (wait for each to finish before starting the next) is a future enhancement.
- New-folder and send-to-project are not yet queue-aware (deferred).

## 2026-05-16 — Action Plan + Activity Log foundation

Added two interlocking systems that give every meaningful file operation a preview before execution and a receipt afterward.

### New files
- `src/action_plan.rs` — `FileOpPlan` / `ActionPlan` struct with factory methods `for_paste()` and `for_trash()`. Checks destination for conflicts via `PathBuf::exists()`. Includes `OpKind`, `WarnLevel`, `short_file_list()`, and `confirm_label()`. Aliased as `FileOpPlan` in main_window.rs to avoid collision with the existing local `ActionPlan` struct.
- `src/ui/activity_log_panel.rs` — `ActivityLogPanel` widget: scrollable list of `ActivityLogEntry` rows with op-icon, friendly summary, relative timestamp, and success/fail dot. `format_relative_time()` produces "just now", "5 min ago", "2 hours ago", "Yesterday", or "N days ago".

### Modified files
- `src/metadata.rs`: Schema bumped to v3. Added `activity_log` SQLite table. Added `ActivityLogEntry` struct, `log_activity()`, `list_recent_activity()`, and `clear_activity_log()` methods. Additive migration — existing databases at v2 get the new table without losing data.
- `src/ui/sidebar.rs`: Added `SidebarTarget::ActivityLog` variant and `activity_log_button` to the TOOLS section.
- `src/ui/modal_host.rs`: Added `show_action_plan()` — a typed modal that shows the operation summary, up to 8 file names, and a conflict warning if applicable.
- `src/ui/main_window.rs`:
  - Added `PaneView::ActivityLog` variant with full match coverage.
  - Added `ActivityLogPanel` to `PaneWidgets`; shown/hidden in `update_view_strip()` swapping with `file_grid`.
  - Added `open_activity_log()` and `load_activity_log_view()`.
  - Wired sidebar button via `connect_sidebar()`.
  - `trash_selected()`: shows `FileOpPlan::for_trash()` modal before executing.
  - `paste_file_clipboard_into_active_pane()`: shows `FileOpPlan::for_paste()` modal before spawning `handle_dnd_drop`. DnD drops are NOT intercepted.
  - `run_trash_op()` completion: logs to `activity_log` table with summary, source path, and errors.
  - `run_copy_move_batch()` completion: logs copy/move receipt.
- `themes/default.css` and `themes/high-contrast.css`: Added `.activity-log-*` and `.ap-*` CSS rules.
- `src/main.rs`: `mod action_plan;`
- `src/ui/mod.rs`: `mod activity_log_panel;`

### Why
All file operations previously executed with no summary and left no receipt. This pass adds an upfront preview for the two most common destructive operations (trash and paste/copy-move) and a SQLite-backed Activity Log view accessible from the sidebar.

### Checks run
- `cargo fmt` — succeeded.
- `cargo check` — 1 warning (`clear_activity_log` is a public API not yet called; intentionally kept for future use). Zero errors.
- `cargo run` — not run; headless environment. Live validation on a display is required.

### Known gaps
- Rename, new-folder, and send-to-project are not yet previewed or logged (deferred to a future pass per scope boundaries).
- Tab state persistence does not restore `ActivityLog` across sessions (treated as transient, like Trash).
- Conflict warning in the ActionPlan modal shows filenames as comma-separated text; a future pass could render them as individual lines.

## 2026-05-17 — Holding Tray first pass

Added a temporary, hideable Holding Tray for staging files and folders from multiple locations.

Changed:
- Added a bottom `HoldingTray` panel with a toolbar toggle, slide-up `Revealer` animation, compact staged-item strip, remove buttons, clear action, path tooltips, and async media thumbnails where available.
- Added `Add to Holding Tray` to default file/folder context menus and the generated config examples.
- Added tray batch actions for Move to Project, Copy to Project, Tag, Move to Trash, and Copy Path.
- Added tray-scoped ActionPlan preview modals before tray actions execute.
- Extended the existing operation panel with dismissible Activity Log receipt rows for completed tray actions.
- Updated project transfer and trash batches so successful move/trash tray items are removed while failed items remain staged.
- Updated `README.md` with Holding Tray behavior, temporary/session-only scope, context menu ID, previews, and receipts.

Why:
- Project work often involves collecting related files from multiple folders before deciding whether to copy, move, tag, trash, or reference them.
- The tray provides that staging area without becoming a real folder or persistent library.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test ui::main_window::tests::holding_tray_deduplicates_items_by_path ui::main_window::tests::tray_action_plan_summarizes_long_path_lists` — failed because `cargo test` accepts only one test-name filter before `--`; reran the focused tests separately.
- `cargo test holding_tray` — succeeded.
- `cargo test tray_action_plan` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

Checks skipped:
- Live desktop validation was not possible because the environment cannot open a GTK display.

Known gaps:
- Visual acceptance still needs a real desktop session to confirm tray height, slide animation, thumbnail appearance, and receipt placement.
- The tray intentionally does not persist across app restarts.
- Drag-and-drop into the tray remains out of scope for this first pass.

## 2026-05-16 — Split-pane visual separators

Added a subtle visual divider between visible split panes.

Changed:
- Added a thin left border and soft inset glow to `.browser-pane-secondary` and `.browser-pane-tertiary` in both bundled themes.
- Kept the divider theme-only and scoped to additional panes so single-pane mode remains unchanged.

Why:
- The stable pane row had no visual separation between panels, making two- and three-panel layouts harder to scan.

Checks run:
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; this is visual polish only.

Known gaps:
- Live desktop validation is still needed to judge the exact separator contrast.

## 2026-05-16 — Split button icon and stable three-pane row

Reworked the split button and three-panel layout after live use showed the custom glyph was confusing and the third panel still did not appear reliably.

Changed:
- Replaced the custom three-segment split button glyph with clean symbolic icons that match the rest of the toolbar.
- Made the split button icon represent the current state: single panel, two panels, or three panels. The tooltip now names the current state and the next click target.
- Replaced the split-pane center area with a stable horizontal pane row where primary, secondary, and tertiary panes are parented once and shown/hidden by layout state.
- Removed the remaining nested `GtkPaned` child swapping path so GTK no longer has to reparent pane widgets when cycling layouts.
- Removed now-unused split glyph CSS from both bundled themes.

Why:
- The split button should read like the rest of the toolbar, and its icon should describe the current layout rather than the next action.
- Keeping all pane widgets in a stable parent avoids the reparenting/allocation edge cases that were preventing the third pane from working reliably.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test launch::tests` — succeeded.
- `cargo test ui::main_window::tests::pane_layout_cycles_and_reports_visible_slots` — succeeded.
- `cargo test ui::main_window::tests::resolve_launch_three_pane_split_preserves_valid_sides_and_falls_back_invalid_side` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.
- `cargo test` — failed only in `metadata::tests::recent_locations_are_capped_and_sorted`; this remains the known unrelated metadata ordering failure.

README reviewed:
- No README update needed; user-facing split behavior is unchanged from the documented two-/three-panel cycle.

Known gaps:
- Live desktop validation is still needed to confirm the stable row sizing and exact icon-theme appearance.
- The center split row no longer exposes draggable `GtkPaned` handles between browser panes; this trades resizing for reliable three-panel visibility and can be revisited after the core behavior is confirmed.
- Existing metadata recent-location test failure remains separate follow-up work.

## 2026-05-16 — Path entry autocomplete

Added local filesystem autocomplete to the toolbar path entry.

Changed:
- Added a GTK/GIO path completion adapter in [`src/ui/main_window.rs`](src/ui/main_window.rs) using `gio::FilenameCompleter` with `gtk::EntryCompletion`.
- Path suggestions now support absolute paths, `~` / `~/...`, and paths relative to the active pane's current folder.
- `Tab` accepts the first completion, `Right Arrow` accepts it when the cursor is at the end, `Up/Down` can move through the native completion popup, and `Enter` keeps using the existing open/navigate behavior.
- Added helper tests for completion query normalization and display-prefix preservation.
- Updated the path-entry placeholder/tooltip in [`src/ui/toolbar.rs`](src/ui/toolbar.rs).
- Updated [`README.md`](README.md) to document path autocomplete.

Why:
- The path box should feel intelligent without turning into a separate fuzzy/global search feature or replacing the existing `Ctrl+F` search strip.
- Native GTK inline/popup completion provides the greyed/selected suggestion behavior requested while preserving the existing path navigation flow.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test ui::main_window::tests::path_completion_query_supports_home_absolute_and_relative_inputs` — succeeded.
- `cargo test ui::main_window::tests::path_completion_display_preserves_user_facing_prefix_style` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated the implemented feature summary for the path bar autocomplete behavior.

Known gaps:
- Live desktop validation is still needed for the exact inline selection color, popup feel, and arrow-key behavior because this environment cannot open a GTK display.
- `gtk::EntryCompletion` and `gtk::ListStore` are deprecated in GTK 4.10 but still available in this gtk4-rs version; the allowance is scoped to the path-completion adapter only.
- The worktree still contains unrelated pending edits from earlier sessions, including three-pane split, animation, thumbnail, sidebar, docs, and theme changes.

## 2026-05-16 — Three-panel split layout and button glyph fix

Fixed the initial three-panel split implementation after the third pane did not behave reliably in the live UI.

Changed:
- Replaced the hidden `Revealer`-inside-`Paned` split layout with explicit `GtkPaned` child swapping for single-, two-, and three-panel layouts.
- Kept two-panel mode as primary + secondary, and three-panel mode as primary + nested secondary/tertiary panes, so the third pane gets a real allocation instead of depending on a hidden revealer.
- Replaced the split button's stock icon swapping with a compact three-segment pane glyph; the lit segments show the target layout for the next click: 2, 3, or 1 panel.
- Added theme rules for the split glyph in both bundled themes.

Why:
- GTK `Paned` allocation with nested hidden `Revealer` children was too fragile for three visible browser panes.
- The previous stock icons were ambiguous; a small pane-count glyph communicates the split cycle more directly.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded with deprecation warnings in existing GTK path-completion code.
- `cargo test launch::tests` — succeeded.
- `cargo test ui::main_window::tests::pane_layout_cycles_and_reports_visible_slots` — succeeded.
- `cargo test ui::main_window::tests::resolve_launch_three_pane_split_preserves_valid_sides_and_falls_back_invalid_side` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.
- `cargo test` — failed only in `metadata::tests::recent_locations_are_capped_and_sorted`; this is the same known unrelated metadata ordering failure.

README reviewed:
- No README update needed; documented split behavior did not change.

Known gaps:
- Live desktop validation is still needed to confirm the final splitter feel and pane-count glyph appearance.
- Existing deprecation warnings in path completion and the metadata recent-location test failure remain separate follow-up work.

## 2026-05-16 — Three-state split pane cycle

Implemented a three-state split button and three-pane browser layout.

Changed:
- Replaced the binary split toggle with a cyclic split action: single pane -> two panes -> three panes -> single pane.
- Added a tertiary browser pane with independent directory/view state, history, selection, loading, search/filter state, drag/drop handling, thumbnail loading, and active-pane routing.
- Updated tab state so each tab remembers whether it is in one-, two-, or three-pane layout.
- Updated `--split` parsing to accept either two or three folder paths; invalid split paths still fall back to Home with a status notice.
- Added dynamic split-button icon and tooltip text for the next split action.
- Added tertiary-pane CSS accents in both bundled themes.
- Updated `README.md`, `docs/roadmap.md`, `docs/theming.md`, `docs/labwc.md`, and `docs/product_brief.md` for the three-panel split behavior.

Why:
- The split control should be a compact mouse-first layout cycle instead of a binary two-pane toggle.
- Three panels are useful for side-by-side file routing without adding another toolbar control or keyboard-first workflow.

Checks run:
- `cargo fmt` — succeeded.
- `cargo fmt --check` — succeeded.
- `cargo check` — succeeded.
- `cargo test launch::tests` — succeeded.
- `cargo test ui::main_window::tests::pane_layout_cycles_and_reports_visible_slots` — succeeded.
- `cargo test ui::main_window::tests::window_shortcuts_dispatch_standard_commands` — succeeded.
- `cargo test ui::main_window::tests::resolve_launch_three_pane_split_preserves_valid_sides_and_falls_back_invalid_side` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.
- `cargo test` — failed only in `metadata::tests::recent_locations_are_capped_and_sorted`; the failure is unrelated to the split-pane changes and also fails when run by itself.

README reviewed:
- Updated the implemented feature summary and CLI examples for three-panel split mode.

Known gaps:
- Live desktop validation is still needed for the actual three-pane divider sizing, reveal animation, and dynamic icon availability on the target icon theme.
- The existing metadata recent-location ordering test needs a separate fix.

## 2026-05-16 — Fix preview pane minimum-size regression

Restored `center_and_preview.set_shrink_end_child(false)` and `outer.set_shrink_start_child(false)` that the animation pass had changed to `true`. The original `false` values protect the preview pane and sidebar from being squeezed below their minimum widths. These flags no longer need to be `true` because space is now freed via deferred `set_visible(false)` on the Revealer — not by relying on the Revealer collapsing inside the Paned.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded, zero warnings.

## 2026-05-16 — Fix Revealer+Paned layout regressions

Fixed two regressions introduced by the animation pass: panels not freeing their Paned space when hidden, and the primary file navigator starting at half-width.

Changed:
- `src/ui/main_window.rs`: Secondary pane revealer now starts with `visible = false` so the Paned allocates all space to the primary browser by default.
- `src/ui/main_window.rs`: `apply_sidebar_visibility()` and `apply_preview_visibility()` now call `set_visible(true)` before revealing, and schedule `set_visible(false)` 230ms after hiding (after the 220ms animation completes) so the Paned divider snaps to the correct position once the content has animated away. A `reveals_child()` guard prevents the deferred hide from firing if the user re-opens the panel before it fires.
- `src/ui/main_window.rs`: `sync_split_visibility()` uses the same deferred-hide pattern for the secondary pane revealer.

Why:
- GTK4 Paned allocates space based on child widget visibility (`visible` property), not on the Revealer's `reveal_child` state. A Revealer that is `visible = true` but `reveal_child = false` still causes the Paned to hold space for it — the file navigator never expanded when panels were hidden.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded, zero warnings.
- `cargo run` — not run; headless environment.

README reviewed:
- No update needed; this is a bug fix for the animation implementation.

Known gaps:
- When SHOWING a sidebar/preview pane, the Paned space appears before the content animates in (a brief snap). This is unavoidable without full programmatic Paned position animation. It is less jarring than having the space never clear.

## 2026-05-16 — Panel and navigation animations

Added lightweight animations across all major toggle points and folder navigation.

Changed:
- `themes/default.css`: Added `@keyframes lattice-card-out` (80ms upward fade-out, `.card-exit` class), `@keyframes lattice-op-in` (140ms slide-in for op rows, `.op-row-enter` class), and `@media (prefers-reduced-motion)` override for all three animation classes.
- `themes/high-contrast.css`: Added matching `prefers-reduced-motion` block.
- `src/ui/ops_panel.rs`: Wrapped the ops panel GtkBox in a `gtk::Revealer` (`SlideUp`, 200ms). `root` is now the Revealer. Added `.op-row-enter` class to each new op row for per-row entrance animation.
- `src/ui/toolbar.rs`: Set `path_stack` transition to `Crossfade` at 120ms so switching between breadcrumbs and path-entry modes fades smoothly.
- `src/ui/file_grid.rs`: `set_loading()` now adds `.card-exit` to all existing cards and defers the actual clear by 80ms via `glib::timeout_add_local_once`. Added `cancel_exit_timer()` so rapid navigation or `set_items()`/`set_empty_message()` cancels any pending exit animation cleanly. Added `exit_timer: Rc<RefCell<Option<SourceId>>>` field.
- `src/ui/main_window.rs`: Wrapped sidebar, preview pane, search strip, tag filter panel, and secondary split pane each in a `gtk::Revealer`. Sidebar uses `SlideRight` (220ms), preview uses `SlideLeft` (220ms), search strip uses `SlideDown` (180ms), tag filter uses `Crossfade` (180ms), secondary split pane uses `SlideLeft` (220ms). Changed all `set_visible()` panel calls to `set_reveal_child()`. Changed `tag_filter.root.is_visible()` checks to `tag_filter_revealer.reveals_child()`. Added revealer fields to `PaneWidgets`, `BrowserController`, `BodyLayout`, and `CenterLayout`. Changed `set_shrink_start_child(false)` to `true` in the outer Paned (sidebar) and `set_shrink_end_child(false)` to `true` in the center Paned (preview) so Revealers can animate to zero width.

Why:
- All panels previously snapped in/out instantly via `set_visible()`. Revealers give them a sense of physical motion matching the folder cascade already in place.
- The folder exit animation completes the cascade: old cards sweep up and out before new ones sweep in.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded, zero warnings.
- `cargo run` — not run; environment is headless. Visual verification requires a display session.

README reviewed:
- No README update needed; this is a polish/animation pass with no behavioral or feature changes.

Known gaps:
- Revealer behavior inside GTK4 Paned widgets may differ from a pure GtkBox layout — specifically how the Paned divider responds as the sidebar/preview Revealers animate. Live testing on a display is required to confirm the slide direction and timing feel correct.
- `prefers-reduced-motion` disables the CSS keyframe animations but GTK Revealer transitions are not governed by CSS media queries; the Revealer durations (≤ 220ms) are short enough to be acceptable.

## 2026-05-16 — FlowBox drag-hover outline suppression

Addressed the persistent faint second outline around folder drop targets by suppressing the outer `FlowBoxChild` hover/selected/focus paint during drag hover.

Changed:
- Added the `drop-hover` CSS class to the folder card's outer `FlowBoxChild` in [`src/ui/main_window.rs`](src/ui/main_window.rs), alongside the existing class on the inner card shell.
- Added `flowboxchild` hover, focus, selected, and drop-hover reset rules in [`themes/default.css`](themes/default.css) and [`themes/high-contrast.css`](themes/high-contrast.css).
- Kept the visible drop target on the inner file card only, using a single inset outline.

Why:
- The remaining double-outline artifact was likely the GTK `FlowBoxChild` container rendering its own hover/selected/focus state behind the styled card.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; this is a visual defect fix for existing drag/drop behavior.

Known gaps:
- Live desktop validation is still needed because the local environment cannot open a GTK display.

## 2026-05-16 — Final drag-hover cascade fix

Removed the remaining double-outline effect reported when dragging over folders.

Changed:
- Added a final drop-hover override in [`themes/default.css`](themes/default.css) after the later per-file-type hover glow rules, so normal hover glows cannot reappear underneath the drop target state.
- Changed folder drop targets in [`themes/default.css`](themes/default.css) and [`themes/high-contrast.css`](themes/high-contrast.css) from border plus external glow to a single inset target outline with normal depth shadow.

Why:
- The default theme's late per-type hover rules could still win during drag hover and create a faint offset outline behind the brighter drop target.
- A single inset outline is clearer and avoids the "two boxes" look during precise drag/drop targeting.

Checks run:
- `cargo check` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; this was a visual defect fix for the documented drag/drop behavior.

Known gaps:
- Live desktop validation is still needed because the local environment cannot open a GTK display.

## 2026-05-16 — Drag-hover outline cleanup

Refined the drag/drop hover styling after folder drop targets showed an ugly stacked outline while dragging over other folders.

Changed:
- Removed the redundant direct `.file-card.drop-hover` styling path from [`themes/default.css`](themes/default.css).
- Made folder drop-hover rules explicitly override selected and normal hover states in both [`themes/default.css`](themes/default.css) and [`themes/high-contrast.css`](themes/high-contrast.css).
- Replaced the stacked inner-outline plus outer-glow effect with one border and one softer glow for folder drop targets.

Why:
- During drag hover, selected-card and drop-target shadows could visually stack into a double outline. Drop targets should read as one deliberate target state.

Checks run:
- `cargo check` — succeeded.
- `git diff --check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- No README update needed; this was a visual refinement of the already documented drag/drop behavior.

Known gaps:
- Live desktop validation is still needed to confirm the exact hover appearance on a real GTK display.

## 2026-05-16 — Drag-and-drop visual polish

Added a custom visual drag/drop treatment for file moves and copies.

Changed:
- Added GTK drag-begin / drag-end handling in [`src/ui/main_window.rs`](src/ui/main_window.rs) so dragged file cards get a lifted source state and use a custom `GtkDragIcon` child instead of GTK's default text-style drag ghost.
- Added `build_drag_preview()` to render a compact themed drag card with the file-type icon, item name or selected-count summary, and file-type/detail text.
- Added themed drag preview, dragging, folder-drop, pane-drop, and sidebar-drop styling to [`themes/default.css`](themes/default.css) and [`themes/high-contrast.css`](themes/high-contrast.css).
- Updated [`README.md`](README.md), [`docs/roadmap.md`](docs/roadmap.md), and [`docs/product_brief.md`](docs/product_brief.md) so drag/drop is described as implemented with polished visual feedback rather than stale "not implemented" status.

Why:
- Dragging files should feel like moving real visual objects in the grid, not like dragging a raw filename label.
- Drop targets need strong, modern visual affordances for folder cards, pane backgrounds, and key sidebar destinations while keeping the existing GIO-backed file operation behavior unchanged.

Checks run:
- `cargo fmt` — succeeded.
- `cargo check` — succeeded.
- `timeout 3 cargo run` — built and launched, then stopped with `Gtk-WARNING **: Failed to open display` because this environment is headless.

README reviewed:
- Updated the implemented feature list to mention the custom visual drag card and highlighted drop targets.

Known gaps:
- Live desktop validation is still needed for the exact drag-card feel, cursor hotspot, and hover timing because this environment cannot open a GTK display.
- The worktree also contains unrelated pending edits in `src/thumbnail.rs`, `src/ui/file_grid.rs`, `src/ui/sidebar.rs`, and pre-existing portions of `themes/default.css`; they were left intact.

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
