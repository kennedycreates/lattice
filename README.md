# Lattice

Lattice is a powerful graphical GTK4 file manager for Linux, built with Rust. Designed for media and computer professionals who constantly deal with file management. The supported distribution families are Ubuntu-based systems, Arch-based systems, and Fedora Workstation.

## Features

**Navigation**
- Icon grid with folder-first sorting, per-pane hidden-file toggle, per-pane Shape badge visibility, and icon/list view switch
- Tabbed browsing with two- and three-panel split view
- Back, Up, Refresh, and a breadcrumb location bar with inline path editing and autocomplete
- Sidebar: Home, user-pinned Places, System Drives, Recent, Trash, Cloud Drives, Palettes, Tints & Tags, Search, Space Viewer, Triage, Bulk Naming, and Activity Log

**File Operations**
- Cut, copy, paste, move, rename, visual bulk naming, new folder, new text document, add folders to Places from the right-click menu, compress to ZIP, extract common archive formats, and move to trash
- Batched conflict resolver before copy/move operations begin, with Keep Both, Replace, and Skip choices
- Drag and drop within and between panes, and onto sidebar targets
- Plan Mode queues file-affecting actions for review before execution: copy/move, drag/drop, rename, duplicate, bulk rename, new file/folder, trash, permanent delete, empty trash, trash restore, tag apply/remove, Mark apply/reset, path-copy receipts, and Palette send actions
- Holding Tray for staging files from multiple locations before batch actions
- Bulk Naming: full-pane bulk rename workspace with tint, shape, tag, kind, and name filters; spreadsheet-style manual edits; recipe buttons for find/replace, prefix, suffix, numbering, and case cleanup; live conflict blocking before files are renamed

**Workspace**
- Tints, Shapes, and Marks: global color categories managed with a visual color picker plus fixed shape categories; every file/folder resolves to one Mark, defaulting to Beige + Square when no explicit mark exists. Tint glow stays visible while each pane can independently hide or show the small Shape badge overlay.
- Optional read-only Watercolor context from Terroir: when the user-level Terroir daemon is available, the preview pane can show `.water` workspace references, clearly labeled Watercolor palettes, referencing object titles/types, and broken-reference warnings for the selected file or folder. A quiet Watercolor sidebar section can also show Terroir status, indexed workspaces, Watercolor palettes, and broken references. Lattice-local Palettes remain separate and unchanged.
- **Painting Mode**: a dedicated paint-style marking mode toggled from the toolbar. When active, a compact tool strip appears with a Tint selector, Shape selector, and four tools — Brush (click or drag to mark files), Eraser (reset to Beige Square), Eyedropper (pick a mark from a file), and Fill Selection (mark all selected files at once). A Paint Contents toggle makes folder painting apply recursively; recursive operations show a confirmation dialog and run off the GTK main thread. All painting operations log to the Activity Log with counts and names ("Marked 12 items Cyan Triangle").
- Tags: secondary text labels; create, rename, delete, and filter folder views by tag
- **Palette Boards**: open any Palette to enter a spatial board — add file, folder, or note cards; move and resize cards freely; draw weak (dashed) or strong (solid) links between cards; all card geometry and links persist across sessions; removing a card never deletes the underlying file
- Space Viewer: disk usage analysis for the active folder — total size, file/folder counts, pie chart by file type, ranked largest files and subfolders, three scan depths (folder only, one level deep, full recursive with cancellation), and row actions (open, reveal, add to Holding Tray, copy path, move to trash)
- Downloads Triage: Images, Videos, Archives, Documents, Large Files, Today, This Week, This Month, and Older filters
- **Cloud Drives**: register mounted cloud and remote drives (rclone, pCloud Drive, GVfs, SFTP, FTP, WebDAV, or any local mount) as named sidebar entries. Landing view shows availability status and launches Space Viewer, Triage, or direct browsing. First-pass uses mounted filesystem locations — no direct provider APIs. See `docs/cloud.md` for mounting instructions.
- **Cloud-First Tools**: Every Lattice tool is cloud-aware — Search, Triage, Space Viewer, Painting Mode, Holding Tray, Action Plans, Activity Log, Trash, and context menus all detect cloud locations and show appropriate messaging, guard unsafe operations, and include cloud context in receipts. See `docs/cloud.md` for tool support details and known limitations.
- **GVfs / GIO Remote URIs**: Cloud entries accept GIO/GVfs URIs (`sftp://`, `ftp://`, `smb://`, `dav://`, `davs://`) as the path. Lattice resolves URIs to local GVfs FUSE mount paths when available, or enumerates directly via GIO (triggering GVfs auth transparently). The address bar also accepts these URIs for direct navigation. Kind is auto-detected from the URI scheme. See `docs/cloud.md` for required packages and troubleshooting.
- **rclone Awareness**: the CLOUD sidebar section includes "⚙ rclone Remotes" — detects rclone, lists configured remote names (no credentials), and provides copy-ready mount commands and quick-add shortcuts.
- **rclone Mount/Unmount**: Cloud entries with `kind = rclone` and a Remote name set show **Mount** and **Unmount** buttons on the landing page. Mount runs `rclone mount --daemon` off the main thread and confirms accessibility; Unmount calls `fusermount3`/`fusermount`/`umount`. Credentials are managed entirely by rclone config — Lattice never reads or modifies them. See `docs/cloud_rclone_mounts.md`.
- Folder search with name, kind, date, size, and tag filters; current-folder or recursive scope

**Media Conversion**
- **Convert…** action in the file right-click menu: opens a full-pane conversion panel for any selection containing image, audio, or video files
- Preset dropdown covering JPEG, PNG, WebP, AVIF, web-sized JPEG/WebP, MP3, FLAC, Opus, Compatible MP4, Smaller MP4, WebM, and audio extraction — auto-selected based on dominant media kind
- Output location: Next to originals, Converted subfolder, or a folder chosen via the built-in Lattice picker
- Conflict policy: Auto-rename (default), Skip existing, or Overwrite
- Preview table shows source → destination name for every file; incompatible-kind files are marked skipped before conversion starts
- Tool dependency warning adapts to the selected preset (ffmpeg, ImageMagick, or Vips)
- **Conversion Queue**: a dedicated progress panel slides up when conversion starts and stays visible while the user browses. Shows batch progress bar, per-job status rows (⏳ queued / ↻ running / ✓ done / ✗ failed / ↷ skipped), expandable error detail with selectable text and a Copy button, and footer controls: Cancel (while running), Retry Failed, Open Output, and Dismiss
- Last preset per media kind, output mode, and conflict policy are remembered in `~/.config/lattice/convert_settings.toml`
- See `docs/conversion.md` for required tools, supported formats, and developer notes

**Preview and History**
- Preview pane for images, text/config files, folders, media metadata, resolved Mark identity, and neutral tag chips
- Activity Log with SQLite-backed receipts plus guarded undo, repeat, reveal, and copy-path actions per row

**Configuration**
- TOML config at `~/.config/lattice/config.toml`
- Rebindable command shortcuts for navigation, file actions, sidebar tools, painting tools, Holding Tray actions, queued plans, conversion controls, and custom actions
- Bundled `default` and `high-contrast` themes
- User themes in `~/.config/lattice/themes/`

## Supported Systems

Lattice is documented and supported for:

- Ubuntu-based systems: Ubuntu, Pop!_OS, Linux Mint, and similar derivatives
- Arch-based systems: Arch Linux, CachyOS, EndeavourOS, and similar derivatives
- Fedora Workstation 43 and 44
- Void Linux (x86_64 glibc)

For Fedora specifically:

- **Source builds and manual system-wide installs are supported by this repository**, and Fedora 43/44 compilation is checked continuously in CI (see [Continuous Integration](#continuous-integration)). CI verifies that Lattice *compiles* on Fedora's own Rust/GTK4 toolchain — it does not, and cannot, test graphical runtime behavior.
- **Runtime behavior on Fedora must still be validated** through the Fedora test checklist in [docs/fedora-testing.md](docs/fedora-testing.md). No claim is made here that a full graphical Fedora runtime pass has been completed.
- **Fedora Atomic variants (Silverblue, Kinoite, Sericea) are not covered** by the normal host installation instructions below. Their immutable `/usr` requires a different workflow (layering with `rpm-ostree`, a toolbox/distrobox dev container, or a Flatpak), which this repository does not yet provide.
- Other RPM-based distributions (RHEL, openSUSE, etc.) are **not** automatically considered supported just because Fedora works.

For Void Linux specifically (Void is rolling release — there is no numbered version; instructions target a fully updated current install):

- **Void x86_64 glibc: source-build and manual-install support.** x86_64 glibc compilation is checked continuously in CI, and manual `/usr/local` installation works **without systemd** (Void uses runit). Runtime behavior must still be validated through [docs/void-testing.md](docs/void-testing.md).
- **Void x86_64 musl: compile-checked / experimental.** musl compilation is exercised in CI but is informational only — no musl **runtime** support is claimed until it is tested on a real musl machine. If a dependency turns out to be musl-incompatible, the specific limitation will be documented rather than declaring Void musl broadly unsupported.
- **Other Void architectures (aarch64, etc.) are not yet validated.** x86_64 working does not imply they do.
- GVfs and the portal need a **graphical D-Bus user session**. Full desktop sessions (GNOME, XFCE, KDE) establish this automatically; a minimal custom Wayland session may need to be started via `dbus-run-session <compositor>`.
- Being runit-based does **not** by itself make a distribution supported — this covers Void specifically.

Other Linux distributions may work if they provide equivalent GTK4, GIO/GVfs, UDisks, Polkit, Rust, font, desktop-entry, and icon-cache packages, but they are not covered by this README.

## Requirements

**Required to build and run:**
- A GTK4-capable Linux desktop session
- Rust stable `1.75+` — see note below
- GTK4 development libraries
- GIO/GVfs, UDisks2, and Polkit for Trash and System Drives integration
- Fontconfig with Noto/DejaVu-compatible sans fallback, emoji fonts, and a monospace font
- `update-desktop-database` from `desktop-file-utils`
- `gtk-update-icon-cache` for icon cache refresh

**Optional — media conversion** (`🔄 Convert Media` sidebar tool):
- `ffmpeg` — required for image, audio, and video conversion (most presets)
- `imagemagick` — required for AVIF output only
- On Fedora, the default `ffmpeg-free` package ships a **reduced codec set**. Common presets (JPEG/PNG/WebP, MP3, H.264 MP4) work, but some presets can legitimately fail with a "required codec is not available" message. Lattice reports the codec failure clearly in the Conversion Queue rather than pretending every preset will run. Install the full `ffmpeg` from [RPM Fusion](https://rpmfusion.org/) if you need the complete codec set.

**Optional — archive actions** (right-click menu):
- `zip` and `unzip` — create ZIP archives and extract ZIP/JAR/EPUB archives
- `tar` — extract TAR, TAR.GZ, TGZ, TAR.XZ, and TAR.BZ2 archives
- `7z` — extract 7-Zip and RAR archives (Ubuntu: `p7zip-full`; Arch: `p7zip`; Fedora/Void: `7zip`)

**Optional — cloud drive mounting** (rclone Cloud entries):
- `rclone` — install from https://rclone.org/install/ or your distro package
- `fuse3` (Ubuntu/Fedora/Void) / `fuse2` (Arch) — provides `fusermount3`/`fusermount` for unmounting

**Optional — GVfs remote URI support** (`sftp://`, `smb://`, `ftp://`, `dav://` paths):
- `gvfs-backends` (Ubuntu) / `gvfs-smb` (Arch/Fedora/Void) — SMB support (on Void, `sftp://`/`ftp://`/`dav://` are already in the base `gvfs` package)
- `gvfs-fuse` (Arch/Fedora) — needed on some systems for SFTP/FTP URIs

**Optional — Watercolor context**
- `terroird` from Terroir — exposes read-only `.water` context over `$XDG_RUNTIME_DIR/watercolor/terroir/terroir.sock`.
- Lattice starts and works normally when Terroir is unavailable, slow, or has malformed `.water` files in its index.
- Watercolor palettes shown by Terroir are labeled separately from Lattice-local Palettes. Lattice does not write `.water` files or migrate local Palette data.

> **Rust version note:** `apt install cargo` on Ubuntu 22.04 LTS provides Rust 1.66, which is below the required 1.75. Ubuntu 24.04 LTS provides 1.75. Fedora 43/44 and current Arch both ship a new-enough Rust/Cargo, so no `rustup` is needed there. If your distro Rust is too old, install via [rustup](https://rustup.rs/) instead: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

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

Optional — media conversion:

```sh
sudo apt install ffmpeg imagemagick
```

Optional — archive actions:

```sh
sudo apt install zip unzip p7zip-full
```

Optional — rclone cloud drive mounting:

```sh
sudo apt install fuse3
# rclone: install from https://rclone.org/install/ (or: sudo apt install rclone)
```

Optional — GVfs remote URIs (sftp://, smb://, ftp://, dav://):

```sh
sudo apt install gvfs-backends
```

Optional filesystem support for removable or external drives:

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

Optional — media conversion:

```sh
sudo pacman -S ffmpeg imagemagick
```

Optional — archive actions:

```sh
sudo pacman -S zip unzip p7zip
```

Optional — rclone cloud drive mounting:

```sh
sudo pacman -S rclone fuse2
```

Optional — GVfs remote URIs (sftp://, smb://, ftp://, dav://):

```sh
sudo pacman -S gvfs-smb
sudo pacman -S gvfs-fuse   # if sftp:// doesn't work without it
```

Optional filesystem support for removable or external drives:

```sh
sudo pacman -Syu --needed exfatprogs dosfstools ntfs-3g btrfs-progs xfsprogs
```

### Fedora Workstation Dependencies

Fedora Workstation 43 and 44. Package names below were verified against Fedora 43/44 repositories.

**1. Required to build and run:**

```sh
sudo dnf install -y \
  gcc \
  gcc-c++ \
  make \
  git \
  rust \
  cargo \
  rustfmt \
  pkgconf-pkg-config \
  gtk4-devel \
  glib2-devel \
  desktop-file-utils \
  gtk-update-icon-cache \
  gvfs \
  gvfs-fuse \
  udisks2 \
  polkit \
  google-noto-sans-fonts \
  google-noto-color-emoji-fonts \
  jetbrains-mono-fonts
```

`pkgconf-pkg-config` provides the `pkg-config` command, `desktop-file-utils` provides `update-desktop-database`, and `gtk-update-icon-cache` is a standalone package on Fedora.

**2. Recommended desktop integration:**

```sh
sudo dnf install -y gvfs-smb   # SMB/SFTP/FTP/WebDAV browsing via GVfs
```

Fedora's Rust and Cargo packages are current enough to build Lattice (well above the required Rust `1.75`), so `rustup` is not needed. This is the same toolchain the Fedora CI jobs use.

**3. Optional — media conversion:**

```sh
sudo dnf install ffmpeg-free ImageMagick
```

`ffmpeg-free` is Fedora's default FFmpeg build and has a **reduced codec set** — see the media-conversion note under [Requirements](#requirements). For the full codec set, enable [RPM Fusion](https://rpmfusion.org/) and install `ffmpeg`.

**4. Optional — archive support:**

```sh
sudo dnf install zip unzip 7zip
```

On Fedora the `7z` binary comes from the `7zip` package (not `p7zip`).

**5. Optional — cloud / rclone support:**

```sh
sudo dnf install rclone fuse3
```

**6. Optional — experimental file-picker portal:**

```sh
sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk
```

`xdg-desktop-portal-gtk` provides the GTK FileChooser backend that Lattice's experimental portal uses as its fallback. See [docs/file_picker_portal.md](docs/file_picker_portal.md).

Optional filesystem support for removable or external drives:

```sh
sudo dnf install exfatprogs dosfstools ntfs-3g btrfs-progs xfsprogs
```

### Void Linux Dependencies

Void Linux is rolling release — there is no numbered version, so these instructions
target a **fully updated current** install (x86_64 glibc). Package names were verified
against the current Void repositories. Void uses **runit**, not systemd, and a graphical
D-Bus session is required for GVfs and the portal (full desktops set this up; a minimal
Wayland session may need `dbus-run-session <compositor>`).

Update first. Void's own rule is to let XBPS update itself before installing anything,
so do it in two steps:

```sh
sudo xbps-install -Suy xbps   # update the package manager first
sudo xbps-install -Suy        # then update the rest of the system
```

**1. Required to compile:**

```sh
sudo xbps-install -S \
  base-devel \
  git \
  rust \
  cargo \
  pkg-config \
  gtk4-devel
```

Void's `rust` package ships `rustfmt` and `cargo-fmt`, so `cargo fmt` works with
`rust` + `cargo` — there is no separate `rustfmt` package. `glib-devel`, `fontconfig`,
and `shared-mime-info` are pulled in transitively by `gtk4-devel`, so you do not need
to list them explicitly.

**2. Required to run:**

```sh
sudo xbps-install -S \
  gtk4 \
  dbus \
  gvfs \
  udisks2 \
  polkit
```

(If you built from source you already have `gtk4` via `gtk4-devel`.) A running `dbus`
service and a graphical D-Bus session are needed for Trash, drives, and the portal.

**3. Recommended desktop integration:**

```sh
sudo xbps-install -S \
  desktop-file-utils \
  gtk-update-icon-cache \
  xdg-utils \
  gsettings-desktop-schemas \
  noto-fonts-ttf \
  noto-fonts-emoji
```

`desktop-file-utils` provides `update-desktop-database`; `gtk-update-icon-cache` is a
standalone package on Void; `noto-fonts-ttf` includes a monospace fallback (Noto Sans
Mono).

**4. Optional — remote / GVfs backends** (install only the backends you need — the base
`gvfs` package already provides Trash and the `sftp://`, `ftp://`, `dav://`, and `davs://`
backends):

```sh
sudo xbps-install gvfs-smb        # smb:// (Windows/Samba shares)
sudo xbps-install gvfs-mtp        # MTP devices (phones)
sudo xbps-install gvfs-gphoto2    # PTP cameras / some media players
```

**5. Optional — media conversion:**

```sh
sudo xbps-install ffmpeg ImageMagick
```

On current Void the concrete FFmpeg package is `ffmpeg6` (the `ffmpeg` name pulls it in);
check `xbps-query -Rs ffmpeg` if the default version has since moved on.

**6. Optional — archive support:**

```sh
sudo xbps-install zip unzip 7zip
```

On Void the `7z` binary comes from `7zip` (the old `p7zip` is a transitional dummy).

**7. Optional — cloud / rclone support:**

```sh
sudo xbps-install rclone fuse3
```

`fuse3` provides `fusermount3` for unmounting rclone/FUSE mounts.

**8. Optional — experimental file-picker portal:**

```sh
sudo xbps-install dbus xdg-desktop-portal xdg-desktop-portal-gtk
```

`xdg-desktop-portal-gtk` provides the GTK FileChooser backend Lattice uses as its
fallback. On Void the portal backend starts via an **XDG autostart entry**, not a systemd
unit — see [docs/file_picker_portal.md](docs/file_picker_portal.md).

## Build From Source

```sh
git clone https://github.com/kennedycreates/lattice.git
cd lattice
cargo build --release
```

The release binary is written to `target/release/lattice`.

## Install

The same install path works on Ubuntu, Arch, and Fedora — it installs only
Lattice's own files and does not touch user data or change your default file
manager.

### Recommended: centralized installer

Build first, then run the installer:

```sh
cargo build --release
sudo ./scripts/install-system.sh
```

`scripts/install-system.sh`:

- installs the binary, desktop entry, application icon, and bundled themes under `/usr/local` (override with `--prefix`),
- refreshes the desktop metadata and icon cache,
- prints every destination it writes,
- never touches `~/.config`, `~/.local/share`, or `~/.cache`, and never sets a default file manager.

To remove exactly those files later:

```sh
sudo ./scripts/install-system.sh --uninstall
```

**Fedora icon-cache note:** a freedesktop icon-theme directory must contain an
`index.theme`. Under `/usr/local` (which has no distro-provided hicolor
`index.theme`), running `gtk-update-icon-cache` bare fails with an
"index.theme not found" error on a fresh Fedora layout. The installer fixes the
cause — it writes a minimal valid `index.theme` (marked, and removed again on
uninstall) before refreshing the cache — rather than hiding the error. The icon
lands at `/usr/local/share/icons/hicolor/256x256/apps/lattice.png` and resolves
as `Icon=lattice`.

### Manual install (equivalent steps)

If you prefer to run the steps yourself:

```sh
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

# Fedora/fresh-prefix safe icon cache: ensure a hicolor index.theme exists so
# gtk-update-icon-cache does not fail with "index.theme not found".
if [ ! -f /usr/local/share/icons/hicolor/index.theme ]; then
  printf '[Icon Theme]\nName=Hicolor\nHidden=true\nDirectories=256x256/apps\n\n[256x256/apps]\nSize=256\nContext=Applications\nType=Fixed\n' \
    | sudo tee /usr/local/share/icons/hicolor/index.theme >/dev/null
fi
sudo gtk-update-icon-cache -f -t /usr/local/share/icons/hicolor/
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
| `lattice --project "<name>"` | Open a Palette by name, case-insensitive. Legacy flag name retained for compatibility. |
| `lattice --split <left> <right>` | Open a two-pane layout. |
| `lattice --split <left> <middle> <right>` | Open a three-pane layout. |

If multiple launch modes are provided, Lattice resolves them in this order: `--split`, then `--path` or a positional folder, then `--downloads`, then `--project`. Invalid or unreadable folders fall back to Home with a status message. A missing palette name opens Home with a status message.

Unknown flags are ignored. Options that take values print a terminal warning when the value is missing.

## Configuration

On first run, Lattice creates `~/.config/lattice/config.toml`.

```toml
theme = "default" # or "high-contrast"
enable_terroir_context = true

[shortcuts]
custom.open_in_gimp = "Ctrl+Alt+G"
open_convert = "Ctrl+Alt+F"
trash = "" # disable a default shortcut

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

The generated config lists every supported built-in shortcut action id. Command shortcuts are secondary accelerators; all core actions remain available by mouse. Standard text editing, path completion, and local list/grid navigation keys keep their GTK behavior and are not part of the command rebinding surface.

Theme lookup order:

1. `~/.config/lattice/themes/<name>.css`
2. `themes/<name>.css` relative to the current working directory
3. `themes/<name>.css` beside the source tree when running a Cargo build
4. `<prefix>/share/lattice/themes/<name>.css` for installed builds

See [docs/theming.md](docs/theming.md) for the full CSS class reference and theme authoring guide.

## Desktop Integration

See [docs/desktop-setup.md](docs/desktop-setup.md) for labwc keybindings, Waybar launcher config, GNOME, COSMIC, and Sway setup.

The desktop entry ID is `com.lattice.filemanager.desktop`. The application icon name is `lattice`.

### Experimental: File Picker Portal Backend

Lattice includes an experimental [xdg-desktop-portal](https://flatpak.github.io/xdg-desktop-portal/) FileChooser backend. When installed and activated, apps that request a file dialog through the portal — including Flatpak apps, Chrome, and GTK apps with `GTK_USE_PORTAL=1` — will use the Lattice picker instead of the system default.

**This is opt-in and does not activate automatically.** Installing the files alone has no effect on your desktop.

```bash
# 1. Build
cargo build --release

# 2. Install system files + enable service (needs sudo)
sudo ./scripts/install-portal.sh

# 3. If the service did not enable automatically, do it manually (no sudo):
systemctl --user daemon-reload
systemctl --user enable --now lattice-filechooser-portal.service

# 4. Opt in to using Lattice as the portal file picker (no sudo):
./scripts/install-portal.sh --portal-config

# 5. Restart the portal daemon
systemctl --user restart xdg-desktop-portal

# 6. Verify
systemctl --user status lattice-filechooser-portal.service
./scripts/test-portal.sh
```

Important notes:
- Step 3 generates a complete `portals.conf` that preserves all your existing desktop portal backends (dark mode, screen sharing, etc.) while only overriding the file chooser. **Do not write a portals.conf by hand** — a partial file silently breaks other portal interfaces.
- For GTK apps (GIMP, Inkscape), you also need `GTK_USE_PORTAL=1`. Chrome and Electron apps use the portal natively without it.

To roll back: `sudo ./scripts/install-portal.sh --uninstall` removes everything; `./scripts/install-portal.sh --remove-portal-config` reverts the portals.conf.

See [docs/file_picker_portal.md](docs/file_picker_portal.md) for the full guide, troubleshooting, and per-app notes.

---

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
# Ubuntu/Pop!_OS
sudo apt update
sudo apt install fonts-dejavu-core fonts-jetbrains-mono fonts-noto-color-emoji fonts-noto-core

# Arch/CachyOS/EndeavourOS
sudo pacman -Syu --needed noto-fonts noto-fonts-emoji ttf-jetbrains-mono

# Fedora Workstation
sudo dnf install google-noto-sans-fonts google-noto-color-emoji-fonts jetbrains-mono-fonts

# Void Linux
sudo xbps-install noto-fonts-ttf noto-fonts-emoji
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
# Ubuntu/Pop!_OS
sudo apt update
sudo apt install gvfs udisks2 polkitd

# Arch/CachyOS/EndeavourOS
sudo pacman -Syu --needed gvfs udisks2 polkit

# Fedora Workstation
sudo dnf install gvfs udisks2 polkit

# Void Linux
sudo xbps-install gvfs udisks2 polkit dbus
```

On Void (runit), make sure the `dbus` service is enabled so a session bus is available:

```sh
sudo ln -s /etc/sv/dbus /var/service/   # enable dbus if not already
sv status dbus
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

If you installed with the centralized script, uninstall the same way:

```sh
sudo ./scripts/install-system.sh --uninstall
```

Or remove the files manually:

```sh
sudo rm -f /usr/local/bin/lattice
sudo rm -f /usr/local/share/applications/com.lattice.filemanager.desktop
sudo rm -f /usr/local/share/icons/hicolor/256x256/apps/lattice.png
sudo rm -rf /usr/local/share/lattice
sudo update-desktop-database /usr/local/share/applications/
sudo gtk-update-icon-cache /usr/local/share/icons/hicolor/ 2>/dev/null || true
```

User data is not removed by those commands. Lattice stores config in `~/.config/lattice/` and metadata in `~/.local/share/lattice/metadata.db`.

## Development

```sh
cargo fmt --check
cargo check
cargo run
```

During development, `cargo run` refreshes the desktop entry and icon under the current account when the source copies are newer. Use the install steps above for release-style testing of the binary, bundled themes, and system-wide paths.

Fedora testers should follow [docs/fedora-testing.md](docs/fedora-testing.md) and Void testers [docs/void-testing.md](docs/void-testing.md); both can attach the read-only report from the matching `scripts/*-diagnostics.sh` to any bug report.

## Continuous Integration

The project uses GitHub Actions (`.github/workflows/ci.yml`):

- **Ubuntu job** — `cargo fmt --check`, `cargo check`, and `cargo test --no-run` using the upstream Rust toolchain.
- **Fedora jobs** — a matrix of `fedora:43` and `fedora:44` containers using Fedora's own `rust`/`cargo`/`rustfmt` and `gtk4-devel`, running `cargo fmt --check`, `cargo check`, `cargo test --no-run`, and `cargo build --release`.
- **Void jobs** — `x86_64` **glibc** and **musl** using the official `ghcr.io/void-linux/void-glibc-full` / `void-musl-full` images and Void's own `rust`/`cargo` (which ship `rustfmt`), running the same four commands. glibc is a required gate; musl is `continue-on-error` (compile-checked and informational until validated on a real musl machine).

CI verifies **compilation only**. Container jobs have no display server, so they do not exercise the GTK graphical runtime, Wayland, Trash, GVfs, UDisks, Polkit, removable drives, desktop launchers, or the file-picker portal. Runtime behavior is validated by a tester via the platform test guides ([Fedora](docs/fedora-testing.md), [Void](docs/void-testing.md)).
