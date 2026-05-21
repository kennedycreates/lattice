#!/usr/bin/env bash
# Test script for lattice-filechooser-portal (EXPERIMENTAL)
#
# Usage:
#   ./scripts/test-portal.sh [path/to/lattice-filechooser-portal]
#
# The portal binary prints diagnostic logs to stderr.
# Requires: busctl (from systemd) or gdbus (from glib2).
# lattice binary must be in the same directory as the portal binary or in PATH.
#
# Build first:
#   cargo build --bin lattice-filechooser-portal

set -euo pipefail

if [[ -n "${1:-}" ]]; then
    PORTAL="$1"
else
    PORTAL=""
    for _c in \
        ./target/release/lattice-filechooser-portal \
        ./target/debug/lattice-filechooser-portal \
        /usr/local/lib/lattice/lattice-filechooser-portal; do
        [[ -f "$_c" ]] && { PORTAL="$_c"; break; }
    done
fi

if [[ -z "$PORTAL" || ! -f "$PORTAL" ]]; then
    echo "Error: lattice-filechooser-portal binary not found."
    echo "Searched: ./target/release/, ./target/debug/, /usr/local/lib/lattice/"
    echo "Build with: cargo build --release"
    exit 1
fi
echo "Using portal binary: $PORTAL"

echo "=== Starting lattice-filechooser-portal ==="
"$PORTAL" &
PORTAL_PID=$!
trap "kill $PORTAL_PID 2>/dev/null; wait $PORTAL_PID 2>/dev/null || true" EXIT
sleep 0.5  # allow time to register on the bus

# ── Verify service is registered ─────────────────────────────────────────────

echo ""
echo "=== Checking service is on the session bus ==="
if command -v busctl &>/dev/null; then
    busctl --user status org.freedesktop.impl.portal.desktop.lattice 2>/dev/null \
        && echo "(service found)" || echo "(service not found — check portal binary logs)"
elif command -v gdbus &>/dev/null; then
    gdbus introspect --session \
        --dest org.freedesktop.impl.portal.desktop.lattice \
        --object-path /org/freedesktop/portal/desktop 2>/dev/null \
        && echo "(introspect OK)" || echo "(introspect failed)"
else
    echo "Neither busctl nor gdbus found; skipping service check."
fi

# ── Call OpenFile ─────────────────────────────────────────────────────────────
# D-Bus signature: OpenFile(o handle, s app_id, s parent_window, s title, a{sv} options)
#                        -> (u response, a{sv} results)

echo ""
echo "=== Calling OpenFile (single file, no options) ==="
echo "The Lattice file picker will open. Select a file and confirm, or cancel."
echo ""

if command -v busctl &>/dev/null; then
    busctl --user call \
        org.freedesktop.impl.portal.desktop.lattice \
        /org/freedesktop/portal/desktop \
        org.freedesktop.impl.portal.FileChooser \
        OpenFile \
        "osssa{sv}" \
        /org/test/request/1 \
        test-app \
        "" \
        "Pick a file — portal test" \
        0
elif command -v gdbus &>/dev/null; then
    gdbus call --session \
        --dest org.freedesktop.impl.portal.desktop.lattice \
        --object-path /org/freedesktop/portal/desktop \
        --method org.freedesktop.impl.portal.FileChooser.OpenFile \
        "/org/test/request/1" \
        "test-app" \
        "" \
        "Pick a file — portal test" \
        "@a{sv} {}"
else
    echo "Neither busctl nor gdbus found; cannot call OpenFile."
    exit 1
fi

echo ""
echo "=== Done ==="
echo "Response key: 0=success, 1=cancelled, 2=error"
echo "On success, 'uris' in results contains the selected file:// URI(s)."

# ── Multi-select test ─────────────────────────────────────────────────────────

echo ""
read -r -p "Run multi-select test? [y/N] " yn
if [[ "$yn" =~ ^[Yy]$ ]]; then
    echo ""
    echo "=== Calling OpenFile with multiple=true ==="
    if command -v busctl &>/dev/null; then
        busctl --user call \
            org.freedesktop.impl.portal.desktop.lattice \
            /org/freedesktop/portal/desktop \
            org.freedesktop.impl.portal.FileChooser \
            OpenFile \
            "osssa{sv}" \
            /org/test/request/2 \
            test-app \
            "" \
            "Pick files — portal test" \
            1 \
            "multiple" b true
    fi
fi

# ── Logging note ──────────────────────────────────────────────────────────────

echo ""
echo "=== Portal log inspection ==="
echo "The portal binary writes [lattice-portal] prefixed lines to stderr."
echo "To capture logs:"
echo "  ./target/debug/lattice-filechooser-portal 2>portal.log &"
echo "  tail -f portal.log"
echo ""
echo "If running as a systemd service (after install):"
echo "  journalctl --user -u lattice-filechooser-portal -f"
