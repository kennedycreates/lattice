# Lattice

A mouse-first GTK4 file manager for custom Linux desktops, built with Rust, GTK4, GIO, and GLib.

Lattice is aimed at a slick, dark, cursor-driven workflow. The default view is an icon grid, not a keyboard-centric file list. It integrates naturally with custom Wayland compositors like [labwc](https://github.com/labwc/labwc).

## Current Status

Lattice is currently in **Milestone 6: Polish, Theme System, and labwc Integration**.

Core M6 features implemented:
- CLI flags for launch modes (`--path`, `--downloads`, `--project`, `--split`)
- TOML config at `~/.config/lattice/config.toml` (theme selection)
- Configurable shortcuts, context menus, and custom safe-argv actions
- Theming system with user and bundled theme paths
- Two bundled themes: `default` (Victorian Gothic dark) and `high-contrast` (maximum contrast dark)
- Desktop entry (`lattice.desktop`) for application launchers and default folder-opener
- labwc integration guide and keybinding examples
- Emoji file-type icons in the grid and preview pane
- Visual polish across the dense floating-icon grid, preview pane, tabs, status bar, and scrollbars

Implemented now:

- Real local folder browsing through GIO async APIs
- Home / Downloads / Documents / System Drives / Recent / Trash / Projects sidebar navigation
- Folder-first alphabetical sorting
- Hidden-file toggle
- Double-click to open folders and files
- Back / Up / Refresh navigation
- Compact icon-only top toolbar with standard file actions and hover tooltips
- Configurable keyboard shortcuts, right-click menu order, and custom context actions
- Breadcrumb-style location bar that flips into full-path editing on click
- Manual sidebar toggle and preview toggle to reclaim browser space during the session
- Toggleable preview pane with real folder, image, and text/config previews
- Right-click context menus on file and folder cards
- New Folder, New Text Document, Rename, Move to Trash, Copy Path, and Open Terminal Here
- Standard desktop keyboard support as secondary input: copy/cut/paste, copy path, new folder, new text document, rename, trash, search, path focus, refresh, hidden files, sidebar, preview, tabs, split view, back/up, pane switching, and grid navigation
- Real tabs with per-tab folder state
- Split-pane browsing with an active pane model
- Drag and drop within and between panes, plus drops onto key sidebar destinations
- App-local SQLite metadata store for projects and tags
- Pin folder as Project and reopen pinned projects from the sidebar
- Send to Project flow with copy/move choice and overwrite conflict prompts
- Tag creation and assignment from the context menu
- Compact tag chips rendered on icon items
- Tag-filtered sidebar views
- Downloads Triage mode with category/time filters for messy Downloads folders
- Trash view backed by `trash:///` with basic restore when the original path is available
- System Drives view backed by mounted GIO/GVfs volumes
- Recent view backed by app-local recent folder history

Not implemented yet:

- Video thumbnails / richer media handling
- Global/indexed search beyond the current folder scope
- Cross-device tag sync or xattrs
- Drag/drop project routing
- Undo-style trash recovery flows beyond basic Restore from Trash

## Requirements

- Rust stable `1.75+`
- GTK4 development libraries
- Build support for the bundled SQLite dependency used by `rusqlite`
- A Linux desktop session with working default app handlers

For the best experience, use a desktop environment or compositor with:

- default app associations for opening files
- a trash implementation
- an installed terminal emulator

### Installing GTK4 on Debian/Ubuntu

```sh
sudo apt install libgtk-4-dev
```

### Installing GTK4 on Arch Linux

```sh
sudo pacman -S gtk4
```

### Installing GTK4 on Fedora

```sh
sudo dnf install gtk4-devel
```

## Build & Run

```sh
cargo run
```

Running from the project root is recommended during development so the CSS themes in `themes/` are found automatically.

## CLI Launch Modes

```sh
lattice                          # open home directory
lattice --path /some/folder      # open a specific folder
lattice /some/folder             # same (positional shorthand)
lattice --downloads              # open Downloads Triage view
lattice --project "My Project"   # open a pinned project's root folder
lattice --split ~/Downloads ~/Documents  # split view with two paths
```

Invalid launch paths and unknown project names fall back to Home with a status
message instead of opening a broken startup view.

## Themes

Lattice reads `~/.config/lattice/config.toml` for the theme name:

```toml
theme = "default"       # Victorian Gothic dark (default)
# theme = "high-contrast"  # maximum contrast dark
```

Place custom `.css` files in `~/.config/lattice/themes/` and reference them by filename.
See [docs/theming.md](docs/theming.md) for the full CSS class reference.

## Configurable Actions

On first run, Lattice creates `~/.config/lattice/config.toml` with commented examples.
Custom actions use an `argv` array and never run through a shell. `{paths}` expands selected paths as separate arguments, `{path}` expands the first selected path, and `{cwd}` expands the active folder.

```toml
[shortcuts]
custom.open_in_gimp = "Ctrl+Alt+G"
custom.compress_here = "Ctrl+Alt+Z"

[context_menu]
file = ["open", "open_with", "separator", "custom.open_in_gimp", "rename", "copy_path", "terminal_here", "separator", "move_to_trash"]
folder = ["open", "open_new_tab", "open_in_pane", "separator", "custom.compress_here", "rename", "copy_path", "terminal_here", "separator", "move_to_trash"]
background = ["new_folder", "new_text_document", "separator", "pin_project", "terminal_here", "copy_path"]

[[custom_actions]]
id = "open_in_gimp"
label = "Open in GIMP"
argv = ["gimp", "{paths}"]
contexts = ["file"]
needs_selection = true

[[custom_actions]]
id = "compress_here"
label = "Compress Here"
argv = ["file-roller", "--add", "{paths}"]
contexts = ["file", "folder"]
needs_selection = true
```

Built-in shortcut IDs include `new_folder`, `new_text_document`, `rename`, `trash`, `search`, `focus_path`, `refresh`, `show_hidden`, `toggle_sidebar`, `toggle_preview`, `new_tab`, `close_tab`, `toggle_split`, `back`, `up`, and `cycle_pane`.

## Install

```sh
cargo build --release
sudo install -m 755 target/release/lattice /usr/local/bin/lattice
sudo install -m 644 lattice.desktop /usr/local/share/applications/lattice.desktop
```

To set Lattice as the default folder opener:

```sh
xdg-mime default lattice.desktop inode/directory
```

See [docs/labwc.md](docs/labwc.md) for labwc keybindings and Waybar launcher examples.

## Development Checks

```sh
cargo fmt --check
cargo check
```

## Project Workflow

- Read `AGENTS.md` and `JOURNAL.md` before starting work.
- Record each meaningful work session in `JOURNAL.md`.
- Keep `README.md` updated after each major step so it stays accurate and low-chaff.
- Follow `docs/agent_rules.md` and `docs/roadmap.md` when making changes.

## GitHub CI

The repo includes a minimal GitHub Actions workflow at `.github/workflows/ci.yml` that installs GTK4 development packages on Ubuntu and runs:

- `cargo fmt --check`
- `cargo check`

## Project Structure

```text
lattice/
  Cargo.toml
  Cargo.lock
  README.md
  AGENTS.md
  JOURNAL.md
  .github/
    workflows/
      ci.yml
  src/
    main.rs            — entry point, GTK application setup
    app.rs             — activation handler, CSS loading
    config.rs          — config loading, shortcut/menu/custom action schema
    launch.rs          — CLI launch-mode parsing
    metadata.rs        — SQLite metadata store and schema init
    ui/
      mod.rs
      main_window.rs   — main window controller and file actions
      toolbar.rs       — top toolbar buttons and path display
      sidebar.rs       — left navigation sidebar
      file_grid.rs     — central icon-grid view
      preview_pane.rs  — right preview panel
      status_bar.rs    — bottom status/status-message bar
  themes/
    default.css        — Victorian Gothic dark CSS theme
    high-contrast.css  — high-contrast dark CSS theme
  lattice.desktop      — desktop entry for application launchers
  docs/
    product_brief.md
    roadmap.md
    agent_rules.md
    metadata.md
    theming.md         — CSS class reference and theme authoring guide
    labwc.md           — labwc keybindings, install, Waybar snippet
```

## Milestone 5 Workflows

Projects:
- Pin the current folder or a folder card as a Project.
- Pinned projects appear in the sidebar as first-class workspace entries.
- Use `Send to Project` from item context menus to copy or move items into the project root.

Tags:
- Use `Add Tag` on a file or folder to create a new tag or reuse an existing one by name.
- Tagged items render tag chips directly on their file cards.
- Click a tag in the sidebar to open a filtered virtual view of all files using that tag.

Downloads Triage:
- Open `Downloads Triage` from the sidebar.
- Switch between `All`, `Today`, `This Week`, `This Month`, `Older Than 1 Month`, `Images`, `Videos`, `Archives`, `Documents`, and `Large Files`.
- Use the existing preview, rename, trash, and context-menu flows while cleaning up Downloads.

## Metadata Storage

Lattice stores Projects and Tags in a local SQLite database under the user data directory, not in the repository and not in filesystem xattrs.

See [docs/metadata.md](docs/metadata.md) for the database path, tables, and current schema scope.

Current limitation:
- Tags are still keyed by full path. Renames and project moves performed inside Lattice update those paths, but external filesystem moves outside Lattice can still break tag associations.
- Recent folders are app-local to Lattice. This is a recent-folder history, not a cross-app recent-files index.

## Roadmap

See [docs/roadmap.md](docs/roadmap.md) for the milestone plan and current status.
