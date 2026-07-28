# Lattice FileChooser Portal

Design reference and implementation status for the Lattice xdg-desktop-portal FileChooser backend.

**Status:** EXPERIMENTAL — all three methods implemented.

---

## How xdg-desktop-portal FileChooser works

`xdg-desktop-portal` is a D-Bus service that sits between sandboxed (Flatpak, Snap) and unsandboxed apps and the desktop environment. When an app needs a file dialog it calls the **frontend** portal interface; the portal daemon forwards the request to the active **backend** implementation for the running desktop.

```
App (any)
  │  calls org.freedesktop.portal.FileChooser.OpenFile(…)
  ▼
xdg-desktop-portal daemon (frontend)
  │  forwards to the backend selected by .portal descriptor
  ▼
lattice-filechooser-portal (backend)  ← this binary
  │  implements org.freedesktop.impl.portal.FileChooser
  │  spawns: lattice --picker <mode> [--path …] [--name …]
  ▼
Lattice picker window
  │  user interacts, prints path(s) to stdout, exits 0 or 1
  ▼
lattice-filechooser-portal
  │  reads stdout, normalizes paths, encodes as file:// URIs
  │  returns D-Bus reply
  ▼
App receives selected URIs
```

The backend is responsible for showing the actual UI and returning results. The frontend portal acts only as a broker.

Implementation note: `lattice-filechooser-portal` uses the same GTK/GIO stack as the main app. It registers the backend through GIO's GDBus APIs and launches the picker with `gio::Subprocess`, avoiding a separate async D-Bus runtime dependency.

---

## Method reference

### `OpenFile` ✅

```
OpenFile(
  handle:         o,
  app_id:         s,
  parent_window:  s,
  title:          s,
  options:        a{sv}
) → (response: u, results: a{sv})
```

| Option | D-Bus type | Support | Notes |
|--------|-----------|---------|-------|
| `multiple` | b | ✅ | Routes to `lattice --picker open-files` |
| `directory` | b | ✅ | Routes to `lattice --picker folder` |
| `current_folder` | ay | ✅ | Passed as `--path` |
| `accept_label` | s | logged | Not forwarded to picker UI |
| `filters` | a(sa(us)) | logged | Presence logged; not applied |
| `choices` | a(ssa(ss)s) | logged | Ignored |
| `writable` | b | logged | Ignored |

Picker subprocess used:
- default → `lattice --picker open [--path …]`
- `multiple=true` → `lattice --picker open-files [--path …]`
- `directory=true` → `lattice --picker folder [--path …]`

Result: `uris` (as) — one or more `file://` URIs of selected files/folder.  
Paths are `canonicalize`d (symlinks resolved, existence verified). Non-normalizable paths are silently dropped. If all paths drop, response=1.

---

### `SaveFile` ✅

```
SaveFile(
  handle, app_id, parent_window, title,
  options: a{sv}
) → (response: u, results: a{sv})
```

| Option | D-Bus type | Support | Notes |
|--------|-----------|---------|-------|
| `current_folder` | ay | ✅ | Initial directory (takes priority) |
| `current_file` | ay | ✅ | Fallback: parent dir → `--path`; filename → `--name` |
| `current_name` | s | ✅ | Suggested filename (overrides current_file name) |
| `filters` | a(sa(us)) | logged | Presence logged; not applied |
| `choices` | a(ssa(ss)s) | logged | Ignored |

Initial dir resolution: `current_folder` → parent of `current_file` → picker default (home).  
Suggested name resolution: `current_name` → filename of `current_file` → empty.

Picker subprocess: `lattice --picker save [--path <dir>] [--name <name>]`

Result: `uris` (as) — exactly one `file://` URI.  
Save paths may not exist yet; only the **parent directory** is canonicalized (must exist). The file itself is not checked.

---

### `SaveFiles` ✅

```
SaveFiles(
  handle, app_id, parent_window, title,
  options: a{sv}
) → (response: u, results: a{sv})
```

| Option | D-Bus type | Support | Notes |
|--------|-----------|---------|-------|
| `current_folder` | ay | ✅ | Initial directory for folder picker |
| `files` | aay | ✅ | List of destination filenames |
| `filters` | a(sa(us)) | logged | Ignored |

Picker subprocess: `lattice --picker folder [--path <dir>]`

Result construction:
- `files` non-empty → URIs are `chosen_dir/each_filename` (one per file)
- `files` absent → single URI for the chosen folder itself

Filenames containing `/` are rejected for safety. The destination folder is canonicalized; individual files are not checked for existence.

---

## Response codes

| Code | Meaning |
|------|---------|
| 0 | Success — `results` contains `uris` |
| 1 | Cancelled — user dismissed picker or no valid paths returned |
| 2 | Error — picker spawn failed or internal URI encoding failure |

---

## URI encoding

The portal encodes paths as `file://` URIs using a conservative encoder: only `A-Z a-z 0-9 / - _ . ~` pass through unencoded. All other bytes (space, `#`, `?`, `%`, non-ASCII, etc.) become `%XX` uppercase hex.

Example: `/home/user/my documents/file#1.txt` → `file:///home/user/my%20documents/file%231.txt`

---

## What apps this can and cannot affect

### Will use the Lattice portal

| App type | Condition |
|----------|-----------|
| **Flatpak apps** | Always — sandboxed apps always go through the portal |
| **GTK3/GTK4 apps using `GtkFileChooserNative`** (Inkscape 1.x, most GTK4 apps) | Requires `GTK_USE_PORTAL=1` in the environment |
| **Chrome / Chromium 104+** | Automatic on Wayland/X11 desktop sessions; no extra env var needed |
| **Electron apps** (Vesktop, etc.) | When the app enables portal file selection; varies by build |

### Will NOT use the Lattice portal

| App type | Reason |
|----------|--------|
| **GTK apps using `GtkFileChooserDialog` directly** (older GIMP, many GTK3 apps) | Portal is bypassed entirely — even `GTK_USE_PORTAL=1` has no effect |
| **Terminal apps** | No dialog involved |
| **Qt apps without libportal** | Use Qt's own file dialog |

### Setting `GTK_USE_PORTAL=1`

For immediate testing — launch the app from a terminal with the variable set:

```bash
GTK_USE_PORTAL=1 inkscape
GTK_USE_PORTAL=1 gimp
```

To set permanently for your user session (takes effect on next login):

```bash
mkdir -p ~/.config/environment.d
echo 'GTK_USE_PORTAL=1' > ~/.config/environment.d/lattice-portal.conf
```

To set it only for the current session without logging out:

```bash
export GTK_USE_PORTAL=1
inkscape  # inherits the var from this shell
```

---

## Known limitations with non-`file://` remote URIs

The xdg-desktop-portal FileChooser protocol returns results exclusively as `file://` URIs. This is a protocol-level constraint.

- **rclone/GVfs mounts** — mounted as local paths (e.g. `/run/user/1000/gvfs/smb:…`) — these work correctly; the app gets a local `file://` path.
- **Unmounted remote paths** — cannot be returned.
- **URIs like `smb://`, `sftp://`, `davs://`** — cannot be returned through the portal.
- **Virtual locations (`recent://`, `trash://`)** — have no `file://` equivalent; cannot be returned.

---

## Installation (EXPERIMENTAL)

> **This will not activate until you explicitly run the `--portal-config` step.**
> Installing the system files alone does not change any desktop behavior.

### Step 0 — Install the portal frontend and a fallback backend

The Lattice backend only implements **FileChooser**. It must sit alongside a
full portal frontend plus at least one backend that implements the other
interfaces (Settings, ScreenCast, …). The GTK backend is the usual fallback and
is what the generated `portals.conf` pairs Lattice with.

```bash
# Ubuntu/Pop!_OS
sudo apt install xdg-desktop-portal xdg-desktop-portal-gtk

# Arch/CachyOS/EndeavourOS
sudo pacman -S xdg-desktop-portal xdg-desktop-portal-gtk

# Fedora Workstation
sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk

# Void Linux
sudo xbps-install dbus xdg-desktop-portal xdg-desktop-portal-gtk
```

The installer does **not** assume the GTK backend is present just because the
frontend is. It checks for the `gtk` FileChooser descriptor and, if it is
missing, warns you and writes Lattice without a fallback rather than silently
breaking other portal apps. Install `xdg-desktop-portal-gtk` before opting in.

### Startup mechanism: systemd vs runit (automatic)

The backend must run **inside your graphical session** so it inherits
`WAYLAND_DISPLAY`/`DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR`, and
`XDG_CURRENT_DESKTOP` — D-Bus auto-activation alone does not reliably propagate
these. `install-portal.sh` detects the service manager (via `/run/systemd/system`)
and picks the right mechanism automatically:

- **systemd systems** → a `systemctl --user` service (`lattice-filechooser-portal.service`).
- **runit / non-systemd systems (Void)** → a per-user **XDG autostart entry** at
  `~/.config/autostart/lattice-filechooser-portal.desktop`, launched by the
  graphical session with the full session environment. No `systemctl` is used or
  required on these systems.

See [Void / runit notes](#void--runit-notes-no-systemd) below for the Void path.

### Step 1 — Build release binaries

```bash
cargo build --release
```

### Step 2 — Install system files and enable the service (needs sudo)

```bash
sudo ./scripts/install-portal.sh
```

This installs the following common files, then sets up startup with the
mechanism this system has (systemd service **or** XDG autostart — see above):

| File | Destination |
|------|-------------|
| `lattice` | `/usr/local/bin/lattice` |
| `lattice-filechooser-portal` | `/usr/local/lib/lattice/lattice-filechooser-portal` |
| `data/portals/lattice.portal` | `/usr/share/xdg-desktop-portal/portals/lattice.portal` |
| D-Bus service file | `/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.lattice.service` |
| Startup (systemd) | `/usr/lib/systemd/user/lattice-filechooser-portal.service` |
| Startup (runit/Void) | `~/.config/autostart/lattice-filechooser-portal.desktop` |

Only the startup file for the detected service manager is installed.

To skip the confirmation prompt: `sudo ./scripts/install-portal.sh --yes`

**Why a session-scoped startup?** xdg-desktop-portal calls the Lattice backend, which spawns `lattice --picker` — a GTK window that requires `WAYLAND_DISPLAY` (or `DISPLAY`) and the session bus. D-Bus auto-activation does not reliably propagate those, so the backend is started **inside the graphical session** — via a `systemctl --user` service on systemd, or an XDG autostart entry on runit systems.

**On systemd, if the service did not enable automatically**, run these as your normal user:

```bash
systemctl --user daemon-reload
systemctl --user enable --now lattice-filechooser-portal.service
systemctl --user status lattice-filechooser-portal.service
```

**On Void/runit**, the autostart entry starts the backend at your next graphical login. To start it now without logging out, run in your desktop session (not via sudo):

```sh
setsid -f /usr/local/lib/lattice/lattice-filechooser-portal
```

### Step 3 — Opt in to the portal backend (no sudo)

Run **as your normal user** (not root):

```bash
./scripts/install-portal.sh --portal-config
```

This generates `~/.config/xdg-desktop-portal/portals.conf`. It scans all installed
`.portal` files in `/usr/share/xdg-desktop-portal/portals/`, maps every interface to
its correct backend (respecting `XDG_CURRENT_DESKTOP`), and overrides only
`FileChooser` to use the Lattice backend with `gtk` as a fallback.

Example output on Pop!_OS COSMIC:

```ini
[preferred]
org.freedesktop.impl.portal.FileChooser=lattice;gtk

# Other interfaces preserved from installed portal backends:
org.freedesktop.impl.portal.Settings=cosmic
org.freedesktop.impl.portal.ScreenCast=cosmic
org.freedesktop.impl.portal.Screenshot=cosmic
# ... and so on for all other installed backends
```

**Why a generated file?** A user-level `portals.conf` completely overrides (not merges with) system auto-detection. Writing only a FileChooser line would silently displace all other backends — breaking dark mode (Settings), screen sharing (ScreenCast), and other portal interfaces for Chrome and other apps. The generated file preserves all existing interface→backend mappings.

If a `portals.conf` already exists, it is backed up to a timestamped `.bak` file before being replaced. If the existing file was not generated by this script, you will be prompted before it is overwritten.

### Step 4 — Restart xdg-desktop-portal

```bash
# systemd:
systemctl --user restart xdg-desktop-portal

# runit / Void (no systemctl) — it re-activates on the next portal call:
pkill -x xdg-desktop-portal
```

### Step 5 — Verify

```bash
# Backend is on the session bus (works on any init):
busctl --user list | grep lattice
# ...or without busctl:
dbus-send --session --dest=org.freedesktop.DBus --type=method_call --print-reply \
  / org.freedesktop.DBus.ListNames | grep lattice

# systemd only — service is running:
systemctl --user status lattice-filechooser-portal.service
# runit / Void — backend process is running:
pgrep -af lattice-filechooser-portal

# End-to-end test: should open the Lattice picker
./scripts/test-portal.sh
```

---

## Rollback

### Remove portals.conf (no sudo)

```bash
./scripts/install-portal.sh --remove-portal-config
systemctl --user restart xdg-desktop-portal   # systemd
# or, on runit/Void:  pkill -x xdg-desktop-portal
```

This deletes the generated `portals.conf` entirely; xdg-desktop-portal reverts to auto-detection.

### Remove system files and disable startup (sudo)

```bash
sudo ./scripts/install-portal.sh --uninstall
```

This removes the installed files and, depending on the system, disables the
systemd user service **or** removes the `~/.config/autostart` entry. It also
removes `/usr/local/lib/lattice/` if empty. It does **not** touch `portals.conf`.

### Manual rollback

```bash
# portals.conf — delete the generated file and restart the portal:
rm ~/.config/xdg-desktop-portal/portals.conf
systemctl --user restart xdg-desktop-portal   # or: pkill -x xdg-desktop-portal

# Startup entry:
#   systemd:
systemctl --user disable --now lattice-filechooser-portal.service
#   runit / Void:
rm -f ~/.config/autostart/lattice-filechooser-portal.desktop
pkill -x lattice-filechooser-portal

# System files:
sudo rm -f /usr/local/lib/lattice/lattice-filechooser-portal
sudo rm -f /usr/share/xdg-desktop-portal/portals/lattice.portal
sudo rm -f /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.lattice.service
sudo rm -f /usr/lib/systemd/user/lattice-filechooser-portal.service   # systemd only
```

---

## Troubleshooting

### Fedora notes and SELinux

The install destinations are the same on Fedora as elsewhere:

```text
/usr/local/lib/lattice/                          # lattice + portal binaries
/usr/share/xdg-desktop-portal/portals/           # lattice.portal descriptor
/usr/share/dbus-1/services/                       # D-Bus activation file
/usr/lib/systemd/user/                            # systemd user service
```

Fedora ships portal packages via `dnf` (RPM), which `install-portal.sh` detects
with `rpm -q` — Debian and Arch detection are unaffected.

Fedora service and bus diagnostics:

```bash
systemctl --user status lattice-filechooser-portal.service
systemctl --user status xdg-desktop-portal
journalctl --user -b -u lattice-filechooser-portal.service
journalctl --user -b -u xdg-desktop-portal
busctl --user list | grep -E 'portal|lattice'
```

**SELinux:** Fedora runs SELinux in enforcing mode. If the portal backend or the
picker it spawns is blocked, inspect the recent audit denials — **do not disable
SELinux**:

```bash
sudo ausearch -m AVC -ts recent
sudo journalctl -b -t setroubleshoot --no-pager
```

The Lattice binaries install to `/usr/local/lib/lattice/` and `/usr/local/bin/`,
which normally carry a permissive `bin_t`/`usr_t` context. If you see denials
tied to those paths, capture the AVC lines above and include them in your report
rather than turning SELinux off.

### Void / runit notes (no systemd)

Void uses **runit**. There is no `systemctl --user` and no `journalctl`, so the
systemd-oriented commands elsewhere in this doc do not apply. `install-portal.sh`
detects this automatically and installs a per-user XDG autostart entry instead of
a systemd unit.

**Startup entry:** `~/.config/autostart/lattice-filechooser-portal.desktop`
(installed by `sudo ./scripts/install-portal.sh`, removed by `--uninstall`).

**A session bus is required.** GVfs and the portal need `DBUS_SESSION_BUS_ADDRESS`
to be set. Full desktops (GNOME, XFCE, KDE) provide it; a minimal Wayland session
usually needs to be launched with `dbus-run-session <compositor>`. Do **not** nest
another `dbus-run-session` inside an already-working desktop. Also make sure the
system `dbus` service is up: `sv status dbus` (enable with
`sudo ln -s /etc/sv/dbus /var/service/`).

**Minimal Wayland compositors** (sway, labwc) do not read XDG autostart. Start the
backend from the compositor config instead:

```sh
# sway (~/.config/sway/config)
exec /usr/local/lib/lattice/lattice-filechooser-portal

# labwc (~/.config/labwc/autostart)
/usr/local/lib/lattice/lattice-filechooser-portal &
```

**Start it now** (in your session, not via sudo):

```sh
setsid -f /usr/local/lib/lattice/lattice-filechooser-portal
```

**Diagnostics without systemctl/journalctl:**

```sh
echo "${DBUS_SESSION_BUS_ADDRESS:-missing}"
echo "${XDG_RUNTIME_DIR:-missing}"
echo "${WAYLAND_DISPLAY:-${DISPLAY:-missing}}"

pgrep -af lattice-filechooser-portal
pgrep -af xdg-desktop-portal

busctl --user list 2>/dev/null | grep -E 'portal|lattice'
# busctl unavailable? ask the bus directly:
dbus-send --session --dest=org.freedesktop.DBus --type=method_call --print-reply \
  / org.freedesktop.DBus.ListNames | grep -E 'portal|lattice'

sv status dbus
ls -l /var/service/dbus
```

**Logs:** run the backend in the foreground to capture its stderr —
`/usr/local/lib/lattice/lattice-filechooser-portal 2>portal.log` — or read the
terminal you launched it from. There is no journald on Void.

The read-only helper `scripts/void-diagnostics.sh` collects all of the above.

### Check the Lattice portal service is running

```bash
systemctl --user status lattice-filechooser-portal.service
```

Expected: `active (running)`. If it shows `inactive` or `failed`:

```bash
systemctl --user daemon-reload
systemctl --user enable --now lattice-filechooser-portal.service
journalctl --user -u lattice-filechooser-portal -n 30 --no-pager
```

### Check portal daemon status

```bash
systemctl --user status xdg-desktop-portal
```

### Stream portal daemon logs

```bash
journalctl --user -u xdg-desktop-portal -f
```

### Verify the Lattice backend is on the session bus

After starting `lattice-filechooser-portal` (or after it's auto-activated via D-Bus):

```bash
busctl --user list | grep lattice
```

Expected output includes `org.freedesktop.impl.portal.desktop.lattice`.

### Check portal binary logs

The portal binary writes `[lattice-portal]` prefixed lines to stderr. When running as a D-Bus activated service:

```bash
journalctl --user | grep lattice-portal
```

When testing manually:

```bash
./target/debug/lattice-filechooser-portal 2>&1 | grep lattice-portal
```

### Picker opens but returns nothing

1. Check that `lattice` is in `$PATH` or in the same directory as `lattice-filechooser-portal`.
2. Check portal logs for "failed to spawn picker" or "no valid file:// URIs" messages.
3. Try calling `lattice --picker open` manually — it should open a picker window and print a path to stdout on confirm.

### App file picker is not Lattice even after setup

**First:** confirm the portal backend itself responds correctly:

```bash
./scripts/test-portal.sh
```

If this opens the Lattice picker and returns a URI, the backend is working. The problem is the app is not routing through the portal.

**For GTK apps (GIMP, Inkscape, etc.)** — set `GTK_USE_PORTAL=1`:

```bash
GTK_USE_PORTAL=1 inkscape   # test immediately
GTK_USE_PORTAL=1 gimp
```

Note: apps that use `GtkFileChooserDialog` directly (common in older GTK3 apps) ignore `GTK_USE_PORTAL=1` entirely — the portal is bypassed in their code. If the test script works but the app still doesn't use the portal, the app is in this category.

**For Chrome / Electron apps** — the portal is used natively without `GTK_USE_PORTAL=1`. If Chrome's file picker is still not Lattice, check portal logs while opening a Chrome file dialog:

```bash
journalctl --user -u xdg-desktop-portal -f &
# then open a file dialog in Chrome
```

### D-Bus service file not picked up

After installing the `.service` file, the session D-Bus daemon may not see it until it reloads:

```bash
dbus-send --session --type=method_call \
  --dest=org.freedesktop.DBus / org.freedesktop.DBus.ReloadConfig
```

If this isn't enough, a session logout/login will always pick it up.

### portals.conf change has no effect

Restart the portal daemon:

```bash
systemctl --user restart xdg-desktop-portal
```

If `xdg-desktop-portal` is not managed by systemd, kill and relaunch it:

```bash
pkill xdg-desktop-portal
# It will relaunch automatically when the next portal call is made,
# or launch it manually: /usr/libexec/xdg-desktop-portal &
```

### Wrong backend still active

Check what `portals.conf` currently says:

```bash
cat ~/.config/xdg-desktop-portal/portals.conf
# Also check the system-wide file:
cat /etc/xdg/xdg-desktop-portal/portals.conf 2>/dev/null
```

The user-level file takes precedence. `UseIn=lattice` in the `.portal` descriptor means auto-selection only applies to desktop sessions identified as `lattice`; all other desktops require an explicit `portals.conf` entry.

### Restore from backup

```bash
# List backups
ls ~/.config/xdg-desktop-portal/portals.conf.bak.*

# Restore a specific backup
cp ~/.config/xdg-desktop-portal/portals.conf.bak.20260521-120000 \
   ~/.config/xdg-desktop-portal/portals.conf
systemctl --user restart xdg-desktop-portal
```

---

## Testing

### Quick start

```bash
cargo build --bin lattice-filechooser-portal
./scripts/test-portal.sh   # starts portal, calls OpenFile via busctl/gdbus
```

### Manual OpenFile

```bash
# Terminal 1
./target/debug/lattice-filechooser-portal

# Terminal 2 — single file
busctl --user call \
  org.freedesktop.impl.portal.desktop.lattice \
  /org/freedesktop/portal/desktop \
  org.freedesktop.impl.portal.FileChooser \
  OpenFile "osssa{sv}" /org/test/1 test-app "" "Pick a file" 0

# Multi-select
busctl --user call … OpenFile "osssa{sv}" /org/test/2 test-app "" "Pick files" \
  1 "multiple" b true
```

### Manual SaveFile

```bash
# With current_name suggestion
busctl --user call \
  org.freedesktop.impl.portal.desktop.lattice \
  /org/freedesktop/portal/desktop \
  org.freedesktop.impl.portal.FileChooser \
  SaveFile "osssa{sv}" /org/test/3 test-app "" "Save As" \
  1 "current_name" s "my-document.txt"

# With current_file fallback (derives dir and name from existing path)
# current_file ay = "/tmp/old.txt\0"
busctl --user call … SaveFile "osssa{sv}" /org/test/4 test-app "" "Save As" \
  1 "current_file" ay 9 47 116 109 112 47 111 108 100 0
```

### Manual SaveFiles

```bash
# With files list: saves report.pdf + data.csv to a user-chosen folder
busctl --user call \
  org.freedesktop.impl.portal.desktop.lattice \
  /org/freedesktop/portal/desktop \
  org.freedesktop.impl.portal.FileChooser \
  SaveFiles "osssa{sv}" /org/test/5 test-app "" "Choose destination" \
  1 "files" aay 2 10 114 101 112 111 114 116 46 112 100 102 0 \
               8 100 97 116 97 46 99 115 118 0
# → returns file:///chosen/report.pdf and file:///chosen/data.csv
```

### gdbus alternative

```bash
gdbus call --session \
  --dest org.freedesktop.impl.portal.desktop.lattice \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.impl.portal.FileChooser.OpenFile \
  "/org/test/1" "test-app" "" "Pick a file" "@a{sv} {}"
```

### Log inspection

```bash
# Capture portal stderr
./target/debug/lattice-filechooser-portal 2>portal.log &
tail -f portal.log

# If installed as a systemd user service
journalctl --user -u lattice-filechooser-portal -f
```

Log lines are prefixed `[lattice-portal]`.

---

## Remaining work

- [ ] `filters` — file type filter bar in picker UI; then forward parsed MIME/glob to picker
- [ ] `accept_label` — forward to picker as custom confirm button label
- [ ] `parent_window` — parse X11/Wayland handle; set picker as transient
- [ ] `handle_token` — per-request subprocess tracking for `Close` cancellation
- [ ] Meson/Makefile install targets
- [ ] Systemd user service unit file
