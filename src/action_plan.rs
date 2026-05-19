use crate::metadata::Shape;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum OpKind {
    Copy,
    Move,
    Trash,
    PermanentDelete,
    Rename,
    BulkRename,
    Duplicate,
    NewFolder,
    NewFile,
    SendToProject {
        is_copy: bool,
    },
    PaintMark {
        tint_id: i64,
        tint_name: String,
        shape: Shape,
        recursive: bool,
    },
    ResetMark {
        recursive: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum WarnLevel {
    None,
    Caution,
    Danger,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ActionPlan {
    pub kind: OpKind,
    pub sources: Vec<PathBuf>,
    pub destination: Option<PathBuf>,
    pub summary: String,
    pub file_list: Vec<String>,
    pub conflicts: Vec<String>,
    pub warn_level: WarnLevel,
}

impl ActionPlan {
    /// Build a plan for a paste/copy-move operation. Checks destination for
    /// conflicts synchronously via `PathBuf::exists()`.
    pub fn for_paste(sources: &[PathBuf], dest: &Path, is_copy: bool) -> Self {
        let verb = if is_copy { "Copy" } else { "Move" };
        let dest_name = dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("destination");
        let n = sources.len();
        let summary = if n == 1 {
            let name = sources[0]
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("item");
            format!("{verb} {name} → {dest_name}")
        } else {
            format!("{verb} {n} files → {dest_name}")
        };

        let file_list: Vec<String> = sources
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(|s| s.to_string()))
            .collect();

        let conflicts: Vec<String> = sources
            .iter()
            .filter_map(|src| {
                let name = src.file_name()?;
                let candidate = dest.join(name);
                if candidate.exists() {
                    name.to_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();

        let warn_level = if conflicts.is_empty() {
            WarnLevel::None
        } else {
            WarnLevel::Caution
        };

        Self {
            kind: if is_copy { OpKind::Copy } else { OpKind::Move },
            sources: sources.to_vec(),
            destination: Some(dest.to_path_buf()),
            summary,
            file_list,
            conflicts,
            warn_level,
        }
    }

    /// Build a plan for moving files to Trash.
    pub fn for_trash(paths: &[PathBuf]) -> Self {
        let n = paths.len();
        let summary = if n == 1 {
            let name = paths[0]
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("item");
            format!("Move \"{name}\" to Trash")
        } else {
            format!("Move {n} files to Trash")
        };

        let file_list: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(|s| s.to_string()))
            .collect();

        Self {
            kind: OpKind::Trash,
            sources: paths.to_vec(),
            destination: None,
            summary,
            file_list,
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
        }
    }

    /// Build a plan for renaming a single file. New name is stored in `file_list[0]`.
    pub fn for_rename(path: &Path, new_name: &str) -> Self {
        let old_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("item");
        Self {
            kind: OpKind::Rename,
            sources: vec![path.to_path_buf()],
            destination: None,
            summary: format!("Rename \"{old_name}\" → \"{new_name}\""),
            file_list: vec![new_name.to_string()],
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
        }
    }

    /// Build a plan for permanently deleting files.
    pub fn for_permanent_delete(paths: &[PathBuf]) -> Self {
        let n = paths.len();
        let summary = if n == 1 {
            let name = paths[0]
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("item");
            format!("Delete \"{name}\" permanently")
        } else {
            format!("Delete {n} items permanently")
        };
        let file_list: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        Self {
            kind: OpKind::PermanentDelete,
            sources: paths.to_vec(),
            destination: None,
            summary,
            file_list,
            conflicts: Vec::new(),
            warn_level: WarnLevel::Danger,
        }
    }

    /// Build a plan for bulk-renaming files. `renames` maps each old path to its new name.
    /// Sources and file_list are stored as parallel arrays.
    pub fn for_bulk_rename(renames: &[(PathBuf, String)]) -> Self {
        let n = renames.len();
        let summary = format!("Bulk rename {} file{}", n, if n == 1 { "" } else { "s" });
        let sources: Vec<PathBuf> = renames.iter().map(|(p, _)| p.clone()).collect();
        let file_list: Vec<String> = renames.iter().map(|(_, name)| name.clone()).collect();
        Self {
            kind: OpKind::BulkRename,
            sources,
            destination: None,
            summary,
            file_list,
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
        }
    }

    /// Build a plan for duplicating files in place.
    pub fn for_duplicate(sources: &[PathBuf]) -> Self {
        let n = sources.len();
        let summary = if n == 1 {
            let name = sources[0]
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("item");
            format!("Duplicate \"{name}\"")
        } else {
            format!("Duplicate {n} items")
        };
        let file_list: Vec<String> = sources
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        Self {
            kind: OpKind::Duplicate,
            sources: sources.to_vec(),
            destination: None,
            summary,
            file_list,
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
        }
    }

    /// Build a plan for creating a new folder.
    pub fn for_new_folder(parent_dir: &Path, name: &str) -> Self {
        Self {
            kind: OpKind::NewFolder,
            sources: Vec::new(),
            destination: Some(parent_dir.to_path_buf()),
            summary: format!("Create folder \"{name}\""),
            file_list: vec![name.to_string()],
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
        }
    }

    /// Build a plan for creating a new (empty) file.
    pub fn for_new_file(parent_dir: &Path, name: &str) -> Self {
        Self {
            kind: OpKind::NewFile,
            sources: Vec::new(),
            destination: Some(parent_dir.to_path_buf()),
            summary: format!("Create file \"{name}\""),
            file_list: vec![name.to_string()],
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
        }
    }

    /// Build a plan for applying a mark (tint + shape) to paths, optionally recursing into folders.
    #[allow(dead_code)]
    pub fn for_paint_mark(
        paths: &[PathBuf],
        tint_id: i64,
        tint_name: &str,
        shape: Shape,
        recursive: bool,
    ) -> Self {
        let n = paths.len();
        let summary = format!(
            "Mark {} item{} {} {}",
            n,
            if n == 1 { "" } else { "s" },
            tint_name,
            shape.display_name(),
        );
        let file_list: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        Self {
            kind: OpKind::PaintMark {
                tint_id,
                tint_name: tint_name.to_string(),
                shape,
                recursive,
            },
            sources: paths.to_vec(),
            destination: None,
            summary,
            file_list,
            conflicts: Vec::new(),
            warn_level: if recursive {
                WarnLevel::Caution
            } else {
                WarnLevel::None
            },
        }
    }

    /// Build a plan for resetting paths to the default mark (Beige Square).
    #[allow(dead_code)]
    pub fn for_reset_mark(paths: &[PathBuf], recursive: bool) -> Self {
        let n = paths.len();
        let summary = format!(
            "Reset {} item{} to Beige Square",
            n,
            if n == 1 { "" } else { "s" },
        );
        let file_list: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        Self {
            kind: OpKind::ResetMark { recursive },
            sources: paths.to_vec(),
            destination: None,
            summary,
            file_list,
            conflicts: Vec::new(),
            warn_level: if recursive {
                WarnLevel::Caution
            } else {
                WarnLevel::None
            },
        }
    }

    /// Build a plan for sending files to a project folder.
    /// `destination` stores the resolved project root so execution doesn't need a project lookup.
    pub fn for_send_to_project(
        sources: &[PathBuf],
        project_name: &str,
        project_root: &Path,
        is_copy: bool,
    ) -> Self {
        let n = sources.len();
        let verb = if is_copy { "Copy" } else { "Move" };
        let summary = format!(
            "{verb} {} item{} → {project_name}",
            n,
            if n == 1 { "" } else { "s" }
        );
        let file_list: Vec<String> = sources
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        Self {
            kind: OpKind::SendToProject { is_copy },
            sources: sources.to_vec(),
            destination: Some(project_root.to_path_buf()),
            summary,
            file_list,
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
        }
    }
}
