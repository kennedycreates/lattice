# Lattice — labwc Integration Guide

Lattice is designed to feel at home in a custom Wayland desktop built around
[labwc](https://github.com/labwc/labwc). This guide covers installation, keybindings,
default folder-opener setup, and an optional Waybar launcher snippet.

---

## Install the binary

```bash
cargo build --release
sudo install -m 755 target/release/lattice /usr/local/bin/lattice
```

---

## Install the desktop entry

The `.desktop` file tells the system how to launch Lattice and makes it appear
in application launchers (wofi, fuzzel, etc.).

```bash
sudo install -m 644 lattice.desktop /usr/local/share/applications/lattice.desktop
```

Or for a per-user install:

```bash
install -m 644 lattice.desktop ~/.local/share/applications/lattice.desktop
```

Update the desktop database after installing:

```bash
update-desktop-database ~/.local/share/applications/
```

---

## Set Lattice as the default folder opener

To open folders with Lattice when you click them in another app:

```bash
xdg-mime default lattice.desktop inode/directory
```

Verify:

```bash
xdg-mime query default inode/directory
# should output: lattice.desktop
```

---

## labwc keybindings

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

---

## CLI reference

| Command | Opens |
|---------|-------|
| `lattice` | Home directory |
| `lattice --path /some/folder` | The specified folder |
| `lattice /some/folder` | The specified folder (positional shorthand) |
| `lattice --downloads` | Downloads Triage view |
| `lattice --project "My Project"` | Root of a pinned project |
| `lattice --split /left /right` | Split view with two explicit paths |

If `--path` or `--project` resolves to a path that doesn't exist or cannot be read,
Lattice opens to the home directory instead.

---

## Optional: Waybar launcher button

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

Add it to your bar's module list:

```jsonc
"modules-right": ["custom/files", ...]
```

---

## Theme for labwc environments

Lattice picks up its theme from `~/.config/lattice/config.toml`.
The default Victorian Gothic dark theme works well on any dark system.
If you prefer a cleaner look with maximum contrast:

```toml
# ~/.config/lattice/config.toml
theme = "high-contrast"
```

See [theming.md](theming.md) for full theming documentation.
