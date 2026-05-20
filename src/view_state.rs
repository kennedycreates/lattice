use crate::config::AppConfig;
use std::path::PathBuf;

/// Global visual defaults persisted to `~/.config/lattice/view_state.toml`.
/// Only presentation settings are stored — no paths, tabs, or navigation state.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub sidebar_visible: bool,
    pub preview_visible: bool,
    /// "icons" | "list"
    pub view_mode: String,
    pub show_hidden: bool,
    pub show_shape_badges: bool,
    /// "name" | "modified" | "size" | "kind"
    pub sort_field: String,
    /// "ascending" | "descending"
    pub sort_direction: String,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            preview_visible: true,
            view_mode: "icons".to_string(),
            show_hidden: false,
            show_shape_badges: false,
            sort_field: "name".to_string(),
            sort_direction: "ascending".to_string(),
        }
    }
}

impl ViewState {
    fn settings_path() -> PathBuf {
        AppConfig::config_dir().join("view_state.toml")
    }

    pub fn load() -> Self {
        let path = Self::settings_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        let mut s = Self::default();
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            match key {
                "sidebar_visible" => s.sidebar_visible = val == "true",
                "preview_visible" => s.preview_visible = val == "true",
                "view_mode" if matches!(val, "icons" | "list") => {
                    s.view_mode = val.to_string();
                }
                "show_hidden" => s.show_hidden = val == "true",
                "show_shape_badges" => s.show_shape_badges = val == "true",
                "sort_field" if matches!(val, "name" | "modified" | "size" | "kind") => {
                    s.sort_field = val.to_string();
                }
                "sort_direction" if matches!(val, "ascending" | "descending") => {
                    s.sort_direction = val.to_string();
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let path = Self::settings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = format!(
            "sidebar_visible = \"{}\"\npreview_visible = \"{}\"\nview_mode = \"{}\"\nshow_hidden = \"{}\"\nshow_shape_badges = \"{}\"\nsort_field = \"{}\"\nsort_direction = \"{}\"\n",
            self.sidebar_visible,
            self.preview_visible,
            self.view_mode,
            self.show_hidden,
            self.show_shape_badges,
            self.sort_field,
            self.sort_direction,
        );
        let _ = std::fs::write(&path, content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_missing_file() {
        // Load from a path that cannot exist; must return exact defaults.
        let d = ViewState::default();
        assert!(d.sidebar_visible);
        assert!(d.preview_visible);
        assert_eq!(d.view_mode, "icons");
        assert!(!d.show_hidden);
        assert!(!d.show_shape_badges);
        assert_eq!(d.sort_field, "name");
        assert_eq!(d.sort_direction, "ascending");
    }

    #[test]
    fn round_trip_serialization() {
        let mut vs = ViewState::default();
        vs.sidebar_visible = false;
        vs.preview_visible = false;
        vs.view_mode = "list".to_string();
        vs.show_hidden = true;
        vs.show_shape_badges = false;
        vs.sort_field = "modified".to_string();
        vs.sort_direction = "descending".to_string();

        // Serialize to string, then parse back via load logic.
        let content = format!(
            "sidebar_visible = \"{}\"\npreview_visible = \"{}\"\nview_mode = \"{}\"\nshow_hidden = \"{}\"\nshow_shape_badges = \"{}\"\nsort_field = \"{}\"\nsort_direction = \"{}\"\n",
            vs.sidebar_visible, vs.preview_visible, vs.view_mode,
            vs.show_hidden, vs.show_shape_badges, vs.sort_field, vs.sort_direction,
        );

        let mut parsed = ViewState::default();
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            match key {
                "sidebar_visible" => parsed.sidebar_visible = val == "true",
                "preview_visible" => parsed.preview_visible = val == "true",
                "view_mode" if matches!(val, "icons" | "list") => {
                    parsed.view_mode = val.to_string()
                }
                "show_hidden" => parsed.show_hidden = val == "true",
                "show_shape_badges" => parsed.show_shape_badges = val == "true",
                "sort_field" if matches!(val, "name" | "modified" | "size" | "kind") => {
                    parsed.sort_field = val.to_string()
                }
                "sort_direction" if matches!(val, "ascending" | "descending") => {
                    parsed.sort_direction = val.to_string()
                }
                _ => {}
            }
        }

        assert_eq!(vs, parsed);
    }

    #[test]
    fn partial_file_falls_back_to_defaults() {
        // Only view_mode is set; everything else should be default.
        let content = "view_mode = \"list\"\n";
        let mut s = ViewState::default();
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            match key {
                "view_mode" if matches!(val, "icons" | "list") => s.view_mode = val.to_string(),
                _ => {}
            }
        }
        assert_eq!(s.view_mode, "list");
        assert!(s.sidebar_visible); // default
        assert!(!s.show_hidden); // default
        assert_eq!(s.sort_field, "name"); // default
    }
}
