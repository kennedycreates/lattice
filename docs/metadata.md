# Lattice Metadata

Lattice stores workspace metadata in a local SQLite database. This is app-local state for now.

## Database Location

Default path:

```text
~/.local/share/lattice/metadata.db
```

The actual path is resolved from GLib's user data directory (`g_get_user_data_dir()` / `glib::user_data_dir()`), then `lattice/metadata.db` is appended.

Lattice does **not** store this database inside the repo and does **not** use filesystem xattrs for tags/projects at this stage.

## Current Tables

`projects`
- `id`
- `name`
- `root_path`
- `accent`

`tags`
- `id`
- `name`
- `color`

`file_tags`
- `id`
- `file_path`
- `tag_id`

`project_destinations`
- `id`
- `project_id`
- `name`
- `relative_path`

`recent_locations`
- `id`
- `folder_path`
- `last_visited_unix`

## Storage Rules

- Files are keyed by full path for now.
- Tags and projects are local to this machine/user profile.
- The schema is initialized automatically on startup.
- The current migration strategy is intentionally simple: create missing tables and set the SQLite `user_version`.
- Lattice updates tagged paths when files are renamed, moved to a project, or moved to trash through Lattice itself.
- External renames or moves performed outside Lattice can still orphan tag associations because the current identity model is path-based.

## Current Scope

Stored now:
- pinned project roots
- tag definitions
- file-to-tag assignments
- project root destinations for `Send to Project`
- recent folder history used by the sidebar Recent view

Not stored yet:
- cloud sync state
- xattrs
- cross-device tag portability
- search indexes
- thumbnail cache data
