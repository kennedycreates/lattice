//! Confirmation-preview builders for tag/trash/mark actions, split out of main_window.

use super::ConfirmationPreview;
use crate::metadata::Shape;
use std::path::PathBuf;

pub(super) fn tag_action_plan(paths: &[PathBuf], tag_name: &str) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Apply tag #{tag_name} to {} staged item(s).",
        paths.len()
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Tag Holding Tray", "Apply Tag", false, lines)
}

pub(super) fn trash_action_plan(paths: &[PathBuf]) -> ConfirmationPreview {
    let mut lines = vec![format!("Move {} staged item(s) to Trash.", paths.len())];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Move Tray to Trash", "Move to Trash", true, lines)
}

pub(super) fn copy_path_action_plan(paths: &[PathBuf]) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Copy {} staged path(s) to the clipboard.",
        paths.len()
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Copy Tray Paths", "Copy Paths", false, lines)
}

pub(super) fn apply_mark_action_plan(paths: &[PathBuf], tint_name: &str, shape: Shape) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Mark {} staged item(s) as {} {}.",
        paths.len(),
        tint_name,
        shape.display_name(),
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Apply Mark to Tray", "Apply Mark", false, lines)
}

pub(super) fn reset_mark_action_plan(paths: &[PathBuf]) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Reset {} staged item(s) to Beige Square.",
        paths.len(),
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Reset Mark", "Reset Mark", false, lines)
}

fn plan_path_lines(paths: &[PathBuf]) -> Vec<String> {
    const MAX_PREVIEW_PATHS: usize = 8;
    let mut lines = paths
        .iter()
        .take(MAX_PREVIEW_PATHS)
        .map(|path| format!("• {}", path.display()))
        .collect::<Vec<_>>();
    if paths.len() > MAX_PREVIEW_PATHS {
        lines.push(format!("… and {} more", paths.len() - MAX_PREVIEW_PATHS));
    }
    lines
}
