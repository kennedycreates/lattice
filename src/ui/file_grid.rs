use crate::metadata::TagRecord;
use crate::thumbnail::ThumbnailTarget;
use gio::FileType;
use gtk::prelude::*;
use gtk::{Align, Box, FlowBox, Label, Orientation, Overlay, Picture, ScrolledWindow, Stack};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

const FILE_CARD_WIDTH: i32 = 96;
const FILE_CARD_HEIGHT: i32 = 116;
const FILE_CARD_THUMB_SIZE: i32 = 48;
const FILE_CARD_MEDIA_SIZE: i32 = 52;
const FILE_CARD_NAME_MAX_WIDTH_CHARS: i32 = 14;
const FILE_CARD_TAGS_HEIGHT: i32 = 13;
const FILE_GRID_MARGIN: i32 = 8;
const FILE_GRID_COLUMN_SPACING: u32 = 4;
const FILE_GRID_ROW_SPACING: u32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileKind {
    Folder,
    Image,
    Video,
    Audio,
    Document,
    Text,
    Archive,
    ConfigCode,
    Unknown,
}

impl FileKind {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Folder => "📁",
            Self::Image => "🖼",
            Self::Video => "🎬",
            Self::Audio => "🎵",
            Self::Document => "📄",
            Self::Text => "📝",
            Self::Archive => "📦",
            Self::ConfigCode => "⚙",
            Self::Unknown => "❓",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Document => "Document",
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
            Self::Audio => "file-type-audio",
            Self::Document => "file-type-document",
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
        "file-type-audio",
        "file-type-document",
        "file-type-text",
        "file-type-archive",
        "file-type-config",
        "file-type-unknown",
    ];

    pub(crate) fn from_path(path: &Path, file_type: FileType, content_type: Option<&str>) -> Self {
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

        if is_image(&extension, &content_type) {
            return Self::Image;
        }

        if is_video(&extension, &content_type) {
            return Self::Video;
        }

        if is_audio(&extension, &content_type) {
            return Self::Audio;
        }

        if is_archive(&extension, &content_type) {
            return Self::Archive;
        }

        if is_document(&extension, &content_type) {
            return Self::Document;
        }

        if is_config_or_code(&name, &extension, &content_type) {
            return Self::ConfigCode;
        }

        if is_text(&extension, &content_type) {
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
    pub size_bytes: Option<u64>,
    pub modified_unix: Option<i64>,
    pub tags: Vec<TagRecord>,
    /// Original path before trashing, populated for items loaded from trash:///
    pub original_path: Option<PathBuf>,
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
            size_bytes: (info.size() >= 0).then_some(info.size() as u64),
            modified_unix: info
                .modification_date_time()
                .and_then(|value| Some(value.to_unix())),
            tags: Vec::new(),
            original_path: None,
        })
    }
}

#[derive(Clone)]
pub struct FileGrid {
    pub root: Overlay,
    pub flow: FlowBox,
    empty_state: Label,
    thumb_targets: RefCell<Vec<ThumbnailTarget>>,
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
        flow.set_can_focus(true);
        flow.set_homogeneous(true);
        flow.set_column_spacing(FILE_GRID_COLUMN_SPACING);
        flow.set_row_spacing(FILE_GRID_ROW_SPACING);
        flow.set_valign(Align::Start);
        flow.set_margin_top(FILE_GRID_MARGIN);
        flow.set_margin_bottom(FILE_GRID_MARGIN);
        flow.set_margin_start(FILE_GRID_MARGIN);
        flow.set_margin_end(FILE_GRID_MARGIN);

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
            thumb_targets: RefCell::new(Vec::new()),
        }
    }

    pub fn set_loading(&self) {
        self.thumb_targets.borrow_mut().clear();
        self.clear();
        self.empty_state.set_label("Loading folder…");
        self.empty_state.set_visible(true);
    }

    pub fn set_empty_message(&self, message: &str) {
        self.thumb_targets.borrow_mut().clear();
        self.clear();
        self.empty_state.set_label(message);
        self.empty_state.set_visible(true);
    }

    pub fn set_items(&self, items: &[FileItem]) {
        self.clear();
        self.empty_state.set_visible(items.is_empty());
        if items.is_empty() {
            self.empty_state.set_label("This folder is empty");
            return;
        }

        let mut targets = Vec::new();
        for item in items {
            let (card, target) = build_card(item);
            self.flow.append(&card);
            if let Some(t) = target {
                targets.push(t);
            }
        }
        *self.thumb_targets.borrow_mut() = targets;
    }

    /// Take all pending thumbnail targets out of this grid so the caller can
    /// submit them to a ThumbnailLoader. Leaves the internal list empty.
    pub fn drain_thumb_targets(&self) -> Vec<ThumbnailTarget> {
        self.thumb_targets.borrow_mut().drain(..).collect()
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

    pub fn select_range(&self, start: i32, end: i32, clear_first: bool) {
        if clear_first {
            self.flow.unselect_all();
        }

        let (from, to) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };

        for index in from..=to {
            if let Some(child) = self.flow.child_at_index(index) {
                self.flow.select_child(&child);
            }
        }
    }

    pub fn toggle_index(&self, index: i32) {
        if let Some(child) = self.flow.child_at_index(index) {
            if child.is_selected() {
                self.flow.unselect_child(&child);
            } else {
                self.flow.select_child(&child);
            }
        }
    }

    pub fn focus_index(&self, index: i32) {
        if let Some(child) = self.flow.child_at_index(index) {
            child.grab_focus();
        } else {
            self.flow.grab_focus();
        }
    }

    pub fn child_count(&self) -> i32 {
        self.flow.observe_children().n_items() as i32
    }

    pub fn estimated_columns(&self) -> i32 {
        let width = self.flow.width();
        if width <= 0 {
            return 1;
        }

        let usable_width = (width - FILE_GRID_MARGIN * 2).max(FILE_CARD_WIDTH);
        let spacing = FILE_GRID_COLUMN_SPACING as i32;
        ((usable_width + spacing) / (FILE_CARD_WIDTH + spacing)).max(1)
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

fn build_card(file: &FileItem) -> (Box, Option<ThumbnailTarget>) {
    let shell = Box::new(Orientation::Vertical, 0);
    shell.add_css_class("file-card-shell");
    shell.set_size_request(FILE_CARD_WIDTH, FILE_CARD_HEIGHT);
    shell.set_hexpand(false);
    shell.set_halign(Align::Center);
    shell.set_valign(Align::Start);

    let card = Box::new(Orientation::Vertical, 2);
    card.add_css_class("file-card");
    card.add_css_class(file.kind.css_class());
    card.set_size_request(FILE_CARD_WIDTH, FILE_CARD_HEIGHT);
    card.set_hexpand(false);
    card.set_vexpand(true);
    card.set_halign(Align::Center);
    card.set_valign(Align::Start);
    shell.append(&card);

    let media = Box::new(Orientation::Vertical, 0);
    media.add_css_class("file-card-media");
    media.set_size_request(FILE_CARD_MEDIA_SIZE, FILE_CARD_MEDIA_SIZE);
    media.set_halign(Align::Center);
    media.set_valign(Align::Start);

    // For image files, slot a Stack so we can crossfade in the real thumbnail
    // once it loads. Other file types just use the emoji badge label.
    let thumb_target = if file.kind == FileKind::Image {
        let badge = Label::new(Some(file.kind.badge()));
        badge.add_css_class("file-card-icon");
        badge.set_halign(Align::Center);
        badge.set_valign(Align::Center);
        badge.set_size_request(FILE_CARD_MEDIA_SIZE, FILE_CARD_MEDIA_SIZE);

        let picture = Picture::new();
        picture.add_css_class("file-thumb");
        picture.set_size_request(FILE_CARD_THUMB_SIZE, FILE_CARD_THUMB_SIZE);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_halign(Align::Center);
        picture.set_valign(Align::Center);

        let stack = Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(180);
        stack.add_named(&badge, Some("icon"));
        stack.add_named(&picture, Some("thumb"));
        stack.set_visible_child_name("icon");
        stack.set_size_request(FILE_CARD_MEDIA_SIZE, FILE_CARD_MEDIA_SIZE);
        stack.set_halign(Align::Center);
        stack.set_valign(Align::Center);
        media.append(&stack);

        Some(ThumbnailTarget {
            path: file.path.clone(),
            mtime: file.modified_unix.unwrap_or(0),
            stack,
            picture,
        })
    } else {
        let icon = Label::new(Some(file.kind.badge()));
        icon.add_css_class("file-card-icon");
        icon.set_halign(Align::Center);
        icon.set_valign(Align::Center);
        icon.set_size_request(FILE_CARD_MEDIA_SIZE, FILE_CARD_MEDIA_SIZE);
        media.append(&icon);
        None
    };

    card.append(&media);

    let name = Label::new(Some(&file.name));
    name.add_css_class("file-card-name");
    name.set_halign(gtk::Align::Center);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_wrap(true);
    name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    name.set_width_chars(FILE_CARD_NAME_MAX_WIDTH_CHARS);
    name.set_max_width_chars(FILE_CARD_NAME_MAX_WIDTH_CHARS);
    name.set_size_request(FILE_CARD_WIDTH - 10, -1);
    name.set_lines(2);
    name.set_justify(gtk::Justification::Center);
    card.append(&name);

    let tags = Box::new(Orientation::Horizontal, 4);
    tags.add_css_class("file-card-tags");
    tags.set_halign(gtk::Align::Center);
    tags.set_size_request(-1, FILE_CARD_TAGS_HEIGHT);

    if !file.tags.is_empty() {
        for tag in file.tags.iter().take(1) {
            let chip = Label::new(Some(&tag.name));
            chip.add_css_class("file-tag-chip");
            tags.append(&chip);
        }

        if file.tags.len() > 1 {
            let overflow = Label::new(Some(&format!("+{}", file.tags.len() - 1)));
            overflow.add_css_class("file-tag-chip");
            overflow.add_css_class("file-tag-chip-muted");
            tags.append(&overflow);
        }
    } else {
        tags.set_visible(false);
    }
    card.append(&tags);

    (shell, thumb_target)
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

fn is_image(extension: &str, content_type: &str) -> bool {
    matches!(
        extension,
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "svg"
            | "avif"
            | "heic"
            | "heif"
            | "ico"
    ) || content_type.starts_with("image/")
}

fn is_video(extension: &str, content_type: &str) -> bool {
    matches!(
        extension,
        "mp4" | "m4v" | "mov" | "mkv" | "avi" | "webm" | "mpg" | "mpeg" | "ogv" | "wmv"
    ) || content_type.starts_with("video/")
}

fn is_audio(extension: &str, content_type: &str) -> bool {
    matches!(
        extension,
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "m4a" | "aac" | "opus" | "wma"
    ) || content_type.starts_with("audio/")
}

fn is_document(extension: &str, content_type: &str) -> bool {
    matches!(
        extension,
        "pdf"
            | "doc"
            | "docx"
            | "odt"
            | "rtf"
            | "pages"
            | "epub"
            | "xls"
            | "xlsx"
            | "ods"
            | "ppt"
            | "pptx"
            | "odp"
    ) || content_type.contains("pdf")
        || content_type.contains("rtf")
        || content_type.contains("msword")
        || content_type.contains("officedocument")
        || content_type.contains("opendocument")
        || content_type.contains("spreadsheet")
        || content_type.contains("presentation")
        || content_type.contains("excel")
        || content_type.contains("powerpoint")
        || content_type.contains("epub")
}

fn is_text(extension: &str, content_type: &str) -> bool {
    matches!(extension, "txt" | "log" | "nfo" | "csv" | "tsv") || content_type.starts_with("text/")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_images_by_extension_without_content_type() {
        assert_eq!(
            FileKind::from_path(Path::new("/tmp/poster.PNG"), FileType::Regular, None),
            FileKind::Image
        );
    }

    #[test]
    fn classifies_documents_by_extension_even_with_generic_content_type() {
        assert_eq!(
            FileKind::from_path(
                Path::new("/tmp/guide.pdf"),
                FileType::Regular,
                Some("application/octet-stream"),
            ),
            FileKind::Document
        );
    }

    #[test]
    fn classifies_audio_by_extension_without_content_type() {
        assert_eq!(
            FileKind::from_path(Path::new("/tmp/track.ogg"), FileType::Regular, None),
            FileKind::Audio
        );
    }
}
