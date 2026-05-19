use gtk::prelude::*;
use gtk::{Box, Button, Entry, Image, Label, Orientation, Separator, Stack, ToggleButton};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct Toolbar {
    pub root: Box,
    pub back_button: Button,
    pub forward_button: Button,
    pub up_button: Button,
    pub refresh_button: Button,
    pub sidebar_toggle: ToggleButton,
    pub split_toggle: Button,
    pub split_icon: Image,
    pub split_tooltip_label: Label,
    pub holding_tray_toggle: ToggleButton,
    pub plan_mode_toggle: ToggleButton,
    pub paint_mode_toggle: ToggleButton,
    pub preview_toggle: ToggleButton,
    pub new_folder_button: Button,
    pub new_text_document_button: Button,
    pub rename_button: Button,
    pub trash_button: Button,
    pub empty_trash_button: Button,
    pub empty_trash_host: Box,
    pub path_button: Button,
    pub path_entry: Entry,
    path_stack: Stack,
    breadcrumb_row: Box,
}

impl Toolbar {
    pub fn build() -> Self {
        let bar = Box::new(Orientation::Horizontal, 3);
        bar.add_css_class("top-toolbar");

        // ── Group 1: Navigation / View Basics ──────────────────────────
        let (back_button, back_host) = nav_button("go-previous-symbolic", "Back (Alt+Left)", true);
        let (forward_button, forward_host) =
            nav_button("go-next-symbolic", "Forward (Alt+Right)", true);
        let (up_button, up_host) = nav_button("go-up-symbolic", "Up (Alt+Up)", true);
        let (refresh_button, refresh_host) =
            nav_button("view-refresh-symbolic", "Refresh (Ctrl+R)", false);
        bar.append(&back_host);
        bar.append(&forward_host);
        bar.append(&up_host);
        bar.append(&refresh_host);

        let (split_toggle, split_icon, split_tooltip_label) = dynamic_icon_button(
            "view-list-symbolic",
            "Switch to 2 panels (Ctrl+\\)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        bar.append(&split_toggle);

        // ── Divider 1 ──────────────────────────────────────────────────
        let sep1 = Separator::new(Orientation::Vertical);
        sep1.add_css_class("toolbar-sep");
        bar.append(&sep1);

        // ── Group 2: Layout / Workspace Surfaces ───────────────────────
        let sidebar_toggle = toggle_icon_button(
            "sidebar-show-symbolic",
            "Sidebar (Ctrl+B)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        sidebar_toggle.set_active(true);
        bar.append(&sidebar_toggle);

        let preview_toggle = toggle_icon_button(
            "document-print-preview-symbolic",
            "Preview (Ctrl+P)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        preview_toggle.set_active(true);
        bar.append(&preview_toggle);

        let holding_tray_toggle = toggle_icon_button(
            "mail-attachment-symbolic",
            "Holding Tray (Ctrl+Alt+H)",
            &["toolbar-action-btn", "toolbar-toggle", "toolbar-icon-btn"],
        );
        holding_tray_toggle.set_active(false);
        bar.append(&holding_tray_toggle);

        let plan_mode_toggle = toggle_icon_button(
            "document-edit-symbolic",
            "Plan actions (Ctrl+Shift+P)",
            &[
                "toolbar-action-btn",
                "toolbar-toggle",
                "toolbar-icon-btn",
                "toolbar-plan-btn",
            ],
        );
        plan_mode_toggle.set_active(false);
        bar.append(&plan_mode_toggle);

        let paint_mode_toggle = toggle_icon_button(
            "preferences-color-symbolic",
            "Painting Mode",
            &[
                "toolbar-action-btn",
                "toolbar-toggle",
                "toolbar-icon-btn",
                "toolbar-paint-btn",
            ],
        );
        paint_mode_toggle.set_active(false);
        bar.append(&paint_mode_toggle);

        // ── Divider 2 ──────────────────────────────────────────────────
        let sep2 = Separator::new(Orientation::Vertical);
        sep2.add_css_class("toolbar-sep");
        bar.append(&sep2);

        // ── Group 3: File Actions ──────────────────────────────────────
        let new_folder_button = action_icon_button(
            "folder-new-symbolic",
            "New folder (Ctrl+N)",
            &["toolbar-action-btn", "toolbar-icon-btn"],
        );
        let new_folder_host = super::tooltip_host(&new_folder_button, "New folder (Ctrl+N)");
        bar.append(&new_folder_host);

        let new_text_document_button = action_icon_button(
            "document-new-symbolic",
            "New text file (Ctrl+Shift+N)",
            &["toolbar-action-btn", "toolbar-icon-btn"],
        );
        let new_text_document_host =
            super::tooltip_host(&new_text_document_button, "New text file (Ctrl+Shift+N)");
        bar.append(&new_text_document_host);

        let rename_button = action_icon_button(
            "document-edit-symbolic",
            "Rename (F2)",
            &["toolbar-action-btn", "toolbar-icon-btn"],
        );
        let rename_host = super::tooltip_host(&rename_button, "Rename (F2)");
        rename_button.set_sensitive(false);
        bar.append(&rename_host);

        let trash_button = action_icon_button(
            "user-trash-full-symbolic",
            "Move to Trash (Delete)",
            &[
                "toolbar-action-btn",
                "toolbar-icon-btn",
                "toolbar-danger-btn",
            ],
        );
        let trash_host = super::tooltip_host(&trash_button, "Move to Trash (Delete)");
        trash_button.set_sensitive(false);
        bar.append(&trash_host);

        let empty_trash_button = Button::new();
        empty_trash_button.add_css_class("toolbar-action-btn");
        empty_trash_button.add_css_class("toolbar-danger-btn");
        let empty_trash_host =
            super::tooltip_host(&empty_trash_button, "Empty Trash (Ctrl+Shift+Delete)");
        empty_trash_host.set_visible(false);
        {
            let inner = Box::new(Orientation::Horizontal, 5);
            let icon = gtk::Image::from_icon_name("user-trash-symbolic");
            let lbl = Label::new(Some("Empty Trash"));
            inner.append(&icon);
            inner.append(&lbl);
            empty_trash_button.set_child(Some(&inner));
        }
        bar.append(&empty_trash_host);

        // ── Divider 3 ──────────────────────────────────────────────────
        let sep3 = Separator::new(Orientation::Vertical);
        sep3.add_css_class("toolbar-sep");
        bar.append(&sep3);

        // ── Path / breadcrumb ──────────────────────────────────────────
        let path_stack = Stack::new();
        path_stack.set_hexpand(true);
        path_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        path_stack.set_transition_duration(120);
        path_stack.add_css_class("toolbar-path-stack");
        bar.append(&path_stack);

        let path_button = Button::new();
        path_button.add_css_class("toolbar-path-button");
        path_button.set_hexpand(true);
        path_button.set_halign(gtk::Align::Fill);
        super::attach_tooltip(&path_button, "Edit path (Ctrl+L)");

        let breadcrumb_row = Box::new(Orientation::Horizontal, 6);
        breadcrumb_row.add_css_class("toolbar-breadcrumbs");
        breadcrumb_row.set_hexpand(true);
        path_button.set_child(Some(&breadcrumb_row));

        path_stack.add_named(&path_button, Some("breadcrumbs"));

        let path_entry = Entry::new();
        path_entry.set_editable(true);
        path_entry.set_can_focus(true);
        path_entry.set_hexpand(true);
        path_entry.set_placeholder_text(Some("Type a path; Tab completes"));
        super::attach_tooltip(&path_entry, "Enter path to open; Tab or Right completes");
        path_entry.add_css_class("toolbar-path");
        path_stack.add_named(&path_entry, Some("entry"));
        path_stack.set_visible_child_name("breadcrumbs");

        Self {
            root: bar,
            back_button,
            forward_button,
            up_button,
            refresh_button,
            sidebar_toggle,
            split_toggle,
            split_icon,
            split_tooltip_label,
            holding_tray_toggle,
            plan_mode_toggle,
            paint_mode_toggle,
            preview_toggle,
            new_folder_button,
            new_text_document_button,
            rename_button,
            trash_button,
            empty_trash_button,
            empty_trash_host,
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

    pub fn set_split_icon_state(&self, icon_name: &str) {
        self.split_icon.set_icon_name(Some(icon_name));
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

fn action_icon_button(icon_name: &str, _tooltip: &str, classes: &[&str]) -> Button {
    let button = Button::from_icon_name(icon_name);
    for class_name in classes {
        button.add_css_class(class_name);
    }
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

fn dynamic_icon_button(icon_name: &str, tooltip: &str, classes: &[&str]) -> (Button, Image, Label) {
    let button = Button::new();
    for class_name in classes {
        button.add_css_class(class_name);
    }

    let icon = Image::from_icon_name(icon_name);
    button.set_child(Some(&icon));

    let frame = Box::new(Orientation::Horizontal, 0);
    frame.add_css_class("app-tooltip-frame");

    let label = Label::new(Some(tooltip));
    label.add_css_class("app-tooltip-label");
    label.set_single_line_mode(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(56);
    frame.append(&label);

    let popover = gtk::Popover::new();
    popover.add_css_class("app-tooltip-popover");
    popover.set_has_arrow(false);
    popover.set_autohide(false);
    popover.set_can_target(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_child(Some(&frame));
    popover.set_parent(&button);

    let hover_timer = Rc::new(RefCell::new(None::<glib::SourceId>));

    let motion = gtk::EventControllerMotion::new();
    {
        let hover_timer = Rc::clone(&hover_timer);
        let popover = popover.clone();
        motion.connect_enter(move |_, _, _| {
            if let Some(source_id) = hover_timer.borrow_mut().take() {
                source_id.remove();
            }

            let hover_timer_for_timeout = Rc::clone(&hover_timer);
            let popover = popover.clone();
            let source_id =
                glib::timeout_add_local(std::time::Duration::from_millis(350), move || {
                    popover.popup();
                    hover_timer_for_timeout.borrow_mut().take();
                    glib::ControlFlow::Break
                });
            hover_timer.borrow_mut().replace(source_id);
        });
    }
    {
        let hover_timer = Rc::clone(&hover_timer);
        let popover = popover.clone();
        motion.connect_leave(move |_| {
            if let Some(source_id) = hover_timer.borrow_mut().take() {
                source_id.remove();
            }
            popover.popdown();
        });
    }
    button.add_controller(motion);

    (button, icon, label)
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
