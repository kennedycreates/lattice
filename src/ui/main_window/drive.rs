//! Removable-drive / mounted-volume / recent-folder discovery, split out of main_window.

use super::sort::sort_items;
use super::view_label::tab_title_for_path;
use super::DriveState;
use crate::metadata::{MetadataStore, Shape};
use crate::ui::file_grid::{FileItem, FileKind};
use crate::ui::sidebar::DriveEntry;
use gio::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;

pub(super) struct MountedVolumeListing {
    pub(super) items: Vec<FileItem>,
    pub(super) gio_mounts: usize,
    pub(super) unmounted_volumes: usize,
    pub(super) detected_drives: usize,
    pub(super) fallback_mounts: usize,
    pub(super) skipped_inaccessible: usize,
    pub(super) skipped_non_local: usize,
}

pub(super) struct RecentFolderListing {
    pub(super) items: Vec<FileItem>,
    pub(super) skipped_missing: usize,
}

pub(super) fn collect_removable_drives() -> Vec<DriveState> {
    let monitor = gio::VolumeMonitor::get();
    let mut states: Vec<DriveState> = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // Pass 1: mounted removable mounts
    for mount in monitor.mounts() {
        let root = mount.root();
        let Some(path) = root.path() else { continue };
        let volume = mount.volume();
        let is_removable = volume
            .as_ref()
            .and_then(|v| v.drive())
            .is_some_and(|d| d.is_removable());
        if !is_removable {
            continue;
        }
        let name = mount.name().to_string();
        seen_names.insert(name.clone());
        states.push(DriveState {
            entry: DriveEntry {
                name,
                path: Some(path),
                is_removable: true,
                is_mounted: true,
            },
            mount: Some(mount),
            volume,
        });
    }

    // Pass 2: unmounted removable volumes (plugged in but not yet mounted)
    for volume in monitor.volumes() {
        if volume.get_mount().is_some() {
            continue; // already handled in pass 1
        }
        let is_removable = volume.drive().is_some_and(|d| d.is_removable());
        if !is_removable {
            continue;
        }
        let name = volume.name().to_string();
        if seen_names.contains(&name) {
            continue;
        }
        seen_names.insert(name.clone());
        states.push(DriveState {
            entry: DriveEntry {
                name,
                path: None,
                is_removable: true,
                is_mounted: false,
            },
            mount: None,
            volume: Some(volume),
        });
    }

    states
}

pub(super) fn collect_mounted_volume_items() -> MountedVolumeListing {
    let monitor = gio::VolumeMonitor::get();
    let mut items = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut seen_volume_names = HashSet::new();
    let mut gio_mounts = 0usize;
    let mut unmounted_volumes = 0usize;
    let mut detected_drives = 0usize;
    let mut skipped_non_local = 0usize;

    for mount in monitor.mounts() {
        let root = mount.root();
        let Some(path) = root.path() else {
            skipped_non_local += 1;
            continue;
        };

        if !seen_paths.insert(path.clone()) {
            continue;
        }

        gio_mounts += 1;
        seen_volume_names.insert(mount.name().to_string());
        let detail = format!("Mounted: {}", path.display());
        items.push(FileItem {
            name: mount.name().to_string(),
            path,
            kind: FileKind::Folder,
            is_dir: true,
            is_openable: true,
            detail: Some(detail),
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
            original_path: None,
        });
    }

    for volume in monitor.volumes() {
        if let Some(mount) = volume.get_mount() {
            seen_volume_names.insert(mount.name().to_string());
            continue;
        }

        let name = volume.name().to_string();
        if !seen_volume_names.insert(name.clone()) {
            continue;
        }

        unmounted_volumes += 1;
        items.push(FileItem {
            name,
            path: PathBuf::new(),
            kind: FileKind::Folder,
            is_dir: false,
            is_openable: false,
            detail: Some("Unmounted volume (mounting not implemented)".to_string()),
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
            original_path: None,
        });
    }

    for drive in monitor.connected_drives() {
        let name = drive.name().to_string();
        if drive.volumes().is_empty() && seen_volume_names.insert(name.clone()) {
            detected_drives += 1;
            let state = if drive.has_media() {
                "Drive detected (no mounted volume)"
            } else {
                "Drive detected (no media mounted)"
            };
            items.push(FileItem {
                name,
                path: PathBuf::new(),
                kind: FileKind::Folder,
                is_dir: false,
                is_openable: false,
                detail: Some(state.to_string()),
                size_bytes: None,
                modified_unix: None,
                tags: Vec::new(),
                mark_tint_id: 0,
                mark_tint_color: None,
                mark_shape: Shape::DEFAULT,
                original_path: None,
            });
        }
    }

    let fallback = collect_fallback_mounted_locations(&mut seen_paths);
    let fallback_mounts = fallback.items.len();
    let skipped_inaccessible = fallback.skipped_inaccessible;
    items.extend(fallback.items);

    sort_items(&mut items);

    MountedVolumeListing {
        items,
        gio_mounts,
        unmounted_volumes,
        detected_drives,
        fallback_mounts,
        skipped_inaccessible,
        skipped_non_local,
    }
}

pub(super) struct FallbackMountListing {
    pub(super) items: Vec<FileItem>,
    pub(super) skipped_inaccessible: usize,
}

pub(super) fn collect_fallback_mounted_locations(seen_paths: &mut HashSet<PathBuf>) -> FallbackMountListing {
    let user_name = glib::user_name();
    let candidates = [
        PathBuf::from("/run/media").join(user_name),
        PathBuf::from("/media"),
        PathBuf::from("/mnt"),
    ];

    let mut items = Vec::new();
    let mut skipped_inaccessible = 0usize;

    for base in candidates {
        let base_file = gio::File::for_path(&base);
        if !base_file.query_exists(None::<&gio::Cancellable>) {
            continue;
        }

        let enumerator = match base_file.enumerate_children(
            "standard::name,standard::display-name,standard::type,standard::is-hidden",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            None::<&gio::Cancellable>,
        ) {
            Ok(enumerator) => enumerator,
            Err(_) => {
                skipped_inaccessible += 1;
                continue;
            }
        };

        loop {
            let next = match enumerator.next_file(None::<&gio::Cancellable>) {
                Ok(next) => next,
                Err(_) => {
                    skipped_inaccessible += 1;
                    break;
                }
            };

            let Some(info) = next else { break };
            if info.is_hidden() || info.file_type() != gio::FileType::Directory {
                continue;
            }

            let path = base.join(info.name());
            if !seen_paths.insert(path.clone()) {
                continue;
            }

            items.push(FileItem {
                name: info.display_name().to_string(),
                path: path.clone(),
                kind: FileKind::Folder,
                is_dir: true,
                is_openable: true,
                detail: Some(format!("Mounted Locations: {}", path.display())),
                size_bytes: None,
                modified_unix: None,
                tags: Vec::new(),
                mark_tint_id: 0,
                mark_tint_color: None,
                mark_shape: Shape::DEFAULT,
                original_path: None,
            });
        }
    }

    FallbackMountListing {
        items,
        skipped_inaccessible,
    }
}

pub(super) fn drive_listing_status_message(listing: &MountedVolumeListing) -> Option<String> {
    let mut parts = Vec::new();

    if listing.gio_mounts > 0 {
        parts.push(format!("{} GIO mount(s)", listing.gio_mounts));
    }
    if listing.unmounted_volumes > 0 {
        parts.push(format!(
            "{} unmounted volume(s); mounting is not implemented",
            listing.unmounted_volumes
        ));
    }
    if listing.fallback_mounts > 0 {
        parts.push(format!(
            "{} fallback mounted location(s)",
            listing.fallback_mounts
        ));
    }
    if listing.detected_drives > 0 {
        parts.push(format!(
            "{} drive(s) detected without mounted volumes",
            listing.detected_drives
        ));
    }
    if listing.skipped_non_local > 0 {
        parts.push(format!(
            "{} non-local mount(s) skipped",
            listing.skipped_non_local
        ));
    }
    if listing.skipped_inaccessible > 0 {
        parts.push(format!(
            "{} mounted-location folder(s) inaccessible",
            listing.skipped_inaccessible
        ));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

pub(super) fn collect_recent_folder_items(
    metadata: &mut MetadataStore,
) -> Result<RecentFolderListing, String> {
    let recent_paths = metadata.list_recent_locations(50)?;
    let mut items = Vec::with_capacity(recent_paths.len());
    let mut stale_paths = Vec::new();

    for path in recent_paths {
        if !path.is_dir() {
            stale_paths.push(path);
            continue;
        }

        items.push(FileItem {
            name: tab_title_for_path(&path),
            path,
            kind: FileKind::Folder,
            is_dir: true,
            is_openable: true,
            detail: None,
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
            original_path: None,
        });
    }

    let skipped_missing = stale_paths.len();
    if !stale_paths.is_empty() {
        metadata.remove_recent_locations(&stale_paths)?;
    }

    Ok(RecentFolderListing {
        items,
        skipped_missing,
    })
}

