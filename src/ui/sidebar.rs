use crate::metadata::{PlaceRecord, ProjectRecord};
use gtk::prelude::*;
use gtk::{Box, Button, Label, Orientation, ScrolledWindow, Separator};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarTarget {
    Home,
    Place(i64),
    Search,
    Triage,
    ActivityLog,
    Tags,
    SystemDrives,
    Recent,
    Trash,
    Project(i64),
}

#[derive(Clone)]
pub struct Sidebar {
    pub root: ScrolledWindow,
    pub home_button: Button,
    pub search_button: Button,
    pub triage_button: Button,
    pub activity_log_button: Button,
    pub tags_button: Button,
    pub drives_button: Button,
    pub recent_button: Button,
    pub trash_button: Button,
    place_list: Box,
    project_list: Box,
    place_buttons: RefCell<Vec<(PlaceRecord, Button)>>,
    project_buttons: RefCell<Vec<(i64, Button)>>,
}

impl Sidebar {
    pub fn build() -> Self {
        let root = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        root.add_css_class("sidebar");
        root.set_min_content_width(188);
        root.set_propagate_natural_width(false);

        let vbox = Box::new(Orientation::Vertical, 0);
        vbox.set_margin_top(8);
        vbox.set_margin_bottom(8);

        let home_button = section_button("🏠  Home", true);
        let places_section = Box::new(Orientation::Vertical, 0);
        places_section.add_css_class("sidebar-section");
        places_section.append(&section_heading("PLACES"));
        places_section.append(&home_button);
        let place_list = Box::new(Orientation::Vertical, 0);
        place_list.add_css_class("sidebar-dynamic-list");
        places_section.append(&place_list);
        vbox.append(&places_section);
        let sep = Separator::new(Orientation::Horizontal);
        sep.add_css_class("sidebar-sep");
        vbox.append(&sep);

        let search_button = section_button("🔍  Search", true);
        let triage_button = section_button("🧹  Triage", true);
        let activity_log_button = section_button("📋  Activity Log", true);
        let tags_button = section_button("🏷  Tags", true);
        append_section(
            &vbox,
            "TOOLS",
            [&search_button, &triage_button, &activity_log_button, &tags_button].as_slice(),
        );

        let workspace_section = Box::new(Orientation::Vertical, 0);
        workspace_section.add_css_class("sidebar-section");

        let projects_heading = section_heading("PROJECTS");
        workspace_section.append(&projects_heading);
        let project_list = Box::new(Orientation::Vertical, 0);
        project_list.add_css_class("sidebar-dynamic-list");
        workspace_section.append(&project_list);

        vbox.append(&workspace_section);
        let sep = Separator::new(Orientation::Horizontal);
        sep.add_css_class("sidebar-sep");
        vbox.append(&sep);

        let drives_button = section_button("💾  System Drives", true);
        let recent_button = section_button("🕐  Recent", true);
        let trash_button = section_button("🗑  Trash", true);
        append_section(
            &vbox,
            "SYSTEM",
            [&drives_button, &recent_button, &trash_button].as_slice(),
        );

        root.set_child(Some(&vbox));

        Self {
            root,
            home_button,
            search_button,
            triage_button,
            activity_log_button,
            tags_button,
            drives_button,
            recent_button,
            trash_button,
            place_list,
            project_list,
            place_buttons: RefCell::new(Vec::new()),
            project_buttons: RefCell::new(Vec::new()),
        }
    }

    pub fn set_places(&self, places: &[PlaceRecord]) {
        clear_box(&self.place_list);
        let mut buttons = Vec::with_capacity(places.len());
        if places.is_empty() {
            self.place_list
                .append(&section_note("Pin folders here for quick access."));
        }

        for place in places {
            let button = dynamic_button("📁", &place.name);
            self.place_list.append(&button);
            buttons.push((place.clone(), button));
        }
        self.place_buttons.replace(buttons);
    }

    pub fn set_projects(&self, projects: &[ProjectRecord]) {
        clear_box(&self.project_list);
        let mut buttons = Vec::with_capacity(projects.len());
        if projects.is_empty() {
            self.project_list
                .append(&section_note("Pin a folder to add it here."));
        }

        for project in projects {
            let button = dynamic_button("📁", &project.name);
            self.project_list.append(&button);
            buttons.push((project.id, button));
        }
        self.project_buttons.replace(buttons);
    }

    pub fn project_buttons(&self) -> Vec<(i64, Button)> {
        self.project_buttons.borrow().clone()
    }

    pub fn place_buttons(&self) -> Vec<(PlaceRecord, Button)> {
        self.place_buttons.borrow().clone()
    }

    pub fn set_active(&self, active: Option<&SidebarTarget>) {
        for (button, location) in [
            (&self.home_button, SidebarTarget::Home),
            (&self.search_button, SidebarTarget::Search),
            (&self.triage_button, SidebarTarget::Triage),
            (&self.activity_log_button, SidebarTarget::ActivityLog),
            (&self.tags_button, SidebarTarget::Tags),
            (&self.drives_button, SidebarTarget::SystemDrives),
            (&self.recent_button, SidebarTarget::Recent),
            (&self.trash_button, SidebarTarget::Trash),
        ] {
            if active == Some(&location) {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }

        for (place, button) in self.place_buttons.borrow().iter() {
            if active == Some(&SidebarTarget::Place(place.id)) {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }

        for (project_id, button) in self.project_buttons.borrow().iter() {
            if active == Some(&SidebarTarget::Project(*project_id)) {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }
    }
}

fn append_section(vbox: &Box, heading_text: &str, buttons: &[&Button]) {
    let section_box = Box::new(Orientation::Vertical, 0);
    section_box.add_css_class("sidebar-section");

    let heading = section_heading(heading_text);
    section_box.append(&heading);

    for button in buttons {
        section_box.append(*button);
    }

    vbox.append(&section_box);

    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("sidebar-sep");
    vbox.append(&sep);
}

fn section_button(label: &str, sensitive: bool) -> Button {
    let button = Button::new();
    button.add_css_class("sidebar-button");
    button.set_halign(gtk::Align::Fill);
    button.set_sensitive(sensitive);

    let text = Label::new(Some(label));
    text.set_halign(gtk::Align::Start);
    text.set_hexpand(true);
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.set_wrap(false);
    text.set_single_line_mode(true);
    button.set_child(Some(&text));

    button
}

fn dynamic_button(prefix: &str, label: &str) -> Button {
    section_button(&format!("{prefix}  {label}"), true)
}

fn section_heading(text: &str) -> Label {
    let heading = Label::new(Some(text));
    heading.add_css_class("sidebar-section-heading");
    heading.set_halign(gtk::Align::Start);
    heading.set_margin_start(12);
    heading.set_margin_top(12);
    heading.set_margin_bottom(4);
    heading
}

fn section_note(text: &str) -> Label {
    let note = Label::new(Some(text));
    note.add_css_class("sidebar-note");
    note.set_halign(gtk::Align::Start);
    note.set_wrap(true);
    note.set_margin_start(16);
    note.set_margin_end(14);
    note.set_margin_bottom(8);
    note
}

fn clear_box(container: &Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
