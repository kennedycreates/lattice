# Fedora Testing Guide

Structured runtime test process for Lattice on **Fedora Workstation 43 and 44**.

CI already proves that Lattice *compiles* on Fedora's own toolchain (see the
Fedora matrix in `.github/workflows/ci.yml`). CI cannot test graphical behavior.
This checklist is what a human tester runs on a real Fedora desktop session to
validate runtime behavior. Nothing here has been auto-verified — every box is a
real thing to click.

> Scope note: this guide targets **Fedora Workstation** (a mutable host with a
> writable `/usr`). Fedora Atomic variants (Silverblue, Kinoite, Sericea) are
> out of scope — their immutable `/usr` needs `rpm-ostree` layering, a
> toolbox/distrobox container, or a Flatpak, none of which this repo provides.

---

## 0. Environment report

Run this first and paste the output into your test notes:

```bash
cat /etc/fedora-release
uname -a
echo "Desktop: ${XDG_CURRENT_DESKTOP:-unknown}"
echo "Session: ${XDG_SESSION_TYPE:-unknown}"
rustc --version
cargo --version
pkg-config --modversion gtk4
```

You can also run the read-only helper, which collects the above plus package
versions, GVfs/UDisks summaries, and recent errors:

```bash
./scripts/fedora-diagnostics.sh
```

---

## 1. Install dependencies

See the **Fedora Workstation Dependencies** section of the README for the full,
grouped list. Minimum to build and run:

```bash
sudo dnf install -y \
  gcc gcc-c++ make git rust cargo rustfmt pkgconf-pkg-config \
  gtk4-devel glib2-devel desktop-file-utils gtk-update-icon-cache \
  gvfs gvfs-fuse udisks2 polkit \
  google-noto-sans-fonts google-noto-color-emoji-fonts jetbrains-mono-fonts
```

Optional groups (media, archive, cloud, portal) are listed in the README.

---

## 2. Reproducible source build

```bash
git clone https://github.com/kennedycreates/lattice.git
cd lattice
cargo fmt --check
cargo check
cargo test --no-run
cargo run
```

- [ ] `cargo fmt --check` passes
- [ ] `cargo check` passes
- [ ] `cargo test --no-run` builds the test binaries
- [ ] `cargo run` launches the Lattice window on the Fedora desktop session

If `cargo run` fails to open a window, capture the terminal output and the
`XDG_SESSION_TYPE` (Wayland vs X11) before filing a report.

---

## 3. Manual system-wide install

```bash
cargo build --release
sudo ./scripts/install-system.sh
```

- [ ] Installer prints every destination it writes
- [ ] No `index.theme not found` / icon-cache error appears
- [ ] `command -v lattice` resolves to `/usr/local/bin/lattice`
- [ ] Lattice appears in the GNOME app grid (Activities → search "Lattice")
- [ ] Launching from the app grid opens Lattice
- [ ] The launcher icon is the Lattice icon (resolves `Icon=lattice`), not a generic fallback

Set as default folder opener **only if you want to test that** (explicit):

```bash
xdg-mime default com.lattice.filemanager.desktop inode/directory
xdg-mime query default inode/directory   # expect: com.lattice.filemanager.desktop
```

Uninstall test:

```bash
sudo ./scripts/install-system.sh --uninstall
```

- [ ] Only Lattice files are removed; user data under `~/.config`, `~/.local/share`, `~/.cache` is untouched

---

## 4. Runtime checklist

Set up disposable test data:

```bash
mkdir -p ~/lattice-fedora-test/a ~/lattice-fedora-test/b
touch ~/lattice-fedora-test/a/file-{1..5}.txt
```

### Basic application

- [ ] Startup from Cargo (`cargo run`)
- [ ] Startup from the installed binary (`lattice`)
- [ ] Open Home and an arbitrary folder (`~/lattice-fedora-test`)
- [ ] Icon view and list view
- [ ] Hidden files toggle
- [ ] Tabs (open, switch, close)
- [ ] Two-panel and three-panel split modes
- [ ] Themes, icons, fonts, emoji, and monospace all render (no tofu/boxes)
- [ ] Window controls (minimize, maximize/restore, close)
- [ ] **Open Terminal Here** launches a terminal in the current folder
      (Fedora GNOME default is Ptyxis; GNOME Console `kgx` and `gnome-terminal`
      are also detected)

### File operations

Use `~/lattice-fedora-test` for all destructive tests.

- [ ] New file / new folder
- [ ] Rename
- [ ] Copy
- [ ] Move
- [ ] Drag and drop (within a pane, between panes, onto sidebar targets)
- [ ] Duplicate
- [ ] Conflict resolver (Keep Both / Replace / Skip) appears before overwrite
- [ ] Holding Tray (stage from multiple locations, then batch-act)
- [ ] Plan Mode (queue actions, review, execute)
- [ ] Trash (Move to Trash is the default delete)
- [ ] Restore from Trash

Confirm: no operation silently overwrote a file, and a failed Trash never fell
back to permanent deletion.

### Removable drives

Use a real USB stick.

- [ ] Plugged in but unmounted → appears in the SYSTEM sidebar section
- [ ] Clicking the unmounted entry mounts it (via GIO)
- [ ] Opening after mount browses the contents
- [ ] Right-click → Unmount
- [ ] Right-click → Eject (if the drive supports eject)
- [ ] Unplugging removes it from the sidebar
- [ ] Sidebar refreshes on drive-added / drive-removed without a manual reload
- [ ] Internal partitions are NOT shown as removable devices
- [ ] exFAT / NTFS drives work when `exfatprogs` / `ntfs-3g` are installed
- [ ] A Polkit auth prompt (or a clear failure message) appears when required — no crash

### Lattice tools

- [ ] Search (name/kind/date/size/tag; current-folder and recursive)
- [ ] Space Viewer (scan depths, cancel, row actions)
- [ ] Triage (Downloads-style filters)
- [ ] Bulk Naming (filters, recipes, live conflict blocking)
- [ ] Painting Mode (Brush/Eraser/Eyedropper/Fill; recursive confirm dialog)
- [ ] Tints and Tags (create, apply, filter)
- [ ] Holding Tray
- [ ] Activity Log (receipts, undo/repeat/reveal/copy-path)
- [ ] Palettes and Palette Boards (cards, links, geometry persists)
- [ ] Internal file picker (see below)
- [ ] Conversion queue (see media note below)

### Internal picker

```bash
cargo run -- --picker open
cargo run -- --picker open-files
cargo run -- --picker folder
cargo run -- --picker save
```

- [ ] `--picker open` returns one path on confirm
- [ ] `--picker open-files` returns multiple paths
- [ ] `--picker folder` returns a folder
- [ ] `--picker save` accepts a name and returns a target path

Also exercise every in-app caller of the picker:

- [ ] Adding Palette files
- [ ] Adding Palette folders
- [ ] Adding Palette Places
- [ ] Adding Palette Board cards
- [ ] Choosing a conversion destination

### Media conversion (ffmpeg-free caveat)

Fedora's default `ffmpeg-free` has a reduced codec set.

- [ ] JPEG/PNG/WebP image conversion works
- [ ] MP3 audio conversion works
- [ ] Compatible MP4 (H.264) works, or fails with a clear "required codec is not available" message (not a crash, not a silent no-op)
- [ ] The Conversion Queue shows per-job status and expandable error detail

If you need every preset, enable [RPM Fusion](https://rpmfusion.org/) and install
the full `ffmpeg`, then re-test.

---

## 5. Cloud and remote locations

Requires `gvfs-smb` for SMB and (on some systems) `gvfs-fuse` for SFTP/FTP.

- [ ] `sftp://` entry resolves / browses
- [ ] `ftp://` entry
- [ ] `smb://` share
- [ ] `dav://` and `davs://` WebDAV
- [ ] GVfs FUSE paths under `/run/user/$(id -u)/gvfs/` browse and support file ops
- [ ] rclone mount (register the mounted path as a Cloud entry)

Safety checks that must still hold on Fedora:

- [ ] No silent overwrite
- [ ] Long scans run off the GTK thread (UI stays responsive) and are cancellable where supported
- [ ] Offline/unavailable remotes show a clear message, not a hang or crash
- [ ] Destructive operations require an Action Plan or confirmation
- [ ] Activity Log receipts include cloud context

See `docs/cloud.md` for details.

---

## 6. Experimental: file-picker portal (do this last)

The portal is **experimental and opt-in**. It never activates during a normal
install. Only run this section if you are specifically testing the portal.

### Setup

```bash
sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk
cargo build --release
sudo ./scripts/install-portal.sh
systemctl --user daemon-reload
systemctl --user enable --now lattice-filechooser-portal.service
./scripts/install-portal.sh --portal-config
systemctl --user restart xdg-desktop-portal
```

### End-to-end verification

```bash
systemctl --user status lattice-filechooser-portal.service   # active (running)
busctl --user list | grep -E 'portal|lattice'
./scripts/test-portal.sh                                       # opens the Lattice picker
GTK_USE_PORTAL=1 gio open .    # or trigger a file dialog in a GTK/Chrome app
```

- [ ] Backend registers on the session bus
- [ ] `test-portal.sh` opens the Lattice picker and returns a `file://` URI
- [ ] A portal-using app (Chrome, or a GTK app with `GTK_USE_PORTAL=1`) shows the Lattice picker
- [ ] Other portal interfaces (dark mode, screenshots) still work — the generated `portals.conf` preserved them

### Rollback (must fully restore the previous state)

```bash
./scripts/install-portal.sh --remove-portal-config
systemctl --user restart xdg-desktop-portal
sudo ./scripts/install-portal.sh --uninstall
```

- [ ] File dialogs revert to the system default
- [ ] No leftover Lattice portal files (see `docs/file_picker_portal.md`)

See `docs/file_picker_portal.md` for the full portal guide and Fedora/SELinux
troubleshooting.

---

## 7. Diagnostics to attach to a bug report

```bash
gio list trash:///
gio trash --list
gio mount -l
udisksctl status
lsblk -f
rpm -q gtk4 glib2 gvfs gvfs-fuse gvfs-smb udisks2 polkit
journalctl --user -b --since "15 minutes ago" | grep -iE 'lattice|gvfs|udisks|portal'
```

Or just run:

```bash
./scripts/fedora-diagnostics.sh > lattice-diagnostics.txt
```

**What to attach to a bug report:**

- The full `fedora-diagnostics.sh` output (or the manual commands above).
- The exact steps that reproduce the problem, and which checklist item failed.
- Whether the session is Wayland or X11 (`echo $XDG_SESSION_TYPE`).
- For portal issues: `journalctl --user -b -u lattice-filechooser-portal.service`
  and `journalctl --user -b -u xdg-desktop-portal`.
- For suspected SELinux blocks: the output of `sudo ausearch -m AVC -ts recent`
  (do **not** disable SELinux).

If a codec-dependent conversion failed, note whether you were using
`ffmpeg-free` or the full RPM Fusion `ffmpeg`.
