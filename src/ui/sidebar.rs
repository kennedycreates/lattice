use crate::metadata::{ProjectRecord, TagRecord};
use gtk::prelude::*;
use gtk::{Box, Button, Label, Orientation, ScrolledWindow, Separator};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarTarget {
    Home,
    Downloads,
    Documents,
    DownloadsTriage,
    SystemDrives,
    Recent,
    Trash,
    Project(i64),
    Tag(i64),
}

#[derive(Clone)]
pub struct Sidebar {
    pub root: ScrolledWindow,
    pub home_button: Button,
    pub downloads_button: Button,
    pub documents_button: Button,
    pub downloads_triage_button: Button,
    pub drives_button: Button,
    pub recent_button: Button,
    pub trash_button: Button,
    project_list: Box,
    tag_list: Box,
    project_buttons: RefCell<Vec<(i64, Button)>>,
    tag_buttons: RefCell<Vec<(i64, Button)>>,
}

impl Sidebar {
    pub fn build() -> Self {
        let root = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        root.add_css_class("sidebar");
        root.set_min_content_width(188);
        root.set_propagate_natural_width(true);

        let vbox = Box::new(Orientation::Vertical, 0);
        vbox.set_margin_top(8);
        vbox.set_margin_bottom(8);

        let home_button = section_button("🏠  Home", true);
        let downloads_button = section_button("⬇  Downloads", true);
        let documents_button = section_button("📄  Documents", true);
        append_section(
            &vbox,
            "PLACES",
            [&home_button, &downloads_button, &documents_button].as_slice(),
        );

        let downloads_triage_button = section_button("🧹  Downloads Triage", true);
        append_section(&vbox, "TOOLS", [&downloads_triage_button].as_slice());

        let workspace_section = Box::new(Orientation::Vertical, 0);
        workspace_section.add_css_class("sidebar-section");

        let projects_heading = section_heading("PROJECTS");
        workspace_section.append(&projects_heading);
        let project_list = Box::new(Orientation::Vertical, 0);
        project_list.add_css_class("sidebar-dynamic-list");
        workspace_section.append(&project_list);

        let tags_heading = section_heading("TAGS");
        workspace_section.append(&tags_heading);
        let tag_list = Box::new(Orientation::Vertical, 0);
        tag_list.add_css_class("sidebar-dynamic-list");
        workspace_section.append(&tag_list);

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
            downloads_button,
            documents_button,
            downloads_triage_button,
            drives_button,
            recent_button,
            trash_button,
            project_list,
            tag_list,
            project_buttons: RefCell::new(Vec::new()),
            tag_buttons: RefCell::new(Vec::new()),
        }
    }

    pub fn set_projects(&self, projects: &[ProjectRecord]) {
        clear_box(&self.project_list);
        let mut buttons = Vec::with_capacity(projects.len());
        if projects.is_empty() {
            self.project_list
                .append(&section_note("Pin a folder to add it here."));
        }

        for project in projects {
            let button = dynamic_button("🗂", &project.name);
            self.project_list.append(&button);
            buttons.push((project.id, button));
        }
        self.project_buttons.replace(buttons);
    }

    pub fn set_tags(&self, tags: &[TagRecord]) {
        clear_box(&self.tag_list);
        let mut buttons = Vec::with_capacity(tags.len());
        if tags.is_empty() {
            self.tag_list
                .append(&section_note("Tags appear after you create them."));
        }

        for tag in tags {
            let button = dynamic_button("#", &tag.name);
            self.tag_list.append(&button);
            buttons.push((tag.id, button));
        }
        self.tag_buttons.replace(buttons);
    }

    pub fn project_buttons(&self) -> Vec<(i64, Button)> {
        self.project_buttons.borrow().clone()
    }

    pub fn tag_buttons(&self) -> Vec<(i64, Button)> {
        self.tag_buttons.borrow().clone()
    }

    pub fn set_active(&self, active: Option<&SidebarTarget>) {
        for (button, location) in [
            (&self.home_button, SidebarTarget::Home),
            (&self.downloads_button, SidebarTarget::Downloads),
            (&self.documents_button, SidebarTarget::Documents),
            (
                &self.downloads_triage_button,
                SidebarTarget::DownloadsTriage,
            ),
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

        for (project_id, button) in self.project_buttons.borrow().iter() {
            if active == Some(&SidebarTarget::Project(*project_id)) {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }

        for (tag_id, button) in self.tag_buttons.borrow().iter() {
            if active == Some(&SidebarTarget::Tag(*tag_id)) {
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
