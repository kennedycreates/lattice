# Void Linux Testing Guide

Structured runtime test process for Lattice on **Void Linux** (rolling release,
x86_64). CI proves that Lattice *compiles* on Void's own toolchain (glibc as a
required gate, musl as informational); it cannot test graphical behavior. This
checklist is what a human tester runs on a real Void desktop session.

> Void uses **runit**, not systemd. Nothing here uses `systemctl` or
> `journalctl`. GVfs and the portal require a **graphical D-Bus user session** —
> full desktops (GNOME, XFCE, KDE) set this up automatically; a minimal Wayland
> compositor may need to be launched with `dbus-run-session <compositor>`.

---

## 0. Environment report

Run this first and paste the output into your test notes:

```sh
cat /etc/os-release
uname -a
xbps-uhelper arch
ldd --version 2>&1 | head
echo "Desktop: ${XDG_CURRENT_DESKTOP:-unknown}"
echo "Session: ${XDG_SESSION_TYPE:-unknown}"
echo "D-Bus: ${DBUS_SESSION_BUS_ADDRESS:-missing}"
echo "Runtime: ${XDG_RUNTIME_DIR:-missing}"
rustc --version
cargo --version
pkg-config --modversion gtk4
```

**Record whether the system is glibc or musl** — `ldd --version` prints
"musl libc" on musl and "GNU libc" on glibc, and `xbps-uhelper arch` ends in
`-musl` on musl. The read-only helper collects all of this plus package and
service state:

```sh
./scripts/void-diagnostics.sh
```

---

## 1. Install dependencies

See the **Void Linux Dependencies** section of the README for the full grouped
list. Update XBPS first (two-step), then install the minimum to build and run:

```sh
sudo xbps-install -Suy xbps
sudo xbps-install -Suy
sudo xbps-install -S \
  base-devel git rust cargo pkg-config gtk4-devel \
  gtk4 dbus gvfs udisks2 polkit \
  desktop-file-utils gtk-update-icon-cache xdg-utils \
  noto-fonts-ttf noto-fonts-emoji
```

Make sure the `dbus` service is enabled (needed for a session bus):

```sh
sudo ln -s /etc/sv/dbus /var/service/ 2>/dev/null || true
sv status dbus
```

Optional groups (remote GVfs backends, media, archive, cloud, portal) are in the
README.

---

## 2. Source build

Do not install system-wide until `cargo run` opens successfully.

```sh
git clone https://github.com/kennedycreates/lattice.git
cd lattice

cargo fmt --check
cargo check
cargo test --no-run
cargo run
```

- [ ] `cargo fmt --check` passes (Void's `rust` package provides `rustfmt`)
- [ ] `cargo check` passes
- [ ] `cargo test --no-run` builds the test binaries
- [ ] `cargo run` opens the Lattice window

If `cargo run` fails to open a window, capture the terminal output and whether
the session is Wayland or X11 (`echo $XDG_SESSION_TYPE`). If launching from a
minimal compositor, confirm `DBUS_SESSION_BUS_ADDRESS` is set — if not, start the
compositor via `dbus-run-session <compositor>`.

---

## 3. Manual install (no systemd)

```sh
cargo build --release
sudo ./scripts/install-system.sh
```

- [ ] Installer prints every destination it writes
- [ ] No `index.theme not found` / icon-cache error (the installer uses
      `gtk-update-icon-cache` or `gtk4-update-icon-cache`, whichever exists)
- [ ] `command -v lattice` resolves to `/usr/local/bin/lattice`
- [ ] Lattice appears in your desktop's application menu
- [ ] The launcher icon resolves `Icon=lattice` (not a generic fallback)

Set as default folder opener **only if testing that** (explicit):

```sh
xdg-mime default com.lattice.filemanager.desktop inode/directory
xdg-mime query default inode/directory   # expect: com.lattice.filemanager.desktop
```

Uninstall test:

```sh
sudo ./scripts/install-system.sh --uninstall
```

- [ ] Only Lattice files are removed; `~/.config`, `~/.local/share`, `~/.cache` untouched

---

## 4. Basic runtime checklist

```sh
mkdir -p ~/lattice-void-test/a ~/lattice-void-test/b
touch ~/lattice-void-test/a/file-{1..5}.txt
```

### Basic application

- [ ] Startup from the source tree (`cargo run`)
- [ ] Startup from the installed binary (`lattice`)
- [ ] Home and an arbitrary folder (`~/lattice-void-test`)
- [ ] Icon view and list view
- [ ] Hidden files toggle
- [ ] Tabs
- [ ] Two-panel and three-panel split modes
- [ ] Preview pane
- [ ] Themes, icons, fonts, emoji, and monospace all render (no tofu/boxes)
- [ ] Client-side window controls (minimize, maximize/restore, close)
- [ ] Open Terminal Here (Void terminals: foot, kitty, alacritty, wezterm,
      xfce4-terminal, gnome-terminal, konsole, xterm — the first installed one is used)
- [ ] Works on Wayland
- [ ] Works on X11 (if available)

### File operations (use `~/lattice-void-test`)

- [ ] New file / new folder
- [ ] Rename
- [ ] Copy
- [ ] Move
- [ ] Duplicate
- [ ] Drag and drop (within a pane, between panes, onto sidebar targets)
- [ ] Conflict resolver (Keep Both / Replace / Skip) appears before overwrite
- [ ] Holding Tray
- [ ] Plan Mode
- [ ] Trash (Move to Trash is the default delete)
- [ ] Restore from Trash
- [ ] Activity Log (receipts, undo/repeat/reveal/copy-path)

Confirm: no operation silently overwrote a file, and a failed Trash never fell
back to permanent deletion.

### Removable drives

Use a real USB stick.

- [ ] Device appears while unmounted
- [ ] Mounting from Lattice (via GIO)
- [ ] Opening the mounted device browses its contents
- [ ] Right-click → Unmount
- [ ] Right-click → Eject (if supported)
- [ ] Sidebar refreshes on plug/unplug without a manual reload
- [ ] Unplug events handled cleanly (no crash)
- [ ] A Polkit auth prompt appears when required; denial is reported, not crashed
- [ ] Internal partitions are NOT shown as removable
- [ ] exFAT / NTFS drives work when `exfatprogs` / `ntfs-3g` are installed

### Lattice tools

- [ ] Search
- [ ] Space Viewer
- [ ] Triage
- [ ] Bulk Naming
- [ ] Painting Mode
- [ ] Tints and Tags
- [ ] Holding Tray
- [ ] Plan Mode
- [ ] Activity Log
- [ ] Palettes and Palette Boards
- [ ] Internal picker (below)
- [ ] Conversion queue
- [ ] Archive operations (zip/unzip/7z)
- [ ] Cloud locations (below)

### Internal picker

```sh
cargo run -- --picker open
cargo run -- --picker open-files
cargo run -- --picker folder
cargo run -- --picker save
```

- [ ] `open` returns one path; `open-files` returns multiple; `folder` returns a folder; `save` accepts a name

Exercise every in-app caller:

- [ ] Adding files to a Palette
- [ ] Adding folders to a Palette
- [ ] Adding Palette Places
- [ ] Adding Palette Board cards
- [ ] Choosing a conversion destination
- [ ] Any other destination-selection workflow

---

## 5. Cloud and remote locations

`gvfs-smb` for SMB; the base `gvfs` package already provides SFTP/FTP/DAV/Trash.
Install `gvfs-mtp` / `gvfs-gphoto2` for phones/cameras.

- [ ] `sftp://` browses
- [ ] `ftp://`
- [ ] `smb://` (needs `gvfs-smb`)
- [ ] `dav://` / `davs://`
- [ ] GVfs FUSE paths under `/run/user/$(id -u)/gvfs/` support file ops
- [ ] MTP device (needs `gvfs-mtp`)
- [ ] Camera (needs `gvfs-gphoto2`)
- [ ] rclone mount registered as a Cloud entry

Safety checks that must still hold:

- [ ] No silent overwrite
- [ ] Destructive actions require a Plan or confirmation; no auto permanent-delete
- [ ] Long scans run off the GTK thread and are cancellable where supported
- [ ] Offline/unavailable remotes show a clear message, not a hang or crash
- [ ] Activity Log receipts include cloud context

---

## 6. Experimental: file-picker portal (do this last)

The portal is **experimental and opt-in** and never activates during a normal
install. On Void it starts via an **XDG autostart entry**, not systemd.

### Setup

```sh
sudo xbps-install dbus xdg-desktop-portal xdg-desktop-portal-gtk
cargo build --release
sudo ./scripts/install-portal.sh          # installs the autostart entry on Void
./scripts/install-portal.sh --portal-config
pkill -x xdg-desktop-portal                # restart the frontend (re-activates on demand)
```

The autostart entry starts the backend at your next graphical login. To start it
now without logging out, run **in your desktop session** (not via sudo):

```sh
setsid -f /usr/local/lib/lattice/lattice-filechooser-portal
```

**Minimal Wayland compositors** (sway, labwc) do not read XDG autostart. Add a
startup line to the compositor instead, e.g.:

```sh
# sway (~/.config/sway/config)
exec /usr/local/lib/lattice/lattice-filechooser-portal

# labwc (~/.config/labwc/autostart)
/usr/local/lib/lattice/lattice-filechooser-portal &
```

The compositor itself must already be running inside a valid user D-Bus session
(commonly `dbus-run-session labwc`). Do **not** nest another `dbus-run-session`
inside an already-working desktop.

### End-to-end verification

- [ ] Backend starts **without systemd**
- [ ] Backend owns `org.freedesktop.impl.portal.desktop.lattice` on the session bus
- [ ] Lattice picker opens for `OpenFile`
- [ ] Multiple selection
- [ ] Folder selection
- [ ] `SaveFile`
- [ ] `SaveFiles`
- [ ] Cancellation returns cleanly
- [ ] Restarting `xdg-desktop-portal` keeps things working
- [ ] Login-session autostart brings the backend up on next login
- [ ] Custom Wayland compositor startup line works
- [ ] Other portal interfaces (dark mode, screenshots) still work — the generated
      `portals.conf` preserved them

Diagnostics (no systemctl/journalctl):

```sh
pgrep -af lattice-filechooser-portal
pgrep -af xdg-desktop-portal
busctl --user list 2>/dev/null | grep -E 'portal|lattice'
# busctl unavailable? ask the bus directly:
dbus-send --session --dest=org.freedesktop.DBus --type=method_call --print-reply \
  / org.freedesktop.DBus.ListNames | grep -E 'portal|lattice'
sv status dbus
```

### Rollback (must fully restore the previous state)

```sh
./scripts/install-portal.sh --remove-portal-config
pkill -x xdg-desktop-portal
sudo ./scripts/install-portal.sh --uninstall   # removes the autostart entry too
```

- [ ] File dialogs revert to the system default
- [ ] The XDG autostart entry (`~/.config/autostart/lattice-filechooser-portal.desktop`) is gone
- [ ] No leftover Lattice portal files

---

## 7. Diagnostics to attach to a bug report

```sh
./scripts/void-diagnostics.sh > lattice-void-diagnostics.txt
```

Attach that file plus:

- The exact steps that reproduce the problem and which checklist item failed.
- Whether the system is **glibc or musl**, and Wayland or X11.
- For portal issues: the backend's terminal output (run it in the foreground:
  `/usr/local/lib/lattice/lattice-filechooser-portal 2>portal.log`).
- For a codec-dependent conversion failure: the FFmpeg package/version in use.

---

## musl status

musl is **compile-checked in CI only** and its runtime is **untested**. If you
run Lattice on a musl Void system, note it explicitly in any report — results
there are experimental. If a build or runtime problem is specific to musl,
capture the exact error so the precise limitation can be documented rather than
declaring musl broadly unsupported.
