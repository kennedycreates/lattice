use crate::action_plan::{ActionPlan, OpKind};
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Label, Orientation, Revealer, ScrolledWindow};

pub enum QueueAction {
    MoveUp(usize),
    MoveDown(usize),
    Remove(usize),
}

#[derive(Clone)]
pub struct PlanQueuePanel {
    pub root: Revealer,
    pub execute_btn: Button,
    pub clear_btn: Button,
    items_box: GtkBox,
    count_label: Label,
}

impl PlanQueuePanel {
    pub fn build() -> Self {
        let panel = GtkBox::new(Orientation::Vertical, 0);
        panel.add_css_class("plan-queue");

        let header = GtkBox::new(Orientation::Horizontal, 8);
        header.add_css_class("plan-queue-header");
        header.set_margin_bottom(4);

        let mode_badge = Label::new(Some("PLAN MODE"));
        mode_badge.add_css_class("plan-queue-mode-badge");
        mode_badge.set_valign(Align::Center);
        header.append(&mode_badge);

        let count_label = Label::new(Some("No actions queued"));
        count_label.add_css_class("plan-queue-count");
        count_label.set_hexpand(true);
        count_label.set_halign(Align::Start);
        count_label.set_valign(Align::Center);
        header.append(&count_label);

        let execute_btn = Button::with_label("▶  Execute All");
        execute_btn.add_css_class("toolbar-action-btn");
        execute_btn.add_css_class("plan-queue-execute-btn");
        execute_btn.set_sensitive(false);
        header.append(&execute_btn);

        let clear_btn = Button::with_label("✕  Clear");
        clear_btn.add_css_class("toolbar-action-btn");
        clear_btn.set_sensitive(false);
        header.append(&clear_btn);

        panel.append(&header);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(160)
            .propagate_natural_height(true)
            .build();

        let items_box = GtkBox::new(Orientation::Vertical, 0);
        items_box.add_css_class("plan-queue-list");
        scroll.set_child(Some(&items_box));
        panel.append(&scroll);

        let root = Revealer::new();
        root.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        root.set_transition_duration(220);
        root.set_child(Some(&panel));
        root.set_reveal_child(false);
        root.set_visible(false);

        Self {
            root,
            execute_btn,
            clear_btn,
            items_box,
            count_label,
        }
    }

    /// Rebuild the queue item rows. Called every time the queue changes.
    pub fn set_items<F>(&self, items: &[ActionPlan], on_action: F)
    where
        F: Fn(QueueAction) + Clone + 'static,
    {
        // Clear existing rows
        while let Some(child) = self.items_box.first_child() {
            self.items_box.remove(&child);
        }

        let n = items.len();
        let has_items = n > 0;

        if has_items {
            self.count_label.set_label(&format!(
                "{n} action{} queued",
                if n == 1 { "" } else { "s" }
            ));
        } else {
            self.count_label.set_label("No actions queued");
        }
        self.execute_btn.set_sensitive(has_items);
        self.clear_btn.set_sensitive(has_items);

        for (index, item) in items.iter().enumerate() {
            let row = build_row(item, index, n, on_action.clone());
            self.items_box.append(&row);
        }
    }
}

fn kind_icon(kind: &OpKind) -> &'static str {
    match kind {
        OpKind::Copy => "📋",
        OpKind::Move => "↗",
        OpKind::Trash => "🗑",
        OpKind::PermanentDelete => "✗",
        OpKind::Rename => "✏",
        OpKind::BulkRename => "✏",
        OpKind::Duplicate => "📋",
        OpKind::NewFolder => "📁",
        OpKind::NewFile => "📄",
        OpKind::SendToProject { .. } => "🗂",
    }
}

fn build_row<F: Fn(QueueAction) + Clone + 'static>(
    item: &ActionPlan,
    index: usize,
    total: usize,
    on_action: F,
) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.add_css_class("plan-queue-row");
    row.set_margin_start(2);
    row.set_margin_end(4);

    // Reorder buttons
    let up_btn = Button::with_label("↑");
    up_btn.add_css_class("pq-reorder-btn");
    up_btn.set_sensitive(index > 0);
    up_btn.set_valign(Align::Center);
    let cb = on_action.clone();
    up_btn.connect_clicked(move |_| cb(QueueAction::MoveUp(index)));
    row.append(&up_btn);

    let down_btn = Button::with_label("↓");
    down_btn.add_css_class("pq-reorder-btn");
    down_btn.set_sensitive(index + 1 < total);
    down_btn.set_valign(Align::Center);
    let cb = on_action.clone();
    down_btn.connect_clicked(move |_| cb(QueueAction::MoveDown(index)));
    row.append(&down_btn);

    // Op icon
    let icon = Label::new(Some(kind_icon(&item.kind)));
    icon.add_css_class("activity-log-op-icon");
    icon.set_valign(Align::Center);
    row.append(&icon);

    // Summary
    let summary = Label::new(Some(&item.summary));
    summary.add_css_class("plan-queue-summary");
    summary.set_hexpand(true);
    summary.set_halign(Align::Start);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_max_width_chars(60);
    summary.set_valign(Align::Center);
    row.append(&summary);

    // Remove button
    let remove_btn = Button::with_label("✕");
    remove_btn.add_css_class("pq-remove-btn");
    remove_btn.set_valign(Align::Center);
    let cb = on_action.clone();
    remove_btn.connect_clicked(move |_| cb(QueueAction::Remove(index)));
    row.append(&remove_btn);

    row
}
