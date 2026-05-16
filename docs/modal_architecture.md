# Lattice Modal Architecture

## Overview

Lattice uses an **in-window modal overlay** for all internal dialog windows.
No separate `GtkWindow`, `GtkDialog`, or `gtk::AlertDialog` is used for normal
app dialogs.  This eliminates the first-frame squash rendering bug that afflicted
separate popup windows on GTK4/Wayland.

---

## Widget Tree

```
ApplicationWindow
└── GtkOverlay  ← ModalHost::overlay  (set as window child)
    ├── [main child] GtkBox  — normal Lattice UI (toolbar, body, etc.)
    └── [overlay]   GtkOverlay  — .modal-layer  (hidden when no dialog open)
        ├── [main child] GtkBox  — .modal-scrim  (dim backdrop)
        └── [overlay]   GtkBox  — .modal-panel  (centered dialog card)
            ├── title row   (.modal-title)
            ├── separator   (.modal-title-sep)
            ├── content     (.modal-content)
            ├── separator   (.modal-actions-sep)
            └── actions row (.modal-actions)
```

---

## Public API  (`src/ui/modal_host.rs`)

| Method | Use case |
|---|---|
| `show_input(title, prompt, initial, confirm, on_accept)` | Rename, New Folder, Pin Project, Add Tag |
| `show_confirm(title, prompt, confirm, dangerous, scrim_dismisses, on_accept)` | Destructive confirmations |
| `show_error(title, detail)` | Error notifications |
| `show_with_custom_ui(title, content, actions, scrim_dismisses, dismiss_cb)` | Complex dialogs (tag list, project chooser, conflict) |
| `hide()` | Close the current modal from within a callback |

Helper functions also exported:
- `build_modal_button(label, kind, on_click) -> Button`
- `build_modal_actions() -> GtkBox`
- `build_modal_prompt(text) -> Label`

### ButtonKind

| Variant | CSS class | Usage |
|---|---|---|
| `Primary` | `.modal-primary-button` | Confirm / accept |
| `Danger` | `.modal-danger-button` | Destructive action |
| `Secondary` | `.modal-secondary-button` | Cancel |

---

## Safety rules

1. **Scrim dismissal** — `show_with_custom_ui(..., scrim_dismisses: false, ...)` must be
   used for any dialog where the user must make an explicit choice (destructive
   confirmations, multi-choice conflicts).  Only error dialogs and input dialogs
   (Rename, New Folder, etc.) allow scrim/Escape dismissal.

2. **Dangerous buttons** — destructive actions must use `ButtonKind::Danger` so they
   render in the crimson-thorn colour distinct from primary actions.

3. **No file operations must be silently dropped** — every dialog that triggers a
   file operation must still require explicit confirmation.

---

## Known exceptions

### DnD / paste conflict dialog (`show_conflict_dialog`)

The drag-and-drop and paste file conflict chooser (`Skip / Keep Both / Replace`) is
implemented as an `async fn` using `gtk::AlertDialog::choose_future`.  This is the
**non-deprecated** GTK4 async dialog API and is not subject to the first-frame
squash bug (it is triggered by background async operations, not from the main UI).

Migrating it to `ModalHost` would require converting the async flow to a
callback chain — a significant structural change with no rendering benefit.
It is intentionally left as-is.

---

## Adding a new dialog

1. If it is a simple text input: call `modal_host.show_input(...)`.
2. If it is a simple confirmation: call `modal_host.show_confirm(...)`.
3. If it needs custom content (checkboxes, radio buttons, lists):
   - Build a `GtkBox` content widget.
   - Build a `build_modal_actions()` row with `build_modal_button(...)` calls.
   - Each button callback must call `host.hide()` before or after its logic.
   - Call `modal_host.show_with_custom_ui(title, &content, &actions, scrim_dismisses, dismiss_cb)`.
4. **Do NOT** create a new `gtk::Dialog`, `gtk::AlertDialog`, or `ApplicationWindow`
   for an internal Lattice dialog.  Any exception requires an explicit comment
   explaining why an in-window modal is insufficient.

---

## CSS classes

All modal styling lives in `themes/default.css` under the heading
`In-window Modal System`.  Classes:

| Class | Element |
|---|---|
| `.modal-layer` | The outer GtkOverlay (hidden when inactive) |
| `.modal-scrim` | The dim backdrop GtkBox |
| `.modal-panel` | The centered dialog card |
| `.modal-title` | The dialog title label |
| `.modal-title-sep` | Separator below the title |
| `.modal-content` | Content area wrapper |
| `.modal-actions-sep` | Separator above the actions row |
| `.modal-actions-wrapper` | Padding wrapper around actions |
| `.modal-actions` | The horizontal button row |
| `.modal-primary-button` | Accept / confirm action |
| `.modal-danger-button` | Destructive action |
| `.modal-secondary-button` | Cancel action |

---

## Migrated dialogs

The following dialogs were converted from `gtk::Dialog` / `gtk::AlertDialog`
to `ModalHost` in the M0 → M1 refactor:

- Single-file Rename
- Bulk Rename
- New Folder
- Pin as Project
- Add Tag
- Remove Tag
- Send to Project
- File Conflict (project transfer)
- All `show_error_dialog` call sites (~25 error notifications)

---

## History

The separate-popup approach (using deprecated `gtk::Dialog::run_async`) caused
a first-frame squash bug on GTK4/Wayland: dialog content appeared squashed for
~100 ms after the popup opened, then expanded.  Patches using `measure()`,
`set_default_size()`, `queue_resize()`, and similar hacks could not fully
eliminate the problem because it was structural — GTK does not guarantee
correct layout until a window is realized and mapped.

The `ModalHost` approach avoids separate toplevels entirely.  The dialog card
is always part of the main window's widget tree, so its layout is computed in
the same pass as the rest of the UI.  Content appears at full size immediately
on the first visible frame.
