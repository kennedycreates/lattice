# Lattice File Picker

Lattice has a built-in file/folder picker — "Lattice Lite" — that replaces both GTK's native `FileDialog` and ad hoc path-entry text fields. It is designed to feel like a stripped-down Lattice browser: same styling, same sidebar places, same navigation model, without any destructive operations.

## Modes

| Mode | Description | Confirm label |
|------|-------------|---------------|
| `OpenFolder` | Select one existing folder | "Select Folder" |
| `OpenFile` | Select one existing file | "Open" |
| `OpenFiles` | Select one or more existing files | "Open" |
| `SaveFile` | Choose a folder and enter a filename | "Save" |

The picker intentionally omits: delete, trash, rename, painting mode, bulk tools, action plans, palette board editing, Space Viewer, destructive file operations.

## Internal API

### `show_picker_modal`

Shows the picker as an in-window modal overlay (via `ModalHost`). Used by all internal Lattice features.

```rust
use crate::ui::picker::{show_picker_modal, PickerConfig, PickerResult};

show_picker_modal(
    &self.modal_host,
    PickerConfig::open_folder(self.current_dir_for(slot)),
    &self.user_places.borrow(),
    &self.cloud_locations.borrow(),
    &self.metadata.borrow().list_recent_locations(8).unwrap_or_default(),
    move |result| {
        let PickerResult::Single(path) = result else { return };
        // use path
    },
    || {}, // on_cancel
);
```

The modal hides itself before calling either callback.

### `PickerConfig` constructors

```rust
PickerConfig::open_folder(initial_dir: PathBuf) -> PickerConfig
PickerConfig::open_file(initial_dir: PathBuf) -> PickerConfig
PickerConfig::open_files(initial_dir: PathBuf) -> PickerConfig
PickerConfig::save_file(initial_dir: PathBuf, suggested_name: &str) -> PickerConfig
```

Pass `glib::home_dir()` as `initial_dir` if no better context is available.

### `PickerResult`

```rust
pub enum PickerResult {
    Single(PathBuf),    // OpenFolder, OpenFile, SaveFile
    Multiple(Vec<PathBuf>), // OpenFiles
}
```

### `launch_picker_window`

Creates a standalone `ApplicationWindow` for CLI mode. Prints paths to stdout and exits.

```rust
launch_picker_window(app, picker_config, &places, &cloud_locs, &recent_dirs);
```

## CLI Usage

```
lattice --picker open              # pick one file, print path to stdout
lattice --picker open-files        # pick multiple files
lattice --picker folder            # pick a folder
lattice --picker save              # pick a save location

# Options
--path /start/dir    Start the picker in this directory
--name suggested.txt Suggested filename for save mode
```

Exit codes:
- `0` — user confirmed a selection
- `1` — user cancelled or window was closed

Example shell usage:

```bash
file=$(lattice --picker open --path ~/Documents)
[ $? -eq 0 ] && echo "Selected: $file"
```

## Current Internal Integration Points

These Lattice features now use `show_picker_modal` instead of text entry or GTK native dialogs:

| Feature | Mode | Location |
|---------|------|----------|
| Palette Board → Add File Card | `OpenFile` | `show_add_file_card_dialog` |
| Palette Board → Add Folder Card | `OpenFolder` | `show_add_folder_card_dialog` |
| Palette Landing → Pin Folder | `OpenFolder` | `show_pin_folder_dialog` (Browse button) |
| Cloud → Add Cloud Drive | `OpenFolder` | `show_add_cloud_dialog` (Browse button) |
| Cloud → Edit Cloud Drive | `OpenFolder` | `show_edit_cloud_dialog` (Browse button) |
| Media Convert → Choose Output Folder | `OpenFolder` | `connect_folder_pick` callback |

## Picker UI Structure

```
picker-root (560×400 min)
├── picker-nav-bar   [← → ↑]  /current/path
├── ─ separator ─
├── picker-body
│   ├── picker-sidebar (160px)
│   │   ├── PLACES
│   │   │   ├── 🏠 Home
│   │   │   └── 📌 Pinned places…
│   │   ├── CLOUD (if any)
│   │   │   └── ☁ Cloud locations…
│   │   └── RECENT
│   │       └── 🕐 Recent dirs…
│   ├── │ (separator)
│   └── picker-pane
│       ├── file list (ListBox, scrollable)
│       │   └── rows: [icon] [name] [size]
│       └── save-row (SaveFile mode only)
│           └── "Save as:" [entry]
└── [Cancel] [Open / Select Folder / Save]  (in modal actions row)
```

Sidebar buttons highlight (`.active`) when the current directory matches.  
Double-clicking a folder navigates into it. Double-clicking a file (in file modes) confirms.

## Navigation

- Back / Forward: standard history stack per picker instance
- Up: navigate to parent directory
- Sidebar buttons: jump to place/cloud/recent location
- Hidden files: not shown by default (`show_hidden: false`)

## Future: xdg-desktop-portal FileChooser

The picker is designed as a foundation for future portal integration. The portal backend would call `launch_picker_window` or a thin wrapper around `build_picker_content` to present the Lattice picker to other applications that request a file dialog via the portal protocol.

The picker's intentional omission of destructive operations makes it safe to surface to untrusted callers.
