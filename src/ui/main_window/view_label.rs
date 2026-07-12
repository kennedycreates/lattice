//! Tab-title and view-label formatting, split out of main_window.

use super::paths::format_path;
use super::{watercolor_display_label, watercolor_tab_title, PaneView};
use std::path::Path;

pub(super) fn tab_title_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if path == Path::new("/") {
                "/".to_string()
            } else {
                path.display().to_string()
            }
        })
}

pub(super) fn tab_title_for_view(view: &PaneView, path: &Path) -> String {
    match view {
        PaneView::Directory(_) => tab_title_for_path(path),
        PaneView::Tag(tag) => format!("#{}", tag.name),
        PaneView::Triage { root, filter } => {
            let folder = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            format!("Triage: {} / {}", folder, filter.label())
        }
        PaneView::SystemDrives => "Drives".to_string(),
        PaneView::Recent => "Recent".to_string(),
        PaneView::Trash => "Trash".to_string(),
        PaneView::Search(q) => {
            if q.name.is_empty() {
                "Search".to_string()
            } else {
                format!("Search \u{201c}{}\u{201d}", q.name)
            }
        }
        PaneView::BulkNaming { .. } => "Bulk Naming".to_string(),
        PaneView::ActivityLog => "Activity Log".to_string(),
        PaneView::ProjectLanding(_) => "Palette".to_string(),
        PaneView::CloudLanding(_) => "Cloud Drive".to_string(),
        PaneView::ProjectManager => "Palettes".to_string(),
        PaneView::TagManager => "Tints & Tags".to_string(),
        PaneView::SpaceViewer { root } => {
            let folder = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Space Viewer".to_string());
            format!("Space: {folder}")
        }
        PaneView::MediaConvert { .. } => "Convert".to_string(),
        PaneView::Watercolor(view) => watercolor_tab_title(view).to_string(),
    }
}

pub(super) fn view_display_label(view: &PaneView, home: &Path) -> String {
    match view {
        PaneView::Directory(path) => format_path(path, home),
        PaneView::Tag(tag) => format!("Tag / #{}", tag.name),
        PaneView::Triage { root, filter } => {
            let folder = format_path(root, home);
            format!("Triage: {} / {}", folder, filter.label())
        }
        PaneView::SystemDrives => "System Drives".to_string(),
        PaneView::Recent => "Recent".to_string(),
        PaneView::Trash => "Trash".to_string(),
        PaneView::Search(q) => {
            let scope = format_path(&q.scope_dir, home);
            if q.name.is_empty() {
                format!("Search in {scope}")
            } else {
                format!("Search \u{201c}{}\u{201d} in {scope}", q.name)
            }
        }
        PaneView::BulkNaming { root } => format!("Bulk Naming in {}", format_path(root, home)),
        PaneView::ActivityLog => "Activity Log".to_string(),
        PaneView::ProjectLanding(_) => "Palette".to_string(),
        PaneView::CloudLanding(_) => "Cloud Drive".to_string(),
        PaneView::ProjectManager => "Palettes".to_string(),
        PaneView::TagManager => "Tints & Tags".to_string(),
        PaneView::SpaceViewer { root } => format!("Space: {}", format_path(root, home)),
        PaneView::MediaConvert { from_dir } => {
            format!("Convert · {}", format_path(from_dir, home))
        }
        PaneView::Watercolor(view) => watercolor_display_label(view).to_string(),
    }
}

pub(super) fn pane_view_uses_file_grid_controls(view: &PaneView) -> bool {
    matches!(
        view,
        PaneView::Directory(_)
            | PaneView::Tag(_)
            | PaneView::Triage { .. }
            | PaneView::SystemDrives
            | PaneView::Recent
            | PaneView::Trash
            | PaneView::Search(_)
    )
}
