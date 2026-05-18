use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub theme: String,
    pub shortcuts: HashMap<String, String>,
    pub context_menu: ContextMenuConfig,
    pub custom_actions: Vec<CustomActionConfig>,
}

#[derive(Clone, Debug, Default)]
pub struct ContextMenuConfig {
    pub file: Option<Vec<String>>,
    pub folder: Option<Vec<String>>,
    pub background: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomActionConfig {
    pub id: String,
    pub label: String,
    pub argv: Vec<String>,
    pub shortcut: Option<String>,
    pub contexts: Vec<String>,
    pub needs_selection: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            shortcuts: default_shortcuts(),
            context_menu: ContextMenuConfig::default(),
            custom_actions: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Load config from `~/.config/lattice/config.toml`.
    /// Falls back to defaults for missing or malformed individual fields.
    pub fn load() -> Self {
        let config_path = Self::config_dir().join("config.toml");
        if !config_path.exists() {
            write_example_config(&config_path);
            return Self::default();
        }

        let Ok(content) = std::fs::read_to_string(config_path) else {
            return Self::default();
        };

        parse_config(&content)
    }

    pub fn config_dir() -> PathBuf {
        glib::user_config_dir().join("lattice")
    }

    pub fn themes_dir() -> PathBuf {
        Self::config_dir().join("themes")
    }

    pub fn custom_action(&self, id: &str) -> Option<&CustomActionConfig> {
        self.custom_actions.iter().find(|action| action.id == id)
    }
}

fn default_shortcuts() -> HashMap<String, String> {
    [
        ("copy_selection", "Ctrl+C"),
        ("cut_selection", "Ctrl+X"),
        ("paste_clipboard", "Ctrl+V"),
        ("copy_path", "Ctrl+Shift+C"),
        ("new_folder", "Ctrl+N"),
        ("new_text_document", "Ctrl+Shift+N"),
        ("rename", "F2"),
        ("trash", "Delete"),
        ("search", "Ctrl+F"),
        ("filter_tags", "Ctrl+G"),
        ("focus_path", "Ctrl+L"),
        ("refresh", "Ctrl+R"),
        ("show_hidden", "Ctrl+H"),
        ("toggle_sidebar", "Ctrl+B"),
        ("toggle_preview", "Ctrl+P"),
        ("toggle_holding_tray", "Ctrl+Alt+H"),
        ("new_tab", "Ctrl+T"),
        ("close_tab", "Ctrl+W"),
        ("toggle_split", "Ctrl+\\"),
        ("previous_tab", "Ctrl+Page_Up"),
        ("next_tab", "Ctrl+Page_Down"),
        ("back", "Alt+Left"),
        ("forward", "Alt+Right"),
        ("up", "Alt+Up"),
        ("cycle_pane", "F6"),
        ("escape", "Escape"),
        ("view_icons", "Ctrl+1"),
        ("view_list", "Ctrl+2"),
        ("toggle_plan_mode", "Ctrl+Shift+P"),
        ("empty_trash", "Ctrl+Shift+Delete"),
        ("tray_add_selection", "Ctrl+Alt+A"),
        ("tray_move_to_project", "Ctrl+Alt+M"),
        ("tray_copy_to_project", "Ctrl+Alt+C"),
        ("tray_tag", "Ctrl+Alt+T"),
        ("tray_trash", "Ctrl+Alt+Delete"),
        ("tray_copy_paths", "Ctrl+Alt+P"),
        ("tray_clear", "Ctrl+Alt+K"),
    ]
    .into_iter()
    .map(|(action, shortcut)| (action.to_string(), shortcut.to_string()))
    .collect()
}

fn parse_config(content: &str) -> AppConfig {
    let mut config = AppConfig::default();
    let mut section = String::new();
    let mut current_custom: Option<CustomActionConfig> = None;

    for raw_line in content.lines() {
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }

        if line == "[shortcuts]" || line == "[context_menu]" {
            finish_custom(&mut config, &mut current_custom);
            section = line.trim_matches(&['[', ']'][..]).to_string();
            continue;
        }

        if line == "[[custom_actions]]" {
            finish_custom(&mut config, &mut current_custom);
            section = "custom_actions".to_string();
            current_custom = Some(CustomActionConfig {
                id: String::new(),
                label: String::new(),
                argv: Vec::new(),
                shortcut: None,
                contexts: vec!["file".to_string(), "folder".to_string()],
                needs_selection: true,
            });
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        match section.as_str() {
            "" => {
                if key == "theme" {
                    if let Some(theme) = parse_string(value) {
                        if !theme.is_empty() {
                            config.theme = theme;
                        }
                    }
                }
            }
            "shortcuts" => {
                if let Some(shortcut) = parse_string(value) {
                    if shortcut.is_empty() {
                        config.shortcuts.remove(key);
                    } else {
                        config.shortcuts.insert(key.to_string(), shortcut);
                    }
                }
            }
            "context_menu" => {
                let entries = parse_string_array(value);
                match key {
                    "file" => config.context_menu.file = Some(entries),
                    "folder" => config.context_menu.folder = Some(entries),
                    "background" => config.context_menu.background = Some(entries),
                    _ => {}
                }
            }
            "custom_actions" => {
                if let Some(action) = current_custom.as_mut() {
                    match key {
                        "id" => {
                            action.id = parse_action_id(value).unwrap_or_default();
                        }
                        "label" => {
                            action.label = parse_string(value).unwrap_or_default();
                        }
                        "argv" => {
                            action.argv = parse_string_array(value);
                        }
                        "shortcut" => {
                            action.shortcut = parse_string(value).filter(|s| !s.is_empty());
                        }
                        "contexts" => {
                            action.contexts = parse_string_array(value);
                        }
                        "needs_selection" => {
                            if let Some(value) = parse_bool(value) {
                                action.needs_selection = value;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    finish_custom(&mut config, &mut current_custom);
    config
}

fn finish_custom(config: &mut AppConfig, current_custom: &mut Option<CustomActionConfig>) {
    let Some(mut action) = current_custom.take() else {
        return;
    };
    action.id = sanitize_action_id(&action.id);
    if action.id.is_empty() || action.label.trim().is_empty() || action.argv.is_empty() {
        return;
    }
    if action.contexts.is_empty() {
        action.contexts = vec!["file".to_string(), "folder".to_string()];
    }
    config.custom_actions.push(action);
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn parse_action_id(value: &str) -> Option<String> {
    parse_string(value).map(|id| sanitize_action_id(&id))
}

fn sanitize_action_id(id: &str) -> String {
    id.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        .collect()
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return None;
    }

    let inner = &value[1..value.len() - 1];
    let mut output = String::new();
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            output.push(match ch {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            output.push(ch);
        }
    }
    Some(output)
}

fn parse_string_array(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('[') || !value.ends_with(']') {
        return Vec::new();
    }

    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    for ch in value[1..value.len() - 1].chars() {
        if escaped {
            current.push(match ch {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' if in_quote => {
                in_quote = false;
                values.push(current.clone());
                current.clear();
            }
            '"' => in_quote = true,
            _ if in_quote => current.push(ch),
            _ => {}
        }
    }
    values
}

fn write_example_config(path: &PathBuf) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let _ = std::fs::write(path, EXAMPLE_CONFIG);
}

const EXAMPLE_CONFIG: &str = r#"# Lattice config
# This file is generated on first run. Uncomment and edit entries as needed.
# Existing config files are not rewritten automatically.

# Theme name. "default" loads Lattice's bundled theme. Other names load
# ~/.config/lattice/themes/<name>.css when present.
theme = "default"

# Built-in keyboard shortcuts. Uncomment a line to override its default.
# Set a shortcut to "" to disable it.
# Supported key names include letters, Delete, Escape, Left, Right, Up, Down,
# Page_Up, Page_Down, F1-F12, and Backslash.
# Supported modifiers are Ctrl, Shift, and Alt, joined with "+".
[shortcuts]
# copy_selection = "Ctrl+C"
# cut_selection = "Ctrl+X"
# paste_clipboard = "Ctrl+V"
# copy_path = "Ctrl+Shift+C"
# new_folder = "Ctrl+N"
# new_text_document = "Ctrl+Shift+N"
# rename = "F2"
# trash = "Delete"
# search = "Ctrl+F"
# filter_tags = "Ctrl+G"
# focus_path = "Ctrl+L"
# refresh = "Ctrl+R"
# show_hidden = "Ctrl+H"
# toggle_sidebar = "Ctrl+B"
# toggle_preview = "Ctrl+P"
# toggle_holding_tray = "Ctrl+Alt+H"
# new_tab = "Ctrl+T"
# close_tab = "Ctrl+W"
# toggle_split = "Ctrl+Backslash"
# previous_tab = "Ctrl+Page_Up"
# next_tab = "Ctrl+Page_Down"
# back = "Alt+Left"
# forward = "Alt+Right"
# up = "Alt+Up"
# cycle_pane = "F6"
# escape = "Escape"
# view_icons = "Ctrl+1"
# view_list = "Ctrl+2"
# toggle_plan_mode = "Ctrl+Shift+P"
# empty_trash = "Ctrl+Shift+Delete"
# tray_add_selection = "Ctrl+Alt+A"
# tray_move_to_project = "Ctrl+Alt+M"
# tray_copy_to_project = "Ctrl+Alt+C"
# tray_tag = "Ctrl+Alt+T"
# tray_trash = "Ctrl+Alt+Delete"
# tray_copy_paths = "Ctrl+Alt+P"
# tray_clear = "Ctrl+Alt+K"
#
# Custom actions can also be bound with custom.<id>.
# custom.open_in_gimp = "Ctrl+Alt+G"
# custom.compress_here = "Ctrl+Alt+Z"
# custom.open_terminal_at_selection = "Ctrl+Alt+O"

# Right-click menu order. Uncomment one of these arrays to replace the default
# menu for that context. Unknown entries are ignored.
#
# Shared item entries:
#   separator, open, rename, bulk_rename, duplicate, copy_path,
#   add_to_holding_tray, terminal_here, send_to_project, add_tag, remove_tag,
#   move_to_trash, delete_permanently, custom.<id>
#
# File-only entries:
#   open_with
#
# Folder-only entries:
#   open_new_tab, open_in_pane, triage_folder, pin_place, pin_project
#
# Background entries:
#   new_folder, new_text_document, pin_place, pin_project, terminal_here,
#   copy_path, custom.<id>
#
# Conditional entries are skipped when they do not apply. For example,
# bulk_rename appears only with two or more selected items; open_in_pane adapts
# to the current pane layout; custom actions appear only in their contexts.
[context_menu]
# file = ["open", "open_with", "separator", "add_to_holding_tray", "separator", "rename", "bulk_rename", "duplicate", "copy_path", "terminal_here", "separator", "send_to_project", "add_tag", "remove_tag", "separator", "move_to_trash", "delete_permanently"]
# folder = ["open", "open_new_tab", "open_in_pane", "triage_folder", "separator", "add_to_holding_tray", "separator", "rename", "bulk_rename", "duplicate", "copy_path", "terminal_here", "separator", "pin_place", "pin_project", "send_to_project", "add_tag", "remove_tag", "separator", "move_to_trash", "delete_permanently"]
# background = ["new_folder", "new_text_document", "separator", "pin_place", "pin_project", "terminal_here", "copy_path"]
#
# Example with custom actions inserted:
# file = ["open", "open_with", "separator", "custom.open_in_gimp", "add_to_holding_tray", "separator", "rename", "copy_path", "terminal_here", "separator", "move_to_trash"]
# folder = ["open", "open_new_tab", "open_in_pane", "triage_folder", "separator", "custom.compress_here", "pin_place", "pin_project", "terminal_here"]
# background = ["new_folder", "new_text_document", "separator", "custom.open_terminal_at_selection", "terminal_here", "copy_path"]

# Custom actions never run through a shell. Each argv entry is passed as a
# separate argument, so shell syntax such as pipes, globs, and "$VAR" is not
# expanded unless you explicitly run a shell as argv[0].
#
# Fields:
#   id              ASCII letters, numbers, "_" and "-" only. Used by
#                   custom.<id> menu entries and shortcuts.
#   label           Text shown in menus and status messages.
#   argv            Command and arguments to spawn.
#   shortcut        Optional key binding. Same syntax as [shortcuts].
#   contexts        Any of "file", "folder", "background".
#   needs_selection If true, the action will not run without selected items.
#
# Placeholders:
#   {paths} expands selected paths into separate argv entries.
#   {path} expands the first selected path, or disappears if none is selected.
#   {cwd} expands the active folder.
#   Embedded {path} and {cwd} are replaced inside ordinary argument strings.
#   Embedded {paths} is not expanded; use "{paths}" as a whole argv entry.

# Example: open selected images in GIMP.
#[[custom_actions]]
#id = "open_in_gimp"
#label = "Open in GIMP"
#argv = ["gimp", "{paths}"]
#shortcut = "Ctrl+Alt+G"
#contexts = ["file"]
#needs_selection = true

# Example: open File Roller's create-archive flow for selected files.
#[[custom_actions]]
#id = "compress_here"
#label = "Compress Here"
#argv = ["file-roller", "--add", "{paths}"]
#shortcut = "Ctrl+Alt+Z"
#contexts = ["file", "folder"]
#needs_selection = true

# Example: run a shell command in the active folder.
#[[custom_actions]]
#id = "open_terminal_at_selection"
#label = "Terminal at Selection"
#argv = ["sh", "-lc", "cd \"${1:-$PWD}\" && exec ${TERMINAL:-x-terminal-emulator}", "sh", "{path}"]
#shortcut = "Ctrl+Alt+O"
#contexts = ["file", "folder", "background"]
#needs_selection = false
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_action_and_context_menu() {
        let config = parse_config(
            r#"
theme = "high-contrast"
[shortcuts]
custom.open_in_gimp = "Ctrl+Alt+G"
[context_menu]
file = ["open", "custom.open_in_gimp"]
[[custom_actions]]
id = "open_in_gimp"
label = "Open in GIMP"
argv = ["gimp", "{paths}"]
contexts = ["file"]
"#,
        );

        assert_eq!(config.theme, "high-contrast");
        assert_eq!(
            config
                .shortcuts
                .get("custom.open_in_gimp")
                .map(String::as_str),
            Some("Ctrl+Alt+G")
        );
        assert_eq!(
            config.context_menu.file,
            Some(vec!["open".to_string(), "custom.open_in_gimp".to_string()])
        );
        assert_eq!(config.custom_actions.len(), 1);
        assert_eq!(config.custom_actions[0].argv, vec!["gimp", "{paths}"]);
    }

    #[test]
    fn generated_example_documents_supported_config_surface() {
        for shortcut in default_shortcuts().keys() {
            assert!(
                EXAMPLE_CONFIG.contains(&format!("# {shortcut} = ")),
                "missing shortcut example for {shortcut}"
            );
        }

        for entry in [
            "open",
            "open_with",
            "open_new_tab",
            "open_in_pane",
            "triage_folder",
            "add_to_holding_tray",
            "rename",
            "bulk_rename",
            "duplicate",
            "copy_path",
            "terminal_here",
            "pin_place",
            "pin_project",
            "send_to_project",
            "add_tag",
            "remove_tag",
            "move_to_trash",
            "delete_permanently",
            "new_folder",
            "new_text_document",
            "custom.<id>",
        ] {
            assert!(
                EXAMPLE_CONFIG.contains(entry),
                "missing context menu entry documentation for {entry}"
            );
        }
    }
}
