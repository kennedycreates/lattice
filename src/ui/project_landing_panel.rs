use crate::metadata::{ActivityLogEntry, ProjectDestinationRecord, ProjectRecord};
use crate::ui::activity_log_panel::format_relative_time;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, FlowBox, GestureClick, Label, Orientation, ScrolledWindow,
    Separator,
};
use std::path::PathBuf;

#[derive(Clone)]
pub struct ProjectLandingPanel {
    pub root: GtkBox,
    inner: GtkBox,
}

impl ProjectLandingPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("project-landing");
        root.set_vexpand(true);
        root.set_hexpand(true);
        root.set_visible(false);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();

        let inner = GtkBox::new(Orientation::Vertical, 0);
        inner.set_hexpand(true);
        scroll.set_child(Some(&inner));
        root.append(&scroll);

        Self { root, inner }
    }

    pub fn populate<FNav, FRemoveDest, FTab, FSplit, FLog, FAdd, FSend>(
        &self,
        project: &ProjectRecord,
        destinations: &[ProjectDestinationRecord],
        activity: &[ActivityLogEntry],
        on_navigate: FNav,
        on_remove_destination: FRemoveDest,
        on_open_new_tab: FTab,
        on_open_split: FSplit,
        on_view_log: FLog,
        on_add_destination: FAdd,
        on_send_holding_tray: FSend,
    ) where
        FNav: Fn(PathBuf) + Clone + 'static,
        FRemoveDest: Fn(i64) + Clone + 'static,
        FTab: Fn(PathBuf) + Clone + 'static,
        FSplit: Fn(PathBuf) + Clone + 'static,
        FLog: Fn() + Clone + 'static,
        FAdd: Fn() + Clone + 'static,
        FSend: Fn() + Clone + 'static,
    {
        while let Some(child) = self.inner.first_child() {
            self.inner.remove(&child);
        }

        self.inner.append(&build_header(
            project,
            on_navigate.clone(),
            on_open_new_tab,
            on_open_split,
            on_view_log,
            on_send_holding_tray,
        ));

        let sep1 = Separator::new(Orientation::Horizontal);
        sep1.add_css_class("landing-sep");
        self.inner.append(&sep1);

        self.inner.append(&build_destinations_section(
            project,
            destinations,
            on_navigate,
            on_remove_destination,
            on_add_destination,
        ));

        let sep2 = Separator::new(Orientation::Horizontal);
        sep2.add_css_class("landing-sep");
        self.inner.append(&sep2);

        self.inner.append(&build_activity_section(activity));
    }
}

fn build_header<FNav, FTab, FSplit, FLog, FSend>(
    project: &ProjectRecord,
    on_navigate: FNav,
    on_open_new_tab: FTab,
    on_open_split: FSplit,
    on_view_log: FLog,
    on_send_holding_tray: FSend,
) -> GtkBox
where
    FNav: Fn(PathBuf) + Clone + 'static,
    FTab: Fn(PathBuf) + Clone + 'static,
    FSplit: Fn(PathBuf) + Clone + 'static,
    FLog: Fn() + Clone + 'static,
    FSend: Fn() + Clone + 'static,
{
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("landing-section");
    section.add_css_class("landing-header");

    // Title row: icon + name + path
    let title_row = GtkBox::new(Orientation::Horizontal, 10);
    title_row.set_valign(Align::Center);

    let icon = Label::new(Some("🗂"));
    icon.add_css_class("landing-project-icon");
    icon.set_valign(Align::Center);
    title_row.append(&icon);

    let name_col = GtkBox::new(Orientation::Vertical, 2);
    name_col.set_hexpand(true);
    name_col.set_valign(Align::Center);

    let name_label = Label::new(Some(&project.name));
    name_label.add_css_class("landing-project-name");
    name_label.set_halign(Align::Start);
    name_col.append(&name_label);

    let path_str = project.root_path.to_string_lossy().to_string();
    let path_label = Label::new(Some(&path_str));
    path_label.add_css_class("landing-project-path");
    path_label.set_halign(Align::Start);
    path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    path_label.set_max_width_chars(60);
    name_col.append(&path_label);

    title_row.append(&name_col);
    section.append(&title_row);

    // Actions bar
    let actions = GtkBox::new(Orientation::Horizontal, 6);
    actions.add_css_class("landing-actions");

    let root_path = project.root_path.clone();
    let open_btn = landing_action_btn("Open Folder");
    open_btn.connect_clicked(move |_| on_navigate(root_path.clone()));
    crate::ui::attach_tooltip(&open_btn, "Browse the project root folder");
    actions.append(&open_btn);

    let root_path = project.root_path.clone();
    let tab_btn = landing_action_btn("New Tab");
    tab_btn.connect_clicked(move |_| on_open_new_tab(root_path.clone()));
    crate::ui::attach_tooltip(&tab_btn, "Open root folder in a new tab");
    actions.append(&tab_btn);

    let root_path = project.root_path.clone();
    let split_btn = landing_action_btn("Split Pane");
    split_btn.connect_clicked(move |_| on_open_split(root_path.clone()));
    crate::ui::attach_tooltip(&split_btn, "Open root folder in a split pane");
    actions.append(&split_btn);

    let log_btn = landing_action_btn("Activity Log");
    log_btn.connect_clicked(move |_| on_view_log());
    crate::ui::attach_tooltip(&log_btn, "View full activity log");
    actions.append(&log_btn);

    let send_btn = landing_action_btn("Send Tray Here");
    send_btn.connect_clicked(move |_| on_send_holding_tray());
    crate::ui::attach_tooltip(&send_btn, "Copy holding tray items to this project");
    actions.append(&send_btn);

    section.append(&actions);
    section
}

fn build_destinations_section<FNav, FRemove, FAdd>(
    project: &ProjectRecord,
    destinations: &[ProjectDestinationRecord],
    on_navigate: FNav,
    on_remove_destination: FRemove,
    on_add_destination: FAdd,
) -> GtkBox
where
    FNav: Fn(PathBuf) + Clone + 'static,
    FRemove: Fn(i64) + Clone + 'static,
    FAdd: Fn() + Clone + 'static,
{
    let section = GtkBox::new(Orientation::Vertical, 10);
    section.add_css_class("landing-section");

    // Heading row with "+ Add" button
    let heading_row = GtkBox::new(Orientation::Horizontal, 0);
    let heading = Label::new(Some("DESTINATIONS"));
    heading.add_css_class("landing-section-heading");
    heading.set_halign(Align::Start);
    heading.set_hexpand(true);
    heading_row.append(&heading);

    let add_btn = Button::with_label("+ Add");
    add_btn.add_css_class("landing-add-btn");
    add_btn.set_valign(Align::Center);
    add_btn.connect_clicked(move |_| on_add_destination());
    crate::ui::attach_tooltip(&add_btn, "Add a subfolder as a destination");
    heading_row.append(&add_btn);

    section.append(&heading_row);

    if destinations.is_empty() {
        let empty = Label::new(Some("No destinations yet. Click + Add to pin a subfolder."));
        empty.add_css_class("landing-dest-empty");
        empty.set_halign(Align::Start);
        section.append(&empty);
        return section;
    }

    let flow = FlowBox::new();
    flow.add_css_class("landing-destinations");
    flow.set_homogeneous(false);
    flow.set_column_spacing(10);
    flow.set_row_spacing(10);
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_halign(Align::Start);

    for dest in destinations {
        let dest_path = project.root_path.join(&dest.relative_path);
        let card = build_dest_card(
            dest.id,
            &dest.name,
            dest_path,
            dest.relative_path.is_empty(),
            on_navigate.clone(),
            on_remove_destination.clone(),
        );
        flow.append(&card);
    }

    section.append(&flow);
    section
}

fn build_dest_card<FNav, FRemove>(
    dest_id: i64,
    name: &str,
    path: PathBuf,
    is_root: bool,
    on_navigate: FNav,
    on_remove: FRemove,
) -> GtkBox
where
    FNav: Fn(PathBuf) + 'static,
    FRemove: Fn(i64) + 'static,
{
    let outer = GtkBox::new(Orientation::Horizontal, 0);

    let card = GtkBox::new(Orientation::Vertical, 4);
    card.add_css_class("landing-dest-card");
    card.set_valign(Align::Start);

    let icon = Label::new(Some("📁"));
    icon.add_css_class("landing-dest-icon");
    icon.set_halign(Align::Center);
    card.append(&icon);

    let name_label = Label::new(Some(name));
    name_label.add_css_class("landing-dest-name");
    name_label.set_halign(Align::Center);
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name_label.set_max_width_chars(14);
    card.append(&name_label);

    let gesture = GestureClick::new();
    gesture.set_button(1);
    let nav_path = path.clone();
    gesture.connect_pressed(move |_, n, _, _| {
        if n == 1 {
            on_navigate(nav_path.clone());
        }
    });
    card.add_controller(gesture);
    outer.append(&card);

    if !is_root {
        let remove_btn = Button::new();
        remove_btn.add_css_class("landing-dest-remove");
        let x_icon = Label::new(Some("×"));
        remove_btn.set_child(Some(&x_icon));
        remove_btn.set_valign(Align::Start);
        remove_btn.connect_clicked(move |_| on_remove(dest_id));
        crate::ui::attach_tooltip(&remove_btn, "Remove this destination");
        outer.append(&remove_btn);
    }

    outer
}

fn build_activity_section(activity: &[ActivityLogEntry]) -> GtkBox {
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("landing-section");

    let heading = Label::new(Some("RECENT ACTIVITY"));
    heading.add_css_class("landing-section-heading");
    heading.set_halign(Align::Start);
    section.append(&heading);

    if activity.is_empty() {
        let empty = Label::new(Some("No recent activity for this project."));
        empty.add_css_class("landing-empty");
        empty.set_halign(Align::Start);
        section.append(&empty);
        return section;
    }

    for entry in activity {
        let row = GtkBox::new(Orientation::Horizontal, 10);
        row.add_css_class("landing-activity-row");

        let op_icon = Label::new(Some(op_icon(&entry.operation)));
        op_icon.add_css_class("landing-activity-op");
        op_icon.set_valign(Align::Center);
        row.append(&op_icon);

        let text_col = GtkBox::new(Orientation::Vertical, 1);
        text_col.set_hexpand(true);
        text_col.set_valign(Align::Center);

        let summary = Label::new(Some(&entry.summary));
        summary.add_css_class("landing-activity-summary");
        summary.set_halign(Align::Start);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.set_max_width_chars(70);
        text_col.append(&summary);

        let ts = Label::new(Some(&format_relative_time(entry.timestamp_ms)));
        ts.add_css_class("landing-activity-time");
        ts.set_halign(Align::Start);
        text_col.append(&ts);

        row.append(&text_col);

        let status_dot = Label::new(Some(if entry.status == "success" {
            "●"
        } else {
            "✕"
        }));
        status_dot.add_css_class("landing-activity-status");
        if entry.status != "success" {
            status_dot.add_css_class("landing-activity-status-fail");
        }
        status_dot.set_valign(Align::Center);
        row.append(&status_dot);

        section.append(&row);
    }

    section
}

fn landing_action_btn(label: &str) -> Button {
    let btn = Button::with_label(label);
    btn.add_css_class("landing-action-btn");
    btn
}

fn op_icon(operation: &str) -> &'static str {
    match operation {
        "copy" => "📋",
        "move" => "↗",
        "trash" => "🗑",
        "permanent_delete" => "✗",
        "rename" | "bulk_rename" => "✏",
        "duplicate" => "📋",
        "new_folder" => "📁",
        "new_file" => "📄",
        "holding_tray" => "▣",
        "send_to_project" => "🗂",
        _ => "·",
    }
}
