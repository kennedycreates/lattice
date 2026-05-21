use crate::metadata::Shape;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RenameSpec {
    pub path: PathBuf,
    pub new_name: String,
}

#[derive(Debug, Clone)]
pub struct RestoreSpec {
    pub trash_path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct TrayPlanCompletion {
    pub action: String,
    pub clear_successful_paths: bool,
}

#[derive(Debug, Clone)]
pub enum OpKind {
    CopyMove {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        is_copy: bool,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    PermanentDelete {
        paths: Vec<PathBuf>,
    },
    Rename(RenameSpec),
    BulkRename {
        renames: Vec<RenameSpec>,
    },
    Duplicate {
        paths: Vec<PathBuf>,
    },
    NewFolder {
        parent: PathBuf,
        name: String,
    },
    NewFile {
        parent: PathBuf,
        name: String,
    },
    SendToProject {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        is_copy: bool,
    },
    PaintMark {
        paths: Vec<PathBuf>,
        tint_id: i64,
        tint_name: String,
        shape: Shape,
        recursive: bool,
    },
    ResetMark {
        paths: Vec<PathBuf>,
        recursive: bool,
    },
    ApplyTag {
        paths: Vec<PathBuf>,
        tag_name: String,
    },
    RemoveTags {
        paths: Vec<PathBuf>,
        tag_ids: Vec<i64>,
    },
    CopyPaths {
        paths: Vec<PathBuf>,
    },
    RestoreTrash {
        items: Vec<RestoreSpec>,
    },
    EmptyTrash,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WarnLevel {
    None,
    Caution,
    Danger,
}

#[derive(Debug, Clone)]
pub struct ActionPlan {
    pub kind: OpKind,
    pub summary: String,
    pub file_list: Vec<String>,
    pub conflicts: Vec<String>,
    pub warn_level: WarnLevel,
    pub cloud_note: Option<String>,
    pub tray_completion: Option<TrayPlanCompletion>,
}

impl ActionPlan {
    pub fn with_cloud_note(mut self, note: String) -> Self {
        self.cloud_note = Some(note);
        self
    }

    pub fn with_tray_completion(mut self, action: &str, clear_successful_paths: bool) -> Self {
        self.tray_completion = Some(TrayPlanCompletion {
            action: action.to_string(),
            clear_successful_paths,
        });
        self
    }

    pub fn cloud_probe_path(&self) -> Option<&Path> {
        match &self.kind {
            OpKind::CopyMove {
                sources,
                destination,
                ..
            }
            | OpKind::SendToProject {
                sources,
                destination,
                ..
            } => sources
                .first()
                .map(PathBuf::as_path)
                .or(Some(destination.as_path())),
            OpKind::Trash { paths }
            | OpKind::PermanentDelete { paths }
            | OpKind::Duplicate { paths }
            | OpKind::PaintMark { paths, .. }
            | OpKind::ResetMark { paths, .. }
            | OpKind::ApplyTag { paths, .. }
            | OpKind::RemoveTags { paths, .. }
            | OpKind::CopyPaths { paths } => paths.first().map(PathBuf::as_path),
            OpKind::Rename(spec) => Some(spec.path.as_path()),
            OpKind::BulkRename { renames } => renames.first().map(|spec| spec.path.as_path()),
            OpKind::NewFolder { parent, .. } | OpKind::NewFile { parent, .. } => {
                Some(parent.as_path())
            }
            OpKind::RestoreTrash { items } => items
                .first()
                .map(|item| item.trash_path.as_path())
                .or_else(|| items.first()?.original_path.as_deref()),
            OpKind::EmptyTrash => None,
        }
    }

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
            format!("{verb} {name} -> {dest_name}")
        } else {
            format!("{verb} {n} files -> {dest_name}")
        };

        let file_list = path_names(sources);
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
            kind: OpKind::CopyMove {
                sources: sources.to_vec(),
                destination: dest.to_path_buf(),
                is_copy,
            },
            summary,
            file_list,
            conflicts,
            warn_level,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_trash(paths: &[PathBuf]) -> Self {
        let n = paths.len();
        let summary = if n == 1 {
            let name = path_name(&paths[0]);
            format!("Move \"{name}\" to Trash")
        } else {
            format!("Move {n} files to Trash")
        };
        Self {
            kind: OpKind::Trash {
                paths: paths.to_vec(),
            },
            summary,
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_rename(path: &Path, new_name: &str) -> Self {
        let old_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("item");
        Self {
            kind: OpKind::Rename(RenameSpec {
                path: path.to_path_buf(),
                new_name: new_name.to_string(),
            }),
            summary: format!("Rename \"{old_name}\" -> \"{new_name}\""),
            file_list: vec![new_name.to_string()],
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_permanent_delete(paths: &[PathBuf]) -> Self {
        let n = paths.len();
        let summary = if n == 1 {
            let name = path_name(&paths[0]);
            format!("Delete \"{name}\" permanently")
        } else {
            format!("Delete {n} items permanently")
        };
        Self {
            kind: OpKind::PermanentDelete {
                paths: paths.to_vec(),
            },
            summary,
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: WarnLevel::Danger,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_bulk_rename(renames: &[(PathBuf, String)]) -> Self {
        let n = renames.len();
        Self {
            kind: OpKind::BulkRename {
                renames: renames
                    .iter()
                    .map(|(path, new_name)| RenameSpec {
                        path: path.clone(),
                        new_name: new_name.clone(),
                    })
                    .collect(),
            },
            summary: format!("Bulk rename {} file{}", n, if n == 1 { "" } else { "s" }),
            file_list: renames.iter().map(|(_, name)| name.clone()).collect(),
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_duplicate(paths: &[PathBuf]) -> Self {
        let n = paths.len();
        let summary = if n == 1 {
            let name = path_name(&paths[0]);
            format!("Duplicate \"{name}\"")
        } else {
            format!("Duplicate {n} items")
        };
        Self {
            kind: OpKind::Duplicate {
                paths: paths.to_vec(),
            },
            summary,
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_new_folder(parent_dir: &Path, name: &str) -> Self {
        Self {
            kind: OpKind::NewFolder {
                parent: parent_dir.to_path_buf(),
                name: name.to_string(),
            },
            summary: format!("Create folder \"{name}\""),
            file_list: vec![name.to_string()],
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_new_file(parent_dir: &Path, name: &str) -> Self {
        Self {
            kind: OpKind::NewFile {
                parent: parent_dir.to_path_buf(),
                name: name.to_string(),
            },
            summary: format!("Create file \"{name}\""),
            file_list: vec![name.to_string()],
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_paint_mark(
        paths: &[PathBuf],
        tint_id: i64,
        tint_name: &str,
        shape: Shape,
        recursive: bool,
    ) -> Self {
        let n = paths.len();
        Self {
            kind: OpKind::PaintMark {
                paths: paths.to_vec(),
                tint_id,
                tint_name: tint_name.to_string(),
                shape,
                recursive,
            },
            summary: format!(
                "Mark {} item{} {} {}",
                n,
                if n == 1 { "" } else { "s" },
                tint_name,
                shape.display_name(),
            ),
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: if recursive {
                WarnLevel::Caution
            } else {
                WarnLevel::None
            },
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_reset_mark(paths: &[PathBuf], recursive: bool) -> Self {
        let n = paths.len();
        Self {
            kind: OpKind::ResetMark {
                paths: paths.to_vec(),
                recursive,
            },
            summary: format!(
                "Reset {} item{} to Beige Square",
                n,
                if n == 1 { "" } else { "s" },
            ),
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: if recursive {
                WarnLevel::Caution
            } else {
                WarnLevel::None
            },
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_send_to_project(
        sources: &[PathBuf],
        project_name: &str,
        project_root: &Path,
        is_copy: bool,
    ) -> Self {
        let n = sources.len();
        let verb = if is_copy { "Copy" } else { "Move" };
        Self {
            kind: OpKind::SendToProject {
                sources: sources.to_vec(),
                destination: project_root.to_path_buf(),
                is_copy,
            },
            summary: format!(
                "{verb} {} item{} -> palette {project_name}",
                n,
                if n == 1 { "" } else { "s" }
            ),
            file_list: path_names(sources),
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_apply_tag(paths: &[PathBuf], tag_name: &str) -> Self {
        let n = paths.len();
        Self {
            kind: OpKind::ApplyTag {
                paths: paths.to_vec(),
                tag_name: tag_name.to_string(),
            },
            summary: format!(
                "Apply tag #{tag_name} to {n} item{}",
                if n == 1 { "" } else { "s" }
            ),
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_remove_tags(paths: &[PathBuf], tag_ids: &[i64], tag_names: &[String]) -> Self {
        let n = paths.len();
        let tag_summary = if tag_names.is_empty() {
            format!(
                "{} tag{}",
                tag_ids.len(),
                if tag_ids.len() == 1 { "" } else { "s" }
            )
        } else {
            tag_names
                .iter()
                .map(|name| format!("#{name}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        Self {
            kind: OpKind::RemoveTags {
                paths: paths.to_vec(),
                tag_ids: tag_ids.to_vec(),
            },
            summary: format!(
                "Remove {tag_summary} from {n} item{}",
                if n == 1 { "" } else { "s" }
            ),
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_copy_paths(paths: &[PathBuf]) -> Self {
        let n = paths.len();
        Self {
            kind: OpKind::CopyPaths {
                paths: paths.to_vec(),
            },
            summary: format!(
                "Copy {n} path{} to clipboard",
                if n == 1 { "" } else { "s" }
            ),
            file_list: path_names(paths),
            conflicts: Vec::new(),
            warn_level: WarnLevel::None,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_restore_trash(items: Vec<RestoreSpec>) -> Self {
        let n = items.len();
        Self {
            kind: OpKind::RestoreTrash {
                items: items.clone(),
            },
            summary: format!(
                "Restore {n} item{} from Trash",
                if n == 1 { "" } else { "s" }
            ),
            file_list: items
                .iter()
                .map(|item| item.display_name.clone())
                .collect::<Vec<_>>(),
            conflicts: Vec::new(),
            warn_level: WarnLevel::Caution,
            cloud_note: None,
            tray_completion: None,
        }
    }

    pub fn for_empty_trash(count: usize) -> Self {
        Self {
            kind: OpKind::EmptyTrash,
            summary: format!(
                "Empty Trash ({count} item{})",
                if count == 1 { "" } else { "s" }
            ),
            file_list: Vec::new(),
            conflicts: Vec::new(),
            warn_level: WarnLevel::Danger,
            cloud_note: None,
            tray_completion: None,
        }
    }
}

fn path_name(path: &Path) -> String {
    path.file_name()
        .and_then(|p| p.to_str())
        .unwrap_or("item")
        .to_string()
}

fn path_names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| path_name(path)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_plan_captures_copy_move_payload() {
        let sources = vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")];
        let dest = PathBuf::from("/tmp/out");
        let plan = ActionPlan::for_paste(&sources, &dest, true);

        match plan.kind {
            OpKind::CopyMove {
                sources: captured,
                destination,
                is_copy,
            } => {
                assert_eq!(captured, sources);
                assert_eq!(destination, dest);
                assert!(is_copy);
            }
            _ => panic!("expected copy/move plan"),
        }
        assert_eq!(plan.file_list, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn tray_completion_is_carried_by_plan() {
        let paths = vec![PathBuf::from("/tmp/a.txt")];
        let plan = ActionPlan::for_trash(&paths).with_tray_completion("Move Tray to Trash", true);
        let completion = plan.tray_completion.expect("tray completion");

        assert_eq!(completion.action, "Move Tray to Trash");
        assert!(completion.clear_successful_paths);
    }

    #[test]
    fn tag_and_restore_plans_have_explicit_payloads() {
        let paths = vec![PathBuf::from("/tmp/a.txt")];
        let tag_plan = ActionPlan::for_apply_tag(&paths, "urgent");
        match tag_plan.kind {
            OpKind::ApplyTag { paths, tag_name } => {
                assert_eq!(paths, vec![PathBuf::from("/tmp/a.txt")]);
                assert_eq!(tag_name, "urgent");
            }
            _ => panic!("expected apply-tag plan"),
        }

        let restore = RestoreSpec {
            trash_path: PathBuf::from("/tmp/.Trash/files/a.txt"),
            original_path: Some(PathBuf::from("/home/me/a.txt")),
            display_name: "a.txt".to_string(),
        };
        let restore_plan = ActionPlan::for_restore_trash(vec![restore.clone()]);
        match restore_plan.kind {
            OpKind::RestoreTrash { items } => {
                assert_eq!(items[0].trash_path, restore.trash_path);
                assert_eq!(items[0].original_path, restore.original_path);
            }
            _ => panic!("expected restore plan"),
        }
    }
}
