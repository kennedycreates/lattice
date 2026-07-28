#!/usr/bin/env bash
# fedora-diagnostics.sh — read-only environment report for Lattice on Fedora.
#
# Safe to run and safe to send to a Lattice maintainer. This script:
#   • makes NO changes — no writes, no installs, no mount/unmount/eject
#   • does NOT require sudo for the basic report
#   • does NOT list your files or print environment secrets
#   • gracefully skips anything that is not installed
#
# Optional privileged SELinux checks are only PRINTED as suggested commands at
# the end; this script never runs sudo for you.
#
# Usage:
#   ./scripts/fedora-diagnostics.sh
#   ./scripts/fedora-diagnostics.sh > lattice-diagnostics.txt   # to attach to a report

set -uo pipefail  # no -e: a missing tool must not abort the whole report

section() { printf '\n=== %s ===\n' "$1"; }
have()    { command -v "$1" >/dev/null 2>&1; }
# Run a command if it exists; otherwise print a skip note.
try()     { if have "$1"; then "$@" 2>&1; else echo "(skipped: '$1' not installed)"; fi; }

section "Lattice diagnostics — $(date)"
echo "This report is read-only. No system changes were made."

# ── OS / kernel / session ─────────────────────────────────────────────────────

section "Fedora release"
if [[ -r /etc/fedora-release ]]; then
    cat /etc/fedora-release
else
    echo "(not a Fedora system — /etc/fedora-release absent)"
    [[ -r /etc/os-release ]] && grep -E '^(NAME|VERSION)=' /etc/os-release
fi

section "Kernel"
uname -a

section "Desktop / session"
echo "Desktop:   ${XDG_CURRENT_DESKTOP:-unknown}"
echo "Session:   ${XDG_SESSION_TYPE:-unknown}"   # wayland or x11
echo "Session desktop: ${XDG_SESSION_DESKTOP:-unknown}"

# ── Toolchain ─────────────────────────────────────────────────────────────────

section "Rust toolchain"
try rustc --version
try cargo --version
have rustfmt && rustfmt --version 2>&1 || echo "(rustfmt not installed)"

section "GTK / GLib"
if have pkg-config; then
    echo "gtk4:  $(pkg-config --modversion gtk4 2>/dev/null || echo '(not found via pkg-config)')"
    echo "glib2: $(pkg-config --modversion glib-2.0 2>/dev/null || echo '(not found via pkg-config)')"
else
    echo "(skipped: pkg-config not installed — install pkgconf-pkg-config)"
fi

# ── Relevant package versions ─────────────────────────────────────────────────

section "Relevant package versions (rpm)"
if have rpm; then
    for pkg in gtk4 glib2 gvfs gvfs-fuse gvfs-smb udisks2 polkit \
               desktop-file-utils gtk-update-icon-cache \
               xdg-desktop-portal xdg-desktop-portal-gtk \
               ffmpeg-free ImageMagick rclone fuse3 \
               google-noto-sans-fonts jetbrains-mono-fonts; do
        printf '%-28s %s\n' "$pkg" "$(rpm -q "$pkg" 2>/dev/null || echo 'not installed')"
    done
else
    echo "(skipped: rpm not available — not an RPM system)"
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
if have gtk-launch || true; then :; fi
_found_icon=false
for d in "$HOME/.local/share/icons" /usr/local/share/icons /usr/share/icons; do
    if [[ -f "$d/hicolor/256x256/apps/lattice.png" ]]; then
        echo "found: $d/hicolor/256x256/apps/lattice.png"
        _found_icon=true
    fi
done
$_found_icon || echo "lattice.png not found in hicolor 256x256/apps locations"

# ── Storage plumbing (read-only) ──────────────────────────────────────────────

section "GVfs mounts (read-only)"
try gio mount -l

section "UDisks status (read-only)"
try udisksctl status

section "Block devices (read-only)"
try lsblk -f

# ── Portal status ─────────────────────────────────────────────────────────────

section "Portal packages / services"
if have rpm; then
    echo "xdg-desktop-portal:     $(rpm -q xdg-desktop-portal 2>/dev/null || echo 'not installed')"
    echo "xdg-desktop-portal-gtk: $(rpm -q xdg-desktop-portal-gtk 2>/dev/null || echo 'not installed')"
fi
if [[ -f /usr/share/xdg-desktop-portal/portals/gtk.portal ]]; then
    echo "gtk.portal descriptor:  present (/usr/share/xdg-desktop-portal/portals/gtk.portal)"
else
    echo "gtk.portal descriptor:  MISSING — install xdg-desktop-portal-gtk for a FileChooser fallback"
fi
if [[ -f /usr/share/xdg-desktop-portal/portals/lattice.portal ]]; then
    echo "lattice.portal descriptor: present (portal backend installed)"
else
    echo "lattice.portal descriptor: not installed (portal backend not installed — this is normal)"
fi
if have systemctl; then
    echo "--- lattice-filechooser-portal.service ---"
    systemctl --user status lattice-filechooser-portal.service --no-pager 2>&1 | head -n 6 \
        || echo "(service not present)"
    echo "--- xdg-desktop-portal ---"
    systemctl --user status xdg-desktop-portal --no-pager 2>&1 | head -n 6 \
        || echo "(service not present)"
fi

# ── Recent errors (read-only) ─────────────────────────────────────────────────

section "Recent Lattice / GVfs / portal log lines (last 15 min)"
if have journalctl; then
    journalctl --user -b --since "15 minutes ago" --no-pager 2>/dev/null \
        | grep -iE 'lattice|gvfs|udisks|portal' | tail -n 40 \
        || echo "(no matching recent log lines)"
else
    echo "(skipped: journalctl not available)"
fi

# ── Optional privileged diagnostics (NOT run automatically) ───────────────────

section "Optional privileged diagnostics (run yourself if needed)"
cat <<'EOF'
These require sudo and are NOT run by this script. If Lattice or the portal is
being blocked and you suspect SELinux, inspect recent denials (do NOT disable
SELinux):

  sudo ausearch -m AVC -ts recent
  sudo journalctl -b -t setroubleshoot --no-pager

Report the AVC lines alongside this diagnostics output.
EOF

section "End of report"
echo "Attach the full output above to your bug report."
