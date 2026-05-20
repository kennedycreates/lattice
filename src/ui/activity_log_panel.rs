use crate::metadata::ActivityLogEntry;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, DropDown, Label, Orientation, ScrolledWindow, StringList,
    ToggleButton,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityLogAction {
    Undo,
    Repeat,
    Reveal,
    CopyPath,
}

// (display label, op strings to match) — empty slice = match all
const OP_FILTERS: &[(&str, &[&str])] = &[
    ("All types", &[]),
    ("Copy", &["copy", "duplicate"]),
    ("Move", &["move"]),
    ("Trash", &["trash", "permanent_delete"]),
    ("Rename", &["rename", "bulk_rename"]),
    ("New", &["new_folder", "new_file"]),
    ("Tray", &["holding_tray", "send_to_project"]),
    ("Mark", &["paint_mark", "erase_mark"]),
];

// (display label, lookback seconds) — 0 = no cutoff
const TIME_FILTERS: &[(&str, i64)] = &[
    ("All time", 0),
    ("Last hour", 3_600),
    ("Last 24 h", 86_400),
    ("Last 7 days", 604_800),
    ("Last 30 days", 2_592_000),
];

// (display label, age in seconds; entries older than this are deleted)
const CLEANUP_AGES: &[(&str, i64)] = &[
    ("1 week", 7 * 86_400),
    ("1 month", 30 * 86_400),
    ("3 months", 90 * 86_400),
    ("1 year", 365 * 86_400),
];

struct State {
    all_entries: RefCell<Vec<ActivityLogEntry>>,
    on_action: RefCell<Option<Box<dyn Fn(ActivityLogAction, ActivityLogEntry)>>>,
    on_cleanup: RefCell<Option<Box<dyn Fn(i64)>>>,
    op_filter_idx: Cell<usize>,
    time_filter_idx: Cell<usize>,
    list_box: GtkBox,
    cleanup_bar: GtkBox,
    cleanup_age_dd: DropDown,
}

#[derive(Clone)]
pub struct ActivityLogPanel {
    pub root: GtkBox,
    state: Rc<State>,
}

impl ActivityLogPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("activity-log");
        root.set_vexpand(true);
        root.set_hexpand(true);
        root.set_visible(false);

        // ── Filter toolbar ────────────────────────────────────────────────
        let toolbar = GtkBox::new(Orientation::Horizontal, 6);
        toolbar.add_css_class("activity-log-toolbar");

        let type_label = Label::new(Some("Type:"));
        type_label.add_css_class("activity-log-filter-label");
        toolbar.append(&type_label);

        let op_labels: Vec<&str> = OP_FILTERS.iter().map(|(l, _)| *l).collect();
        let op_dd = DropDown::builder()
            .model(&StringList::new(&op_labels))
            .build();
        op_dd.add_css_class("activity-log-filter-dd");
        toolbar.append(&op_dd);

        let time_label = Label::new(Some("Time:"));
        time_label.add_css_class("activity-log-filter-label");
        toolbar.append(&time_label);

        let time_labels: Vec<&str> = TIME_FILTERS.iter().map(|(l, _)| *l).collect();
        let time_dd = DropDown::builder()
            .model(&StringList::new(&time_labels))
            .build();
        time_dd.add_css_class("activity-log-filter-dd");
        toolbar.append(&time_dd);

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);

        let cleanup_toggle = ToggleButton::with_label("Clean up…");
        cleanup_toggle.add_css_class("activity-log-cleanup-btn");
        toolbar.append(&cleanup_toggle);

        root.append(&toolbar);

        // ── Cleanup bar (hidden until toggled) ────────────────────────────
        let cleanup_bar = GtkBox::new(Orientation::Horizontal, 8);
        cleanup_bar.add_css_class("activity-log-cleanup-bar");
        cleanup_bar.set_visible(false);

        let age_label = Label::new(Some("Delete entries older than:"));
        age_label.add_css_class("activity-log-filter-label");
        cleanup_bar.append(&age_label);

        let age_labels: Vec<&str> = CLEANUP_AGES.iter().map(|(l, _)| *l).collect();
        let cleanup_age_dd = DropDown::builder()
            .model(&StringList::new(&age_labels))
            .build();
        cleanup_age_dd.add_css_class("activity-log-filter-dd");
        cleanup_bar.append(&cleanup_age_dd);

        let delete_btn = Button::with_label("Delete");
        delete_btn.add_css_class("destructive-action");
        delete_btn.add_css_class("activity-log-delete-btn");
        cleanup_bar.append(&delete_btn);

        let cancel_btn = Button::with_label("Cancel");
        cancel_btn.add_css_class("activity-log-cancel-btn");
        cleanup_bar.append(&cancel_btn);

        root.append(&cleanup_bar);

        // ── Entry list ────────────────────────────────────────────────────
        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let list_box = GtkBox::new(Orientation::Vertical, 0);
        list_box.set_vexpand(true);
        scroll.set_child(Some(&list_box));
        root.append(&scroll);

        let state = Rc::new(State {
            all_entries: RefCell::new(Vec::new()),
            on_action: RefCell::new(None),
            on_cleanup: RefCell::new(None),
            op_filter_idx: Cell::new(0),
            time_filter_idx: Cell::new(0),
            list_box,
            cleanup_bar: cleanup_bar.clone(),
            cleanup_age_dd: cleanup_age_dd.clone(),
        });

        // Wire type filter dropdown
        {
            let state = Rc::clone(&state);
            op_dd.connect_selected_notify(move |dd| {
                state.op_filter_idx.set(dd.selected() as usize);
                re_render(&state);
            });
        }

        // Wire time filter dropdown
        {
            let state = Rc::clone(&state);
            time_dd.connect_selected_notify(move |dd| {
                state.time_filter_idx.set(dd.selected() as usize);
                re_render(&state);
            });
        }

        // Wire Clean up… toggle
        {
            let bar = cleanup_bar.clone();
            cleanup_toggle.connect_toggled(move |btn| {
                bar.set_visible(btn.is_active());
            });
        }

        // Wire Delete button
        {
            let state = Rc::clone(&state);
            let toggle = cleanup_toggle.clone();
            delete_btn.connect_clicked(move |_| {
                let age_idx = state.cleanup_age_dd.selected() as usize;
                let age_secs = CLEANUP_AGES.get(age_idx).map(|(_, s)| *s).unwrap_or(0);
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let cutoff_ms = now_ms - age_secs * 1000;
                state.cleanup_bar.set_visible(false);
                toggle.set_active(false);
                if let Some(cb) = state.on_cleanup.borrow().as_ref() {
                    cb(cutoff_ms);
                }
            });
        }

        // Wire Cancel button
        {
            let bar = cleanup_bar.clone();
            cancel_btn.connect_clicked(move |_| {
                bar.set_visible(false);
                cleanup_toggle.set_active(false);
            });
        }

        Self { root, state }
    }

    pub fn populate<F>(&self, entries: &[ActivityLogEntry], on_action: F)
    where
        F: Fn(ActivityLogAction, ActivityLogEntry) + 'static,
    {
        *self.state.all_entries.borrow_mut() = entries.to_vec();
        *self.state.on_action.borrow_mut() = Some(Box::new(on_action));
        re_render(&self.state);
    }

    pub fn connect_cleanup(&self, callback: impl Fn(i64) + 'static) {
        *self.state.on_cleanup.borrow_mut() = Some(Box::new(callback));
    }

}

fn re_render(state: &Rc<State>) {
    clear_list(&state.list_box);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let time_secs = TIME_FILTERS
        .get(state.time_filter_idx.get())
        .map(|(_, s)| *s)
        .unwrap_or(0);
    let cutoff_ms = if time_secs == 0 {
        0
    } else {
        now_ms - time_secs * 1000
    };

    let op_ops = OP_FILTERS
        .get(state.op_filter_idx.get())
        .map(|(_, ops)| *ops)
        .unwrap_or(&[]);

    let entries = state.all_entries.borrow();
    let filtered: Vec<ActivityLogEntry> = entries
        .iter()
        .filter(|e| op_ops.is_empty() || op_ops.contains(&e.operation.as_str()))
        .filter(|e| cutoff_ms == 0 || e.timestamp_ms >= cutoff_ms)
        .cloned()
        .collect();
    drop(entries);

    if filtered.is_empty() {
        let msg = if state.all_entries.borrow().is_empty() {
            "No file operations recorded yet."
        } else {
            "No entries match the current filters."
        };
        let hint = Label::new(Some(msg));
        hint.add_css_class("activity-log-empty");
        hint.set_halign(Align::Center);
        hint.set_valign(Align::Center);
        hint.set_vexpand(true);
        state.list_box.append(&hint);
        return;
    }

    for entry in &filtered {
        let state2 = Rc::clone(state);
        let on_action = move |action: ActivityLogAction, e: ActivityLogEntry| {
            if let Some(cb) = state2.on_action.borrow().as_ref() {
                cb(action, e);
            }
        };
        state.list_box.append(&build_row(entry, on_action));
    }
}

fn clear_list(list_box: &GtkBox) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
}

fn build_row<F>(entry: &ActivityLogEntry, on_action: F) -> GtkBox
where
    F: Fn(ActivityLogAction, ActivityLogEntry) + Clone + 'static,
{
    let row = GtkBox::new(Orientation::Horizontal, 10);
    row.add_css_class("activity-log-row");
    row.set_margin_start(2);
    row.set_margin_end(2);

    let icon = Label::new(Some(op_icon(&entry.operation)));
    icon.add_css_class("activity-log-op-icon");
    icon.set_valign(Align::Center);
    row.append(&icon);

    let text_col = GtkBox::new(Orientation::Vertical, 2);
    text_col.set_hexpand(true);
    text_col.set_valign(Align::Center);

    let summary = Label::new(Some(&entry.summary));
    summary.add_css_class("activity-log-summary");
    summary.set_halign(Align::Start);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_max_width_chars(60);
    text_col.append(&summary);

    let ts = Label::new(Some(&format_relative_time(entry.timestamp_ms)));
    ts.add_css_class("activity-log-timestamp");
    ts.set_halign(Align::Start);
    text_col.append(&ts);

    row.append(&text_col);

    let actions = GtkBox::new(Orientation::Horizontal, 4);
    actions.add_css_class("activity-log-actions");
    actions.set_valign(Align::Center);

    let undo_btn = activity_button("↶", "Undo this operation");
    undo_btn.set_sensitive(can_undo(entry));
    connect_action(&undo_btn, entry, ActivityLogAction::Undo, on_action.clone());
    actions.append(&undo_btn);

    let repeat_btn = activity_button("↻", "Repeat this operation");
    repeat_btn.set_sensitive(can_repeat(entry));
    connect_action(
        &repeat_btn,
        entry,
        ActivityLogAction::Repeat,
        on_action.clone(),
    );
    actions.append(&repeat_btn);

    let reveal_btn = activity_button("⌕", "Reveal related folder");
    reveal_btn.set_sensitive(has_reveal_target(entry));
    connect_action(
        &reveal_btn,
        entry,
        ActivityLogAction::Reveal,
        on_action.clone(),
    );
    actions.append(&reveal_btn);

    let copy_btn = activity_button("⧉", "Copy related path");
    copy_btn.set_sensitive(has_copy_target(entry));
    connect_action(&copy_btn, entry, ActivityLogAction::CopyPath, on_action);
    actions.append(&copy_btn);

    row.append(&actions);

    let dot = Label::new(Some(if entry.status == "success" {
        "●"
    } else {
        "✕"
    }));
    dot.add_css_class(if entry.status == "success" {
        "activity-log-status-ok"
    } else {
        "activity-log-status-fail"
    });
    dot.set_valign(Align::Center);
    row.append(&dot);

    row
}

fn activity_button(label: &str, tooltip: &str) -> Button {
    let button = Button::with_label(label);
    button.add_css_class("activity-log-action-btn");
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(Align::Center);
    button
}

fn connect_action<F>(
    button: &Button,
    entry: &ActivityLogEntry,
    action: ActivityLogAction,
    on_action: F,
) where
    F: Fn(ActivityLogAction, ActivityLogEntry) + 'static,
{
    let entry = entry.clone();
    button.connect_clicked(move |_| on_action(action, entry.clone()));
}

fn can_undo(entry: &ActivityLogEntry) -> bool {
    entry.status == "success"
        && !entry.items.is_empty()
        && matches!(
            entry.operation.as_str(),
            "copy"
                | "move"
                | "trash"
                | "rename"
                | "bulk_rename"
                | "duplicate"
                | "new_folder"
                | "new_file"
        )
}

fn can_repeat(entry: &ActivityLogEntry) -> bool {
    entry.status == "success"
        && !entry.items.is_empty()
        && matches!(
            entry.operation.as_str(),
            "copy" | "move" | "rename" | "bulk_rename" | "duplicate" | "new_folder" | "new_file"
        )
}

fn has_reveal_target(entry: &ActivityLogEntry) -> bool {
    !first_relevant_path(entry).is_empty()
}

fn has_copy_target(entry: &ActivityLogEntry) -> bool {
    !first_relevant_path(entry).is_empty()
}

fn first_relevant_path(entry: &ActivityLogEntry) -> String {
    entry
        .items
        .first()
        .and_then(|item| {
            item.destination_path
                .clone()
                .or_else(|| Some(item.source_path.clone()))
        })
        .or_else(|| entry.destination_path.clone())
        .unwrap_or_else(|| entry.source_path.clone())
}

fn op_icon(operation: &str) -> &'static str {
    match operation {
        "copy" => "📋",
        "move" => "↗",
        "trash" => "🗑",
        "permanent_delete" => "✗",
        "rename" => "✏",
        "bulk_rename" => "✏",
        "duplicate" => "📋",
        "new_folder" => "📁",
        "new_file" => "📄",
        "holding_tray" => "▣",
        "send_to_project" => "🗂",
        "paint_mark" => "🎨",
        "erase_mark" => "◻",
        _ => "·",
    }
}

pub fn format_relative_time(timestamp_ms: i64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let delta_secs = (now_ms - timestamp_ms).max(0) / 1000;

    if delta_secs < 60 {
        return "just now".to_string();
    }
    if delta_secs < 3600 {
        let m = delta_secs / 60;
        return format!("{m} min ago");
    }
    if delta_secs < 86400 {
        let h = delta_secs / 3600;
        return format!("{h} hour{} ago", if h == 1 { "" } else { "s" });
    }
    if delta_secs < 172800 {
        return "Yesterday".to_string();
    }
    let days = delta_secs / 86400;
    format!("{days} days ago")
}
