use std::path::PathBuf;

/// Parsed command-line launch configuration.
#[derive(Debug, Default, Clone)]
pub struct LaunchConfig {
    /// Open a specific folder on startup.
    pub path: Option<PathBuf>,
    /// Open the Downloads Triage view on startup.
    pub downloads: bool,
    /// Legacy --project compatibility: open a Palette by name on startup.
    pub project: Option<String>,
    /// Open in split view with explicit pane paths.
    pub split: Option<Vec<PathBuf>>,
}

impl LaunchConfig {
    fn from_args(args: &[String]) -> Self {
        let mut config = LaunchConfig::default();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--path" => {
                    if i + 1 < args.len() {
                        config.path = Some(PathBuf::from(&args[i + 1]));
                        i += 2;
                    } else {
                        log_err!("--path requires a folder argument");
                        i += 1;
                    }
                }
                "--downloads" => {
                    config.downloads = true;
                    i += 1;
                }
                "--project" => {
                    if i + 1 < args.len() {
                        config.project = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        log_err!("--project requires a palette name");
                        i += 1;
                    }
                }
                "--split" => {
                    if i + 2 < args.len() {
                        let mut paths =
                            vec![PathBuf::from(&args[i + 1]), PathBuf::from(&args[i + 2])];
                        if i + 3 < args.len() && !args[i + 3].starts_with('-') {
                            paths.push(PathBuf::from(&args[i + 3]));
                            i += 4;
                        } else {
                            i += 3;
                        }
                        config.split = Some(paths);
                    } else {
                        log_err!("--split requires two or three folder arguments");
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    config.path = Some(PathBuf::from(arg));
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
        config
    }
}

// ── Picker CLI ────────────────────────────────────────────────────────────────

/// Picker subcommand: what kind of selection the picker performs.
#[derive(Debug, Clone)]
pub enum PickerSubcommand {
    /// Open one existing file.
    OpenFile,
    /// Open one or more existing files.
    OpenFiles,
    /// Select an existing folder.
    OpenFolder,
    /// Choose a save path (directory + filename).
    SaveFile,
}

/// Parsed `lattice --picker ...` configuration.
#[derive(Debug, Clone)]
pub struct PickerLaunchConfig {
    pub subcommand: PickerSubcommand,
    /// Starting directory for the picker.
    pub initial_path: Option<PathBuf>,
    /// Suggested filename for SaveFile mode.
    pub suggested_name: Option<String>,
}

/// Top-level launch decision — either a browser window or a picker window.
#[derive(Debug)]
pub enum LaunchMode {
    Browser(LaunchConfig),
    Picker(PickerLaunchConfig),
}

impl LaunchMode {
    /// Parse from process arguments.
    ///
    /// If `--picker` is the first argument, parse as picker mode.
    /// Otherwise parse as the normal browser launch config.
    pub fn from_env() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(|s| s.as_str()) == Some("--picker") {
            Self::Picker(PickerLaunchConfig::from_args(&args[1..]))
        } else {
            Self::Browser(LaunchConfig::from_args(&args))
        }
    }
}

impl PickerLaunchConfig {
    /// Parse picker arguments after `--picker`.
    ///
    /// Syntax:
    ///   lattice --picker open          [--path /start/dir]
    ///   lattice --picker open-files    [--path /start/dir]
    ///   lattice --picker folder        [--path /start/dir]
    ///   lattice --picker save          [--path /start/dir] [--name suggested.txt]
    fn from_args(args: &[String]) -> Self {
        let subcommand = match args.first().map(|s| s.as_str()) {
            Some("open") => PickerSubcommand::OpenFile,
            Some("open-files") => PickerSubcommand::OpenFiles,
            Some("folder") => PickerSubcommand::OpenFolder,
            Some("save") => PickerSubcommand::SaveFile,
            other => {
                if let Some(s) = other {
                    log_err!("--picker: unknown subcommand '{s}'; expected open, open-files, folder, or save");
                } else {
                    log_err!(
                        "--picker: missing subcommand; expected open, open-files, folder, or save"
                    );
                }
                PickerSubcommand::OpenFile
            }
        };

        let rest = if args.is_empty() { args } else { &args[1..] };
        let mut initial_path: Option<PathBuf> = None;
        let mut suggested_name: Option<String> = None;
        let mut i = 0;
        while i < rest.len() {
            match rest[i].as_str() {
                "--path" => {
                    if i + 1 < rest.len() {
                        initial_path = Some(PathBuf::from(&rest[i + 1]));
                        i += 2;
                    } else {
                        log_err!("--picker: --path requires a directory argument");
                        i += 1;
                    }
                }
                "--name" => {
                    if i + 1 < rest.len() {
                        suggested_name = Some(rest[i + 1].clone());
                        i += 2;
                    } else {
                        log_err!("--picker: --name requires a filename argument");
                        i += 1;
                    }
                }
                _ => {
                    i += 1;
                }
            }
        }

        PickerLaunchConfig {
            subcommand,
            initial_path,
            suggested_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LaunchConfig, PickerSubcommand};
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_path_flag() {
        let config = LaunchConfig::from_args(&args(&["--path", "/tmp/demo"]));
        assert_eq!(config.path, Some(PathBuf::from("/tmp/demo")));
        assert!(!config.downloads);
        assert!(config.project.is_none());
        assert!(config.split.is_none());
    }

    #[test]
    fn parses_downloads_flag() {
        let config = LaunchConfig::from_args(&args(&["--downloads"]));
        assert!(config.downloads);
    }

    #[test]
    fn parses_project_flag() {
        let config = LaunchConfig::from_args(&args(&["--project", "Alpha"]));
        assert_eq!(config.project.as_deref(), Some("Alpha"));
    }

    #[test]
    fn parses_split_flag() {
        let config = LaunchConfig::from_args(&args(&["--split", "/left", "/right"]));
        assert_eq!(
            config.split,
            Some(vec![PathBuf::from("/left"), PathBuf::from("/right")])
        );
    }

    #[test]
    fn parses_three_pane_split_flag() {
        let config = LaunchConfig::from_args(&args(&["--split", "/left", "/middle", "/right"]));
        assert_eq!(
            config.split,
            Some(vec![
                PathBuf::from("/left"),
                PathBuf::from("/middle"),
                PathBuf::from("/right")
            ])
        );
    }

    #[test]
    fn positional_path_is_shorthand_for_path_flag() {
        let config = LaunchConfig::from_args(&args(&["/tmp/downloads"]));
        assert_eq!(config.path, Some(PathBuf::from("/tmp/downloads")));
    }

    #[test]
    fn picker_mode_detected() {
        let args_raw: Vec<String> = ["--picker", "folder", "--path", "/tmp"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let all: Vec<String> = std::iter::once("lattice".to_string())
            .chain(args_raw)
            .collect();
        // Simulate from_env parsing logic
        let rest = &all[1..];
        if rest.first().map(|s| s.as_str()) == Some("--picker") {
            let picker = super::PickerLaunchConfig::from_args(&rest[1..]);
            assert!(matches!(picker.subcommand, PickerSubcommand::OpenFolder));
            assert_eq!(picker.initial_path, Some(PathBuf::from("/tmp")));
        } else {
            panic!("expected picker mode");
        }
    }
}
