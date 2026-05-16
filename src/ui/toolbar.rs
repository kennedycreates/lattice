use gtk::prelude::*;
use gtk::{Box, Button, Entry, Orientation, Separator, Stack, ToggleButton};

#[derive(Clone)]
pub struct Toolbar {
    pub root: Box,
    pub back_button: Button,
    pub up_button: Button,
    pub refresh_button: Button,
    pub sidebar_toggle: ToggleButton,
    pub split_toggle: ToggleButton,
    pub preview_toggle: ToggleButton,
    pub new_folder_button: Button,
    pub new_text_document_button: Button,
    pub rename_button: Button,
    pub trash_button: Button,
    pub search_button: Button,
    pub filter_toggle: ToggleButton,
    pub show_hidden_toggle: ToggleButton,
    pub path_button: Button,
    pub path_entry: Entry,
    path_stack: Stack,
    breadcrumb_row: Box,
}

impl Toolbar {
    pub fn build() -> Self {
        let bar = Box::new(Orientation::Horizontal, 3);
        bar.add_css_class("top-toolbar");

        let (back_button, back_host) = nav_button("go-previous-symbolic", "Back (Alt+Left)", true);
        let (up_button, up_host) = nav_button("go-up-symbolic", "Up (Alt+Up)", true);
        let (refresh_button, refresh_host) =
            nav_button("view-refresh-symbolic", "Refresh (Ctrl+R)", false);
        bar.append(&back_host);
        bar.append(&up_host);
        bar.append(&refresh_host);

        let sidebar_toggle = toggle_icon_button(
            "sidebar-show-symbolic",
            "Toggle sidebar (Ctrl+B)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        sidebar_toggle.set_active(true);
        bar.append(&sidebar_toggle);

        let preview_toggle = toggle_icon_button(
            "document-print-preview-symbolic",
            "Toggle preview pane (Ctrl+P)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        preview_toggle.set_active(true);
        bar.append(&preview_toggle);

        let split_toggle = toggle_icon_button(
            "view-dual-symbolic",
            "Toggle split view (Ctrl+\\)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        bar.append(&split_toggle);

        let sep = Separator::new(Orientation::Vertical);
        sep.add_css_class("toolbar-sep");
        bar.append(&sep);

        let new_folder_button = action_icon_button(
            "folder-new-symbolic",
            "Create folder (Ctrl+N)",
            &["toolbar-action-btn", "toolbar-icon-btn"],
        );
        bar.append(&new_folder_button);

        let new_text_document_button = action_icon_button(
            "document-new-symbolic",
            "Create text document (Ctrl+Shift+N)",
            &["toolbar-action-btn", "toolbar-icon-btn"],
        );
        bar.append(&new_text_document_button);

        let rename_button = action_icon_button(
            "document-edit-symbolic",
            "Rename selected files (F2)",
            &["toolbar-action-btn", "toolbar-icon-btn"],
        );
        let rename_host = super::tooltip_host(&rename_button, "Rename selected files (F2)");
        rename_button.set_sensitive(false);
        bar.append(&rename_host);

        let trash_button = action_icon_button(
            "user-trash-symbolic",
            "Move selection to Trash (Delete)",
            &[
                "toolbar-action-btn",
                "toolbar-icon-btn",
                "toolbar-danger-btn",
            ],
        );
        let trash_host = super::tooltip_host(&trash_button, "Move selection to Trash (Delete)");
        trash_button.set_sensitive(false);
        bar.append(&trash_host);

        let search_button = action_icon_button(
            "system-search-symbolic",
            "Search current folder (Ctrl+F)",
            &["toolbar-action-btn", "toolbar-icon-btn"],
        );
        bar.append(&search_button);

        let filter_toggle = toggle_icon_button(
            "object-select-symbolic",
            "Filter by tags (Ctrl+G)",
            &[
                "toolbar-action-btn",
                "toolbar-toggle",
                "toolbar-filter-btn",
                "toolbar-icon-btn",
            ],
        );
        bar.append(&filter_toggle);

        let show_hidden_toggle = toggle_icon_button(
            "view-reveal-symbolic",
            "Toggle hidden files (Ctrl+H)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        bar.append(&show_hidden_toggle);

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
        super::attach_tooltip(&path_button, "Edit current path (Ctrl+L)");

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
        super::attach_tooltip(&path_entry, "Enter path to open");
        path_entry.add_css_class("toolbar-path");
        path_stack.add_named(&path_entry, Some("entry"));
        path_stack.set_visible_child_name("breadcrumbs");

        Self {
            root: bar,
            back_button,
            up_button,
            refresh_button,
            sidebar_toggle,
            split_toggle,
            preview_toggle,
            new_folder_button,
            new_text_document_button,
            rename_button,
            trash_button,
            search_button,
            filter_toggle,
            show_hidden_toggle,
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

fn nav_button(icon_name: &str, tooltip: &str, show_when_disabled: bool) -> (Button, gtk::Widget) {
    let button = Button::from_icon_name(icon_name);
    button.add_css_class("toolbar-nav-btn");
    button.add_css_class("toolbar-icon-btn");
    if show_when_disabled {
        let host = super::tooltip_host(&button, tooltip);
        (button, host.upcast())
    } else {
        super::attach_tooltip(&button, tooltip);
        (button.clone(), button.upcast())
    }
}

fn action_icon_button(icon_name: &str, tooltip: &str, classes: &[&str]) -> Button {
    let button = Button::from_icon_name(icon_name);
    for class_name in classes {
        button.add_css_class(class_name);
    }
    super::attach_tooltip(&button, tooltip);
    button
}

fn toggle_icon_button(icon_name: &str, tooltip: &str, classes: &[&str]) -> ToggleButton {
    let button = ToggleButton::builder().icon_name(icon_name).build();
    for class_name in classes {
        button.add_css_class(class_name);
    }
    super::attach_tooltip(&button, tooltip);
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
