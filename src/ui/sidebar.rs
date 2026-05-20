use crate::metadata::{CloudRecord, PlaceRecord};
use gtk::prelude::*;
use gtk::{Box, Button, Label, Orientation, Revealer, ScrolledWindow};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarTarget {
    Home,
    Place(i64),
    Cloud(i64),
    Search,
    BulkNaming,
    SpaceViewer,
    Triage,
    ActivityLog,
    Tags,
    Projects,
    SystemDrives,
    Recent,
    Trash,
    Convert,
}

#[derive(Clone)]
pub struct Sidebar {
    pub root: ScrolledWindow,
    pub home_button: Button,
    pub search_button: Button,
    pub bulk_naming_button: Button,
    pub space_viewer_button: Button,
    pub triage_button: Button,
    pub activity_log_button: Button,
    pub tags_button: Button,
    pub projects_button: Button,
    pub drives_button: Button,
    pub recent_button: Button,
    pub trash_button: Button,
    pub convert_button: Button,
    place_list: Box,
    place_buttons: RefCell<Vec<(PlaceRecord, Button)>>,
    cloud_list: Box,
    pub cloud_add_button: Button,
    pub rclone_setup_button: Button,
    cloud_buttons: RefCell<Vec<(CloudRecord, Button)>>,
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
        let (places_hdr, places_content_box, places_revealer) =
            collapsible_section_header("PLACES");
        places_content_box.append(&home_button);
        let place_list = Box::new(Orientation::Vertical, 0);
        place_list.add_css_class("sidebar-dynamic-list");
        places_content_box.append(&place_list);
        places_section.append(&places_hdr);
        places_section.append(&places_revealer);
        vbox.append(&places_section);

        let drives_button = section_button("💾  System Drives", true);
        let recent_button = section_button("🕐  Recent", true);
        let trash_button = section_button("🗑  Trash", true);
        append_section(
            &vbox,
            "SYSTEM",
            [&drives_button, &recent_button, &trash_button].as_slice(),
        );

        // CLOUD section (dynamic, between SYSTEM and TOOLS)
        let cloud_section = Box::new(Orientation::Vertical, 0);
        cloud_section.add_css_class("sidebar-section");
        let (cloud_hdr, cloud_content_box, cloud_revealer) = collapsible_section_header("CLOUD");
        let cloud_list = Box::new(Orientation::Vertical, 0);
        cloud_list.add_css_class("sidebar-dynamic-list");
        cloud_content_box.append(&cloud_list);
        let cloud_add_button = section_button("☁  Add Cloud Drive", true);
        cloud_content_box.append(&cloud_add_button);
        let rclone_setup_button = section_button("⚙  rclone Remotes", true);
        cloud_content_box.append(&rclone_setup_button);
        cloud_section.append(&cloud_hdr);
        cloud_section.append(&cloud_revealer);
        vbox.append(&cloud_section);

        let search_button = section_button("🔍  Search", true);
        let bulk_naming_button = section_button("✏  Bulk Naming", true);
        let space_viewer_button = section_button("📊  Space Viewer", true);
        let triage_button = section_button("🧹  Triage", true);
        let activity_log_button = section_button("📋  Activity Log", true);
        let tags_button = section_button("🎨  Tints & Tags", true);
        let projects_button = section_button("🗂  Palettes", true);
        let convert_button = section_button("🔄  Convert Media", true);
        append_section(
            &vbox,
            "TOOLS",
            [
                &projects_button,
                &tags_button,
                &search_button,
                &space_viewer_button,
                &triage_button,
                &bulk_naming_button,
                &convert_button,
                &activity_log_button,
            ]
            .as_slice(),
        );

        root.set_child(Some(&vbox));

        Self {
            root,
            home_button,
            search_button,
            bulk_naming_button,
            space_viewer_button,
            triage_button,
            activity_log_button,
            tags_button,
            projects_button,
            drives_button,
            recent_button,
            trash_button,
            convert_button,
            place_list,
            place_buttons: RefCell::new(Vec::new()),
            cloud_list,
            cloud_add_button,
            rclone_setup_button,
            cloud_buttons: RefCell::new(Vec::new()),
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

    pub fn place_buttons(&self) -> Vec<(PlaceRecord, Button)> {
        self.place_buttons.borrow().clone()
    }

    pub fn set_cloud_locations(&self, locations: &[CloudRecord]) {
        clear_box(&self.cloud_list);
        let mut buttons = Vec::with_capacity(locations.len());
        if locations.is_empty() {
            self.cloud_list
                .append(&section_note("Add mounted cloud drives here."));
        }

        for loc in locations {
            let button = dynamic_button("☁", &loc.name);
            self.cloud_list.append(&button);
            buttons.push((loc.clone(), button));
        }
        self.cloud_buttons.replace(buttons);
    }

    pub fn cloud_buttons(&self) -> Vec<(CloudRecord, Button)> {
        self.cloud_buttons.borrow().clone()
    }

    pub fn set_active(&self, active: Option<&SidebarTarget>) {
        for (button, location) in [
            (&self.home_button, SidebarTarget::Home),
            (&self.search_button, SidebarTarget::Search),
            (&self.bulk_naming_button, SidebarTarget::BulkNaming),
            (&self.space_viewer_button, SidebarTarget::SpaceViewer),
            (&self.triage_button, SidebarTarget::Triage),
            (&self.activity_log_button, SidebarTarget::ActivityLog),
            (&self.tags_button, SidebarTarget::Tags),
            (&self.projects_button, SidebarTarget::Projects),
            (&self.drives_button, SidebarTarget::SystemDrives),
            (&self.recent_button, SidebarTarget::Recent),
            (&self.trash_button, SidebarTarget::Trash),
            (&self.convert_button, SidebarTarget::Convert),
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

        for (loc, button) in self.cloud_buttons.borrow().iter() {
            if active == Some(&SidebarTarget::Cloud(loc.id)) {
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

    let (hdr, content_box, revealer) = collapsible_section_header(heading_text);

    for button in buttons {
        content_box.append(*button);
    }

    section_box.append(&hdr);
    section_box.append(&revealer);
    vbox.append(&section_box);
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

fn collapsible_section_header(heading_text: &str) -> (Button, Box, Revealer) {
    let btn = Button::new();
    btn.add_css_class("sidebar-section-toggle");
    btn.set_halign(gtk::Align::Fill);
    btn.set_focus_on_click(false);

    let row = Box::new(Orientation::Horizontal, 0);
    let arrow = Label::new(Some("▾"));
    arrow.add_css_class("sidebar-section-arrow");
    let title = Label::new(Some(heading_text));
    title.add_css_class("sidebar-section-heading");
    title.set_halign(gtk::Align::Start);
    title.set_hexpand(true);
    row.append(&arrow);
    row.append(&title);
    btn.set_child(Some(&row));

    let content_box = Box::new(Orientation::Vertical, 0);
    content_box.add_css_class("sidebar-section-content");
    let revealer = Revealer::new();
    revealer.add_css_class("sidebar-section-revealer");
    revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    revealer.set_transition_duration(150);
    revealer.set_reveal_child(true);
    revealer.set_child(Some(&content_box));

    let revealer_c = revealer.clone();
    let arrow_c = arrow.clone();
    btn.connect_clicked(move |_| {
        let expanding = !revealer_c.reveals_child();
        revealer_c.set_reveal_child(expanding);
        arrow_c.set_label(if expanding { "▾" } else { "▸" });
    });

    (btn, content_box, revealer)
}

fn dynamic_button(prefix: &str, label: &str) -> Button {
    section_button(&format!("{prefix}  {label}"), true)
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
