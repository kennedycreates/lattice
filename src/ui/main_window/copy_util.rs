//! Filesystem copy/move path helpers (unique names, recursive copy) plus the
//! bulk-naming directory collectors, split out of main_window.

use super::DIRECTORY_ATTRIBUTES;
use crate::ui::file_grid::FileItem;
use gio::prelude::*;
use std::path::{Path, PathBuf};

pub(super) fn common_parent(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?.parent()?.to_path_buf();
    paths
        .iter()
        .all(|path| path.parent() == Some(first.as_path()))
        .then_some(first)
}

pub(super) fn next_available_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Archive");
    let extension = path.extension().and_then(|value| value.to_str());

    for attempt in 2.. {
        let name = match extension {
            Some(ext) if !ext.is_empty() => format!("{stem} ({attempt}).{ext}"),
            _ => format!("{stem} ({attempt})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

pub(super) fn next_copy_path(destination: &Path) -> PathBuf {
    let stem = destination
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Copy");
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let parent = destination.parent().unwrap_or_else(|| Path::new("/"));

    let mut attempt = 1;
    loop {
        let suffix = if attempt == 1 {
            " copy".to_string()
        } else {
            format!(" copy {attempt}")
        };
        let candidate_name = if extension.is_empty() {
            format!("{stem}{suffix}")
        } else {
            format!("{stem}{suffix}.{extension}")
        };
        let candidate = parent.join(candidate_name);
        if !gio::File::for_path(&candidate).query_exists(None::<&gio::Cancellable>) {
            return candidate;
        }
        attempt += 1;
    }
}

pub(super) fn copy_path_recursively(
    source: &gio::File,
    destination: &gio::File,
    cancellable: &gio::Cancellable,
    overwrite: bool,
) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(glib::Error::new(gio::IOErrorEnum::Cancelled, "Cancelled."));
    }

    let source_type = source.query_file_type(gio::FileQueryInfoFlags::NONE, Some(cancellable));
    if source_type != gio::FileType::Directory {
        return source.copy(
            destination,
            if overwrite {
                gio::FileCopyFlags::OVERWRITE
            } else {
                gio::FileCopyFlags::NONE
            },
            Some(cancellable),
            None::<&mut dyn FnMut(i64, i64)>,
        );
    }

    if destination.query_exists(Some(cancellable)) {
        let destination_type =
            destination.query_file_type(gio::FileQueryInfoFlags::NONE, Some(cancellable));
        if destination_type != gio::FileType::Directory {
            if overwrite {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "Cannot safely replace a file with a folder.",
                ));
            } else {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "Destination already exists.",
                ));
            }
        }
    } else {
        destination.make_directory_with_parents(Some(cancellable))?;
    }

    let enumerator = source.enumerate_children(
        DIRECTORY_ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        Some(cancellable),
    )?;

    while let Some(info) = enumerator.next_file(Some(cancellable))? {
        if cancellable.is_cancelled() {
            return Err(glib::Error::new(gio::IOErrorEnum::Cancelled, "Cancelled."));
        }
        let child_source = source.child(info.name());
        let child_destination = destination.child(info.name());
        copy_path_recursively(&child_source, &child_destination, cancellable, overwrite)?;
    }

    Ok(())
}

pub(super) fn collect_bulk_naming_items_blocking(
    root: &Path,
    recursive: bool,
    show_hidden: bool,
) -> Vec<FileItem> {
    let mut items = Vec::new();
    collect_bulk_naming_items_from_dir(root, recursive, show_hidden, &mut items, 0);
    items
}

fn collect_bulk_naming_items_from_dir(
    root: &Path,
    recursive: bool,
    show_hidden: bool,
    items: &mut Vec<FileItem>,
    depth: usize,
) {
    const MAX_BULK_NAMING_RECURSION_DEPTH: usize = 32;
    if depth > MAX_BULK_NAMING_RECURSION_DEPTH {
        return;
    }
    let directory = gio::File::for_path(root);
    let Ok(enumerator) = directory.enumerate_children(
        DIRECTORY_ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    ) else {
        return;
    };

    loop {
        let info = match enumerator.next_file(None::<&gio::Cancellable>) {
            Ok(Some(info)) => info,
            Ok(None) | Err(_) => break,
        };
        let Some(item) = FileItem::from_info(&directory, &info, show_hidden) else {
            continue;
        };
        let should_recurse = recursive && item.is_dir;
        let child_path = item.path.clone();
        items.push(item);
        if should_recurse {
            collect_bulk_naming_items_from_dir(
                &child_path,
                recursive,
                show_hidden,
                items,
                depth + 1,
            );
        }
    }
}

pub(super) fn common_parent_for_items(items: &[FileItem]) -> Option<PathBuf> {
    let mut parent = items.first()?.path.parent()?.to_path_buf();
    for item in items.iter().skip(1) {
        let item_parent = item.path.parent()?;
        while !item_parent.starts_with(&parent) {
            if !parent.pop() {
                return None;
            }
        }
    }
    Some(parent)
}
