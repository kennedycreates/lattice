# Lattice — Journal

Chronological development journal for the active project state.

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
