# Lattice — Desktop Setup Guide

Lattice runs on any GTK4-compatible Linux desktop. This guide covers installation,
xdg-mime default-opener setup, and desktop-specific configuration for common environments.

---

## Install the binary

```bash
cargo build --release
sudo install -m 755 target/release/lattice /usr/local/bin/lattice
```

---

## Install the desktop entry

The `.desktop` file makes Lattice appear in application launchers and registers it
as a file manager with the system.

```bash
sudo install -m 644 com.lattice.filemanager.desktop \
  /usr/local/share/applications/com.lattice.filemanager.desktop
```

Or for a per-user install:

```bash
install -Dm 644 com.lattice.filemanager.desktop \
  ~/.local/share/applications/com.lattice.filemanager.desktop
```

Update the desktop database after installing:

```bash
update-desktop-database ~/.local/share/applications/
```

---

## Set Lattice as the default folder opener

To open folders with Lattice when you click them in another app:

```bash
xdg-mime default com.lattice.filemanager.desktop inode/directory
```

Verify:

```bash
xdg-mime query default inode/directory
# should output: com.lattice.filemanager.desktop
```

---

## CLI reference

| Command | Opens |
|---------|-------|
| `lattice` | Home directory |
| `lattice --path /some/folder` | The specified folder |
| `lattice /some/folder` | The specified folder (positional shorthand) |
| `lattice --downloads` | Downloads Triage view |
| `lattice --project "My Palette"` | Palette by name; legacy flag retained for compatibility |
| `lattice --split /left /right` | Split view with two explicit paths |
| `lattice --split /left /middle /right` | Split view with three explicit paths |

If `--path` resolves to a path that doesn't exist or cannot be read, Lattice opens
to the home directory instead. If legacy `--project` names a missing Palette,
Lattice opens Home with a status message.

---

## Theming

Lattice picks up its theme from `~/.config/lattice/config.toml`.
The default Victorian Gothic dark theme works well on any dark system.
For maximum contrast:

```toml
# ~/.config/lattice/config.toml
theme = "high-contrast"
```

See [theming.md](theming.md) for full theming documentation.

---

## labwc

Add the following to your labwc `rc.xml` inside the `<keyboard>` section:

```xml
<!-- File manager -->
<keybind key="Super-e">
  <action name="Execute">
    <command>lattice</command>
  </action>
</keybind>

<!-- Downloads Triage — jump straight into the sorted download view -->
<keybind key="Super-Shift-e">
  <action name="Execute">
    <command>lattice --downloads</command>
  </action>
</keybind>

<!-- Split view — open two folders side by side -->
<keybind key="Super-Alt-e">
  <action name="Execute">
    <command>lattice --split ~/Downloads ~/Documents</command>
  </action>
</keybind>
```

Reload labwc after editing:

```bash
labwc --reconfigure
```

### Optional: Waybar launcher button

Add a click-to-open Files button to your Waybar config:

```jsonc
// In modules-right or modules-center:
"custom/files": {
    "format": "󰉋",
    "tooltip": "Open Files (Lattice)",
    "on-click": "lattice",
    "on-click-right": "lattice --downloads",
    "on-click-middle": "lattice --split ~/Downloads ~/Documents"
}
```

With Nerd Fonts installed, `󰉋` renders as a folder icon. Replace with any
character or text you prefer.

---

## GNOME

Lattice will appear in the GNOME application grid automatically once the desktop entry
is installed. To set Lattice as the default folder opener:

```bash
xdg-mime default com.lattice.filemanager.desktop inode/directory
```

GNOME does not use a separate keybinding config file. Add a keyboard shortcut in
**Settings → Keyboard → Keyboard Shortcuts → Custom Shortcuts**, setting the command
to `lattice`.

---

## COSMIC

Install the binary and desktop entry as above. COSMIC reads `.desktop` files from
standard XDG paths. To set Lattice as the default folder opener:

```bash
xdg-mime default com.lattice.filemanager.desktop inode/directory
```

Add keybindings in **COSMIC Settings → Keyboard → Keyboard Shortcuts**.

---

## Sway / other wlroots compositors

The install and xdg-mime steps are the same as labwc. Add a keybinding to your Sway
config (`~/.config/sway/config`):

```
bindsym $mod+e exec lattice
bindsym $mod+Shift+e exec lattice --downloads
bindsym $mod+Mod1+e exec lattice --split ~/Downloads ~/Documents
```

Reload Sway after editing: `swaymsg reload`
