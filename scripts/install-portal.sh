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

# ── Service-manager capability detection (not distro-name detection) ──────────

# True only when systemd is the running init AND a systemctl exists. This is the
# canonical sd_booted() check: /run/systemd/system exists iff systemd is PID 1.
# On runit systems (Void) it is absent, so we take the XDG-autostart path.
system_is_systemd() {
    [[ -d /run/systemd/system ]] && command -v systemctl >/dev/null 2>&1
}

# Home directory of the user who invoked sudo (empty if not run via sudo).
sudo_user_home() {
    [[ -n "${SUDO_USER:-}" ]] || { echo ""; return; }
    getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6
}

# Print the correct "restart the portal frontend" command for this system.
portal_restart_hint() {
    if system_is_systemd; then
        echo "  systemctl --user restart xdg-desktop-portal"
    else
        echo "  pkill -x xdg-desktop-portal   # it re-activates on the next portal call"
    fi
}

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
DEST_SYSTEMD_SERVICE=/usr/lib/systemd/user/lattice-filechooser-portal.service

# Non-systemd startup: a per-user XDG autostart entry launched by the graphical
# session. Scoped to the invoking user (mirrors `systemctl --user enable`).
SRC_AUTOSTART="$SOURCE_ROOT/data/autostart/lattice-filechooser-portal.desktop"
AUTOSTART_BASENAME="lattice-filechooser-portal.desktop"

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
        xbps-query -p pkgver xdg-desktop-portal &>/dev/null 2>&1 && _xdp_found=true
    fi
    if ! $_xdp_found; then
        warn "xdg-desktop-portal does not appear to be installed."
        warn "The portal backend will not be callable until it is installed."
        warn "  Ubuntu/Pop!_OS: sudo apt install xdg-desktop-portal xdg-desktop-portal-gtk"
        warn "  Arch:           sudo pacman -S xdg-desktop-portal xdg-desktop-portal-gtk"
        warn "  Fedora:         sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk"
        warn "  Void:           sudo xbps-install dbus xdg-desktop-portal xdg-desktop-portal-gtk"
        echo ""
    fi

    # The frontend portal being present does NOT imply a GTK FileChooser backend
    # is installed. The --portal-config step needs a real fallback backend that
    # implements FileChooser (normally 'gtk'). Warn early if the gtk descriptor
    # is missing so the user can install it before opting in.
    if [[ ! -f /usr/share/xdg-desktop-portal/portals/gtk.portal ]]; then
        warn "The GTK portal backend (xdg-desktop-portal-gtk) does not appear to be installed."
        warn "Lattice uses it as the FileChooser fallback. Without a fallback backend,"
        warn "non-FileChooser portal apps could lose their file dialog if you opt in."
        warn "  Ubuntu/Pop!_OS: sudo apt install xdg-desktop-portal-gtk"
        warn "  Arch:           sudo pacman -S xdg-desktop-portal-gtk"
        warn "  Fedora:         sudo dnf install xdg-desktop-portal-gtk"
        warn "  Void:           sudo xbps-install xdg-desktop-portal-gtk"
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

    # ── Startup mechanism: pick the one this system actually has ───────────────
    # A backend needs the live graphical + session-bus environment to spawn the
    # picker window. On systemd we use a `systemctl --user` service; on runit
    # systems (Void) there is no `systemctl --user`, so we install a per-user
    # XDG autostart entry that the graphical session launches with the right env.
    if system_is_systemd; then
        install -Dm 644 \
            "$SOURCE_ROOT/data/systemd/lattice-filechooser-portal.service" \
            "$DEST_SYSTEMD_SERVICE"
        info "✓ $DEST_SYSTEMD_SERVICE"

        # Enable and start the systemd user service for the invoking user.
        if [[ -n "${SUDO_USER:-}" ]]; then
            if runuser -l "$SUDO_USER" -c 'systemctl --user daemon-reload' 2>/dev/null; then
                runuser -l "$SUDO_USER" -c \
                    'systemctl --user enable --now lattice-filechooser-portal.service' 2>/dev/null \
                    && info "✓ systemd user service enabled and started" \
                    || info "  (service enable failed — run manually: systemctl --user enable --now lattice-filechooser-portal.service)"
            else
                info "  (systemd reload skipped — run manually: systemctl --user daemon-reload && systemctl --user enable --now lattice-filechooser-portal.service)"
            fi
        fi
    else
        # Non-systemd (e.g. Void/runit): install a per-user XDG autostart entry.
        info "No systemd user manager detected — using the XDG autostart startup path."
        _user_home="$(sudo_user_home)"
        if [[ -n "$_user_home" && -d "$_user_home" ]]; then
            _autostart_dir="$_user_home/.config/autostart"
            _autostart_dest="$_autostart_dir/$AUTOSTART_BASENAME"
            install -Dm 644 "$SRC_AUTOSTART" "$_autostart_dest"
            chown "$SUDO_USER":"$(id -gn "$SUDO_USER" 2>/dev/null || echo "$SUDO_USER")" \
                "$_autostart_dest" "$_autostart_dir" 2>/dev/null || true
            info "✓ $_autostart_dest"
            info "  It starts at your next graphical login. To start it now without"
            info "  logging out, run in your desktop session (NOT via sudo):"
            info "    setsid -f /usr/local/lib/lattice/lattice-filechooser-portal"
            info "  Minimal Wayland sessions that don't read XDG autostart (sway, labwc)"
            info "  need a compositor exec line — see docs/file_picker_portal.md."
        else
            warn "Could not resolve the invoking user's home for the autostart entry."
            warn "Install it yourself (as your normal user):"
            warn "  install -Dm 644 data/autostart/$AUTOSTART_BASENAME \\"
            warn "    ~/.config/autostart/$AUTOSTART_BASENAME"
        fi
    fi

    # Reload the D-Bus session daemon so it picks up the new service file.
    # Must run as the invoking user, not root, because we need their session bus.
    if [[ -n "${SUDO_USER:-}" ]]; then
        _dbus_addr=$(runuser -l "$SUDO_USER" -c 'echo "$DBUS_SESSION_BUS_ADDRESS"' 2>/dev/null || true)
        if [[ -n "$_dbus_addr" ]]; then
            DBUS_SESSION_BUS_ADDRESS="$_dbus_addr" \
                dbus-send --session --type=method_call \
                --dest=org.freedesktop.DBus / org.freedesktop.DBus.ReloadConfig \
                2>/dev/null \
                && info "✓ D-Bus session config reloaded" \
                || info "  (D-Bus reload failed — run manually: dbus-send --session --type=method_call --dest=org.freedesktop.DBus / org.freedesktop.DBus.ReloadConfig)"
        else
            info "  (D-Bus session address not found — reload manually after login)"
            echo "  Run as your normal user:"
            echo "    dbus-send --session --type=method_call --dest=org.freedesktop.DBus / org.freedesktop.DBus.ReloadConfig"
        fi
    fi

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

    local CONF_DIR="$HOME/.config/xdg-desktop-portal"
    local CONF_FILE="$CONF_DIR/portals.conf"
    local PORTALS_DIR="/usr/share/xdg-desktop-portal/portals"
    local desktop="${XDG_CURRENT_DESKTOP:-}"

    echo ""
    echo "Configuring portals.conf for Lattice FileChooser backend..."
    echo "Target: $CONF_FILE"
    [[ -n "$desktop" ]] && echo "Desktop: $desktop"
    echo ""

    mkdir -p "$CONF_DIR"

    # Backup any existing file
    if [[ -f "$CONF_FILE" ]]; then
        local BACKUP="$CONF_FILE.bak.$(date +%Y%m%d-%H%M%S)"
        cp "$CONF_FILE" "$BACKUP"
        if grep -q 'Generated by install-portal.sh' "$CONF_FILE" 2>/dev/null; then
            info "Regenerating (was previously generated by this script)"
        else
            info "Backed up existing portals.conf → $BACKUP"
            if [[ "$YES" != "true" ]]; then
                warn "portals.conf already exists and was not generated by this script."
                echo ""
                sed 's/^/  /' "$CONF_FILE"
                echo ""
                read -r -p "Overwrite with a generated config? [y/N] " answer
                [[ "$answer" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 0; }
            fi
        fi
    fi

    # ── Scan installed .portal files to discover interface → backend mappings ──
    # Backends matching XDG_CURRENT_DESKTOP (UseIn=) get priority 0;
    # backends with no UseIn get priority 1; mismatches get priority 2.
    declare -A _iface_backend
    declare -A _iface_prio

    if [[ -d "$PORTALS_DIR" ]]; then
        for _pfile in "$PORTALS_DIR"/*.portal; do
            [[ -f "$_pfile" ]] || continue
            local _bname="${_pfile##*/}"
            _bname="${_bname%.portal}"
            [[ "$_bname" == "lattice" ]] && continue  # skip our own

            local _in_portal=false _use_in="" _interfaces=""
            while IFS= read -r _line; do
                _line="${_line%%#*}"  # strip inline comments
                [[ -z "${_line//[[:space:]]/}" ]] && continue  # skip blank
                if [[ "$_line" =~ ^\[portal\] ]]; then
                    _in_portal=true
                elif [[ "$_line" =~ ^\[.* ]]; then
                    _in_portal=false
                elif $_in_portal; then
                    [[ "$_line" =~ ^UseIn= ]]      && _use_in="${_line#UseIn=}"
                    [[ "$_line" =~ ^Interfaces= ]] && _interfaces="${_line#Interfaces=}"
                fi
            done < "$_pfile"

            # Determine priority
            local _prio=1
            if [[ -n "$_use_in" ]]; then
                _prio=2
                if [[ -n "$desktop" ]]; then
                    local _u
                    IFS=';' read -ra _ulist <<< "$_use_in"
                    for _u in "${_ulist[@]}"; do
                        _u="${_u// /}"
                        [[ "$_u" == "$desktop" ]] && { _prio=0; break; }
                    done
                fi
            fi

            # Register each interface if this backend has higher priority
            local _iface
            IFS=';' read -ra _ilist <<< "$_interfaces"
            for _iface in "${_ilist[@]}"; do
                _iface="${_iface// /}"
                [[ -z "$_iface" ]] && continue
                local _existing="${_iface_prio[$_iface]:-9}"
                if (( _prio < _existing )); then
                    _iface_backend["$_iface"]="$_bname"
                    _iface_prio["$_iface"]=$_prio
                fi
            done
        done
    fi

    # ── Determine a real, installed FileChooser fallback backend ───────────────
    # The frontend portal existing does NOT mean a GTK FileChooser backend is
    # installed. Verify a descriptor actually implements FileChooser before we
    # write it as the fallback. Prefer 'gtk'; otherwise use whatever installed
    # backend the scan found for FileChooser; otherwise write Lattice alone and
    # tell the user how to get a fallback.
    local _fc_fallback=""
    if [[ -f "$PORTALS_DIR/gtk.portal" ]] \
        && grep -qE '^[[:space:]]*Interfaces=.*org\.freedesktop\.impl\.portal\.FileChooser' \
            "$PORTALS_DIR/gtk.portal"; then
        _fc_fallback="gtk"
    elif [[ -n "${_iface_backend[org.freedesktop.impl.portal.FileChooser]:-}" ]]; then
        _fc_fallback="${_iface_backend[org.freedesktop.impl.portal.FileChooser]}"
    fi

    local _fc_line _fc_comment
    if [[ -n "$_fc_fallback" ]]; then
        _fc_line="org.freedesktop.impl.portal.FileChooser=lattice;$_fc_fallback"
        _fc_comment="# FileChooser: Lattice (experimental) with '$_fc_fallback' fallback (verified installed)"
    else
        _fc_line="org.freedesktop.impl.portal.FileChooser=lattice"
        _fc_comment="# FileChooser: Lattice (experimental). No installed fallback backend found."
        warn "No installed portal backend implements FileChooser as a fallback."
        warn "If the Lattice backend is ever unavailable, apps will have no file dialog."
        warn "Install a fallback backend, then re-run --portal-config:"
        warn "  Ubuntu/Pop!_OS: sudo apt install xdg-desktop-portal-gtk"
        warn "  Arch:           sudo pacman -S xdg-desktop-portal-gtk"
        warn "  Fedora:         sudo dnf install xdg-desktop-portal-gtk"
        warn "  Void:           sudo xbps-install xdg-desktop-portal-gtk"
        echo ""
    fi

    # ── Write complete portals.conf ───────────────────────────────────────────
    {
        echo "# Generated by install-portal.sh — $(date)"
        echo "# Revert with: ./scripts/install-portal.sh --remove-portal-config"
        echo "[preferred]"
        echo ""
        echo "$_fc_comment"
        echo "$_fc_line"
        if (( ${#_iface_backend[@]} > 0 )); then
            echo ""
            echo "# Other interfaces: auto-detected from installed portal backends"
            local _k
            for _k in $(printf '%s\n' "${!_iface_backend[@]}" | sort); do
                [[ "$_k" == "org.freedesktop.impl.portal.FileChooser" ]] && continue
                echo "${_k}=${_iface_backend[$_k]}"
            done
        fi
    } > "$CONF_FILE"

    info "✓ portals.conf written"
    echo ""
    echo "Generated config:"
    sed 's/^/  /' "$CONF_FILE"
    echo ""
    echo "Restart xdg-desktop-portal for the change to take effect:"
    echo ""
    portal_restart_hint
    echo ""
    echo "────────────────────────────────────────────────────────────────"
    echo "  GTK apps (GIMP, Inkscape, etc.) require GTK_USE_PORTAL=1"
    echo "  to route their file dialogs through the portal."
    echo ""
    echo "  Test immediately (open the app and trigger a file dialog):"
    echo "    GTK_USE_PORTAL=1 inkscape"
    echo "    GTK_USE_PORTAL=1 gimp"
    echo ""
    echo "  To set permanently for your user session:"
    echo "    mkdir -p ~/.config/environment.d"
    echo "    echo 'GTK_USE_PORTAL=1' > ~/.config/environment.d/lattice-portal.conf"
    echo "    # Then log out and back in."
    echo ""
    echo "  Chrome and Electron apps (Vesktop) use the portal natively —"
    echo "  GTK_USE_PORTAL is not needed for them."
    echo "────────────────────────────────────────────────────────────────"
    echo ""
    echo "To verify the backend responds end-to-end:"
    echo "  ./scripts/test-portal.sh"
    echo ""
    echo "To roll back: ./scripts/install-portal.sh --remove-portal-config"
}

# ── Mode: remove portals.conf ─────────────────────────────────────────────────

do_remove_portal_config() {
    [[ $EUID -ne 0 ]] \
        || die "Run --remove-portal-config WITHOUT sudo."

    local CONF_FILE="$HOME/.config/xdg-desktop-portal/portals.conf"

    if [[ ! -f "$CONF_FILE" ]]; then
        echo "No portals.conf found at $CONF_FILE — nothing to remove."
        exit 0
    fi

    if ! grep -q 'Generated by install-portal.sh\|FileChooser=.*lattice' "$CONF_FILE" 2>/dev/null; then
        echo "portals.conf was not generated by install-portal.sh — not removing."
        echo "Edit $CONF_FILE manually if needed."
        exit 1
    fi

    local BACKUP="$CONF_FILE.bak.$(date +%Y%m%d-%H%M%S)"
    cp "$CONF_FILE" "$BACKUP"
    info "Backed up $CONF_FILE → $BACKUP"

    rm "$CONF_FILE"
    info "✓ portals.conf removed (xdg-desktop-portal will use auto-detection again)"
    echo ""
    echo "Restart xdg-desktop-portal for the change to take effect:"
    portal_restart_hint
}

# ── Mode: uninstall system files ──────────────────────────────────────────────

do_uninstall() {
    [[ $EUID -eq 0 ]] || die "System uninstall requires sudo. Run: sudo $0 --uninstall"

    echo "Removing Lattice portal backend system files..."
    echo ""

    # Disable the systemd user service before removing files — only where a
    # systemd user manager actually exists (skipped cleanly on runit/Void).
    if system_is_systemd && [[ -n "${SUDO_USER:-}" ]]; then
        runuser -l "$SUDO_USER" -c \
            'systemctl --user disable --now lattice-filechooser-portal.service 2>/dev/null || true'
        info "✓ systemd user service disabled"
    fi

    removed=0
    for f in "$DEST_PORTAL" "$DEST_PORTAL_FILE" "$DEST_DBUS_SERVICE" "$DEST_SYSTEMD_SERVICE"; do
        if [[ -f "$f" ]]; then
            rm -f "$f"
            info "✓ removed $f"
            removed=$((removed + 1))
        else
            info "  (not found) $f"
        fi
    done

    # Remove the per-user XDG autostart entry (non-systemd startup path).
    _user_home="$(sudo_user_home)"
    if [[ -n "$_user_home" ]]; then
        _autostart_dest="$_user_home/.config/autostart/$AUTOSTART_BASENAME"
        if [[ -f "$_autostart_dest" ]]; then
            rm -f "$_autostart_dest"
            info "✓ removed $_autostart_dest"
            removed=$((removed + 1))
        fi
    fi

    if system_is_systemd && [[ -n "${SUDO_USER:-}" ]]; then
        runuser -l "$SUDO_USER" -c 'systemctl --user daemon-reload 2>/dev/null || true'
    fi

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
