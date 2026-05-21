#!/usr/bin/env bash
# install-portal.sh — EXPERIMENTAL Lattice FileChooser portal backend installer
#
# Usage (run from the lattice source tree):
#
#   sudo ./scripts/install-portal.sh            # install system files
#   sudo ./scripts/install-portal.sh --yes      # skip confirmation prompt
#
#   ./scripts/install-portal.sh --portal-config         # opt-in portals.conf
#   ./scripts/install-portal.sh --remove-portal-config  # revert portals.conf
#
#   sudo ./scripts/install-portal.sh --uninstall  # remove system files
#
# The system install and the portals.conf step are intentionally separate
# so that portals.conf is always written to your real home directory, not
# root's, even if you forget and run the whole thing with sudo.
#
# Build the binaries first:
#   cargo build --release

set -euo pipefail

# ── Helpers ───────────────────────────────────────────────────────────────────

die()  { echo "ERROR: $*" >&2; exit 1; }
warn() { echo "WARN:  $*" >&2; }
info() { echo "  $*"; }

# Resolve script directory to locate source files regardless of cwd
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Argument parsing ──────────────────────────────────────────────────────────

MODE="install"  # default
YES=false

for arg in "$@"; do
    case "$arg" in
        --portal-config)        MODE="portal-config" ;;
        --remove-portal-config) MODE="remove-portal-config" ;;
        --uninstall)            MODE="uninstall" ;;
        --yes|-y)               YES=true ;;
        --help|-h)
            sed -n '2,/^set -/p' "${BASH_SOURCE[0]}" | grep '^#' | sed 's/^# \?//'
            exit 0
            ;;
        *) die "Unknown argument: $arg. Run with --help for usage." ;;
    esac
done

# ── Install destinations ──────────────────────────────────────────────────────

DEST_LATTICE=/usr/local/bin/lattice
DEST_PORTAL=/usr/local/lib/lattice/lattice-filechooser-portal
DEST_PORTAL_FILE=/usr/share/xdg-desktop-portal/portals/lattice.portal
DEST_DBUS_SERVICE=/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.lattice.service

# ── Mode: system install ──────────────────────────────────────────────────────

do_install() {
    [[ $EUID -eq 0 ]] || die "System install requires sudo. Run: sudo $0"

    echo ""
    echo "╔══════════════════════════════════════════════════════════════════╗"
    echo "║   ⚠  EXPERIMENTAL — Lattice FileChooser portal backend          ║"
    echo "║                                                                  ║"
    echo "║  This installs a portal backend that replaces the system file   ║"
    echo "║  picker for apps that use xdg-desktop-portal, but ONLY after    ║"
    echo "║  you explicitly run the --portal-config step.                   ║"
    echo "║                                                                  ║"
    echo "║  Rollback:  sudo $0 --uninstall              ║"
    echo "╚══════════════════════════════════════════════════════════════════╝"
    echo ""

    if [[ "$YES" != "true" ]]; then
        read -r -p "Continue? [y/N] " answer
        [[ "$answer" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 0; }
    fi

    # Verify release binaries exist
    REL="$SOURCE_ROOT/target/release"
    [[ -f "$REL/lattice" ]] \
        || die "Release binary not found: $REL/lattice\nBuild with: cargo build --release"
    [[ -f "$REL/lattice-filechooser-portal" ]] \
        || die "Release binary not found: $REL/lattice-filechooser-portal\nBuild with: cargo build --release"

    # Warn if xdg-desktop-portal is not installed.
    # The binary is not in PATH on most systems (/usr/libexec/), and
    # `systemctl --user` is unreliable under sudo, so check known paths +
    # package databases instead.
    _xdp_found=false
    for _c in \
        /usr/libexec/xdg-desktop-portal \
        /usr/lib/xdg-desktop-portal/xdg-desktop-portal \
        /usr/bin/xdg-desktop-portal \
        /usr/local/bin/xdg-desktop-portal; do
        [[ -x "$_c" ]] && { _xdp_found=true; break; }
    done
    if ! $_xdp_found; then
        dpkg-query -W -f='${Status}' xdg-desktop-portal 2>/dev/null \
            | grep -q "install ok installed" && _xdp_found=true
        pacman -Qq xdg-desktop-portal &>/dev/null 2>&1 && _xdp_found=true
        rpm -q xdg-desktop-portal &>/dev/null 2>&1 && _xdp_found=true
    fi
    if ! $_xdp_found; then
        warn "xdg-desktop-portal does not appear to be installed."
        warn "The portal backend will not be callable until it is installed."
        warn "  Ubuntu/Pop!_OS: sudo apt install xdg-desktop-portal xdg-desktop-portal-gtk"
        warn "  Arch:           sudo pacman -S xdg-desktop-portal xdg-desktop-portal-gtk"
        echo ""
    fi

    echo "Installing system files..."

    install -m 755 "$REL/lattice" "$DEST_LATTICE"
    info "✓ $DEST_LATTICE"

    install -Dm 755 "$REL/lattice-filechooser-portal" "$DEST_PORTAL"
    info "✓ $DEST_PORTAL"

    install -Dm 644 "$SOURCE_ROOT/data/portals/lattice.portal" "$DEST_PORTAL_FILE"
    info "✓ $DEST_PORTAL_FILE"

    install -Dm 644 \
        "$SOURCE_ROOT/data/dbus/org.freedesktop.impl.portal.desktop.lattice.service" \
        "$DEST_DBUS_SERVICE"
    info "✓ $DEST_DBUS_SERVICE"

    echo ""
    echo "System files installed."
    echo ""
    echo "The portal backend is NOT active yet."
    echo "To opt in, run as your normal user (without sudo):"
    echo ""
    echo "  $0 --portal-config"
    echo ""
    echo "See docs/file_picker_portal.md for details and rollback instructions."
}

# ── Mode: user portals.conf ───────────────────────────────────────────────────

do_portal_config() {
    [[ $EUID -ne 0 ]] \
        || die "Run --portal-config WITHOUT sudo so it writes to your real home directory."

    CONF_DIR="$HOME/.config/xdg-desktop-portal"
    CONF_FILE="$CONF_DIR/portals.conf"

    echo ""
    echo "Configuring portals.conf for Lattice FileChooser backend..."
    echo "Target: $CONF_FILE"
    echo ""

    mkdir -p "$CONF_DIR"

    # Backup any existing file
    if [[ -f "$CONF_FILE" ]]; then
        BACKUP="$CONF_FILE.bak.$(date +%Y%m%d-%H%M%S)"
        cp "$CONF_FILE" "$BACKUP"
        info "Backed up existing portals.conf → $BACKUP"
    fi

    # Check if FileChooser is already configured
    if grep -q 'FileChooser' "$CONF_FILE" 2>/dev/null; then
        echo ""
        warn "A FileChooser= line already exists in $CONF_FILE:"
        grep 'FileChooser' "$CONF_FILE" | sed 's/^/  /'
        echo ""
        warn "Not modifying. Edit $CONF_FILE manually if you want to change it."
        warn "Your existing config was backed up to $BACKUP"
        echo ""
        echo "To use Lattice with gtk fallback, set:"
        echo "  org.freedesktop.impl.portal.FileChooser=lattice;gtk"
        echo ""
        exit 1
    fi

    # Append the preferred section
    cat >> "$CONF_FILE" <<'EOF'

[preferred]
# EXPERIMENTAL: Use Lattice file picker, fall back to gtk if unavailable.
# To disable, remove these lines or run: ./scripts/install-portal.sh --remove-portal-config
org.freedesktop.impl.portal.FileChooser=lattice;gtk
EOF

    info "✓ portals.conf updated"
    echo ""
    echo "Restart xdg-desktop-portal for the change to take effect:"
    echo ""
    echo "  systemctl --user restart xdg-desktop-portal"
    echo ""
    echo "To verify the backend is registered:"
    echo "  busctl --user list | grep lattice"
    echo ""
    echo "To test the portal manually:"
    echo "  ./scripts/test-portal.sh"
    echo ""
    echo "To roll back: ./scripts/install-portal.sh --remove-portal-config"
}

# ── Mode: remove portals.conf entry ──────────────────────────────────────────

do_remove_portal_config() {
    [[ $EUID -ne 0 ]] \
        || die "Run --remove-portal-config WITHOUT sudo."

    CONF_FILE="$HOME/.config/xdg-desktop-portal/portals.conf"

    if [[ ! -f "$CONF_FILE" ]]; then
        echo "No portals.conf found at $CONF_FILE — nothing to remove."
        exit 0
    fi

    if ! grep -q 'FileChooser.*lattice' "$CONF_FILE" 2>/dev/null; then
        echo "No Lattice FileChooser entry found in $CONF_FILE — nothing to remove."
        exit 0
    fi

    BACKUP="$CONF_FILE.bak.$(date +%Y%m%d-%H%M%S)"
    cp "$CONF_FILE" "$BACKUP"
    info "Backed up $CONF_FILE → $BACKUP"

    # Remove the Lattice FileChooser line and the comment line above it
    sed -i '/# EXPERIMENTAL: Use Lattice file picker/d' "$CONF_FILE"
    sed -i '/# To disable, remove these lines/d' "$CONF_FILE"
    sed -i '/org\.freedesktop\.impl\.portal\.FileChooser=.*lattice/d' "$CONF_FILE"
    # Remove any [preferred] section that is now empty
    sed -i '/^\[preferred\]$/{N;/^\[preferred\]\n$/d}' "$CONF_FILE"

    info "✓ Lattice FileChooser entry removed from portals.conf"
    echo ""
    echo "Restart xdg-desktop-portal for the change to take effect:"
    echo "  systemctl --user restart xdg-desktop-portal"
}

# ── Mode: uninstall system files ──────────────────────────────────────────────

do_uninstall() {
    [[ $EUID -eq 0 ]] || die "System uninstall requires sudo. Run: sudo $0 --uninstall"

    echo "Removing Lattice portal backend system files..."
    echo ""

    removed=0
    for f in "$DEST_PORTAL" "$DEST_PORTAL_FILE" "$DEST_DBUS_SERVICE"; do
        if [[ -f "$f" ]]; then
            rm -f "$f"
            info "✓ removed $f"
            removed=$((removed + 1))
        else
            info "  (not found) $f"
        fi
    done

    # Remove the lib directory if it's empty
    PORTAL_DIR="$(dirname "$DEST_PORTAL")"
    if [[ -d "$PORTAL_DIR" ]] && [[ -z "$(ls -A "$PORTAL_DIR")" ]]; then
        rmdir "$PORTAL_DIR"
        info "✓ removed $PORTAL_DIR"
    fi

    echo ""
    if [[ $removed -gt 0 ]]; then
        echo "Portal backend system files removed."
    else
        echo "No portal backend system files found — nothing removed."
    fi
    echo ""
    echo "NOTE: Your portals.conf was NOT modified."
    echo "To also revert portals.conf, run as your normal user:"
    echo "  $0 --remove-portal-config"
}

# ── Dispatch ──────────────────────────────────────────────────────────────────

case "$MODE" in
    install)               do_install ;;
    portal-config)         do_portal_config ;;
    remove-portal-config)  do_remove_portal_config ;;
    uninstall)             do_uninstall ;;
esac
