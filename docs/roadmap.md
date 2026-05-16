# Lattice — Roadmap

## Milestone 0 — Static Shell ✅

Goal: a beautiful, compilable app shell with no real functionality.

- [x] GTK4 app compiles and runs with `cargo run`
- [x] Victorian Gothic dark CSS theme (`themes/default.css`)
- [x] Layout: toolbar, sidebar, tab strip, file grid, preview pane, status bar
- [x] Placeholder sidebar sections: Home, Downloads, Documents, Projects, Tags, Drives, Recent
- [x] Placeholder toolbar buttons: Back, Up, Refresh, New Folder, Split, View, Search
- [x] Placeholder file cards: Projects, Downloads, screenshot.png, edit.mp4, notes.txt, config.toml
- [x] Placeholder preview pane with selected-file metadata
- [x] Stable CSS classes on all major regions
- [x] Docs: README, product_brief, roadmap, agent_rules

---

## Milestone 1 — Real File Browsing ✅

Goal: navigate the filesystem with the mouse.

- [x] Use `gio::File` and `gio::FileEnumerator` to list directory contents
- [x] Start in the user home directory
- [x] Sidebar buttons navigate to Home, Downloads, Documents, and Projects when available
- [x] File cards reflect actual files and folders
- [x] Folders sort before files, alphabetically within each group
- [x] Hidden files are hidden by default, with a toolbar toggle to show them
- [x] Double-click a folder to enter it
- [x] Double-click a file to open it with the default associated app
- [x] Back / Up / Refresh buttons work
- [x] Toolbar path display updates with the current folder
- [x] Status bar shows current path, item count, and selected count
- [x] Preview pane updates with selected item name, type, and path
- [x] Async loading (GIO async APIs, no blocking the main thread)

---

## Milestone 2 — Core File Actions ✅

Goal: right-click menus and safe day-to-day file actions.

- [x] Right-click context menu on file cards with Open, Rename, Copy Path, Open Terminal Here, and Move to Trash
- [x] Toolbar actions for New Folder, Rename, and Trash
- [x] Rename dialog with empty-name guardrails and name-conflict handling
- [x] New Folder creation with `New Folder`, `New Folder 2`, … fallback naming
- [x] New Text Document creation from the toolbar and current-folder right-click menu, using extensionless `Untitled`, `Untitled 2`, … default naming
- [x] Move to Trash as the default destructive path
- [x] Copy Path to clipboard and Open Terminal Here support
- [x] Basic mouse selection flow: single-click selects, double-click opens

---

## Milestone 3 — Real Preview Pane ✅

Goal: make the preview pane real, useful, and toggleable.

- [x] Preview pane can be shown and hidden from the toolbar
- [x] Folder previews show name, path, type label, and modified time when available
- [x] Image previews render directly in the preview pane
- [x] Text / config / code files show safe partial text previews with a size limit
- [x] Video / audio / unknown files show lightweight metadata-only previews
- [x] Preview actions for Open, Copy Path, and Open Parent Folder

---

## Milestone 4 — Multi-View Navigation

Goal: extend the browser shell into a more capable workspace.

- [x] Functional tab strip — open new tabs, close tabs, switch tabs
- [x] Split-pane view — two independent directory views side by side
- [x] Active-pane routing for toolbar actions, preview updates, and context actions
- [ ] Manual desktop acceptance pass on a real GTK display session
- [x] Keyboard shortcuts as secondary input (not primary)

---

## Milestone 5 — Projects, Tags, and Downloads Triage

Goal: make Lattice meaningfully different from a generic folder browser.

- [x] Local SQLite metadata database in the user data directory
- [x] App-local tables for projects, tags, file tags, and project destinations
- [x] Sidebar Projects section populated from pinned folders
- [x] Pin current folder or folder card as Project
- [x] `Send to Project` with copy/move choice and conflict prompts
- [x] Sidebar Tags section populated from created tags
- [x] Create tags and apply them to files/folders
- [x] Show tag chips on file cards
- [x] Click a tag to open a virtual tagged-files view
- [x] Downloads Triage view rooted at `~/Downloads`
- [x] Downloads Triage filters for Today / This Week / This Month / Older Than 1 Month / Images / Videos / Archives / Documents / Large Files
- [ ] Manual desktop acceptance pass on a real GTK display session

---

## Milestone 6 — Polish, Theme System, and labwc Integration

Goal: fit, finish, and daily-driver readiness on custom Wayland desktops.

**Config & Themes**
- [x] TOML config at `~/.config/lattice/config.toml` (theme selection)
- [x] Configurable keyboard shortcuts, right-click menu order, and custom context actions
- [x] Theme loading from `~/.config/lattice/themes/` with bundled fallback
- [x] Bundled theme: `default` (Victorian Gothic dark)
- [x] Bundled theme: `high-contrast` (maximum contrast dark)
- [x] `docs/theming.md` — CSS class reference and theme authoring guide

**CLI Launch Modes**
- [x] `lattice --path /folder` — open a specific folder
- [x] `lattice /folder` — positional shorthand
- [x] `lattice --downloads` — open Downloads Triage view
- [x] `lattice --project "Name"` — open a pinned project's root
- [x] `lattice --split /left /right` — launch in split-pane mode

**Desktop Integration**
- [x] `lattice.desktop` for application launchers and `xdg-mime` default-opener
- [x] `docs/labwc.md` — keybindings, Waybar snippet, install instructions

**Virtual System Views**
- [x] Sidebar Trash view backed by `trash:///`
- [x] Basic `Restore from Trash` action when the original path is available
- [x] Sidebar System Drives view listing mounted local volumes from GIO VolumeMonitor
- [x] Sidebar Recent view backed by app-local recent folder history

**Visual Polish**
- [x] Emoji file-type icons replacing text badges for folders, images, video, audio, documents, text, archives, config/code, and unknown files
- [x] Gradient file cards with enhanced hover glow
- [x] Per-type radial aura glow on preview pane icon
- [x] Secondary pane uses violet accent when active in split view
- [x] Active tab underline thickened + background tint
- [x] Status bar inset accent glow (mirrors toolbar)
- [x] Scrollbar hover glow
- [x] Empty folder state: brighter, larger text

**Not yet in scope for M6**
- [ ] Drag-and-drop within and between panes
- [ ] Undo for destructive operations
- [ ] Search across folders
- [ ] Manual desktop acceptance pass on a real GTK display session
