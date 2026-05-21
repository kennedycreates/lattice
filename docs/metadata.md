# Lattice Metadata

Lattice stores workspace metadata in a local SQLite database. This is app-local state for now.

## Database Location

Default path:

```text
~/.local/share/lattice/metadata.db
```

The actual path is resolved from GLib's user data directory (`g_get_user_data_dir()` / `glib::user_data_dir()`), then `lattice/metadata.db` is appended.

Lattice does **not** store this database inside the repo and does **not** use filesystem xattrs for metadata at this stage.

## Organization Model

Tints, Shapes, Marks, Tags, and Palettes are separate concepts.

- Tint: global user-defined color category.
- Shape: fixed literal category: `circle`, `square`, `triangle`, `pentagon`, `hexagon`, `octagon`, or `trapezoid`.
- Mark: exactly one Tint plus one Shape applied to a file/folder.
- Tags: secondary text labels. Tags do not own strong visual color.
- Palette: flexible workspace/board, not a rigid project-management object.

Every file/folder resolves to exactly one Mark. If `file_marks` has no row for a path, the resolved Mark is the default Tint `Beige` plus Shape `square`. Lattice does not create a `file_marks` row for every browsed file.

## Current Tables

`tints`
- `id`
- `name`
- `color`
- `position`
- `is_default`

`file_marks`
- `file_path`
- `tint_id`
- `shape`
- `created_at`
- `updated_at`

`tags`
- `id`
- `name`
- `color` (deprecated; ignored for user-facing visual ownership)
- `associated_tint_id` (optional suggestion/default relationship)
- `associated_shape` (optional suggestion/default relationship)

`file_tags`
- `id`
- `file_path`
- `tag_id`

`palettes`
- `id`
- `name`
- `description`
- `created_at`
- `updated_at`

`palette_places`
- `id`
- `palette_id`
- `name`
- `path`
- `position`

`palette_items`
- `id`
- `palette_id`
- `item_type` (`file`, `folder`, or `note`)
- `path`
- `title`
- `body`
- `tint_id`
- `shape`
- `x`
- `y`
- `width`
- `height`
- `position`

Board cards carry spatial state (x, y, width, height) and optional Tint + Shape marks. Removing a card from the board deletes the `palette_items` row only; the underlying file is never touched.

`palette_links`
- `id`
- `palette_id`
- `source_item_id`
- `target_item_id`
- `strength` (`weak` or `strong`)
- `label`

`places`
- `id`
- `name`
- `folder_path`
- `position`

`recent_locations`
- `id`
- `folder_path`
- `last_visited_unix`

`activity_log` and `activity_log_items`
- operation receipt history for Lattice file actions

Legacy `projects` and `project_destinations` tables may remain in existing databases. This is compatibility vocabulary for older Lattice data; the current user-facing concept is Palette. Legacy rows are preserved and migrated into `palettes` and `palette_places` where practical.

## Migration Rules

- Migrations are additive and non-destructive.
- Existing legacy projects are copied to palettes with matching ids when possible.
- Existing legacy project destinations are copied to palette places with matching ids when possible.
- Existing tags remain tags; tag color is kept only as deprecated stored data.
- Existing `file_tags` relationships are preserved.
- Before migrating an on-disk metadata database from an older schema, Lattice creates a timestamped backup next to the database.

## Storage Rules

- Files are keyed by full path for now.
- Tints, Marks, Tags, and Palettes are local to this machine/user profile.
- The schema is initialized automatically on startup.
- Lattice updates tagged paths and explicit file marks when files are renamed or moved through Lattice itself.
- External renames or moves performed outside Lattice can still orphan associations because the current identity model is path-based.

## Workflow Integration (as of 2026-05-18)

Marks (Tint + Shape) are now readable by all major workflow tools:

**Holding Tray**
- Add files by Tint: picker popover → adds all folder items with that tint
- Add files by Shape: picker popover → adds all folder items with that shape
- Apply Mark to tray items: uses active paint mark, requires confirmation
- Reset Mark: reverts tray items to Beige Square, requires confirmation
- Mark badge (color chip + glyph) shown on each tray item

**Search**
- Filter by Tint, by Shape, or by "Default (Beige □)" mark
- Mark filter row appears in the search panel alongside Kind/Age/Size/Tag rows
- Mark filter is applied after tag and filesystem filters

**Action Plans (plan queue)**
- Plan Mode queues file-affecting operations as explicit action payloads instead of executing them immediately.
- Covered actions include copy/move, drag/drop, rename, bulk rename, duplicate, new file/folder, trash/permanent delete, empty trash, trash restore, tag apply/remove, Mark apply/reset, copy paths, and send to Palette.
- `PaintMark` variant: apply Tint + Shape to queued paths; summary "Mark N items TintName Shape"
- `ResetMark` variant: reset queued paths to Beige Square; summary "Reset N items to Beige Square"
- Recursive variants route through `do_paint_folder_recursive` / `reset_mark_recursive`

**Space Viewer**
- Two new views: `BY TINT` and `BY SHAPE`
- After a folder scan completes, mark stats are loaded from the DB and file sizes are computed
- Each row shows: color chip / shape glyph · count · total size · progress bar

**Context Menus (file browser)**
- Select Same Tint: selects all items in the folder with the same tint
- Select Same Shape: selects all items with the same shape
- Select Same Mark: selects all items with matching tint + shape
- Add Same Mark to Tray: adds all matching items to the Holding Tray
- Reset Mark: already wired (paint eraser tool)

**Tints & Tags Panel**
- Tag rows that have an associated Tint or Shape show a small hint ("→ TintName ▲")

**Activity Log**
- Mark apply operations logged as `paint_mark` with summary "Marked N items TintName Shape"
- Mark reset operations logged as `erase_mark` with summary "Reset N items to Beige Square"

## Current Scope

Stored now:
- Tint definitions, including seeded default `Beige` (`#806040`)
- Explicit file/folder Marks
- Tag definitions and file-to-tag assignments
- Palette definitions and pinned Palette places
- Palette Board items and links (active — spatial position, size, Tint/Shape, and link strength)
- Recent folder history used by the sidebar Recent view
- Activity receipts

Not stored yet:
- cloud sync state
- xattrs
- cross-device metadata portability
- search indexes
- thumbnail cache data
