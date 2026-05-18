use crate::metadata::{ProjectDestinationRecord, ProjectRecord};
use crate::ui::tag_panel::tag_color_class;
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

    pub fn populate<FBack, FNav, FRemove, FPin>(
        &self,
        project: &ProjectRecord,
        destinations: &[ProjectDestinationRecord],
        on_back: FBack,
        on_navigate: FNav,
        on_remove_pin: FRemove,
        on_pin_folder: FPin,
    ) where
        FBack: Fn() + 'static,
        FNav: Fn(PathBuf) + Clone + 'static,
        FRemove: Fn(i64) + Clone + 'static,
        FPin: Fn() + 'static,
    {
        while let Some(child) = self.inner.first_child() {
            self.inner.remove(&child);
        }

        self.inner.append(&build_header(project, on_back));

        let sep = Separator::new(Orientation::Horizontal);
        sep.add_css_class("landing-sep");
        self.inner.append(&sep);

        self.inner.append(&build_pins_section(
            destinations,
            on_navigate,
            on_remove_pin,
            on_pin_folder,
        ));
    }
}

fn build_header<FBack>(project: &ProjectRecord, on_back: FBack) -> GtkBox
where
    FBack: Fn() + 'static,
{
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("landing-section");
    section.add_css_class("landing-header");

    // Back breadcrumb
    let back_btn = Button::with_label("← Projects");
    back_btn.add_css_class("landing-back-btn");
    back_btn.set_halign(Align::Start);
    back_btn.connect_clicked(move |_| on_back());
    section.append(&back_btn);

    // Title row: color dot + name
    let title_row = GtkBox::new(Orientation::Horizontal, 10);
    title_row.set_valign(Align::Center);

    let dot = Label::new(Some("●"));
    dot.add_css_class("landing-project-dot");
    dot.add_css_class(tag_color_class(project.color.as_deref()));
    dot.set_valign(Align::Center);
    title_row.append(&dot);

    let name_label = Label::new(Some(&project.name));
    name_label.add_css_class("landing-project-name");
    name_label.set_halign(Align::Start);
    name_label.set_hexpand(true);
    title_row.append(&name_label);

    section.append(&title_row);
    section
}

fn build_pins_section<FNav, FRemove, FPin>(
    destinations: &[ProjectDestinationRecord],
    on_navigate: FNav,
    on_remove_pin: FRemove,
    on_pin_folder: FPin,
) -> GtkBox
where
    FNav: Fn(PathBuf) + Clone + 'static,
    FRemove: Fn(i64) + Clone + 'static,
    FPin: Fn() + 'static,
{
    let section = GtkBox::new(Orientation::Vertical, 10);
    section.add_css_class("landing-section");

    let heading_row = GtkBox::new(Orientation::Horizontal, 0);
    let heading = Label::new(Some("PINNED FOLDERS"));
    heading.add_css_class("landing-section-heading");
    heading.set_halign(Align::Start);
    heading.set_hexpand(true);
    heading_row.append(&heading);

    let pin_btn = Button::with_label("+ Pin Folder");
    pin_btn.add_css_class("landing-add-btn");
    pin_btn.set_valign(Align::Center);
    pin_btn.connect_clicked(move |_| on_pin_folder());
    crate::ui::attach_tooltip(&pin_btn, "Pin a folder to this project");
    heading_row.append(&pin_btn);

    section.append(&heading_row);

    if destinations.is_empty() {
        let empty = Label::new(Some(
            "No folders pinned yet. Click + Pin Folder to add one.",
        ));
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
        let path = PathBuf::from(&dest.path);
        let card = build_pin_card(
            dest.id,
            &dest.name,
            &dest.path,
            path,
            on_navigate.clone(),
            on_remove_pin.clone(),
        );
        flow.append(&card);
    }

    section.append(&flow);
    section
}

fn build_pin_card<FNav, FRemove>(
    dest_id: i64,
    name: &str,
    path_str: &str,
    path: PathBuf,
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

    let path_label = Label::new(Some(path_str));
    path_label.add_css_class("landing-pin-path");
    path_label.set_halign(Align::Center);
    path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    path_label.set_max_width_chars(18);
    card.append(&path_label);

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

    let remove_btn = Button::new();
    remove_btn.add_css_class("landing-dest-remove");
    let x_icon = Label::new(Some("×"));
    remove_btn.set_child(Some(&x_icon));
    remove_btn.set_valign(Align::Start);
    remove_btn.connect_clicked(move |_| on_remove(dest_id));
    crate::ui::attach_tooltip(&remove_btn, "Unpin this folder");
    outer.append(&remove_btn);

    outer
}
