# Lattice — Product Brief

## Vision

Lattice is a mouse-first, visually polished file manager for Linux desktops — it runs on any GTK4-compatible environment, including labwc, GNOME, COSMIC, Sway, and traditional X11 desktops.

It is **not** a keyboard-driven power tool. It is a slick, dark, visually intentional graphical file manager that feels at home on a well-customized desktop.

## Target User

- Linux power users on any GTK4-compatible desktop
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
| Sidebar navigation (click to browse) | ✅ Home fixed, user-pinned Places removable, Projects separate |
| Right-click context menus | ✅ M2 |
| Core file actions (rename, new folder, trash, copy path, terminal) | ✅ M2 |
| Tabbed browsing | ✅ M4 |
| Split-pane view | ✅ M4, now supports two or three panels |
| File preview (images, text, metadata) | ✅ M3 |
| Projects, tags, and Downloads Triage | ✅ M5 |
| TOML config and launch modes | ✅ M6 |
| Folder search | ✅ Current-folder or recursive scope with name, kind, date, size, and tag filters |
| Drag and drop | ✅ Within/between panes and key sidebar destinations, with polished visual feedback |
| Basic trash restore from Trash view | ✅ |
| Activity Log undo / repeat | ✅ Reversible new log rows support guarded undo, repeat, reveal, and copy-path actions |
| Permanent-delete undo | Not implemented |
