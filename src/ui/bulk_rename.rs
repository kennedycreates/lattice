use gtk::prelude::*;
use gtk::{
    Adjustment, Align, Box as GtkBox, Button, CheckButton, Entry, Label, ListBox, ListBoxRow,
    Orientation, ScrolledWindow, Separator, SpinButton, Stack,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use crate::ui::file_grid::FileItem;
use crate::ui::modal_host::{build_modal_actions, build_modal_button, ButtonKind, ModalHost};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenameMode {
    FindReplace,
    Numbering,
}

struct State {
    selected: Rc<Vec<FileItem>>,
    existing_names: Rc<HashSet<String>>,
    mode: Rc<Cell<RenameMode>>,
    find_entry: Entry,
    replace_entry: Entry,
    prefix_entry: Entry,
    suffix_entry: Entry,
    start_spin: SpinButton,
    digits_spin: SpinButton,
    preview_list: ListBox,
    warn_label: Label,
    apply_btn: Button,
    pending: Rc<RefCell<Vec<(PathBuf, String)>>>,
}

impl State {
    fn refresh(&self) {
        let new_names = compute_new_names(
            &self.selected,
            self.mode.get(),
            &self.find_entry.text(),
            &self.replace_entry.text(),
            &self.prefix_entry.text(),
            self.start_spin.value() as u32,
            self.digits_spin.value() as u32,
            &self.suffix_entry.text(),
        );

        let mut freq: HashMap<&str, usize> = HashMap::new();
        for name in &new_names {
            *freq.entry(name.as_str()).or_insert(0) += 1;
        }

        let mut conflicts = 0usize;
        let pairs: Vec<(&FileItem, &String, bool)> = self
            .selected
            .iter()
            .zip(&new_names)
            .map(|(item, new_name)| {
                let empty = new_name.is_empty();
                let dup = freq.get(new_name.as_str()).copied().unwrap_or(0) > 1;
                let clash =
                    new_name != &item.name && self.existing_names.contains(new_name.as_str());
                let conflict = empty || dup || clash;
                if conflict {
                    conflicts += 1;
                }
                (item, new_name, conflict)
            })
            .collect();

        while let Some(child) = self.preview_list.first_child() {
            self.preview_list.remove(&child);
        }
        for (item, new_name, conflict) in &pairs {
            self.preview_list
                .append(&build_preview_row(&item.name, new_name, *conflict));
        }

        *self.pending.borrow_mut() = pairs
            .iter()
            .filter(|(item, new_name, conflict)| {
                !conflict && !new_name.is_empty() && **new_name != item.name
            })
            .map(|(item, new_name, _)| (item.path.clone(), (*new_name).clone()))
            .collect();

        if conflicts > 0 {
            self.warn_label.set_label(&format!(
                "⚠  {} name conflict{} — fix before applying.",
                conflicts,
                if conflicts == 1 { "" } else { "s" }
            ));
            self.warn_label.set_visible(true);
            self.apply_btn.set_sensitive(false);
        } else {
            self.warn_label.set_visible(false);
            self.apply_btn.set_sensitive(true);
        }
    }
}

fn compute_new_names(
    items: &[FileItem],
    mode: RenameMode,
    find: &str,
    replace: &str,
    prefix: &str,
    start_num: u32,
    num_digits: u32,
    suffix: &str,
) -> Vec<String> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| match mode {
            RenameMode::FindReplace => {
                if find.is_empty() {
                    item.name.clone()
                } else {
                    item.name.replace(find, replace)
                }
            }
            RenameMode::Numbering => {
                let ext = std::path::Path::new(&item.name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .unwrap_or_default();
                let num = start_num as usize + i;
                format!(
                    "{prefix}{num:0>width$}{suffix}{ext}",
                    width = num_digits as usize
                )
            }
        })
        .collect()
}

pub fn show(
    modal_host: &ModalHost,
    selected: Vec<FileItem>,
    existing_names: HashSet<String>,
    on_apply: impl Fn(Vec<(PathBuf, String)>) + 'static,
) {
    let title = format!("Bulk Rename — {} files", selected.len());

    // ── Content box ───────────────────────────────────────────────────────
    let content = GtkBox::new(Orientation::Vertical, 0);
    content.add_css_class("bulk-rename-content");
    content.set_margin_top(4);

    // ── Mode toggle ───────────────────────────────────────────────────────
    let mode_row = GtkBox::new(Orientation::Horizontal, 0);
    mode_row.add_css_class("bulk-rename-mode-row");
    mode_row.set_halign(Align::Center);
    mode_row.set_margin_bottom(14);

    let fr_btn = CheckButton::with_label("Find & Replace");
    fr_btn.add_css_class("bulk-rename-mode-btn");
    fr_btn.set_active(true);

    let num_btn = CheckButton::with_label("Add Numbering");
    num_btn.add_css_class("bulk-rename-mode-btn");
    num_btn.set_group(Some(&fr_btn));

    mode_row.append(&fr_btn);
    mode_row.append(&num_btn);
    content.append(&mode_row);

    // ── Mode stack ────────────────────────────────────────────────────────
    let mode_stack = Stack::new();
    mode_stack.set_transition_type(gtk::StackTransitionType::None);
    mode_stack.set_margin_bottom(12);

    let fr_panel = GtkBox::new(Orientation::Vertical, 10);
    fr_panel.add_css_class("bulk-rename-panel");
    let (find_row, find_entry) = field_row("Find:");
    let (replace_row, replace_entry) = field_row("Replace with:");
    fr_panel.append(&find_row);
    fr_panel.append(&replace_row);
    mode_stack.add_named(&fr_panel, Some("find_replace"));

    let num_panel = GtkBox::new(Orientation::Vertical, 10);
    num_panel.add_css_class("bulk-rename-panel");
    let (prefix_row, prefix_entry) = field_row("Prefix:");
    num_panel.append(&prefix_row);

    let counters = GtkBox::new(Orientation::Horizontal, 24);
    counters.set_halign(Align::Start);
    let (start_col, start_spin) = spin_col("Start at:", 1.0, 1.0, 99999.0);
    let (digits_col, digits_spin) = spin_col("Digits:", 3.0, 1.0, 9.0);
    counters.append(&start_col);
    counters.append(&digits_col);
    num_panel.append(&counters);

    let (suffix_row, suffix_entry) = field_row("Suffix (optional):");
    num_panel.append(&suffix_row);

    mode_stack.add_named(&num_panel, Some("numbering"));
    content.append(&mode_stack);

    // ── Separator + preview header ────────────────────────────────────────
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("bulk-rename-sep");
    content.append(&sep);

    let hdr = Label::new(Some("Preview"));
    hdr.add_css_class("bulk-rename-section-label");
    hdr.set_halign(Align::Start);
    hdr.set_margin_top(10);
    hdr.set_margin_bottom(6);
    content.append(&hdr);

    // ── Preview list ──────────────────────────────────────────────────────
    let preview_list = ListBox::new();
    preview_list.add_css_class("bulk-rename-preview");
    preview_list.set_selection_mode(gtk::SelectionMode::None);

    let preview_scroll = ScrolledWindow::builder()
        .child(&preview_list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    preview_scroll.add_css_class("bulk-rename-preview-scroll");
    content.append(&preview_scroll);

    // ── Warning label ─────────────────────────────────────────────────────
    let warn_label = Label::new(None);
    warn_label.add_css_class("bulk-rename-warn");
    warn_label.set_halign(Align::Start);
    warn_label.set_visible(false);
    warn_label.set_margin_top(6);
    content.append(&warn_label);

    // ── Action buttons ────────────────────────────────────────────────────
    // The Apply button is created before State so State can hold a reference
    // and toggle its sensitivity via refresh().
    let actions = build_modal_actions();

    let host_cancel = modal_host.clone();
    let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || {
        host_cancel.hide();
    });
    actions.append(&cancel_btn);

    let apply_btn = Button::with_label("Apply Rename");
    apply_btn.add_css_class("modal-primary-button");
    apply_btn.set_sensitive(false);

    // ── Shared state ──────────────────────────────────────────────────────
    let mode = Rc::new(Cell::new(RenameMode::FindReplace));
    let pending: Rc<RefCell<Vec<(PathBuf, String)>>> = Rc::new(RefCell::new(Vec::new()));

    let state = Rc::new(State {
        selected: Rc::new(selected),
        existing_names: Rc::new(existing_names),
        mode: Rc::clone(&mode),
        find_entry: find_entry.clone(),
        replace_entry: replace_entry.clone(),
        prefix_entry: prefix_entry.clone(),
        suffix_entry: suffix_entry.clone(),
        start_spin: start_spin.clone(),
        digits_spin: digits_spin.clone(),
        preview_list,
        warn_label,
        apply_btn: apply_btn.clone(),
        pending: Rc::clone(&pending),
    });

    // ── Mode toggle signals ───────────────────────────────────────────────
    {
        let mode_stack = mode_stack.clone();
        let state = Rc::clone(&state);
        fr_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                state.mode.set(RenameMode::FindReplace);
                mode_stack.set_visible_child_name("find_replace");
                state.refresh();
            }
        });
    }
    {
        let mode_stack = mode_stack.clone();
        let state = Rc::clone(&state);
        num_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                state.mode.set(RenameMode::Numbering);
                mode_stack.set_visible_child_name("numbering");
                state.refresh();
            }
        });
    }

    // ── Input change signals ──────────────────────────────────────────────
    {
        let s = Rc::clone(&state);
        find_entry.connect_changed(move |_| s.refresh());
    }
    {
        let s = Rc::clone(&state);
        replace_entry.connect_changed(move |_| s.refresh());
    }
    {
        let s = Rc::clone(&state);
        prefix_entry.connect_changed(move |_| s.refresh());
    }
    {
        let s = Rc::clone(&state);
        suffix_entry.connect_changed(move |_| s.refresh());
    }
    {
        let s = Rc::clone(&state);
        start_spin.connect_value_changed(move |_| s.refresh());
    }
    {
        let s = Rc::clone(&state);
        digits_spin.connect_value_changed(move |_| s.refresh());
    }

    // ── Apply button click ────────────────────────────────────────────────
    {
        let host = modal_host.clone();
        let on_apply = Rc::new(on_apply);
        apply_btn.connect_clicked(move |_| {
            let renames = pending.borrow().clone();
            if !renames.is_empty() {
                on_apply(renames);
            }
            host.hide();
        });
    }
    actions.append(&apply_btn);

    state.refresh();

    modal_host.show_with_custom_ui(
        &title, &content, &actions,
        false, // bulk rename must not be dismissed by clicking the scrim
        None,
    );
}

fn field_row(label_text: &str) -> (GtkBox, Entry) {
    let row = GtkBox::new(Orientation::Horizontal, 12);
    row.set_hexpand(true);

    let label = Label::new(Some(label_text));
    label.add_css_class("bulk-rename-field-label");
    label.set_width_chars(20);
    label.set_halign(Align::End);
    row.append(&label);

    let entry = Entry::new();
    entry.set_hexpand(true);
    entry.add_css_class("dialog-entry");
    row.append(&entry);

    (row, entry)
}

fn spin_col(label_text: &str, value: f64, min: f64, max: f64) -> (GtkBox, SpinButton) {
    let col = GtkBox::new(Orientation::Vertical, 4);

    let label = Label::new(Some(label_text));
    label.add_css_class("bulk-rename-field-label");
    label.set_halign(Align::Start);
    col.append(&label);

    let adj = Adjustment::new(value, min, max, 1.0, 10.0, 0.0);
    let spin = SpinButton::new(Some(&adj), 1.0, 0);
    spin.set_numeric(true);
    spin.add_css_class("bulk-rename-spin");
    col.append(&spin);

    (col, spin)
}

fn build_preview_row(old_name: &str, new_name: &str, conflict: bool) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.add_css_class("bulk-rename-row");
    if conflict {
        row.add_css_class("bulk-rename-row-conflict");
    }
    row.set_activatable(false);
    row.set_selectable(false);

    let inner = GtkBox::new(Orientation::Horizontal, 10);
    inner.set_margin_top(5);
    inner.set_margin_bottom(5);
    inner.set_margin_start(10);
    inner.set_margin_end(10);

    let old_lbl = Label::new(Some(old_name));
    old_lbl.add_css_class("bulk-rename-old");
    old_lbl.set_hexpand(true);
    old_lbl.set_halign(Align::Start);
    old_lbl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    old_lbl.set_single_line_mode(true);
    inner.append(&old_lbl);

    let arrow = Label::new(Some("→"));
    arrow.add_css_class("bulk-rename-arrow");
    inner.append(&arrow);

    let new_css = if conflict {
        "bulk-rename-new-conflict"
    } else if new_name == old_name {
        "bulk-rename-new-unchanged"
    } else {
        "bulk-rename-new"
    };
    let new_lbl = Label::new(Some(new_name));
    new_lbl.add_css_class(new_css);
    new_lbl.set_hexpand(true);
    new_lbl.set_halign(Align::Start);
    new_lbl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    new_lbl.set_single_line_mode(true);
    inner.append(&new_lbl);

    if conflict {
        let warn = Label::new(Some("⚠"));
        warn.add_css_class("bulk-rename-conflict-icon");
        inner.append(&warn);
    }

    row.set_child(Some(&inner));
    row
}
