use gtk::prelude::*;
use gtk::{Box, Label, Orientation};

#[derive(Clone)]
pub struct StatusBar {
    pub root: Box,
    info_label: Label,
    message_label: Label,
    path_label: Label,
}

impl StatusBar {
    pub fn build() -> Self {
        let root = Box::new(Orientation::Horizontal, 12);
        root.add_css_class("status-bar");

        let info_label = Label::new(Some("0 items · 0 selected"));
        info_label.add_css_class("status-info");
        info_label.set_halign(gtk::Align::Start);
        info_label.set_margin_start(10);
        root.append(&info_label);

        let message_label = Label::new(None);
        message_label.add_css_class("status-message");
        message_label.set_halign(gtk::Align::Center);
        message_label.set_hexpand(true);
        root.append(&message_label);

        let path_label = Label::new(Some(""));
        path_label.add_css_class("status-path");
        path_label.set_halign(gtk::Align::End);
        path_label.set_margin_end(10);
        root.append(&path_label);

        Self {
            root,
            info_label,
            message_label,
            path_label,
        }
    }

    pub fn set_counts(&self, item_count: usize, selected_count: usize) {
        self.info_label
            .set_label(&format!("{item_count} item(s) · {selected_count} selected"));
    }

    pub fn set_message(&self, message: &str) {
        self.message_label.set_label(message);
    }

    pub fn clear_message(&self) {
        self.message_label.set_label("");
    }

    pub fn set_path(&self, path: &str) {
        self.path_label.set_label(path);
    }
}
