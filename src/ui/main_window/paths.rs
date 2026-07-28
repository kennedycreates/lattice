//! Path/URI classification, new-item naming, launch resolution and terminal
//! detection, split out of main_window.

use super::format::command_exists;
use super::{LaunchResolution, PaneLayout, PaneView, Places, TriageFilter, TERMINAL_ENV_VAR};
use gio::prelude::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(super) fn looks_like_explicit_path(input: &str) -> bool {
    input.starts_with('/') || input.starts_with('~') || input.contains('/') || is_gio_uri(input)
}

/// Returns true for GIO/GVfs remote URI schemes (sftp, ftp, smb, dav, davs, nfs, ssh, afp).
/// Does NOT match file://, trash://, or other virtual GIO backends.
pub(super) fn is_gio_uri(s: &str) -> bool {
    let scheme = s.split_once("://").map(|(s, _)| s).unwrap_or("");
    matches!(
        scheme,
        "sftp" | "ftp" | "smb" | "dav" | "davs" | "nfs" | "ssh" | "afp"
    )
}

pub(super) fn format_path(path: &Path, home: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(home) {
        if relative.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", relative.display())
        }
    } else {
        path.display().to_string()
    }
}

pub(super) fn next_new_folder_path(current_dir: &Path) -> PathBuf {
    let mut attempt = 1;
    loop {
        let name = if attempt == 1 {
            "New Folder".to_string()
        } else {
            format!("New Folder {attempt}")
        };
        let candidate = current_dir.join(name);
        if !gio::File::for_path(&candidate).query_exists(None::<&gio::Cancellable>) {
            return candidate;
        }
        attempt += 1;
    }
}

pub(super) fn next_new_text_document_path(current_dir: &Path) -> PathBuf {
    let mut attempt = 1;
    loop {
        let name = if attempt == 1 {
            "Untitled".to_string()
        } else {
            format!("Untitled {attempt}")
        };
        let candidate = current_dir.join(name);
        if !gio::File::for_path(&candidate).query_exists(None::<&gio::Cancellable>) {
            return candidate;
        }
        attempt += 1;
    }
}

pub(super) fn suggested_project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format_path(path, &glib::home_dir()))
}

pub(super) fn resolve_launch(
    launch: &crate::launch::LaunchConfig,
    places: &Places,
    metadata: &crate::metadata::MetadataStore,
) -> LaunchResolution {
    if let Some(split_paths) = &launch.split {
        let left = &split_paths[0];
        let right = &split_paths[1];
        let third = split_paths.get(2);
        let (left_path, left_notice) = validate_launch_directory(left, places, "Left split path");
        let (right_path, right_notice) =
            validate_launch_directory(right, places, "Middle split path");
        let (third_path, third_notice) = third
            .map(|path| validate_launch_directory(path, places, "Right split path"))
            .unwrap_or_else(|| (places.home.clone(), None));
        return LaunchResolution {
            primary_dir: left_path.clone(),
            primary_view: PaneView::Directory(left_path),
            secondary_dir: right_path,
            tertiary_dir: third_path,
            pane_layout: if third.is_some() {
                PaneLayout::Three
            } else {
                PaneLayout::Two
            },
            notice: combine_launch_notices([left_notice, right_notice, third_notice]),
        };
    }

    if let Some(path) = &launch.path {
        let (resolved_path, notice) = validate_launch_directory(path, places, "Launch path");
        return LaunchResolution {
            primary_dir: resolved_path.clone(),
            primary_view: PaneView::Directory(resolved_path),
            secondary_dir: places.home.clone(),
            tertiary_dir: places.home.clone(),
            pane_layout: PaneLayout::Single,
            notice,
        };
    }

    if launch.downloads {
        let downloads_path = &places.downloads;
        let notice = if is_launchable_directory(downloads_path) {
            None
        } else {
            Some("Downloads folder is unavailable. Opened Home instead.".to_string())
        };
        let primary_dir = if notice.is_some() {
            places.home.clone()
        } else {
            downloads_path.clone()
        };
        let primary_view = if notice.is_some() {
            PaneView::Directory(primary_dir.clone())
        } else {
            PaneView::Triage {
                root: primary_dir.clone(),
                filter: TriageFilter::All,
            }
        };
        return LaunchResolution {
            primary_dir,
            primary_view,
            secondary_dir: places.home.clone(),
            tertiary_dir: places.home.clone(),
            pane_layout: PaneLayout::Single,
            notice,
        };
    }

    if let Some(project_name) = &launch.project {
        let project = metadata.list_projects().ok().and_then(|projects| {
            projects
                .into_iter()
                .find(|project| project.name.eq_ignore_ascii_case(project_name))
        });
        return match project {
            Some(project) => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::ProjectLanding(project.id),
                secondary_dir: places.home.clone(),
                tertiary_dir: places.home.clone(),
                pane_layout: PaneLayout::Single,
                notice: None,
            },
            None => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::Directory(places.home.clone()),
                secondary_dir: places.home.clone(),
                tertiary_dir: places.home.clone(),
                pane_layout: PaneLayout::Single,
                notice: Some(format!(
                    "Palette '{}' was not found. Opened Home instead.",
                    project_name
                )),
            },
        };
    }

    LaunchResolution {
        primary_dir: places.home.clone(),
        primary_view: PaneView::Directory(places.home.clone()),
        secondary_dir: places.home.clone(),
        tertiary_dir: places.home.clone(),
        pane_layout: PaneLayout::Single,
        notice: None,
    }
}

fn validate_launch_directory(
    candidate: &Path,
    places: &Places,
    label: &str,
) -> (PathBuf, Option<String>) {
    if is_launchable_directory(candidate) {
        (candidate.to_path_buf(), None)
    } else {
        (
            places.home.clone(),
            Some(format!(
                "{label} '{}' is not a readable folder. Opened Home instead.",
                candidate.display()
            )),
        )
    }
}

fn combine_launch_notices(notices: [Option<String>; 3]) -> Option<String> {
    let notices = notices.into_iter().flatten().collect::<Vec<_>>();
    (!notices.is_empty()).then(|| notices.join(" "))
}

fn is_launchable_directory(path: &Path) -> bool {
    gio::File::for_path(path)
        .query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>)
        == gio::FileType::Directory
}

pub(super) fn pane_view_scope_dir(view: &PaneView) -> Option<PathBuf> {
    match view {
        PaneView::Directory(path) => Some(path.clone()),
        PaneView::Triage { root, .. } => Some(root.clone()),
        PaneView::Search(query) => Some(query.scope_dir.clone()),
        PaneView::BulkNaming { root } => Some(root.clone()),
        PaneView::SpaceViewer { root } => Some(root.clone()),
        PaneView::MediaConvert { from_dir } => Some(from_dir.clone()),
        PaneView::Tag(_)
        | PaneView::SystemDrives
        | PaneView::Recent
        | PaneView::Trash
        | PaneView::ActivityLog
        | PaneView::ProjectLanding(_)
        | PaneView::CloudLanding(_)
        | PaneView::ProjectManager
        | PaneView::TagManager
        | PaneView::Watercolor(_) => None,
    }
}

pub(super) fn resolve_tool_scope_dir(view: &PaneView, current_dir: &Path, home: &Path) -> PathBuf {
    pane_view_scope_dir(view)
        .or_else(|| is_launchable_directory(current_dir).then(|| current_dir.to_path_buf()))
        .unwrap_or_else(|| home.to_path_buf())
}

pub(super) fn detect_terminal_command() -> Option<Vec<OsString>> {
    if let Ok(value) = std::env::var(TERMINAL_ENV_VAR) {
        let parts = value
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .map(OsString::from)
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Some(parts);
        }
    }

    // Candidates cover Ubuntu/Arch defaults plus Fedora GNOME. Fedora Workstation
    // 43/44 ships Ptyxis as the default GNOME terminal; older/alt GNOME uses GNOME
    // Console (`kgx`). All of these honour the launcher's cwd when spawned with no
    // args (see open_terminal_for_path), so no per-terminal argv is needed.
    for candidate in [
        "kitty",
        "x-terminal-emulator", // Debian/Ubuntu alternatives symlink
        "ptyxis",              // Fedora Workstation 43/44 default GNOME terminal
        "kgx",                 // GNOME Console
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "foot", // common Wayland terminal
        "alacritty",
        "wezterm",
        "xterm", // last-resort X11 fallback (common on minimal Void setups)
    ] {
        if command_exists(candidate) {
            return Some(vec![OsString::from(candidate)]);
        }
    }

    None
}
