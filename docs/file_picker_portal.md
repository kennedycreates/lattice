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
| `filters` | a(sa(us)) | logged | Filter names decoded and printed; not applied |
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
| `filters` | a(sa(us)) | logged | Filter names decoded and printed; not applied |
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

### Will use the Lattice portal (when configured as the active backend)

- GTK4 apps calling `gtk::FileDialog` or `FileChooserNative` with `GTK_USE_PORTAL=1` or inside a Flatpak/Snap sandbox
- Any app calling `org.freedesktop.portal.FileChooser` directly (KDE apps, Electron apps with portal support, etc.)
- Flatpak-packaged apps (always route file dialogs through the portal)

### Will NOT use the Lattice portal

- Apps embedding `gtk::FileChooserWidget` directly (portal bypassed entirely)
- Terminal apps using readline/path completion
- GTK3 apps not built against `GtkFileChooserNative`

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
> The install script is safe to run even on a production machine — it only becomes
> active after you opt in.

### Step 1 — Build release binaries

```bash
cargo build --release
```

### Step 2 — Install system files (needs sudo)

```bash
sudo ./scripts/install-portal.sh
```

This installs:

| File | Destination |
|------|-------------|
| `lattice` | `/usr/local/bin/lattice` |
| `lattice-filechooser-portal` | `/usr/local/lib/lattice/lattice-filechooser-portal` |
| `data/portals/lattice.portal` | `/usr/share/xdg-desktop-portal/portals/lattice.portal` |
| D-Bus service file | `/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.lattice.service` |

To skip the confirmation prompt: `sudo ./scripts/install-portal.sh --yes`

### Step 3 — Opt in to the portal backend (no sudo)

Run **as your normal user** (not root):

```bash
./scripts/install-portal.sh --portal-config
```

This appends to `~/.config/xdg-desktop-portal/portals.conf`:

```ini
[preferred]
# EXPERIMENTAL: Use Lattice file picker, fall back to gtk if unavailable.
org.freedesktop.impl.portal.FileChooser=lattice;gtk
```

The `lattice;gtk` value means: try the Lattice backend first; if it's unavailable or fails, fall back to the `gtk` backend. This prevents a broken picker from locking you out of file dialogs.

If a `portals.conf` already exists, the script backs it up to a timestamped `.bak` file before touching it. If a `FileChooser=` line already exists in the file, the script exits without modifying it and asks you to resolve the conflict manually.

### Step 4 — Restart xdg-desktop-portal

```bash
systemctl --user restart xdg-desktop-portal
```

`portals.conf` changes do not take effect until the portal daemon is restarted.

---

## Rollback

### Remove portals.conf entry (no sudo)

```bash
./scripts/install-portal.sh --remove-portal-config
systemctl --user restart xdg-desktop-portal
```

### Remove system files (sudo)

```bash
sudo ./scripts/install-portal.sh --uninstall
```

This removes the four installed system files and the `/usr/local/lib/lattice/` directory if empty. It does **not** touch `portals.conf`.

### Manual rollback

```bash
# portals.conf — remove the Lattice line(s) and restart:
nano ~/.config/xdg-desktop-portal/portals.conf
systemctl --user restart xdg-desktop-portal

# System files:
sudo rm -f /usr/local/lib/lattice/lattice-filechooser-portal
sudo rm -f /usr/share/xdg-desktop-portal/portals/lattice.portal
sudo rm -f /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.lattice.service
```

---

## Troubleshooting

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
- [ ] Workspace refactor to scope `zbus`/`tokio` deps to portal binary only
