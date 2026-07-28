//! Triage-filter classification, split out of main_window.

use super::{TriageFilter, TRIAGE_LARGE_FILE_BYTES};
use crate::ui::file_grid::FileItem;
use std::path::{Path, PathBuf};

pub(super) fn filter_triage_items(
    items: Vec<FileItem>,
    filter: TriageFilter,
    duplicate_set: Option<&std::collections::HashSet<PathBuf>>,
) -> Vec<FileItem> {
    items
        .into_iter()
        .filter(|item| matches_triage_filter(item, filter, duplicate_set))
        .collect()
}

fn matches_triage_filter(
    item: &FileItem,
    filter: TriageFilter,
    duplicate_set: Option<&std::collections::HashSet<PathBuf>>,
) -> bool {
    match filter {
        TriageFilter::All => true,
        TriageFilter::Today => item
            .modified_unix
            .and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok())
            .and_then(|value| value.format("%Y-%m-%d").ok())
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| value.format("%Y-%m-%d").ok()),
            )
            .map(|(left, right)| left.as_str() == right.as_str())
            .unwrap_or(false),
        TriageFilter::ThisWeek => item
            .modified_unix
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .map(|value| value.to_unix()),
            )
            .map(|(modified, now)| now.saturating_sub(modified) <= 7 * 24 * 60 * 60)
            .unwrap_or(false),
        TriageFilter::ThisMonth => item
            .modified_unix
            .and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok())
            .and_then(|value| value.format("%Y-%m").ok())
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| value.format("%Y-%m").ok()),
            )
            .map(|(left, right)| left.as_str() == right.as_str())
            .unwrap_or(false),
        TriageFilter::OlderThanOneMonth => item
            .modified_unix
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .map(|value| value.to_unix()),
            )
            .map(|(modified, now)| now.saturating_sub(modified) > 30 * 24 * 60 * 60)
            .unwrap_or(false),
        TriageFilter::Images => item.kind == crate::ui::file_grid::FileKind::Image,
        TriageFilter::Videos => item.kind == crate::ui::file_grid::FileKind::Video,
        TriageFilter::Archives => item.kind == crate::ui::file_grid::FileKind::Archive,
        TriageFilter::Documents => {
            matches!(
                item.kind,
                crate::ui::file_grid::FileKind::Document
                    | crate::ui::file_grid::FileKind::Text
                    | crate::ui::file_grid::FileKind::ConfigCode
            )
        }
        TriageFilter::LargeFiles => item.size_bytes.unwrap_or(0) >= TRIAGE_LARGE_FILE_BYTES,
        TriageFilter::Audio => item.kind == crate::ui::file_grid::FileKind::Audio,
        TriageFilter::Executables => !item.is_dir && is_executable(&item.path),
        TriageFilter::Empty => !item.is_dir && item.size_bytes == Some(0),
        TriageFilter::Duplicates => duplicate_set
            .map(|set| set.contains(&item.path))
            .unwrap_or(false),
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}
