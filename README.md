# Lattice

Lattice is a mouse-first GTK4 file manager for Linux, built with Rust. It is designed for polished custom desktops such as labwc, GNOME, COSMIC, Sway, and other GTK-capable environments.

## Features

**Navigation**
- Icon grid with folder-first sorting, per-pane hidden-file toggle, and icon/list view switch
- Tabbed browsing with two- and three-panel split view
- Back, Up, Refresh, and a breadcrumb location bar with inline path editing and autocomplete
- Sidebar: Home, user-pinned Places, System Drives, Recent, Trash, Search, Space Viewer, Triage, Activity Log, Tags, and Projects

**File Operations**
- Copy, move, rename, new folder, new text document, and move to trash
- Batched conflict resolver before copy/move operations begin, with Keep Both, Replace, and Skip choices
- Drag and drop within and between panes, and onto sidebar targets
- Holding Tray for staging files from multiple locations before batch actions

**Workspace**
- Projects: user-defined named collections with a color; manage them in the Project Manager panel and pin folders to each project landing page
- Tags: create, rename, recolor, delete, and filter folder views by tag
- Space Viewer: disk usage analysis for the active folder — total size, file/folder counts, pie chart by file type, ranked largest files and subfolders, three scan depths (folder only, one level deep, full recursive with cancellation), and row actions (open, reveal, add to Holding Tray, copy path, move to trash)
- Downloads Triage: Images, Videos, Archives, Documents, Large Files, Today, This Week, This Month, and Older filters
- Folder search with name, kind, date, size, and tag filters; current-folder or recursive scope

**Preview and History**
- Preview pane for images, text/config files, folders, and media metadata
- Activity Log with SQLite-backed receipts plus guarded undo, repeat, reveal, and copy-path actions per row

**Configuration**
- TOML config at `~/.config/lattice/config.toml`
- Bundled `default` and `high-contrast` themes
- User themes in `~/.config/lattice/themes/`

## Requirements

- Linux with a GTK4-capable desktop session
- Rust stable `1.75+`
- GTK4 development libraries
- `update-desktop-database` from `desktop-file-utils` for launcher database refresh
- `gtk-update-icon-cache` for icon cache refresh

Install common build/runtime dependencies:

```sh
# Debian / Ubuntu
sudo apt install cargo libgtk-4-dev desktop-file-utils

# Fedora
sudo dnf install cargo gtk4-devel desktop-file-utils gtk-update-icon-cache

# Arch Linux
sudo pacman -S rust gtk4 desktop-file-utils
```

## Build From Source

```sh
git clone <repository-url> lattice
cd lattice
cargo build --release
```

The release binary is written to `target/release/lattice`.

## Install

### Per-User Install

Use this when installing for the current account only:

```sh
mkdir -p ~/.local/bin
install -m 755 target/release/lattice ~/.local/bin/lattice

install -Dm 644 com.lattice.filemanager.desktop \
  ~/.local/share/applications/com.lattice.filemanager.desktop

install -Dm 644 icons/lattice-icon.png \
  ~/.local/share/lattice/icons/lattice-icon.png
install -Dm 644 icons/lattice-icon.png \
  ~/.local/share/icons/hicolor/256x256/apps/lattice.png

install -Dm 644 themes/default.css \
  ~/.local/share/lattice/themes/default.css
install -Dm 644 themes/high-contrast.css \
  ~/.local/share/lattice/themes/high-contrast.css

update-desktop-database ~/.local/share/applications/
gtk-update-icon-cache ~/.local/share/icons/hicolor/
```

Make sure `~/.local/bin` is in `PATH`.

### System-Wide Install

Use this when installing for all users:

```sh
sudo install -m 755 target/release/lattice /usr/local/bin/lattice

sudo install -Dm 644 com.lattice.filemanager.desktop \
  /usr/local/share/applications/com.lattice.filemanager.desktop

sudo install -Dm 644 icons/lattice-icon.png \
  /usr/local/share/lattice/icons/lattice-icon.png
sudo install -Dm 644 icons/lattice-icon.png \
  /usr/local/share/icons/hicolor/256x256/apps/lattice.png

sudo install -Dm 644 themes/default.css \
  /usr/local/share/lattice/themes/default.css
sudo install -Dm 644 themes/high-contrast.css \
  /usr/local/share/lattice/themes/high-contrast.css

sudo update-desktop-database /usr/local/share/applications/
sudo gtk-update-icon-cache /usr/local/share/icons/hicolor/
```

### Set as Default Folder Opener

```sh
xdg-mime default com.lattice.filemanager.desktop inode/directory
xdg-mime query default inode/directory
```

Expected output:

```text
com.lattice.filemanager.desktop
```

## Run

```sh
lattice                                             # home directory
lattice --path /some/folder                         # specific folder
lattice /some/folder                                # positional shorthand
lattice --downloads                                 # Downloads Triage view
lattice --project "My Project"                      # project landing page
lattice --split ~/Downloads ~/Documents             # two-panel split
lattice --split ~/Downloads ~/Documents ~/Pictures  # three-panel split
```

If `--path` or `--project` cannot resolve to a readable folder, Lattice falls back to the home directory.

## Configuration

On first run, Lattice creates `~/.config/lattice/config.toml`.

```toml
theme = "default" # or "high-contrast"

[shortcuts]
custom.open_in_gimp = "Ctrl+Alt+G"

[context_menu]
file = ["open", "open_with", "separator", "rename", "copy_path",
        "terminal_here", "separator", "move_to_trash"]

[[custom_actions]]
id = "open_in_gimp"
label = "Open in GIMP"
argv = ["gimp", "{paths}"]
contexts = ["file"]
needs_selection = true
```

`{paths}` expands all selected paths as separate arguments, `{path}` expands the first selected path, and `{cwd}` expands the active folder.

Theme lookup order:

1. `~/.config/lattice/themes/<name>.css`
2. `themes/<name>.css` relative to the current working directory
3. `themes/<name>.css` beside the source tree when running a Cargo build
4. `<prefix>/share/lattice/themes/<name>.css` for installed builds

See [docs/theming.md](docs/theming.md) for the full CSS class reference and theme authoring guide.

## Desktop Integration

See [docs/desktop-setup.md](docs/desktop-setup.md) for labwc keybindings, Waybar launcher config, GNOME, COSMIC, and Sway setup.

The desktop entry ID is `com.lattice.filemanager.desktop`. The application icon name is `lattice`.

## Uninstall

Per-user install:

```sh
rm -f ~/.local/bin/lattice
rm -f ~/.local/share/applications/com.lattice.filemanager.desktop
rm -f ~/.local/share/icons/hicolor/256x256/apps/lattice.png
rm -rf ~/.local/share/lattice
update-desktop-database ~/.local/share/applications/
gtk-update-icon-cache ~/.local/share/icons/hicolor/
```

System-wide install:

```sh
sudo rm -f /usr/local/bin/lattice
sudo rm -f /usr/local/share/applications/com.lattice.filemanager.desktop
sudo rm -f /usr/local/share/icons/hicolor/256x256/apps/lattice.png
sudo rm -rf /usr/local/share/lattice
sudo update-desktop-database /usr/local/share/applications/
sudo gtk-update-icon-cache /usr/local/share/icons/hicolor/
```

User data is not removed by those commands. Lattice stores config in `~/.config/lattice/` and metadata in `~/.local/share/lattice/metadata.db`.

## Project Structure

```text
lattice/
  Cargo.toml
  Cargo.lock
  LICENSE
  README.md
  com.lattice.filemanager.desktop
  icons/
    lattice-icon.png
  src/
    main.rs
    app.rs
    config.rs
    launch.rs
    metadata.rs
    action_plan.rs
    thumbnail.rs
    ui/
      main_window.rs
      toolbar.rs
      sidebar.rs
      file_grid.rs
      preview_pane.rs
      status_bar.rs
      tab_strip.rs
      search_panel.rs
      tag_filter.rs
      tag_panel.rs
      project_manager_panel.rs
      project_landing_panel.rs
      space_viewer_panel.rs
      ops_panel.rs
      holding_tray.rs
      activity_log_panel.rs
      plan_queue_panel.rs
      conflict_resolver.rs
      bulk_rename.rs
      modal_host.rs
  themes/
    default.css
    high-contrast.css
  docs/
    agent_rules.md
    desktop-setup.md
    metadata.md
    modal_architecture.md
    product_brief.md
    theming.md
```

## Development

```sh
cargo fmt --check
cargo check
cargo run
```

During development, `cargo run` auto-installs the desktop entry and icon into `~/.local/share/` when they are newer than the installed copies. Manual install is still required for release-style testing of the binary, bundled themes, and system-wide paths.

The project uses GitHub Actions (`.github/workflows/ci.yml`) for `cargo fmt --check` and `cargo check` on every push.
