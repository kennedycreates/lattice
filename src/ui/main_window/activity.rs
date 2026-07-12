//! Activity-log entry parsing helpers, split out of main_window.

use crate::metadata::ActivityLogEntry;
use std::path::{Path, PathBuf};

pub(super) fn activity_sources(entry: &ActivityLogEntry) -> Vec<PathBuf> {
    entry
        .items
        .iter()
        .map(|item| PathBuf::from(&item.source_path))
        .collect()
}

pub(super) fn activity_destinations(entry: &ActivityLogEntry) -> Vec<PathBuf> {
    entry
        .items
        .iter()
        .filter_map(|item| item.destination_path.as_ref().map(PathBuf::from))
        .collect()
}

pub(super) fn activity_renames(entry: &ActivityLogEntry) -> Vec<(PathBuf, String)> {
    entry
        .items
        .iter()
        .filter_map(|item| {
            let destination = item.destination_path.as_ref().map(PathBuf::from)?;
            let name = destination.file_name()?.to_str()?.to_string();
            Some((PathBuf::from(&item.source_path), name))
        })
        .collect()
}

pub(super) fn activity_created_parent_and_name(entry: &ActivityLogEntry) -> Option<(PathBuf, String)> {
    let path = activity_destinations(entry).into_iter().next()?;
    let parent = path.parent()?.to_path_buf();
    let name = path.file_name()?.to_str()?.to_string();
    Some((parent, name))
}

pub(super) fn activity_relevant_path(entry: &ActivityLogEntry) -> Option<PathBuf> {
    entry
        .items
        .first()
        .map(|item| {
            item.destination_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&item.source_path))
        })
        .or_else(|| entry.destination_path.as_ref().map(PathBuf::from))
        .or_else(|| (!entry.source_path.is_empty()).then(|| PathBuf::from(&entry.source_path)))
}

pub(super) fn common_activity_destination_parent(entry: &ActivityLogEntry) -> Option<PathBuf> {
    let mut parents = entry
        .items
        .iter()
        .filter_map(|item| item.destination_path.as_ref())
        .filter_map(|path| Path::new(path).parent().map(Path::to_path_buf));
    let first = parents.next()?;
    parents.all(|path| path == first).then_some(first)
}

