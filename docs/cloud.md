# Cloud Drives in Lattice

Lattice's Cloud section lets you register mounted cloud and remote drives as named entries in the sidebar. The first-pass implementation uses **mounted filesystem locations** — it does not communicate with provider APIs directly.

Any path accessible as a local directory (or GIO/GVfs URI) can be added: rclone mounts, pCloud Drive, GVfs network locations, SFTP/FTP/WebDAV via fusermount or gvfs backends, and any other mounted remote location.

---

## How It Works

Cloud entries are stored in Lattice's local metadata database (`~/.local/share/lattice/metadata.db`). Each entry has:

- **Name** — display name shown in the sidebar
- **Path / URI** — absolute mount path (e.g. `/mnt/gdrive`) or GIO URI (e.g. `sftp://host/path`)
- **Kind** — label: `rclone`, `pcloud`, `gvfs`, `sftp`, `ftp`, `webdav`, or `manual`
- **Notes** — optional free-text notes

Clicking a Cloud entry opens a landing view. From there you can:

- **Open Drive** — browse files
- **Space Viewer** — analyze disk usage
- **Triage** — sort files by date, type, or size
- **Add to Palette** — pin the cloud drive to a Palette board
- **Edit** / **Remove** — manage the entry

All standard Lattice tools (Tints, Shapes, Tags, Action Plans, Holding Tray, Activity Log, Palettes) work normally on cloud paths — they operate on file paths and require no cloud-specific changes.

---

## Recommended Mount Approaches

### rclone

rclone is a command-line tool for mounting many cloud providers (Google Drive, Dropbox, OneDrive, S3, SFTP, and more) as local filesystems.

1. Install rclone: `sudo pacman -S rclone` (Arch), `sudo apt install rclone` (Ubuntu), `sudo dnf install rclone` (Fedora), or `sudo xbps-install rclone` (Void)
2. Configure a remote: `rclone config`
3. Mount it:
   ```bash
   mkdir -p ~/mnt/gdrive
   rclone mount gdrive: ~/mnt/gdrive --vfs-cache-mode writes &
   ```
4. In Lattice, add a Cloud entry with Path `~/mnt/gdrive` and Kind `rclone`.

Lattice does not manage rclone credentials. Use `rclone config` to manage remotes.

### pCloud Drive

pCloud Drive provides a native desktop client that mounts your pCloud storage as a local drive, typically at `/run/user/<uid>/pCloudDrive` or a user-configured path.

1. Install and log into the [pCloud Drive desktop app](https://www.pcloud.com/download-free-online-cloud-file-storage.html).
2. Note the mount path (shown in pCloud Drive settings).
3. In Lattice, add a Cloud entry with that path and Kind `pcloud`.

### GVfs / GIO Network Locations

GNOME's GVfs provides transparent mounting of network locations. When a remote volume is mounted via Nautilus, Thunar, or `gio mount`, it appears under `/run/user/<uid>/gvfs/`.

```bash
gio mount sftp://user@host/path
ls /run/user/$(id -u)/gvfs/
```

In Lattice, add the GVfs path with Kind `gvfs`. You can also use a GIO URI directly (e.g. `sftp://host/path`), though availability detection for URIs uses GIO and may require GVfs to be running.

### SFTP / FTP / WebDAV via fusermount

Many tools mount remote filesystems via FUSE:

```bash
# SFTP with sshfs
sshfs user@host:/path ~/mnt/remote -o reconnect

# WebDAV with davfs2
mount.davfs https://dav.example.com ~/mnt/webdav
```

After mounting, add the local path as a Cloud entry in Lattice.

---

## Cloud Scan Performance

Cloud and network drives can be significantly slower than local storage. Lattice's Space Viewer and Triage show progress indicators and can be cancelled — use them with care on large remote directories. Recursive scans over slow connections may take a long time.

---

## rclone Awareness

Lattice can detect whether rclone is installed and list your configured remote names — without reading credentials or config contents.

**How to access:** In the sidebar, expand the **CLOUD** section and click **⚙ rclone Remotes**.

What the dialog shows:
- Whether rclone is installed (`rclone version --quiet`)
- Your configured remote names (`rclone listremotes` — names only, no config or credentials)
- A copy-ready mount command for each remote
- Step-by-step mount guidance

**Suggested mount path:** `~/Cloud/Lattice/<remote-name>`

**Example mount command (copy from the dialog or run manually):**
```bash
rclone mount myremote: ~/Cloud/Lattice/myremote --vfs-cache-mode writes
```

Keep the mount process running, then use **Add Cloud Drive** or click **Add to Cloud** in the rclone dialog to register the folder in Lattice.

Lattice never reads rclone credentials and never runs mount commands. `rclone config` manages all credentials.

---

## GVfs / GIO URI Support

Lattice accepts GIO/GVfs remote URIs directly as the **Path** field for a Cloud entry. Supported schemes:

| URI scheme | Kind auto-detected | GVfs backend needed |
|---|---|---|
| `sftp://user@host/path` | sftp | gvfs-fuse (usually bundled) |
| `ftp://host/path` | ftp | gvfs-fuse |
| `smb://host/share` | gvfs | gvfs-smb |
| `dav://host/path` or `davs://host/path` | webdav | gvfs (built-in) |

When you type a supported URI into the **Path** field in the Add Cloud Drive form, the **Kind** dropdown is auto-selected.

### Required packages

**Arch / CachyOS / EndeavourOS:**
```bash
sudo pacman -S gvfs gvfs-smb
```

For SFTP/FTP access, `gvfs-fuse` is also needed on some systems:
```bash
sudo pacman -S gvfs-fuse   # if sftp:// doesn't work without it
```

After installing, restart the desktop session (log out/in or `systemctl restart --user gvfs-daemon` if available).

**Ubuntu / Debian / Pop!_OS:**
```bash
sudo apt install gvfs gvfs-backends
```

**Fedora Workstation:**
```bash
sudo dnf install gvfs gvfs-smb gvfs-fuse
```

**Void Linux:**
```bash
sudo xbps-install gvfs gvfs-smb
```

On Void the backends are split into separate packages, and the base `gvfs`
package already includes the FUSE bridge plus the `sftp://`, `ftp://`, `dav://`,
and `davs://` backends. Add only the extra backends you need:

| Backend | Void package |
|---|---|
| SFTP / FTP / WebDAV, Trash, FUSE bridge | `gvfs` (base) |
| SMB / CIFS (`smb://`) | `gvfs-smb` |
| MTP devices (phones) | `gvfs-mtp` |
| PTP cameras / some media players | `gvfs-gphoto2` |
| Apple mobile devices | `gvfs-afc` |

`gvfs-fuse` (Ubuntu/Fedora) exposes remotes under `/run/user/$(id -u)/gvfs/` so
full file operations work; `gvfs-smb` adds SMB. After installing, log out/in so
the session picks up the new backends. On Void, also confirm the `dbus` service
is running (`sv status dbus`).

### How Open Drive works for URI Cloud entries

1. Lattice calls `gio::File::for_uri(uri).path()` to resolve the URI to a local GVfs FUSE mount path (e.g., `/run/user/1000/gvfs/sftp:host=server,user=alice`).
2. If resolved, Lattice navigates to that local path — all normal file operations work.
3. If unresolved (GVfs not running or remote not yet connected), Lattice navigates directly via the URI. GVfs triggers authentication in the background (the desktop may show a credentials dialog from gnome-keyring or polkit). Once authenticated, the directory contents load normally.

### Troubleshooting GVfs remotes

If a URI Cloud entry shows "GVfs remote is unavailable":

1. **Test from terminal:**
   ```bash
   gio mount sftp://user@host/path
   gio mount -l
   ```
2. **Check GVfs backends are installed** (see packages above).
3. **Verify connectivity** — the remote host must be reachable.
4. **Credential expiry** — re-run `gio mount <uri>` to re-authenticate.
5. **Diagnostics:**
   ```bash
   ls /run/user/$(id -u)/gvfs/
   gio list sftp://user@host/path
   ```

### What you can do in URI-backed Cloud entries

- Browse the directory tree
- Open files (GIO passes the URI to the default application)
- Check availability on the Cloud landing page

Destructive file operations (rename, move, copy, trash) on files without a local FUSE path are not currently supported. To enable full file operations, ensure GVfs FUSE is running so files appear under `/run/user/.../gvfs/`.

---

## Tool Cloud Support Reference

Every major Lattice tool has been audited for cloud behavior. Here is the current state:

| Tool | Cloud Support | Notes |
|---|---|---|
| **Directory browser** | ✅ Full | Status bar shows cloud badge; context menu shows cloud header |
| **Search** | ✅ Cloud-aware | Status badge + "may be slow" message; results fully actionable |
| **Space Viewer** | ✅ Cloud-aware | Cloud badge + caution message; cancel is available |
| **Triage** | ✅ Cloud-aware | Cloud badge + "may be slow" message; trash failures reported |
| **Holding Tray** | ✅ Cloud-aware | Cloud badge on tray items; action plans include cloud note |
| **Action Plans** | ✅ Cloud-aware | All queued plans annotated with cloud note automatically |
| **Activity Log** | ✅ Cloud-aware | Cloud summaries use ☁ prefix; receipts record cloud context |
| **Tints & Tags** | ✅ Works | Mark metadata is local to Lattice; works on any mounted path |
| **Painting Mode** | ✅ Cloud-aware | Recursive paint confirmation includes cloud caution note |
| **Palettes** | ✅ Works | Cards can hold cloud file/folder paths; open/reveal delegates to OS |
| **Preview Pane** | ✅ Works | Existing size limits (64 KB text) apply; large files show metadata only |
| **Trash** | ✅ Cloud-aware | Post-failure modal explains unsupported Trash; never auto-deletes |
| **Context Menus** | ✅ Cloud-aware | Cloud drive header; Terminal disabled for URI paths |

### Known limitations

- **Recursive scans (Space Viewer, Search, Triage, Paint Contents)** use `std::fs::read_dir`, which works for FUSE-mounted cloud paths but not for unmounted GVfs URI paths. URI-based Cloud entries must be FUSE-mounted before tools can scan them.
- **Mark metadata** (`~/.local/share/lattice/metadata.db`) is stored by absolute local path. Marks survive as long as the mount path stays consistent. If a FUSE mount path changes, marks are orphaned.
- **Trash** may be unsupported on some cloud providers (rclone, SFTP, FTP). Lattice reports the failure and never silently deletes.
- **Terminal Here** is disabled for raw GVfs URI paths (e.g., `sftp://host/path`). It works normally for FUSE-mounted cloud paths (e.g., `/run/user/.../gvfs/sftp:.../`).
- **File operations** (copy, move, rename) work on FUSE-mounted cloud paths but not on files whose path is a raw GIO URI without a local FUSE bridge.
- **Preview** works for cloud files via FUSE; large files are subject to the same 64 KB text limit as local files.

---

## What Is NOT Implemented in This Version

- Direct Google Drive API
- Direct pCloud API
- Direct Dropbox / OneDrive APIs
- Full rclone mount manager UI
- Background indexing daemon
- Automatic sync or reconnection engine

Future versions may add Cloud Profiles that store provider identity and relative paths, so entries survive if the mount path changes.
