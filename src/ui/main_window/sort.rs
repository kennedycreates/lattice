//! File-list sorting, split out of main_window.

use super::{SortDirection, SortField};
use crate::ui::file_grid::FileItem;

pub(super) fn sort_items_with(items: &mut [FileItem], field: SortField, direction: SortDirection) {
    items.sort_by(|a, b| {
        let dir_ord = b.is_dir.cmp(&a.is_dir);
        if dir_ord != std::cmp::Ordering::Equal {
            return dir_ord;
        }
        let base = match field {
            SortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortField::Modified => a
                .modified_unix
                .unwrap_or(0)
                .cmp(&b.modified_unix.unwrap_or(0)),
            SortField::Size => a.size_bytes.unwrap_or(0).cmp(&b.size_bytes.unwrap_or(0)),
            SortField::Kind => a
                .kind
                .sort_key()
                .cmp(&b.kind.sort_key())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        };
        match direction {
            SortDirection::Ascending => base,
            SortDirection::Descending => base.reverse(),
        }
    });
}

pub(super) fn sort_items(items: &mut [FileItem]) {
    sort_items_with(items, SortField::Name, SortDirection::Ascending);
}
