//! Keyboard-shortcut parsing and matching, split out of the main_window module.

use crate::config::AppConfig;
use gtk::gdk;

use super::WindowCommand;

pub(super) fn relevant_modifiers(modifiers: gdk::ModifierType) -> gdk::ModifierType {
    modifiers
        & (gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK)
}

pub(super) fn key_char(key: gdk::Key) -> Option<char> {
    key.to_unicode().map(|value| value.to_ascii_lowercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyBinding {
    ctrl: bool,
    shift: bool,
    alt: bool,
    key: BindingKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKey {
    Char(char),
    Named(&'static str),
}

#[cfg(test)]
pub(super) fn window_command_from_key(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<WindowCommand> {
    configured_window_command_from_key(&AppConfig::default(), key, modifiers)
}

pub(super) fn configured_window_command_from_key(
    config: &AppConfig,
    key: gdk::Key,
    modifiers: gdk::ModifierType,
) -> Option<WindowCommand> {
    for (action_id, shortcut) in &config.shortcuts {
        let Some(binding) = parse_key_binding(shortcut) else {
            continue;
        };
        if binding.matches(key, modifiers) {
            if let Some(custom_id) = action_id.strip_prefix("custom.") {
                return Some(WindowCommand::CustomAction(custom_id.to_string()));
            }
            if let Some(command) = builtin_command(action_id) {
                return Some(command);
            }
        }
    }

    for action in &config.custom_actions {
        let Some(shortcut) = &action.shortcut else {
            continue;
        };
        let Some(binding) = parse_key_binding(shortcut) else {
            continue;
        };
        if binding.matches(key, modifiers) {
            return Some(WindowCommand::CustomAction(action.id.clone()));
        }
    }

    None
}

pub(super) fn builtin_command(action_id: &str) -> Option<WindowCommand> {
    match action_id {
        "copy_selection" => Some(WindowCommand::CopySelection),
        "cut_selection" => Some(WindowCommand::CutSelection),
        "paste_clipboard" => Some(WindowCommand::PasteClipboard),
        "copy_path" => Some(WindowCommand::CopyPathText),
        "new_folder" => Some(WindowCommand::NewFolder),
        "new_text_document" => Some(WindowCommand::NewTextDocument),
        "rename" => Some(WindowCommand::RenameSelection),
        "trash" => Some(WindowCommand::TrashSelection),
        "search" => Some(WindowCommand::OpenSearch),
        "filter_tags" => Some(WindowCommand::ToggleFilter),
        "focus_path" => Some(WindowCommand::FocusPath),
        "refresh" => Some(WindowCommand::Refresh),
        "show_hidden" => Some(WindowCommand::ToggleHidden),
        "toggle_shape_badges" => Some(WindowCommand::ToggleShapeBadges),
        "sort_order" => Some(WindowCommand::SortOrder),
        "toggle_sidebar" => Some(WindowCommand::ToggleSidebar),
        "toggle_preview" => Some(WindowCommand::TogglePreview),
        "toggle_holding_tray" => Some(WindowCommand::ToggleHoldingTray),
        "tray_add_selection" => Some(WindowCommand::TrayAddSelection),
        "tray_move_to_project" => Some(WindowCommand::TrayMoveToProject),
        "tray_copy_to_project" => Some(WindowCommand::TrayCopyToProject),
        "tray_tag" => Some(WindowCommand::TrayTag),
        "tray_trash" => Some(WindowCommand::TrayTrash),
        "tray_copy_paths" => Some(WindowCommand::TrayCopyPaths),
        "tray_clear" => Some(WindowCommand::TrayClear),
        "new_tab" => Some(WindowCommand::NewTab),
        "close_tab" => Some(WindowCommand::CloseTab),
        "toggle_split" => Some(WindowCommand::ToggleSplit),
        "previous_tab" => Some(WindowCommand::PreviousTab),
        "next_tab" => Some(WindowCommand::NextTab),
        "back" => Some(WindowCommand::GoBack),
        "forward" => Some(WindowCommand::GoForward),
        "up" => Some(WindowCommand::GoUp),
        "cycle_pane" => Some(WindowCommand::CyclePane),
        "escape" => Some(WindowCommand::Escape),
        "open_home" => Some(WindowCommand::OpenHome),
        "open_system_drives" => Some(WindowCommand::OpenSystemDrives),
        "open_recent" => Some(WindowCommand::OpenRecent),
        "open_trash" => Some(WindowCommand::OpenTrash),
        "open_palettes" => Some(WindowCommand::OpenPalettes),
        "open_tints_tags" => Some(WindowCommand::OpenTintsTags),
        "open_space_viewer" => Some(WindowCommand::OpenSpaceViewer),
        "open_triage" => Some(WindowCommand::OpenTriage),
        "open_bulk_naming" => Some(WindowCommand::OpenBulkNaming),
        "open_convert" => Some(WindowCommand::OpenConvert),
        "open_activity_log" => Some(WindowCommand::OpenActivityLog),
        "view_icons" => Some(WindowCommand::SetViewIcons),
        "view_list" => Some(WindowCommand::SetViewList),
        "toggle_plan_mode" => Some(WindowCommand::TogglePlanMode),
        "toggle_paint_mode" => Some(WindowCommand::TogglePaintMode),
        "paint_cursor" => Some(WindowCommand::PaintCursor),
        "paint_brush" => Some(WindowCommand::PaintBrush),
        "paint_eraser" => Some(WindowCommand::PaintEraser),
        "paint_eyedropper" => Some(WindowCommand::PaintEyedropper),
        "paint_fill" => Some(WindowCommand::PaintFill),
        "paint_undo" => Some(WindowCommand::PaintUndo),
        "paint_redo" => Some(WindowCommand::PaintRedo),
        "paint_toggle_contents" => Some(WindowCommand::PaintToggleContents),
        "empty_trash" => Some(WindowCommand::EmptyTrash),
        "tray_add_by_tint" => Some(WindowCommand::TrayAddByTint),
        "tray_add_by_shape" => Some(WindowCommand::TrayAddByShape),
        "tray_apply_mark" => Some(WindowCommand::TrayApplyMark),
        "tray_reset_mark" => Some(WindowCommand::TrayResetMark),
        "plan_execute" => Some(WindowCommand::PlanExecute),
        "plan_clear" => Some(WindowCommand::PlanClear),
        "convert_start" => Some(WindowCommand::ConvertStart),
        "convert_cancel" => Some(WindowCommand::ConvertCancel),
        "convert_retry_failed" => Some(WindowCommand::ConvertRetryFailed),
        "convert_open_output" => Some(WindowCommand::ConvertOpenOutput),
        "convert_dismiss" => Some(WindowCommand::ConvertDismiss),
        _ => None,
    }
}

fn parse_key_binding(shortcut: &str) -> Option<KeyBinding> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut key = None;

    for part in shortcut.split('+') {
        let part = part.trim();
        let normalized = part.to_ascii_lowercase().replace('_', "");
        match normalized.as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            "esc" | "escape" => key = Some(BindingKey::Named("escape")),
            "delete" | "del" => key = Some(BindingKey::Named("delete")),
            "backspace" => key = Some(BindingKey::Named("backspace")),
            "enter" | "return" => key = Some(BindingKey::Named("enter")),
            "home" => key = Some(BindingKey::Named("home")),
            "end" => key = Some(BindingKey::Named("end")),
            "left" => key = Some(BindingKey::Named("left")),
            "right" => key = Some(BindingKey::Named("right")),
            "up" => key = Some(BindingKey::Named("up")),
            "down" => key = Some(BindingKey::Named("down")),
            "pageup" => key = Some(BindingKey::Named("pageup")),
            "pagedown" => key = Some(BindingKey::Named("pagedown")),
            "backslash" => key = Some(BindingKey::Char('\\')),
            f if f.len() > 1 && f.starts_with('f') => match f {
                "f1" => key = Some(BindingKey::Named("f1")),
                "f2" => key = Some(BindingKey::Named("f2")),
                "f3" => key = Some(BindingKey::Named("f3")),
                "f4" => key = Some(BindingKey::Named("f4")),
                "f5" => key = Some(BindingKey::Named("f5")),
                "f6" => key = Some(BindingKey::Named("f6")),
                "f7" => key = Some(BindingKey::Named("f7")),
                "f8" => key = Some(BindingKey::Named("f8")),
                "f9" => key = Some(BindingKey::Named("f9")),
                "f10" => key = Some(BindingKey::Named("f10")),
                "f11" => key = Some(BindingKey::Named("f11")),
                "f12" => key = Some(BindingKey::Named("f12")),
                _ => return None,
            },
            value => {
                let mut chars = value.chars();
                let ch = chars.next()?;
                if chars.next().is_none() {
                    key = Some(BindingKey::Char(ch.to_ascii_lowercase()));
                } else {
                    return None;
                }
            }
        }
    }

    Some(KeyBinding {
        ctrl,
        shift,
        alt,
        key: key?,
    })
}

impl KeyBinding {
    fn matches(self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        let modifiers = relevant_modifiers(modifiers);
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        self.ctrl == ctrl && self.shift == shift && self.alt == alt && self.key.matches(key)
    }
}

impl BindingKey {
    fn matches(self, key: gdk::Key) -> bool {
        match self {
            Self::Char(ch) => key_char(key) == Some(ch),
            Self::Named(name) => matches!(
                (name, key),
                ("escape", gdk::Key::Escape)
                    | ("delete", gdk::Key::Delete)
                    | ("backspace", gdk::Key::BackSpace)
                    | ("enter", gdk::Key::Return)
                    | ("enter", gdk::Key::KP_Enter)
                    | ("home", gdk::Key::Home)
                    | ("end", gdk::Key::End)
                    | ("left", gdk::Key::Left)
                    | ("right", gdk::Key::Right)
                    | ("up", gdk::Key::Up)
                    | ("down", gdk::Key::Down)
                    | ("pageup", gdk::Key::Page_Up)
                    | ("pagedown", gdk::Key::Page_Down)
                    | ("f1", gdk::Key::F1)
                    | ("f2", gdk::Key::F2)
                    | ("f3", gdk::Key::F3)
                    | ("f4", gdk::Key::F4)
                    | ("f5", gdk::Key::F5)
                    | ("f6", gdk::Key::F6)
                    | ("f7", gdk::Key::F7)
                    | ("f8", gdk::Key::F8)
                    | ("f9", gdk::Key::F9)
                    | ("f10", gdk::Key::F10)
                    | ("f11", gdk::Key::F11)
                    | ("f12", gdk::Key::F12)
            ),
        }
    }
}
