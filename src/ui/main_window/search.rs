//! Blocking recursive filesystem search, split out of main_window.

use crate::metadata::Shape;
use crate::ui::file_grid::{FileItem, FileKind};
use crate::ui::search_panel::{SearchAgeFilter, SearchKindFilter, SearchQuery, SearchSizeFilter};
use gio::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_SEARCH_DEPTH: u32 = 8;
pub(super) const MAX_SEARCH_RESULTS: usize = 2_000;

struct SearchEntry {
    path: PathBuf,
    fname: String,
    is_dir: bool,
    size: u64,
    modified_secs: i64,
}

pub(super) fn search_directory_blocking(
    dir: &Path,
    query: &SearchQuery,
    query_name_lower: &str,
    show_hidden: bool,
    depth: u32,
    results: &mut Vec<FileItem>,
    cancelled: &AtomicBool,
) {
    if cancelled.load(Ordering::Relaxed)
        || depth > MAX_SEARCH_DEPTH
        || results.len() >= MAX_SEARCH_RESULTS
    {
        return;
    }
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Collect all entries with metadata up-front so we can do two passes:
    // files first, then subdirectories. Without this, a large subdirectory that
    // appears early in inode order (e.g. target/, node_modules/) can exhaust
    // MAX_SEARCH_RESULTS before files in the same parent directory are reached.
    let mut files: Vec<SearchEntry> = Vec::new();
    let mut subdirs: Vec<SearchEntry> = Vec::new();

    for entry in read_dir.flatten() {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !show_hidden && fname.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let modified_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let e = SearchEntry {
            path,
            fname,
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified_secs,
        };
        if e.is_dir {
            subdirs.push(e);
        } else {
            files.push(e);
        }
    }

    // Pass 1: files in this directory
    for e in &files {
        if cancelled.load(Ordering::Relaxed) || results.len() >= MAX_SEARCH_RESULTS {
            return;
        }
        if let Some(item) = match_entry(e, query, query_name_lower, now_secs) {
            results.push(item);
        }
    }

    // Pass 2: subdirectories — add matching ones then recurse
    for e in &subdirs {
        if cancelled.load(Ordering::Relaxed) || results.len() >= MAX_SEARCH_RESULTS {
            return;
        }
        if let Some(item) = match_entry(e, query, query_name_lower, now_secs) {
            results.push(item);
        }
        if query.recursive {
            search_directory_blocking(
                &e.path,
                query,
                query_name_lower,
                show_hidden,
                depth + 1,
                results,
                cancelled,
            );
        }
    }
}

fn match_entry(
    e: &SearchEntry,
    query: &SearchQuery,
    query_name_lower: &str,
    now_secs: i64,
) -> Option<FileItem> {
    // Name filter
    if !query_name_lower.is_empty() && !e.fname.to_lowercase().contains(query_name_lower) {
        return None;
    }

    // Kind filter
    let gio_type = if e.is_dir {
        gio::FileType::Directory
    } else {
        gio::FileType::Regular
    };
    let kind = search_entry_kind(e, gio_type);

    let kind_ok = match &query.kind {
        SearchKindFilter::All => true,
        SearchKindFilter::Folders => e.is_dir,
        SearchKindFilter::Images => matches!(kind, FileKind::Image),
        SearchKindFilter::Videos => matches!(kind, FileKind::Video),
        SearchKindFilter::Text => matches!(kind, FileKind::Text),
        SearchKindFilter::Archives => matches!(kind, FileKind::Archive),
        SearchKindFilter::Code => matches!(kind, FileKind::ConfigCode),
    };
    if !kind_ok {
        return None;
    }

    // Size filter (skip for directories)
    let size_ok = e.is_dir
        || match &query.size {
            SearchSizeFilter::Any => true,
            SearchSizeFilter::Small => e.size < 1_000_000,
            SearchSizeFilter::Medium => e.size >= 1_000_000 && e.size < 50_000_000,
            SearchSizeFilter::Large => e.size >= 50_000_000,
        };
    if !size_ok {
        return None;
    }

    // Age filter
    let age_ok = match &query.age {
        SearchAgeFilter::Any => true,
        SearchAgeFilter::Today => now_secs - e.modified_secs < 86_400,
        SearchAgeFilter::ThisWeek => now_secs - e.modified_secs < 7 * 86_400,
        SearchAgeFilter::ThisMonth => now_secs - e.modified_secs < 30 * 86_400,
        SearchAgeFilter::Older => now_secs - e.modified_secs >= 30 * 86_400,
    };
    if !age_ok {
        return None;
    }

    Some(FileItem {
        name: e.fname.clone(),
        path: e.path.clone(),
        is_dir: e.is_dir,
        is_openable: true,
        detail: None,
        kind,
        size_bytes: if e.is_dir { None } else { Some(e.size) },
        modified_unix: Some(e.modified_secs),
        tags: Vec::new(),
        mark_tint_id: 0,
        mark_tint_color: None,
        mark_shape: Shape::DEFAULT,
        original_path: None,
    })
}

fn search_entry_kind(e: &SearchEntry, gio_type: gio::FileType) -> FileKind {
    let guessed_content_type = gio::content_type_guess(Some(e.fname.as_str()), &[])
        .0
        .to_string();
    let mut kind = FileKind::from_path(&e.path, gio_type, Some(&guessed_content_type));

    if kind == FileKind::Unknown && gio_type != gio::FileType::Directory {
        let file = gio::File::for_path(&e.path);
        if let Ok(info) = file.query_info(
            "standard::content-type",
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        ) {
            kind = FileKind::from_path(&e.path, gio_type, info.content_type().as_deref());
        }
    }

    kind
}
