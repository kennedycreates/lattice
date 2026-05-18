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

Use this after `cargo build --release` when installing for all users:

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

Installed binary:

```sh
lattice
lattice ~/Documents
lattice --path ~/Documents
lattice --downloads
lattice --project "My Project"
lattice --split ~/Downloads ~/Documents
lattice --split ~/Downloads ~/Documents ~/Pictures
```

From the source tree:

```sh
cargo run -- ~/Documents
cargo run -- --downloads
cargo run -- --project "My Project"
cargo run -- --split ~/Downloads ~/Documents
```

Launch options:

| Command | Result |
| --- | --- |
| `lattice` | Open Home. |
| `lattice <folder>` | Open a folder using positional shorthand. |
| `lattice --path <folder>` | Open a specific folder. |
| `lattice --downloads` | Open Downloads Triage. |
| `lattice --project "<name>"` | Open a project landing page by project name, case-insensitive. |
| `lattice --split <left> <right>` | Open a two-pane layout. |
| `lattice --split <left> <middle> <right>` | Open a three-pane layout. |

If multiple launch modes are provided, Lattice resolves them in this order: `--split`, then `--path` or a positional folder, then `--downloads`, then `--project`. Invalid or unreadable folders fall back to Home with a status message. A missing project name opens Home with a status message.

Unknown flags are ignored. Options that take values print a terminal warning when the value is missing.

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

## Update an Existing Install

From your Lattice source repository:

```fish
cd ~/Development/lattice

git status
git pull --ff-only

cargo fmt --check
cargo check
cargo build --release

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

Updating Lattice does not remove your user config, metadata, projects, tags, receipts, or cache. Those live under:

```text
~/.config/lattice/
~/.local/share/lattice/
~/.cache/lattice/
```

Optional backup before updating:

```fish
set stamp (date +%Y%m%d-%H%M%S)

test -d ~/.config/lattice; and cp -a ~/.config/lattice ~/.config/lattice.backup.$stamp
test -d ~/.local/share/lattice; and cp -a ~/.local/share/lattice ~/.local/share/lattice.backup.$stamp
```

Verify the installed app:

```fish
which lattice
xdg-mime query default inode/directory
```

The expected default folder opener is:

```text
com.lattice.filemanager.desktop
```

If the launcher icon or app entry does not update immediately in COSMIC/GNOME, log out and back in or restart the desktop session.

`cargo run` is for development and testing. Your daily-use installed copy should come from the release binary installed to `/usr/local/bin/lattice`.

## Uninstall

```sh
sudo rm -f /usr/local/bin/lattice
sudo rm -f /usr/local/share/applications/com.lattice.filemanager.desktop
sudo rm -f /usr/local/share/icons/hicolor/256x256/apps/lattice.png
sudo rm -rf /usr/local/share/lattice
sudo update-desktop-database /usr/local/share/applications/
sudo gtk-update-icon-cache /usr/local/share/icons/hicolor/
```

User data is not removed by those commands. Lattice stores config in `~/.config/lattice/` and metadata in `~/.local/share/lattice/metadata.db`.

## Development

```sh
cargo fmt --check
cargo check
cargo run
```

During development, `cargo run` refreshes the desktop entry and icon under the current account when the source copies are newer. Use the install steps above for release-style testing of the binary, bundled themes, and system-wide paths.

The project uses GitHub Actions (`.github/workflows/ci.yml`) for `cargo fmt --check` and `cargo check` on every push.
