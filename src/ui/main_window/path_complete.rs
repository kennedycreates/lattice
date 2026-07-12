//! Path-entry filename autocompletion, split out of main_window.
#![allow(deprecated)]

use gtk::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathCompletionMode {
    Absolute,
    Home,
    Relative,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PathCompletionQuery {
    pub(super) query: String,
    pub(super) mode: PathCompletionMode,
}

#[allow(deprecated)]
pub(super) fn update_path_completion_model(
    store: &gtk::ListStore,
    completer: &gio::FilenameCompleter,
    input: &str,
    current_dir: &Path,
    home: &Path,
) {
    store.clear();

    let Some(query) = path_completion_query(input, current_dir, home) else {
        return;
    };

    let mut seen = HashSet::new();
    let mut completions = completer
        .completions(&query.query)
        .into_iter()
        .filter_map(|completion| {
            path_completion_display(&completion, query.mode, current_dir, home)
        })
        .filter(|completion| completion != input)
        .filter(|completion| seen.insert(completion.clone()))
        .collect::<Vec<_>>();

    completions.sort_by_key(|completion| completion.to_ascii_lowercase());
    completions.truncate(24);

    for completion in completions {
        store.insert_with_values(None, &[(0, &completion)]);
    }
}

#[allow(deprecated)]
pub(super) fn first_path_completion(store: &gtk::ListStore) -> Option<String> {
    let iter = store.iter_first()?;
    Some(store.get::<String>(&iter, 0))
}

pub(super) fn path_completion_query(
    input: &str,
    current_dir: &Path,
    home: &Path,
) -> Option<PathCompletionQuery> {
    let input = input.trim_start();
    if input.is_empty() || input.starts_with("file://") {
        return None;
    }

    if input == "~" {
        return Some(PathCompletionQuery {
            query: home.display().to_string(),
            mode: PathCompletionMode::Home,
        });
    }

    if let Some(relative_home) = input.strip_prefix("~/") {
        return Some(PathCompletionQuery {
            query: home.join(relative_home).display().to_string(),
            mode: PathCompletionMode::Home,
        });
    }

    let candidate = PathBuf::from(input);
    if candidate.is_absolute() {
        Some(PathCompletionQuery {
            query: input.to_string(),
            mode: PathCompletionMode::Absolute,
        })
    } else {
        Some(PathCompletionQuery {
            query: current_dir.join(input).display().to_string(),
            mode: PathCompletionMode::Relative,
        })
    }
}

pub(super) fn path_completion_display(
    completion: &str,
    mode: PathCompletionMode,
    current_dir: &Path,
    home: &Path,
) -> Option<String> {
    match mode {
        PathCompletionMode::Absolute => Some(completion.to_string()),
        PathCompletionMode::Home => display_completion_under_root(completion, home, "~"),
        PathCompletionMode::Relative => display_completion_under_root(completion, current_dir, ""),
    }
}

fn display_completion_under_root(completion: &str, root: &Path, prefix: &str) -> Option<String> {
    let root = root.display().to_string();
    if completion == root {
        return Some(prefix.to_string());
    }

    let root_with_slash = format!("{root}/");
    let suffix = completion.strip_prefix(&root_with_slash)?;
    if prefix.is_empty() {
        Some(suffix.to_string())
    } else {
        Some(format!("{prefix}/{suffix}"))
    }
}
