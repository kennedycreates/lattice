use std::path::PathBuf;

/// Parsed command-line launch configuration.
#[derive(Debug, Default, Clone)]
pub struct LaunchConfig {
    /// Open a specific folder on startup.
    pub path: Option<PathBuf>,
    /// Open the Downloads Triage view on startup.
    pub downloads: bool,
    /// Open a pinned project by name on startup.
    pub project: Option<String>,
    /// Open in split view with explicit left/right paths.
    pub split: Option<(PathBuf, PathBuf)>,
}

impl LaunchConfig {
    pub fn from_env() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::from_args(&args)
    }

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
                        eprintln!("lattice: --path requires a folder argument");
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
                        eprintln!("lattice: --project requires a project name");
                        i += 1;
                    }
                }
                "--split" => {
                    if i + 2 < args.len() {
                        config.split =
                            Some((PathBuf::from(&args[i + 1]), PathBuf::from(&args[i + 2])));
                        i += 3;
                    } else {
                        eprintln!("lattice: --split requires two folder arguments");
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    // Bare positional arg: treat as an open-path shorthand.
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

#[cfg(test)]
mod tests {
    use super::LaunchConfig;
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
            Some((PathBuf::from("/left"), PathBuf::from("/right")))
        );
    }

    #[test]
    fn positional_path_is_shorthand_for_path_flag() {
        let config = LaunchConfig::from_args(&args(&["/tmp/downloads"]));
        assert_eq!(config.path, Some(PathBuf::from("/tmp/downloads")));
    }
}
