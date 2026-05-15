use gtk::prelude::*;
use gtk::{Box, Button, Entry, Orientation, Separator, Stack, ToggleButton};

#[derive(Clone)]
pub struct Toolbar {
    pub root: Box,
    pub back_button: Button,
    pub up_button: Button,
    pub refresh_button: Button,
    pub split_toggle: ToggleButton,
    pub new_folder_button: Button,
    pub rename_button: Button,
    pub trash_button: Button,
    pub show_hidden_toggle: ToggleButton,
    pub preview_toggle: ToggleButton,
    pub path_button: Button,
    pub path_entry: Entry,
    path_stack: Stack,
    breadcrumb_row: Box,
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

        let split_toggle = ToggleButton::with_label("Split");
        split_toggle.add_css_class("toolbar-action-btn");
        split_toggle.add_css_class("toolbar-toggle");
        split_toggle.set_tooltip_text(Some("Show or hide the split pane"));
        bar.append(&split_toggle);

        let sep = Separator::new(Orientation::Vertical);
        sep.add_css_class("toolbar-sep");
        bar.append(&sep);

        let new_folder_button = Button::with_label("New Folder");
        new_folder_button.add_css_class("toolbar-action-btn");
        new_folder_button.set_tooltip_text(Some("Create a new folder here"));
        bar.append(&new_folder_button);

        let rename_button = Button::with_label("Rename");
        rename_button.add_css_class("toolbar-action-btn");
        rename_button.set_tooltip_text(Some("Rename the selected item"));
        rename_button.set_sensitive(false);
        bar.append(&rename_button);

        let trash_button = Button::with_label("Trash");
        trash_button.add_css_class("toolbar-action-btn");
        trash_button.add_css_class("toolbar-danger-btn");
        trash_button.set_tooltip_text(Some("Move selected items to the trash"));
        trash_button.set_sensitive(false);
        bar.append(&trash_button);

        let show_hidden_toggle = ToggleButton::with_label("Show Hidden Files");
        show_hidden_toggle.add_css_class("toolbar-action-btn");
        show_hidden_toggle.add_css_class("toolbar-toggle");
        show_hidden_toggle.set_tooltip_text(Some("Show or hide hidden files"));
        bar.append(&show_hidden_toggle);

        let preview_toggle = ToggleButton::with_label("Preview");
        preview_toggle.add_css_class("toolbar-action-btn");
        preview_toggle.add_css_class("toolbar-toggle");
        preview_toggle.set_tooltip_text(Some("Show or hide the preview pane"));
        preview_toggle.set_active(true);
        bar.append(&preview_toggle);

        let sep2 = Separator::new(Orientation::Vertical);
        sep2.add_css_class("toolbar-sep");
        bar.append(&sep2);

        let path_stack = Stack::new();
        path_stack.set_hexpand(true);
        path_stack.add_css_class("toolbar-path-stack");
        bar.append(&path_stack);

        let path_button = Button::new();
        path_button.add_css_class("toolbar-path-button");
        path_button.set_hexpand(true);
        path_button.set_halign(gtk::Align::Fill);
        path_button.set_tooltip_text(Some("Click to edit the current path"));

        let breadcrumb_row = Box::new(Orientation::Horizontal, 6);
        breadcrumb_row.add_css_class("toolbar-breadcrumbs");
        breadcrumb_row.set_hexpand(true);
        path_button.set_child(Some(&breadcrumb_row));

        path_stack.add_named(&path_button, Some("breadcrumbs"));

        let path_entry = Entry::new();
        path_entry.set_editable(true);
        path_entry.set_can_focus(true);
        path_entry.set_hexpand(true);
        path_entry.set_placeholder_text(Some("Type a path and press Enter"));
        path_entry.set_tooltip_text(Some("Type a path and press Enter to navigate"));
        path_entry.add_css_class("toolbar-path");
        path_stack.add_named(&path_entry, Some("entry"));
        path_stack.set_visible_child_name("breadcrumbs");

        Self {
            root: bar,
            back_button,
            up_button,
            refresh_button,
            split_toggle,
            new_folder_button,
            rename_button,
            trash_button,
            show_hidden_toggle,
            preview_toggle,
            path_button,
            path_entry,
            path_stack,
            breadcrumb_row,
        }
    }

    pub fn show_breadcrumb_mode(&self) {
        self.path_stack.set_visible_child_name("breadcrumbs");
    }

    pub fn show_entry_mode(&self) {
        self.path_stack.set_visible_child_name("entry");
    }

    pub fn set_breadcrumb_path(&self, display_path: &str) {
        clear_box(&self.breadcrumb_row);

        let segments = breadcrumb_segments(display_path);
        for (index, segment) in segments.iter().enumerate() {
            if index > 0 {
                let separator = gtk::Label::new(Some("›"));
                separator.add_css_class("toolbar-path-separator");
                self.breadcrumb_row.append(&separator);
            }

            let label = gtk::Label::new(Some(segment));
            label.add_css_class("toolbar-path-segment");
            if index == segments.len() - 1 {
                label.add_css_class("toolbar-path-current");
            }
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_single_line_mode(true);
            self.breadcrumb_row.append(&label);
        }
    }
}

fn nav_button(label: &str, tooltip: &str) -> Button {
    let button = Button::with_label(label);
    button.add_css_class("toolbar-nav-btn");
    button.set_tooltip_text(Some(tooltip));
    button
}

fn clear_box(container: &Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn breadcrumb_segments(display_path: &str) -> Vec<String> {
    if display_path == "~" {
        return vec!["~".to_string()];
    }

    if let Some(relative) = display_path.strip_prefix("~/") {
        let mut segments = vec!["~".to_string()];
        segments.extend(
            relative
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(ToString::to_string),
        );
        return segments;
    }

    let mut segments = vec!["/".to_string()];
    segments.extend(
        display_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(ToString::to_string),
    );
    segments
}
