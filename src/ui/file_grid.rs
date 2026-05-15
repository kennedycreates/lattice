use gio::FileType;
use gtk::prelude::*;
use gtk::{Align, Box, FlowBox, Label, Orientation, Overlay, ScrolledWindow};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileKind {
    Folder,
    Image,
    Video,
    Text,
    Archive,
    ConfigCode,
    Unknown,
}

impl FileKind {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Folder => "DIR",
            Self::Image => "IMG",
            Self::Video => "VID",
            Self::Text => "TXT",
            Self::Archive => "ARC",
            Self::ConfigCode => "CFG",
            Self::Unknown => "???",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Text => "Text",
            Self::Archive => "Archive",
            Self::ConfigCode => "Config / Code",
            Self::Unknown => "Unknown File",
        }
    }

    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Folder => "file-type-folder",
            Self::Image => "file-type-image",
            Self::Video => "file-type-video",
            Self::Text => "file-type-text",
            Self::Archive => "file-type-archive",
            Self::ConfigCode => "file-type-config",
            Self::Unknown => "file-type-unknown",
        }
    }

    pub const ALL_CSS_CLASSES: &'static [&'static str] = &[
        "file-type-folder",
        "file-type-image",
        "file-type-video",
        "file-type-text",
        "file-type-archive",
        "file-type-config",
        "file-type-unknown",
    ];

    fn from_path(path: &Path, file_type: FileType, content_type: Option<&str>) -> Self {
        if file_type == FileType::Directory {
            return Self::Folder;
        }

        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let content_type = content_type.unwrap_or_default().to_ascii_lowercase();

        if content_type.starts_with("image/") {
            return Self::Image;
        }

        if content_type.starts_with("video/") {
            return Self::Video;
        }

        if is_archive(&extension, &content_type) {
            return Self::Archive;
        }

        if is_config_or_code(&name, &extension, &content_type) {
            return Self::ConfigCode;
        }

        if content_type.starts_with("text/") {
            return Self::Text;
        }

        Self::Unknown
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileItem {
    pub name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub is_dir: bool,
}

impl FileItem {
    pub fn from_info(parent: &gio::File, info: &gio::FileInfo, show_hidden: bool) -> Option<Self> {
        if !show_hidden && info.is_hidden() {
            return None;
        }

        let child = parent.child(&info.name());
        let path = child.path()?;
        let kind = FileKind::from_path(&path, info.file_type(), info.content_type().as_deref());

        Some(Self {
            name: info.display_name().to_string(),
            path,
            is_dir: info.file_type() == FileType::Directory,
            kind,
        })
    }
}

#[derive(Clone)]
pub struct FileGrid {
    pub root: Overlay,
    pub flow: FlowBox,
    empty_state: Label,
}

impl FileGrid {
    pub fn build() -> Self {
        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        scroll.add_css_class("file-grid-scroll");

        let flow = FlowBox::new();
        flow.add_css_class("file-grid");
        flow.set_selection_mode(gtk::SelectionMode::Multiple);
        flow.set_activate_on_single_click(false);
        flow.set_homogeneous(true);
        flow.set_column_spacing(8);
        flow.set_row_spacing(8);
        flow.set_valign(Align::Start);
        flow.set_margin_top(16);
        flow.set_margin_bottom(16);
        flow.set_margin_start(16);
        flow.set_margin_end(16);

        scroll.set_child(Some(&flow));

        let empty_state = Label::new(Some("Loading folder…"));
        empty_state.add_css_class("file-grid-empty");
        empty_state.set_halign(Align::Center);
        empty_state.set_valign(Align::Center);

        let root = Overlay::new();
        root.set_child(Some(&scroll));
        root.add_overlay(&empty_state);

        Self {
            root,
            flow,
            empty_state,
        }
    }

    pub fn set_loading(&self) {
        self.clear();
        self.empty_state.set_label("Loading folder…");
        self.empty_state.set_visible(true);
    }

    pub fn set_empty_message(&self, message: &str) {
        self.clear();
        self.empty_state.set_label(message);
        self.empty_state.set_visible(true);
    }

    pub fn set_items(&self, items: &[FileItem]) {
        self.clear();
        self.empty_state.set_visible(items.is_empty());
        if items.is_empty() {
            self.empty_state.set_label("This folder is empty.");
            return;
        }

        for item in items {
            self.flow.append(&build_card(item));
        }
    }

    pub fn clear_selection(&self) {
        self.flow.unselect_all();
    }

    pub fn select_only_index(&self, index: i32) {
        self.flow.unselect_all();
        if let Some(child) = self.flow.child_at_index(index) {
            self.flow.select_child(&child);
        }
    }

    pub fn selected_indices(&self) -> Vec<i32> {
        self.flow
            .selected_children()
            .into_iter()
            .map(|child| child.index())
            .filter(|index| *index >= 0)
            .collect()
    }

    fn clear(&self) {
        while let Some(child) = self.flow.child_at_index(0) {
            self.flow.remove(&child);
        }
    }
}

fn build_card(file: &FileItem) -> Box {
    let card = Box::new(Orientation::Vertical, 6);
    card.add_css_class("file-card");
    card.add_css_class(file.kind.css_class());

    let icon = Label::new(Some(file.kind.badge()));
    icon.add_css_class("file-card-icon");
    icon.set_halign(gtk::Align::Center);
    card.append(&icon);

    let name = Label::new(Some(&file.name));
    name.add_css_class("file-card-name");
    name.set_halign(gtk::Align::Center);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_wrap(true);
    name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    name.set_max_width_chars(14);
    card.append(&name);

    let kind = Label::new(Some(file.kind.label()));
    kind.add_css_class("file-card-kind");
    kind.set_halign(gtk::Align::Center);
    card.append(&kind);

    card
}

fn is_archive(extension: &str, content_type: &str) -> bool {
    matches!(
        extension,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "iso" | "jar"
    ) || content_type.contains("zip")
        || content_type.contains("tar")
        || content_type.contains("archive")
        || content_type.contains("compressed")
}

fn is_config_or_code(name: &str, extension: &str, content_type: &str) -> bool {
    matches!(
        extension,
        "rs" | "toml"
            | "json"
            | "yaml"
            | "yml"
            | "ini"
            | "conf"
            | "cfg"
            | "xml"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "java"
            | "go"
            | "sh"
            | "bash"
            | "zsh"
            | "css"
            | "scss"
            | "html"
            | "md"
            | "sql"
            | "kt"
            | "swift"
            | "lock"
    ) || matches!(
        name,
        "makefile" | "dockerfile" | ".env" | ".gitignore" | ".editorconfig"
    ) || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("yaml")
        || content_type.contains("x-python")
        || content_type.contains("javascript")
        || content_type.contains("typescript")
        || content_type.contains("rust")
        || content_type.contains("shellscript")
}
