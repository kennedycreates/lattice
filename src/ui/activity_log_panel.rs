use crate::metadata::ActivityLogEntry;
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Label, Orientation, ScrolledWindow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityLogAction {
    Undo,
    Repeat,
    Reveal,
    CopyPath,
}

#[derive(Clone)]
pub struct ActivityLogPanel {
    pub root: GtkBox,
    list_box: GtkBox,
}

impl ActivityLogPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("activity-log");
        root.set_vexpand(true);
        root.set_hexpand(true);
        root.set_visible(false);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let list_box = GtkBox::new(Orientation::Vertical, 0);
        list_box.set_vexpand(true);
        scroll.set_child(Some(&list_box));
        root.append(&scroll);

        Self { root, list_box }
    }

    pub fn populate<F>(&self, entries: &[ActivityLogEntry], on_action: F)
    where
        F: Fn(ActivityLogAction, ActivityLogEntry) + Clone + 'static,
    {
        self.clear_rows();
        if entries.is_empty() {
            let hint = Label::new(Some("No file operations recorded yet."));
            hint.add_css_class("activity-log-empty");
            hint.set_halign(Align::Center);
            hint.set_valign(Align::Center);
            hint.set_vexpand(true);
            self.list_box.append(&hint);
            return;
        }
        for entry in entries {
            self.list_box.append(&build_row(entry, on_action.clone()));
        }
    }

    pub fn clear_rows(&self) {
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
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
