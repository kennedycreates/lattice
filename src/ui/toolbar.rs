use gtk::prelude::*;
use gtk::{Box, Button, Entry, Orientation, Separator, ToggleButton};

#[derive(Clone)]
pub struct Toolbar {
    pub root: Box,
    pub back_button: Button,
    pub up_button: Button,
    pub refresh_button: Button,
    pub new_folder_button: Button,
    pub rename_button: Button,
    pub trash_button: Button,
    pub show_hidden_toggle: ToggleButton,
    pub path_entry: Entry,
}

impl Toolbar {
    pub fn build() -> Self {
        let bar = Box::new(Orientation::Horizontal, 4);
        bar.add_css_class("top-toolbar");

        let back_button = nav_button("◀", "Back");
        let up_button = nav_button("▲", "Up");
        let refresh_button = nav_button("⟳", "Refresh");
        bar.append(&back_button);
        bar.append(&up_button);
        bar.append(&refresh_button);

        let sep = Separator::new(Orientation::Vertical);
        sep.add_css_class("toolbar-sep");
        bar.append(&sep);

        let new_folder_button = Button::with_label("New Folder");
        new_folder_button.add_css_class("toolbar-action-btn");
        bar.append(&new_folder_button);

        let rename_button = Button::with_label("Rename");
        rename_button.add_css_class("toolbar-action-btn");
        rename_button.set_sensitive(false);
        bar.append(&rename_button);

        let trash_button = Button::with_label("Trash");
        trash_button.add_css_class("toolbar-action-btn");
        trash_button.add_css_class("toolbar-danger-btn");
        trash_button.set_sensitive(false);
        bar.append(&trash_button);

        let show_hidden_toggle = ToggleButton::with_label("Show Hidden Files");
        show_hidden_toggle.add_css_class("toolbar-action-btn");
        show_hidden_toggle.add_css_class("toolbar-toggle");
        show_hidden_toggle.set_tooltip_text(Some("Toggle hidden files"));
        bar.append(&show_hidden_toggle);

        let sep2 = Separator::new(Orientation::Vertical);
        sep2.add_css_class("toolbar-sep");
        bar.append(&sep2);

        let path_entry = Entry::new();
        path_entry.set_editable(false);
        path_entry.set_can_focus(false);
        path_entry.set_hexpand(true);
        path_entry.set_placeholder_text(Some("Current folder"));
        path_entry.add_css_class("toolbar-path");
        bar.append(&path_entry);

        Self {
            root: bar,
            back_button,
            up_button,
            refresh_button,
            new_folder_button,
            rename_button,
            trash_button,
            show_hidden_toggle,
            path_entry,
        }
    }
}

fn nav_button(label: &str, tooltip: &str) -> Button {
    let button = Button::with_label(label);
    button.add_css_class("toolbar-nav-btn");
    button.set_tooltip_text(Some(tooltip));
    button
}
