# Lattice — Agent Rules

Rules for AI agents (Claude Code and others) working on this codebase.

---

## Core Principles

### 1. Mouse-first, always

Lattice is a **mouse-first** application. Every feature must be fully usable with a mouse.

- Primary interactions: click, double-click, right-click, drag
- Keyboard shortcuts are optional secondary affordances; never the only way to invoke an action
- Do not design UI flows that require the keyboard (e.g., no "press Enter to confirm" as the only path)

### 2. Do not make it keyboard-first

Do not add vim-style navigation, keybinding-heavy workflows, or any UI that assumes the user prefers the keyboard. If a keyboard shortcut is added, the same action must also be reachable by mouse.

### 3. Stay within the current milestone

Do not implement features outside the current milestone. Check `docs/roadmap.md` for what is in scope. A feature listed in M3 must not be added while working on M1, even if it seems simple. Keep scope tight so each milestone ships cleanly.

### 4. Every milestone must compile

Every commit must leave the project in a state where `cargo run` succeeds. Do not commit code that does not compile. Do not leave dead code stubs that introduce warnings unless they are clearly temporary scaffolding for the current milestone.

### 5. All styling goes through CSS classes

- Visual styling belongs in `themes/default.css`
- Do not hardcode colors, fonts, margins, or paths inline in Rust code
- Use stable CSS class names (see list below) so the theme can be swapped without touching Rust
- Do not use GTK `StyleContext` direct color overrides; use CSS

### 6. Use GIO/GTK-native APIs for file operations

- File enumeration: `gio::FileEnumerator`
- File info: `gio::FileInfo`
- File operations (copy, move, delete): `gio::File` methods
- Async I/O: GIO async variants (do not block the GTK main thread)
- Do not shell out to `cp`, `mv`, `rm`, `ls`, etc.
- Do not use `std::fs` for anything the user will see; use GIO so portal/Wayland sandboxing works correctly

### 7. Dangerous operations require safeguards (M4+)

Starting from Milestone 4, any operation that permanently modifies or destroys files must:

- Show a confirmation dialog before executing
- Support undo where feasible (trash instead of permanent delete)
- Never silently overwrite files

---

## Stable CSS Class Reference

| Class | Element |
|---|---|
| `app-window` | Root `ApplicationWindow` |
| `top-toolbar` | Toolbar `Box` |
| `sidebar` | Sidebar `ScrolledWindow` |
| `sidebar-section` | Each sidebar group `Box` |
| `sidebar-button` | Each sidebar nav `Button` |
| `tab-strip` | Tab bar `Box` |
| `file-grid` | `FlowBox` holding file cards |
| `file-card` | Individual file card `Box` |
| `file-card-selected` | Selected state modifier on `file-card` |
| `preview-pane` | Right preview `Box` |
| `status-bar` | Bottom status `Box` |

---

## Code Style

- Keep each UI component in its own file under `src/ui/`
- `build()` functions return a GTK widget — keep them focused on construction only
- No business logic in widget builders; no widget construction in business logic
- Prefer `gtk::prelude::*` imports, scoped to each file
- Do not use `unwrap()` on GTK operations that can fail at runtime; log and degrade gracefully
