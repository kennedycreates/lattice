use crate::ui::file_grid::{FileItem, FileKind};
use gtk::prelude::*;
use gtk::{Box, Label, Orientation, Separator};

#[derive(Clone)]
pub struct PreviewPane {
    pub root: Box,
    icon: Label,
    name: Label,
    type_value: Label,
    path_value: Label,
    note_value: Label,
}

impl PreviewPane {
    pub fn build() -> Self {
        let root = Box::new(Orientation::Vertical, 0);
        root.add_css_class("preview-pane");

        let header = Label::new(Some("Preview"));
        header.add_css_class("preview-header");
        header.set_halign(gtk::Align::Start);
        header.set_margin_top(16);
        header.set_margin_bottom(8);
        header.set_margin_start(16);
        header.set_margin_end(16);
        root.append(&header);

        let sep = Separator::new(Orientation::Horizontal);
        root.append(&sep);

        let icon = Label::new(Some("DIR"));
        icon.add_css_class("preview-icon");
        icon.set_halign(gtk::Align::Center);
        root.append(&icon);

        let name = Label::new(Some("No Selection"));
        name.add_css_class("preview-filename");
        name.set_halign(gtk::Align::Center);
        name.set_wrap(true);
        name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        name.set_margin_start(16);
        name.set_margin_end(16);
        root.append(&name);

        let sep2 = Separator::new(Orientation::Horizontal);
        sep2.add_css_class("preview-meta-sep");
        root.append(&sep2);

        let type_value = append_meta_row(&root, "Type", "Folder");
        let path_value = append_meta_row(&root, "Path", "");
        path_value.set_wrap(true);
        path_value.set_wrap_mode(gtk::pango::WrapMode::Char);
        let note_value = append_meta_row(&root, "Note", "Select an item to inspect it here.");
        note_value.set_wrap(true);
        note_value.set_wrap_mode(gtk::pango::WrapMode::WordChar);

        Self {
            root,
            icon,
            name,
            type_value,
            path_value,
            note_value,
        }
    }

    fn set_icon_kind(&self, kind: &FileKind) {
        for class in FileKind::ALL_CSS_CLASSES {
            self.icon.remove_css_class(class);
        }
        self.icon.add_css_class(kind.css_class());
        self.icon.set_label(kind.badge());
    }

    pub fn show_current_folder(&self, path: &str, item_count: usize) {
        self.set_icon_kind(&FileKind::Folder);
        self.name.set_label("No Selection");
        self.type_value.set_label("Folder");
        self.path_value.set_label(path);
        self.note_value
            .set_label(&format!("{item_count} item(s) in the current folder."));
    }

    pub fn show_item(&self, item: &FileItem, path: &str) {
        self.set_icon_kind(&item.kind);
        self.name.set_label(&item.name);
        self.type_value.set_label(item.kind.label());
        self.path_value.set_label(path);
        self.note_value.set_label(if item.is_dir {
            "Double-click to open this folder."
        } else {
            "Double-click to open this file with its default app."
        });
    }

    pub fn show_error(&self, path: &str, message: &str) {
        self.set_icon_kind(&FileKind::Unknown);
        self.icon.set_label("ERR");
        self.name.set_label("Folder Unavailable");
        self.type_value.set_label("Error");
        self.path_value.set_label(path);
        self.note_value.set_label(message);
    }
}

fn append_meta_row(root: &Box, key: &str, value: &str) -> Label {
    let row = Box::new(Orientation::Horizontal, 8);
    row.add_css_class("preview-meta-row");
    row.set_margin_start(16);
    row.set_margin_end(16);
    row.set_margin_top(4);
    row.set_margin_bottom(4);

    let key_label = Label::new(Some(key));
    key_label.add_css_class("preview-meta-key");
    key_label.set_halign(gtk::Align::Start);
    key_label.set_hexpand(true);
    row.append(&key_label);

    let value_label = Label::new(Some(value));
    value_label.add_css_class("preview-meta-value");
    value_label.set_halign(gtk::Align::End);
    value_label.set_hexpand(true);
    value_label.set_justify(gtk::Justification::Right);
    row.append(&value_label);

    root.append(&row);
    value_label
}
