# Lattice — Theming Guide

Lattice uses plain GTK4 CSS for all visual styling. Themes are single `.css` files
that override the entire appearance of the app.

---

## Where themes live

### Bundled themes (read-only)

Shipped with the binary, stored alongside it at build time.

| Name | File | Description |
|------|------|-------------|
| `default` | `themes/default.css` | Victorian Gothic dark — the default look |
| `high-contrast` | `themes/high-contrast.css` | High-contrast dark — brighter text, full-saturation accents |

### User themes (editable)

Place custom `.css` files in:

```
~/.config/lattice/themes/
```

Any `.css` file you place here is available as a theme by its filename (without extension).
User themes take precedence over bundled themes of the same name.

---

## How to switch themes

Edit (or create) `~/.config/lattice/config.toml`:

```toml
theme = "high-contrast"
```

Restart Lattice to apply the new theme.

Available theme names are the filenames in `themes/` (bundled) and
`~/.config/lattice/themes/` (user), without the `.css` extension.

---

## How to create a custom theme

The easiest starting point is copying the default theme:

```bash
mkdir -p ~/.config/lattice/themes
cp themes/default.css ~/.config/lattice/themes/my-theme.css
```

Then edit `~/.config/lattice/themes/my-theme.css` and set:

```toml
# ~/.config/lattice/config.toml
theme = "my-theme"
```

---

## Major CSS classes

All stable CSS classes Lattice applies to its widgets. These are safe to target
in any theme — they will not change between releases within a major version.

## Font stacks

Bundled themes use fallback stacks instead of requiring a single nonstandard font:

- Main UI: `"Inter", "Noto Sans", "DejaVu Sans", sans-serif`
- Paths, code, file sizes, dates, and text previews: `"JetBrains Mono", "Noto Sans Mono", "DejaVu Sans Mono", monospace`

Do not rely on special icon fonts for interface controls. Prefer GTK symbolic icon names or bundled image assets.

### Layout regions

| Class | Element | Notes |
|-------|---------|-------|
| `.app-window` | Root application window | |
| `.top-toolbar` | Horizontal toolbar at the top | |
| `.sidebar` | Left navigation panel | |
| `.tab-strip` | Tab bar row | |
| `.browser-pane` | File browsing pane (primary, secondary, or tertiary) | |
| `.browser-pane-primary` | Primary pane | always visible |
| `.browser-pane-secondary` | Secondary pane | visible in two- and three-panel layouts |
| `.browser-pane-tertiary` | Tertiary pane | visible in three-panel layouts |
| `.browser-pane-active` | Active pane | added to whichever pane has focus |
| `.browser-pane-header` | Path header inside a pane | |
| `.browser-pane-path` | Path label inside the pane header | |
| `.preview-host` | Container that holds the preview pane region | |
| `.preview-pane` | Right preview panel | |
| `.status-bar` | Bottom status strip | |

### Window controls

| Class | Notes |
|-------|-------|
| `.lattice-titlebar` | Client-side GTK headerbar |
| `.lattice-window-controls` | Container for built-in minimize, maximize/restore, and close controls |
| `.lattice-window-control-button` | Shared compact titlebar control button |
| `.lattice-window-close-button` | Close button danger/hover variant |

### Toolbar buttons

| Class | Notes |
|-------|-------|
| `.toolbar-nav-btn` | Back / Up / Refresh buttons |
| `.toolbar-action-btn` | Sidebar, Preview, Split, New Folder, New Text Document, Rename, Trash |
| `.toolbar-toggle` | Toggle variant (Sidebar, Preview, Split) |
| `.toolbar-danger-btn` | Destructive action variant (Trash) |
| `.toolbar-sep` | Vertical separator between button groups |
| `.toolbar-path-button` | Breadcrumb path bar button |
| `.toolbar-path` | Path entry field (active when editing) |
| `.toolbar-breadcrumbs` | Container for breadcrumb segments |
| `.toolbar-path-segment` | One breadcrumb segment label |
| `.toolbar-path-current` | Last (current) breadcrumb segment |
| `.toolbar-path-separator` | `›` separator between segments |

### Tabs

| Class | Notes |
|-------|-------|
| `.tab-strip` | Container for all tabs |
| `.tab-chip` | One tab (button + close button wrapper) |
| `.tab-chip.active` | Currently active tab chip |
| `.tab-button` | Tab title button |
| `.tab-button.active` | Active tab title button |
| `.tab-close-button` | `×` close button on a tab |
| `.tab-add-button` | `+` new tab button |

### Sidebar

| Class | Notes |
|-------|-------|
| `.sidebar-button` | Any sidebar navigation button |
| `.sidebar-button.active` | Currently highlighted sidebar button |
| `.sidebar-section` | Section container (Places, Workspace, System) |
| `.sidebar-section-heading` | Uppercase section label |
| `.sidebar-sep` | Horizontal separator |
| `.sidebar-note` | Muted explanatory text |
| `.sidebar-dynamic-list` | Container for dynamic Palette/tag lists |

### File grid

| Class | Notes |
|-------|-------|
| `.file-grid` | FlowBox container |
| `.file-grid-scroll` | ScrolledWindow wrapper |
| `.file-grid-empty` | Empty / loading state label |
| `.file-card` | Individual file/folder card |
| `.file-card-icon` | Large emoji icon label on a card |
| `.file-card-name` | Filename label |
| `.file-card-kind` | File type label |
| `.file-card-tags` | Tag chip row at the bottom of a card |
| `.file-tag-chip` | One tag pill |
| `.file-tag-chip-muted` | Overflow indicator (`+N`) |

### File type colours

Applied to the card box and can be combined with child selectors:

```
.file-type-folder   .file-type-image   .file-type-video
.file-type-text     .file-type-archive  .file-type-config
.file-type-unknown
```

Example:
```css
.file-type-folder .file-card-icon { color: #ffaa00; }
```

### Preview pane

| Class | Notes |
|-------|-------|
| `.preview-icon` | Large file-type emoji label |
| `.preview-title` | File name heading |
| `.preview-header` | "PREVIEW" section label |
| `.preview-actions` | Open / Copy Path / Open Parent row |
| `.preview-image` | Image preview widget |
| `.preview-text` | Text preview widget |
| `.preview-meta` | Metadata row container |
| `.preview-meta-key` | Metadata key label |
| `.preview-meta-value` | Metadata value label |
| `.preview-path-value` | Path value variant |
| `.preview-meta-sep` | Separator between metadata sections |

### Pane view strip (tag/triage views)

| Class | Notes |
|-------|-------|
| `.pane-view-strip` | Header strip shown in tag/triage views |
| `.pane-view-btn` | Compact per-pane header button for tag filter, hidden files, and icon/list view |
| `.pane-filter-btn` | Per-pane tag filter header button |
| `.pane-hidden-btn` | Per-pane hidden-files header button |
| `.pane-control-active` | Active state for per-pane header controls |
| `.pane-view-title` | View name label |
| `.pane-filter-row` | Row of filter buttons |
| `.pane-filter-button` | Individual filter pill |
| `.pane-filter-button.active` | Currently selected filter |

### Activity Log

| Class | Notes |
|-------|-------|
| `.activity-log` | Activity Log root container |
| `.activity-log-row` | One receipt row |
| `.activity-log-op-icon` | Operation icon at the start of a row |
| `.activity-log-summary` | Main receipt summary |
| `.activity-log-timestamp` | Relative timestamp text |
| `.activity-log-actions` | Compact row-action button group |
| `.activity-log-action-btn` | Undo / repeat / reveal / copy-path row button |
| `.activity-log-status-ok` | Success marker |
| `.activity-log-status-fail` | Failure marker |
| `.activity-log-empty` | Empty-state message |

### Context menus and dialogs

| Class | Notes |
|-------|-------|
| `.context-menu` | Right-click popover container |
| `.context-menu-button` | Menu action button |
| `.context-menu-danger` | Destructive action variant |
| `.dialog-column` | Dialog content column |
| `.dialog-prompt` | Dialog description label |
| `.dialog-entry` | Dialog text entry field |
| `.tooltip-host` | Wrapper used when a disabled child still needs hover tooltips |
| `.app-tooltip-popover` | Custom tooltip popover shell |
| `.app-tooltip-frame` | Tooltip surface box |
| `.app-tooltip-label` | Tooltip label text |

---

## Color palette conventions

The default theme uses this palette (you can reuse or redefine all of these):

```
bg0   #0d0e14   deepest background
bg1   #13141c   panels / toolbars
bg2   #1a1b26   widget backgrounds / hover surfaces
bg3   #22243a   raised widgets
bg4   #2e3055   active / focused surfaces
acc   #00e5ff   primary cyan accent
acc2  #7b2fff   secondary violet accent (split pane)
warn  #ff4b6e   danger / destructive
fg0   #e2e4f0   primary text
fg1   #8890b0   secondary / muted text
fg2   #4a5070   disabled / decorative text
sep   #1e2035   separators and borders
```

You are not required to use these exact names — they are just comments in the CSS.
