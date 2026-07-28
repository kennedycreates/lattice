#!/usr/bin/env bash
# void-diagnostics.sh — read-only environment report for Lattice on Void Linux.
#
# Safe to run and safe to send to a Lattice maintainer. This script:
#   • makes NO changes — no writes, no installs, no mount/unmount/eject
#   • does NOT require sudo for the basic report
#   • does NOT crawl your home directory or print environment secrets
#   • does NOT dump the full package database (targeted xbps queries only)
#   • gracefully skips anything that is not installed
#
# Optional privileged checks are only PRINTED as suggestions at the end; this
# script never runs sudo for you.
#
# Usage:
#   ./scripts/void-diagnostics.sh
#   ./scripts/void-diagnostics.sh > lattice-void-diagnostics.txt   # to attach to a report

set -uo pipefail  # no -e: a missing tool must not abort the whole report

section() { printf '\n=== %s ===\n' "$1"; }
have()    { command -v "$1" >/dev/null 2>&1; }
try()     { if have "$1"; then "$@" 2>&1; else echo "(skipped: '$1' not installed)"; fi; }

section "Lattice Void diagnostics — $(date)"
echo "This report is read-only. No system changes were made."

# ── OS / arch / libc ──────────────────────────────────────────────────────────

section "Void release"
if [[ -r /etc/os-release ]]; then
    grep -E '^(NAME|PRETTY_NAME|ID)=' /etc/os-release
else
    echo "(no /etc/os-release)"
fi

section "Architecture / libc"
if have xbps-uhelper; then
    echo "xbps arch: $(xbps-uhelper arch 2>/dev/null || echo unknown)"
fi
uname -m
# musl vs glibc: musl's ldd prints "musl libc"; glibc prints "GNU libc".
_libc="unknown"
if have ldd; then
    if ldd --version 2>&1 | grep -qi musl; then
        _libc="musl"
    elif ldd --version 2>&1 | grep -qiE 'glibc|gnu libc'; then
        _libc="glibc"
    fi
fi
# Fallback: arch string carries a -musl suffix on musl systems.
if [[ "$_libc" == "unknown" ]] && have xbps-uhelper; then
    case "$(xbps-uhelper arch 2>/dev/null)" in
        *-musl) _libc="musl" ;;
        *)      _libc="glibc" ;;
    esac
fi
echo "libc: $_libc"
ldd --version 2>&1 | head -1

section "Kernel"
uname -a

# ── Desktop / session / D-Bus ─────────────────────────────────────────────────

section "Desktop / session / D-Bus"
echo "Desktop:      ${XDG_CURRENT_DESKTOP:-unknown}"
echo "Session type: ${XDG_SESSION_TYPE:-unknown}"   # wayland or x11
echo "Display:      ${WAYLAND_DISPLAY:-${DISPLAY:-none}}"
echo "XDG runtime:  ${XDG_RUNTIME_DIR:-missing}"
# Report only whether the session bus address is set, never its value (secret-ish).
if [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
    echo "Session D-Bus: present (DBUS_SESSION_BUS_ADDRESS is set)"
else
    echo "Session D-Bus: MISSING — GVfs/portals need a session bus."
    echo "  Full desktops set this automatically; a minimal Wayland session may"
    echo "  need to be launched with: dbus-run-session <compositor>"
fi

# ── Toolchain ─────────────────────────────────────────────────────────────────

section "Rust toolchain"
try rustc --version
try cargo --version
have rustfmt && rustfmt --version 2>&1 || echo "(rustfmt not found — Void ships it in the 'rust' package)"

section "GTK / GLib"
if have pkg-config; then
    echo "gtk4:  $(pkg-config --modversion gtk4 2>/dev/null || echo '(not found via pkg-config)')"
    echo "glib2: $(pkg-config --modversion glib-2.0 2>/dev/null || echo '(not found via pkg-config)')"
else
    echo "(skipped: pkg-config not installed)"
fi

# ── Relevant package versions (targeted xbps queries; never the full DB) ───────

section "Relevant package versions (xbps)"
if have xbps-query; then
    for pkg in gtk4 glib gvfs gvfs-smb gvfs-mtp gvfs-gphoto2 udisks2 polkit dbus \
               desktop-file-utils gtk-update-icon-cache xdg-utils \
               xdg-desktop-portal xdg-desktop-portal-gtk \
               ffmpeg ImageMagick rclone fuse3 \
               noto-fonts-ttf noto-fonts-emoji rust cargo; do
        printf '%-26s %s\n' "$pkg" "$(xbps-query -p pkgver "$pkg" 2>/dev/null || echo 'not installed')"
    done
else
    echo "(skipped: xbps-query not available — not a Void system)"
fi

# ── Lattice resolution ────────────────────────────────────────────────────────

section "Lattice binary resolution"
if have lattice; then
    command -v lattice
else
    echo "'lattice' not in PATH (may only be a source-tree cargo build)"
fi

section "Desktop file resolution"
_found_desktop=false
for d in "$HOME/.local/share/applications" /usr/local/share/applications /usr/share/applications; do
    if [[ -f "$d/com.lattice.filemanager.desktop" ]]; then
        echo "found: $d/com.lattice.filemanager.desktop"
        _found_desktop=true
    fi
done
$_found_desktop || echo "com.lattice.filemanager.desktop not found in standard locations"

section "Default folder handler (inode/directory)"
try xdg-mime query default inode/directory

section "Icon lookup (lattice)"
_found_icon=false
for d in "$HOME/.local/share/icons" /usr/local/share/icons /usr/share/icons; do
    if [[ -f "$d/hicolor/256x256/apps/lattice.png" ]]; then
        echo "found: $d/hicolor/256x256/apps/lattice.png"
        _found_icon=true
    fi
done
$_found_icon || echo "lattice.png not found in hicolor 256x256/apps locations"

# ── Capability probes (what commands are actually available) ──────────────────

section "Available terminal emulators (Open Terminal Here)"
_any_term=false
for t in kitty x-terminal-emulator ptyxis kgx gnome-terminal konsole \
         xfce4-terminal foot alacritty wezterm xterm; do
    if have "$t"; then echo "  $t"; _any_term=true; fi
done
$_any_term || echo "  (none found — set LATTICE_TERMINAL to your terminal)"

section "Available icon-cache command"
_any_ic=false
for c in gtk-update-icon-cache gtk4-update-icon-cache; do
    if have "$c"; then echo "  $c"; _any_ic=true; fi
done
$_any_ic || echo "  (none found — install gtk-update-icon-cache)"

section "Available unmount commands (rclone / FUSE cloud unmount)"
_any_um=false
for u in fusermount3 fusermount umount; do
    if have "$u"; then echo "  $u"; _any_um=true; fi
done
$_any_um || echo "  (none found)"

# ── Storage plumbing (read-only) ──────────────────────────────────────────────

section "GVfs mounts (read-only)"
try gio mount -l

section "Trash backend (read-only)"
# Confirm the trash backend responds WITHOUT dumping your trashed filenames into
# a shareable report. We report only reachability and an item count.
if have gio; then
    if _trash="$(gio list trash:/// 2>&1)"; then
        echo "trash:/// is reachable — $(printf '%s\n' "$_trash" | grep -c . ) entries (names withheld)"
    else
        echo "trash:/// not reachable: $_trash"
    fi
else
    echo "(skipped: gio not installed)"
fi

section "UDisks status (read-only)"
try udisksctl status

section "Block devices (read-only)"
try lsblk -f

# ── D-Bus / service state (runit-aware, no journalctl) ────────────────────────

section "D-Bus service state (runit)"
if have sv; then
    echo "sv status dbus:"
    sv status dbus 2>&1 || echo "  (dbus not managed by runit here)"
fi
if [[ -e /var/service/dbus ]]; then
    ls -l /var/service/dbus 2>&1
else
    echo "/var/service/dbus: not present"
fi

# ── Portal state ──────────────────────────────────────────────────────────────

section "Portal processes"
if have pgrep; then
    echo "lattice-filechooser-portal:"
    pgrep -af lattice-filechooser-portal 2>/dev/null || echo "  (not running)"
    echo "xdg-desktop-portal:"
    pgrep -af xdg-desktop-portal 2>/dev/null || echo "  (not running)"
else
    echo "(pgrep not available)"
fi

section "Portal descriptors"
for p in gtk lattice; do
    f="/usr/share/xdg-desktop-portal/portals/$p.portal"
    if [[ -f "$f" ]]; then echo "  present: $f"; else echo "  absent:  $f"; fi
done

section "Portal D-Bus names on the session bus"
if have busctl; then
    busctl --user list 2>/dev/null | grep -E 'portal|lattice' || echo "  (none found via busctl)"
elif have dbus-send; then
    # Fallback when busctl is unavailable: ask the bus directly for names.
    dbus-send --session --dest=org.freedesktop.DBus --type=method_call --print-reply \
        / org.freedesktop.DBus.ListNames 2>/dev/null \
        | grep -E 'portal|lattice' || echo "  (none found via dbus-send)"
else
    echo "  (neither busctl nor dbus-send available)"
fi

section "User portals.conf"
_conf="$HOME/.config/xdg-desktop-portal/portals.conf"
if [[ -f "$_conf" ]]; then
    echo "present: $_conf"
    grep -E 'FileChooser|Generated by' "$_conf" 2>/dev/null || true
else
    echo "not present (portal not opted in) — $_conf"
fi

# ── Optional privileged diagnostics (NOT run automatically) ───────────────────

section "Optional privileged diagnostics (run yourself if needed)"
cat <<'EOF'
These need root and are NOT run by this script:

  sudo xbps-query -Rs <package>      # check a package in the remote repos
  sudo udevadm monitor               # watch device add/remove events live

Void uses runit, not systemd — do not expect `journalctl`/`systemctl`.
Capture the terminal output of the backend for portal logs, e.g.:

  /usr/local/lib/lattice/lattice-filechooser-portal 2>portal.log &
EOF

section "End of report"
echo "Attach the full output above to your bug report."
