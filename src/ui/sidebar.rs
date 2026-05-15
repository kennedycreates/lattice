use gtk::prelude::*;
use gtk::{Box, Button, Label, Orientation, ScrolledWindow, Separator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarLocation {
    Home,
    Downloads,
    Documents,
    Projects,
}

#[derive(Clone)]
pub struct Sidebar {
    pub root: ScrolledWindow,
    pub home_button: Button,
    pub downloads_button: Button,
    pub documents_button: Button,
    pub projects_button: Button,
}

impl Sidebar {
    pub fn build(projects_enabled: bool) -> Self {
        let root = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        root.add_css_class("sidebar");

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

        let projects_button = section_button("🗂  Projects", projects_enabled);
        let tags_button = section_button("🏷  Tags", false);
        append_section(
            &vbox,
            "WORKSPACE",
            [&projects_button, &tags_button].as_slice(),
        );

        let drives_button = section_button("💾  Drives", false);
        let recent_button = section_button("🕐  Recent", false);
        append_section(&vbox, "SYSTEM", [&drives_button, &recent_button].as_slice());

        root.set_child(Some(&vbox));

        Self {
            root,
            home_button,
            downloads_button,
            documents_button,
            projects_button,
        }
    }

    pub fn set_active(&self, active: Option<SidebarLocation>) {
        for (button, location) in [
            (&self.home_button, SidebarLocation::Home),
            (&self.downloads_button, SidebarLocation::Downloads),
            (&self.documents_button, SidebarLocation::Documents),
            (&self.projects_button, SidebarLocation::Projects),
        ] {
            if Some(location) == active {
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

    let heading = Label::new(Some(heading_text));
    heading.add_css_class("sidebar-section-heading");
    heading.set_halign(gtk::Align::Start);
    heading.set_margin_start(12);
    heading.set_margin_top(12);
    heading.set_margin_bottom(4);
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
    button.set_child(Some(&text));

    button
}
