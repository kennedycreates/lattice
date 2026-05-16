# Lattice — Product Brief

## Vision

Lattice is a mouse-first, visually polished file manager for custom Wayland desktops — especially setups using labwc, Sway, or similar compositors where GTK4 apps are the primary citizens.

It is **not** a keyboard-driven power tool. It is a slick, dark, visually intentional graphical file manager that feels at home on a well-customized desktop.

## Target User

- Linux power users who run custom Wayland compositors
- Users who prefer a mouse-first workflow but want full GTK4 integration
- People who care about desktop aesthetics and want a cohesive dark theme

## Core Design Principles

1. **Mouse-first** — navigation, selection, and actions are designed for cursor interaction. Keyboard shortcuts are a bonus, not the primary interface.
2. **Visual and polished** — icons, previews, and a rich dark theme are first-class features, not afterthoughts.
3. **GTK4-native** — uses GTK4/GIO APIs throughout for proper Wayland, theme, and portal integration.
4. **Modular and extensible** — code is structured so features like tabs, split panes, tags, and previews can be added cleanly in future milestones.

## Feature Summary (across milestones)

| Feature | Status |
|---|---|
| Distinctive dark CSS theme | ✅ M0 |
| App shell layout (toolbar, sidebar, grid, preview, status) | ✅ M0 |
| Real file grid | ✅ M1 |
| Real GIO directory browsing | ✅ M1 |
| Sidebar navigation (click to browse) | ✅ M1 |
| Right-click context menus | ✅ M2 |
| Core file actions (rename, new folder, trash, copy path, terminal) | ✅ M2 |
| Tabbed browsing | ✅ M4 |
| Split-pane view | ✅ M4 |
| File preview (images, text, metadata) | ✅ M3 |
| Projects, tags, and Downloads Triage | ✅ M5 |
| TOML config and launch modes | ✅ M6 |
| Search | Not implemented |
| Drag and drop | Not implemented |
| Basic trash restore from Trash view | ✅ |
| Trash undo / deeper safety flows | Not implemented |
