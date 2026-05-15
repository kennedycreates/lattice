# Lattice — Roadmap

## Milestone 0 — Static Shell ✅

Goal: a beautiful, compilable app shell with no real functionality.

- [x] GTK4 app compiles and runs with `cargo run`
- [x] Dark cyberpunk CSS theme (`themes/default.css`)
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

- [x] Right-click context menu on file cards with Open, Open With, Rename, Copy Path, Open Terminal Here, and Move to Trash
- [x] Toolbar actions for New Folder, Rename, and Trash
- [x] Rename dialog with empty-name guardrails and name-conflict handling
- [x] New Folder creation with `New Folder`, `New Folder 2`, … fallback naming
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

- [ ] Functional tab strip — open new tabs, close tabs, switch tabs
- [ ] Split-pane view — two independent directory views side by side
- [ ] Active-pane routing for toolbar actions, preview updates, and context actions
- [ ] Manual desktop acceptance pass on a real GTK display session
- [ ] Keyboard shortcuts as secondary input (not primary)

---

## Milestone 5 — Search & Rich Media

Goal: actually useful file information.

- [ ] Video thumbnail via GStreamer or ffmpegthumbnailer
- [ ] Search bar filters the current view
- [ ] Search across subdirectories (async, cancellable)

---

## Milestone 6 — Polish & Power Features

Goal: power-user workflows and configurability.

- [ ] SQLite tags — tag files/folders, filter by tag
- [ ] Project metadata panel
- [ ] TOML config (`~/.config/lattice/config.toml`)
- [ ] Drag-and-drop within and between panes
- [ ] Trash integration (send to trash, restore)
- [ ] Undo for destructive operations
- [ ] Confirmation dialogs for delete / overwrite
- [ ] Bookmarks / pinned paths in sidebar
