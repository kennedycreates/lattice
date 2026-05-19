# Lattice

Lattice is a mouse-first GTK4 file manager for Linux, built with Rust. The supported distribution families are Ubuntu-based systems and Arch-based systems.

## Features

**Navigation**
- Icon grid with folder-first sorting, per-pane hidden-file toggle, per-pane Shape badge visibility, and icon/list view switch
- Tabbed browsing with two- and three-panel split view
- Back, Up, Refresh, and a breadcrumb location bar with inline path editing and autocomplete
- Sidebar: Home, user-pinned Places, System Drives, Recent, Trash, Palettes, Tints & Tags, Search, Space Viewer, Triage, Bulk Naming, and Activity Log

**File Operations**
- Copy, move, rename, visual bulk naming, new folder, new text document, and move to trash
- Batched conflict resolver before copy/move operations begin, with Keep Both, Replace, and Skip choices
- Drag and drop within and between panes, and onto sidebar targets
- Holding Tray for staging files from multiple locations before batch actions
- Bulk Naming: full-pane bulk rename workspace with tint, shape, tag, kind, and name filters; spreadsheet-style manual edits; recipe buttons for find/replace, prefix, suffix, numbering, and case cleanup; live conflict blocking before files are renamed

**Workspace**
- Tints, Shapes, and Marks: global color categories managed with a visual color picker plus fixed shape categories; every file/folder resolves to one Mark, defaulting to Beige + Square when no explicit mark exists. Tint glow stays visible while each pane can independently hide or show the small Shape badge overlay.
- **Painting Mode**: a dedicated paint-style marking mode toggled from the toolbar. When active, a compact tool strip appears with a Tint selector, Shape selector, and four tools — Brush (click or drag to mark files), Eraser (reset to Beige Square), Eyedropper (pick a mark from a file), and Fill Selection (mark all selected files at once). A Paint Contents toggle makes folder painting apply recursively; recursive operations show a confirmation dialog and run off the GTK main thread. All painting operations log to the Activity Log with counts and names ("Marked 12 items Cyan Triangle").
- Tags: secondary text labels; create, rename, delete, and filter folder views by tag
- **Palette Boards**: open any Palette to enter a spatial board — add file, folder, or note cards; move and resize cards freely; draw weak (dashed) or strong (solid) links between cards; all card geometry and links persist across sessions; removing a card never deletes the underlying file
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

## Supported Systems

Lattice is documented and supported for:

- Ubuntu-based systems: Ubuntu, Pop!_OS, Linux Mint, and similar derivatives
- Arch-based systems: Arch Linux, CachyOS, EndeavourOS, and similar derivatives

Other Linux distributions may work if they provide equivalent GTK4, GIO/GVfs, UDisks, Polkit, Rust, font, desktop-entry, and icon-cache packages, but they are not covered by this README.

## Requirements

- A GTK4-capable Linux desktop session
- Rust stable `1.75+`
- GTK4 development libraries
- GIO/GVfs, UDisks2, and Polkit for Trash and System Drives integration
- Fontconfig with Noto/DejaVu-compatible sans fallback, emoji fonts, and a monospace font
- `update-desktop-database` from `desktop-file-utils`
- `gtk-update-icon-cache` for icon cache refresh

### Ubuntu-Based Dependencies

```sh
sudo apt update
sudo apt install \
  build-essential \
  cargo \
  desktop-file-utils \
  fonts-dejavu-core \
  fonts-jetbrains-mono \
  fonts-noto-color-emoji \
  fonts-noto-core \
  git \
  gvfs \
  gtk-update-icon-cache \
  libglib2.0-bin \
  libgtk-4-dev \
  pkg-config \
  polkitd \
  udisks2
```

Optional filesystem support packages for removable or external drives:

```sh
sudo apt install exfatprogs dosfstools ntfs-3g btrfs-progs xfsprogs
```

### Arch-Based Dependencies

```sh
sudo pacman -Syu --needed \
  base-devel \
  desktop-file-utils \
  git \
  gtk4 \
  gtk-update-icon-cache \
  gvfs \
  noto-fonts \
  noto-fonts-emoji \
  pkgconf \
  polkit \
  rust \
  ttf-jetbrains-mono \
  udisks2
```

Optional filesystem support packages for removable or external drives:

```sh
sudo pacman -Syu --needed exfatprogs dosfstools ntfs-3g btrfs-progs xfsprogs
```

## Build From Source

```sh
git clone https://github.com/kennedycreates/lattice.git
cd lattice
cargo build --release
```

The release binary is written to `target/release/lattice`.

## Install

Build first:

```sh
cargo build --release
```

Then install for all users:

```sh
sudo install -m 755 target/release/lattice /usr/local/bin/lattice

sudo install -Dm 644 com.lattice.filemanager.desktop \
  /usr/local/share/applications/com.lattice.filemanager.desktop

sudo install -Dm 644 icons/lattice-icon.png \
  /usr/local/share/lattice/icons/lattice-icon.png
sudo install -Dm 644 icons/lattice.png \
  /usr/local/share/icons/hicolor/256x256/apps/lattice.png

sudo install -Dm 644 themes/default.css \
  /usr/local/share/lattice/themes/default.css
sudo install -Dm 644 themes/high-contrast.css \
  /usr/local/share/lattice/themes/high-contrast.css

sudo update-desktop-database /usr/local/share/applications/
sudo gtk-update-icon-cache /usr/local/share/icons/hicolor/
```

Make sure `/usr/local/bin` is in your `PATH`:

```sh
command -v lattice
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
lattice --project "My Palette"
lattice --split ~/Downloads ~/Documents
lattice --split ~/Downloads ~/Documents ~/Pictures
```

From the source tree:

```sh
cargo run -- ~/Documents
cargo run -- --downloads
cargo run -- --project "My Palette"
cargo run -- --split ~/Downloads ~/Documents
```

Launch options:

| Command | Result |
| --- | --- |
| `lattice` | Open Home. |
| `lattice <folder>` | Open a folder using positional shorthand. |
| `lattice --path <folder>` | Open a specific folder. |
| `lattice --downloads` | Open Downloads Triage. |
| `lattice --project "<name>"` | Open a palette landing page by name, case-insensitive. Legacy flag name retained for compatibility. |
| `lattice --split <left> <right>` | Open a two-pane layout. |
| `lattice --split <left> <middle> <right>` | Open a three-pane layout. |

If multiple launch modes are provided, Lattice resolves them in this order: `--split`, then `--path` or a positional folder, then `--downloads`, then `--project`. Invalid or unreadable folders fall back to Home with a status message. A missing palette name opens Home with a status message.

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

```sh
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

sudo install -Dm 644 icons/lattice.png \
  /usr/local/share/icons/hicolor/256x256/apps/lattice.png

sudo install -Dm 644 themes/default.css \
  /usr/local/share/lattice/themes/default.css

sudo install -Dm 644 themes/high-contrast.css \
  /usr/local/share/lattice/themes/high-contrast.css

sudo update-desktop-database /usr/local/share/applications/
sudo gtk-update-icon-cache /usr/local/share/icons/hicolor/
```

Updating Lattice does not remove your user config, metadata, palettes, tags, receipts, or cache. Those live under:

```text
~/.config/lattice/
~/.local/share/lattice/
~/.cache/lattice/
```

Optional backup before updating:

```sh
stamp="$(date +%Y%m%d-%H%M%S)"

[ -d "$HOME/.config/lattice" ] && cp -a "$HOME/.config/lattice" "$HOME/.config/lattice.backup.$stamp"
[ -d "$HOME/.local/share/lattice" ] && cp -a "$HOME/.local/share/lattice" "$HOME/.local/share/lattice.backup.$stamp"
```

Verify the installed app:

```sh
command -v lattice
xdg-mime query default inode/directory
```

The expected default folder opener is:

```text
com.lattice.filemanager.desktop
```

If the launcher icon or app entry does not update immediately in COSMIC/GNOME, log out and back in or restart the desktop session.

`cargo run` is for development and testing. Your daily-use installed copy should come from the release binary installed to `/usr/local/bin/lattice`.

## Troubleshooting

### Missing Window Buttons

Lattice includes its own client-side minimize, maximize/restore, and close buttons in the top titlebar. Desktops or compositors that do not provide external window decorations, including some Arch/CachyOS/KDE/Wayland setups, should still show usable window controls inside the app.

### Stretched Launcher Icon

If the launcher icon looks stretched after an older install, reinstall the current square hicolor icon and refresh the icon cache:

```sh
sudo install -Dm 644 icons/lattice.png \
  /usr/local/share/icons/hicolor/256x256/apps/lattice.png
sudo gtk-update-icon-cache /usr/local/share/icons/hicolor/
```

Some desktops cache launcher icons aggressively. If the icon does not update immediately, log out/in or restart the desktop session.

### Font Warnings

Lattice uses font stacks rather than requiring a single UI font. The bundled themes prefer `Inter` when it exists, then fall back to `Noto Sans`, `DejaVu Sans`, and the system sans-serif default. Path and text-preview surfaces prefer `JetBrains Mono`, then `Noto Sans Mono`, `DejaVu Sans Mono`, and the system monospace default.

Install the recommended runtime fonts:

```sh
# Ubuntu-based
sudo apt update
sudo apt install fonts-dejavu-core fonts-jetbrains-mono fonts-noto-color-emoji fonts-noto-core

# Arch-based
sudo pacman -Syu --needed noto-fonts noto-fonts-emoji ttf-jetbrains-mono
```

If font warnings still appear, launch from a terminal and check the fallback that Fontconfig is selecting:

```sh
lattice
fc-match "Inter"
fc-match "Noto Sans"
fc-match "JetBrains Mono"
fc-match "monospace"
fc-cache -fv
```

### Trash or System Drives Missing

Lattice reads Trash through GIO/GVfs (`trash:///`) and discovers drives through GIO's volume monitor. Missing GVfs, UDisks2, or Polkit can prevent GTK/GIO apps from seeing Trash or system drives. After installing those packages, log out/in or reboot so the desktop services are available to your session.

Install the recommended desktop storage plumbing:

```sh
# Ubuntu-based
sudo apt update
sudo apt install gvfs udisks2 polkitd

# Arch-based
sudo pacman -Syu --needed gvfs udisks2 polkit
```

Useful diagnostics:

```sh
gio list trash:///
gio trash --list
gio mount -l
udisksctl status
lsblk -f
```

Some filesystems and mounts do not support Trash. Lattice keeps Move to Trash as the default delete action and reports unsupported-trash failures clearly; Permanent Delete remains available only through the explicit confirmation flow.

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
