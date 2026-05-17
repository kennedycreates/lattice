use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Label, Orientation, ScrolledWindow};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use crate::ui::modal_host::{build_modal_actions, build_modal_button, ButtonKind, ModalHost};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ConflictItem {
    pub name: String,
    pub incoming: PathBuf,
    pub existing: PathBuf,
    pub dest_dir: PathBuf,
    pub incoming_size: Option<u64>,
    pub incoming_modified: Option<SystemTime>,
    pub existing_size: Option<u64>,
    pub existing_modified: Option<SystemTime>,
    pub mime_label: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resolution {
    KeepBoth,
    Replace,
    Skip,
}

pub struct ConflictDecision {
    pub item: ConflictItem,
    pub resolution: Resolution,
}

// ── Collection ────────────────────────────────────────────────────────────────

/// Collect all files in `sources` that already exist at `dest`.
pub fn collect_conflicts(sources: &[PathBuf], dest: &Path) -> Vec<ConflictItem> {
    sources
        .iter()
        .filter_map(|src| {
            let name = src.file_name()?.to_string_lossy().into_owned();
            let existing_path = dest.join(&name);
            if !existing_path.exists() {
                return None;
            }
            let (inc_size, inc_modified) = read_meta(src);
            let (ex_size, ex_modified) = read_meta(&existing_path);
            let mime_label = guess_mime_label(&name);
            Some(ConflictItem {
                name,
                incoming: src.clone(),
                existing: existing_path,
                dest_dir: dest.to_path_buf(),
                incoming_size: inc_size,
                incoming_modified: inc_modified,
                existing_size: ex_size,
                existing_modified: ex_modified,
                mime_label,
            })
        })
        .collect()
}

fn read_meta(path: &Path) -> (Option<u64>, Option<SystemTime>) {
    match std::fs::metadata(path) {
        Ok(m) => (Some(m.len()), m.modified().ok()),
        Err(_) => (None, None),
    }
}

fn guess_mime_label(name: &str) -> Option<String> {
    let (mime, _) = gio::functions::content_type_guess(Some(name), &[]);
    if mime == "application/octet-stream" {
        return None;
    }
    let desc = gio::functions::content_type_get_description(&mime);
    if desc.is_empty() {
        None
    } else {
        Some(desc.into())
    }
}

// ── Resolution application ────────────────────────────────────────────────────

/// Merge conflict decisions with non-conflicting items into a batch list.
/// `None` decisions are produced only by Cancel; callers should check for
/// an empty result if all conflicts were skipped.
pub fn apply_decisions(
    decisions: &[ConflictDecision],
    non_conflicting: &[(PathBuf, PathBuf, gio::FileCopyFlags)],
) -> Vec<(PathBuf, PathBuf, gio::FileCopyFlags)> {
    let plain = gio::FileCopyFlags::ALL_METADATA;
    let overwrite = gio::FileCopyFlags::OVERWRITE | gio::FileCopyFlags::ALL_METADATA;

    let mut items: Vec<(PathBuf, PathBuf, gio::FileCopyFlags)> = non_conflicting.to_vec();
    for d in decisions {
        match d.resolution {
            Resolution::KeepBoth => {
                let free = free_name_in(&d.item.dest_dir, &d.item.name);
                items.push((d.item.incoming.clone(), free, plain));
            }
            Resolution::Replace => {
                items.push((d.item.incoming.clone(), d.item.existing.clone(), overwrite));
            }
            Resolution::Skip => {}
        }
    }
    items
}

/// Build a human-readable note about conflict decisions for the activity log.
pub fn decisions_note(decisions: &[ConflictDecision]) -> String {
    let renamed = decisions
        .iter()
        .filter(|d| d.resolution == Resolution::KeepBoth)
        .count();
    let replaced = decisions
        .iter()
        .filter(|d| d.resolution == Resolution::Replace)
        .count();
    let skipped = decisions
        .iter()
        .filter(|d| d.resolution == Resolution::Skip)
        .count();
    let mut parts: Vec<String> = Vec::new();
    if renamed > 0 {
        parts.push(format!("{renamed} renamed"));
    }
    if replaced > 0 {
        parts.push(format!("{replaced} replaced"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

// ── UI ────────────────────────────────────────────────────────────────────────

/// Show the conflict resolver inside ModalHost.
/// `on_done(Some(decisions))` is called when the user applies choices;
/// `on_done(None)` is called when the user cancels the entire operation.
pub fn show(
    modal_host: &ModalHost,
    conflicts: Vec<ConflictItem>,
    on_done: impl Fn(Option<Vec<ConflictDecision>>) + 'static,
) {
    let n = conflicts.len();
    let conflicts = Rc::new(conflicts);
    let on_done = Rc::new(on_done);

    // Outer content box
    let content = GtkBox::new(Orientation::Vertical, 10);
    content.add_css_class("conflict-resolver-content");

    // Header prompt
    let header_text = if n == 1 {
        "1 file already exists at the destination.".to_string()
    } else {
        format!("{n} files already exist at the destination.")
    };
    let header = Label::new(Some(&header_text));
    header.add_css_class("dialog-prompt");
    header.set_halign(Align::Start);
    header.set_wrap(true);
    header.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    content.append(&header);

    // Scrolled list
    let scroll = ScrolledWindow::new();
    scroll.add_css_class("conflict-scroll");
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

    let rows_box = GtkBox::new(Orientation::Vertical, 6);
    rows_box.add_css_class("conflict-rows-box");
    rows_box.set_margin_end(4); // room for scrollbar

    // Per-row state: (Rc<Cell<Resolution>>, keep_btn, replace_btn, skip_btn)
    let mut row_states: Vec<(Rc<Cell<Resolution>>, Button, Button, Button)> = Vec::new();

    for item in conflicts.iter() {
        let state = Rc::new(Cell::new(Resolution::KeepBoth));

        let row = GtkBox::new(Orientation::Vertical, 6);
        row.add_css_class("conflict-item-row");

        // Name row
        let name_row = GtkBox::new(Orientation::Horizontal, 8);
        name_row.set_valign(Align::Center);

        let name_label = Label::new(Some(&item.name));
        name_label.add_css_class("conflict-item-name");
        name_label.set_halign(Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        name_row.append(&name_label);

        if let Some(ref mime) = item.mime_label {
            let chip = Label::new(Some(mime));
            chip.add_css_class("conflict-item-type-chip");
            chip.set_halign(Align::End);
            name_row.append(&chip);
        }
        row.append(&name_row);

        // Existing metadata row
        row.append(&build_meta_row(
            "Existing",
            item.existing_size,
            item.existing_modified,
        ));
        // Incoming metadata row
        row.append(&build_meta_row(
            "Incoming",
            item.incoming_size,
            item.incoming_modified,
        ));

        // Choice buttons
        let choice_row = GtkBox::new(Orientation::Horizontal, 6);
        choice_row.add_css_class("conflict-choice-row");

        let keep_btn = Button::with_label("Keep Both");
        keep_btn.add_css_class("conflict-choice-btn");
        keep_btn.add_css_class("selected"); // default active

        let replace_btn = Button::with_label("Replace \u{26A0}");
        replace_btn.add_css_class("conflict-choice-btn");
        replace_btn.add_css_class("conflict-choice-replace");

        let skip_btn = Button::with_label("Skip");
        skip_btn.add_css_class("conflict-choice-btn");

        choice_row.append(&keep_btn);
        choice_row.append(&replace_btn);
        choice_row.append(&skip_btn);
        row.append(&choice_row);
        rows_box.append(&row);

        connect_choice(
            &keep_btn,
            Resolution::KeepBoth,
            Rc::clone(&state),
            keep_btn.clone(),
            replace_btn.clone(),
            skip_btn.clone(),
        );
        connect_choice(
            &replace_btn,
            Resolution::Replace,
            Rc::clone(&state),
            keep_btn.clone(),
            replace_btn.clone(),
            skip_btn.clone(),
        );
        connect_choice(
            &skip_btn,
            Resolution::Skip,
            Rc::clone(&state),
            keep_btn.clone(),
            replace_btn.clone(),
            skip_btn.clone(),
        );

        row_states.push((state, keep_btn, replace_btn, skip_btn));
    }

    scroll.set_child(Some(&rows_box));
    content.append(&scroll);

    // Batch actions
    if n > 1 {
        let batch_row = GtkBox::new(Orientation::Horizontal, 8);
        batch_row.add_css_class("conflict-batch-row");

        let rs_keep = row_states
            .iter()
            .map(|(s, kb, rb, sb)| (Rc::clone(s), kb.clone(), rb.clone(), sb.clone()))
            .collect::<Vec<_>>();
        let rs_skip = row_states
            .iter()
            .map(|(s, kb, rb, sb)| (Rc::clone(s), kb.clone(), rb.clone(), sb.clone()))
            .collect::<Vec<_>>();

        let keep_all_btn = Button::with_label("Keep Both for All");
        keep_all_btn.add_css_class("conflict-batch-btn");
        keep_all_btn.connect_clicked(move |_| {
            for (state, keep_btn, replace_btn, skip_btn) in &rs_keep {
                state.set(Resolution::KeepBoth);
                keep_btn.add_css_class("selected");
                replace_btn.remove_css_class("selected");
                skip_btn.remove_css_class("selected");
            }
        });

        let skip_all_btn = Button::with_label("Skip All");
        skip_all_btn.add_css_class("conflict-batch-btn");
        skip_all_btn.connect_clicked(move |_| {
            for (state, keep_btn, replace_btn, skip_btn) in &rs_skip {
                state.set(Resolution::Skip);
                keep_btn.remove_css_class("selected");
                replace_btn.remove_css_class("selected");
                skip_btn.add_css_class("selected");
            }
        });

        batch_row.append(&keep_all_btn);
        batch_row.append(&skip_all_btn);
        content.append(&batch_row);
    }

    // Action buttons
    let actions = build_modal_actions();

    let on_done_cancel = Rc::clone(&on_done);
    let cancel_btn = build_modal_button("Cancel All", ButtonKind::Secondary, move || {
        on_done_cancel(None);
    });
    cancel_btn.add_css_class("conflict-cancel-btn");
    actions.append(&cancel_btn);

    let conflicts_apply = Rc::clone(&conflicts);
    let rs_apply = row_states
        .iter()
        .map(|(s, _, _, _)| Rc::clone(s))
        .collect::<Vec<_>>();
    let on_done_apply = Rc::clone(&on_done);
    let apply_btn = build_modal_button("Apply Choices", ButtonKind::Primary, move || {
        let decisions: Vec<ConflictDecision> = conflicts_apply
            .iter()
            .zip(rs_apply.iter())
            .map(|(item, res)| ConflictDecision {
                item: item.clone(),
                resolution: res.get(),
            })
            .collect();
        on_done_apply(Some(decisions));
    });
    actions.append(&apply_btn);

    modal_host.show_with_custom_ui("File Conflicts", &content, &actions, false, None);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn connect_choice(
    btn: &Button,
    choice: Resolution,
    state: Rc<Cell<Resolution>>,
    keep_btn: Button,
    replace_btn: Button,
    skip_btn: Button,
) {
    btn.connect_clicked(move |_| {
        state.set(choice);
        keep_btn.remove_css_class("selected");
        replace_btn.remove_css_class("selected");
        skip_btn.remove_css_class("selected");
        match choice {
            Resolution::KeepBoth => keep_btn.add_css_class("selected"),
            Resolution::Replace => replace_btn.add_css_class("selected"),
            Resolution::Skip => skip_btn.add_css_class("selected"),
        }
    });
}

fn build_meta_row(label: &str, size: Option<u64>, modified: Option<SystemTime>) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.add_css_class("conflict-item-meta");

    let lbl = Label::new(Some(label));
    lbl.add_css_class("conflict-item-meta-label");
    lbl.set_halign(Align::Start);
    lbl.set_width_chars(8);
    row.append(&lbl);

    let size_text = size.map(fmt_bytes).unwrap_or_else(|| "—".to_string());
    let size_lbl = Label::new(Some(&size_text));
    size_lbl.add_css_class("conflict-item-meta-value");
    size_lbl.set_halign(Align::Start);
    row.append(&size_lbl);

    if let Some(mtime) = modified {
        let sep = Label::new(Some("·"));
        sep.add_css_class("conflict-item-meta-sep");
        row.append(&sep);

        let date_text = fmt_mtime(mtime);
        let date_lbl = Label::new(Some(&date_text));
        date_lbl.add_css_class("conflict-item-meta-value");
        date_lbl.set_halign(Align::Start);
        row.append(&date_lbl);
    }

    row
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

fn fmt_mtime(t: SystemTime) -> String {
    let now = SystemTime::now();
    let age = now.duration_since(t).unwrap_or_default();
    let secs = age.as_secs();
    if secs < 60 {
        return "just now".to_string();
    }
    if secs < 3600 {
        return format!("{} min ago", secs / 60);
    }
    if secs < 86400 {
        return format!("{} hr ago", secs / 3600);
    }
    if secs < 2 * 86400 {
        return "yesterday".to_string();
    }
    format!("{} days ago", secs / 86400)
}

/// Return a path in `dir` that does not yet exist by appending " (2)", " (3)"…
pub fn free_name_in(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) => (&name[..dot], Some(&name[dot..])),
        None => (name, None),
    };
    let mut n = 2u32;
    loop {
        let candidate = match ext {
            Some(e) => format!("{stem} ({n}){e}"),
            None => format!("{stem} ({n})"),
        };
        let path = dir.join(&candidate);
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}
