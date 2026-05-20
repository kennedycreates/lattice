use crate::config::AppConfig;
use crate::converter::{OutputConflictPolicy, OutputLocationMode};
use std::path::PathBuf;

/// Lightweight per-session conversion preferences persisted to
/// `~/.config/lattice/convert_settings.toml`.
#[derive(Clone, Debug)]
pub struct ConvertSettings {
    /// Last-used preset id for image files.
    pub last_preset_image: String,
    /// Last-used preset id for audio files.
    pub last_preset_audio: String,
    /// Last-used preset id for video files.
    pub last_preset_video: String,
    /// Output location mode: "next_to_source" | "converted_subfolder" | "chosen_folder"
    pub output_mode: String,
    /// Conflict policy: "auto_rename" | "skip" | "overwrite"
    pub conflict_policy: String,
}

impl Default for ConvertSettings {
    fn default() -> Self {
        Self {
            last_preset_image: "to_jpeg".to_string(),
            last_preset_audio: "to_mp3".to_string(),
            last_preset_video: "mp4_compatible".to_string(),
            output_mode: "next_to_source".to_string(),
            conflict_policy: "auto_rename".to_string(),
        }
    }
}

impl ConvertSettings {
    fn settings_path() -> PathBuf {
        AppConfig::config_dir().join("convert_settings.toml")
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
                "last_preset_image" if !val.is_empty() => s.last_preset_image = val.to_string(),
                "last_preset_audio" if !val.is_empty() => s.last_preset_audio = val.to_string(),
                "last_preset_video" if !val.is_empty() => s.last_preset_video = val.to_string(),
                "output_mode" if !val.is_empty() => s.output_mode = val.to_string(),
                "conflict_policy" if !val.is_empty() => s.conflict_policy = val.to_string(),
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
            "last_preset_image = \"{}\"\nlast_preset_audio = \"{}\"\nlast_preset_video = \"{}\"\noutput_mode = \"{}\"\nconflict_policy = \"{}\"\n",
            self.last_preset_image,
            self.last_preset_audio,
            self.last_preset_video,
            self.output_mode,
            self.conflict_policy,
        );
        let _ = std::fs::write(&path, content);
    }

    pub fn output_location_mode(&self) -> OutputLocationMode {
        match self.output_mode.as_str() {
            "converted_subfolder" => OutputLocationMode::Subfolder("Converted".to_string()),
            "chosen_folder" => OutputLocationMode::NextToSource,
            _ => OutputLocationMode::NextToSource,
        }
    }

    pub fn conflict_policy_enum(&self) -> OutputConflictPolicy {
        match self.conflict_policy.as_str() {
            "skip" => OutputConflictPolicy::Skip,
            "overwrite" => OutputConflictPolicy::Overwrite,
            _ => OutputConflictPolicy::AutoRename,
        }
    }
}
