use crate::metadata::{Shape, TagRecord};
use crate::thumbnail::{ThumbnailKind, ThumbnailTarget};
use crate::ui::mark_badge::make_shape_badge;
use gio::FileType;
use glib::SourceId;
use gtk::prelude::*;
use gtk::{
    Align, Box, DrawingArea, FlowBox, Label, ListBox, ListBoxRow, Orientation, Overlay, Picture,
    ScrolledWindow, Stack,
};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

struct MarkRef {
    icon_card: Box,
    icon_overlay: Overlay,
    icon_tags: Box,
    list_row: ListBoxRow,
    list_overlay: Overlay,
    list_tags: Box,
    shape: Shape,
    tint_color: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Icons,
    List,
}

const FILE_CARD_WIDTH: i32 = 112;
const FILE_CARD_HEIGHT: i32 = 136;
const FILE_CARD_THUMB_SIZE: i32 = 56;
const FILE_CARD_MEDIA_SIZE: i32 = 60;
const FILE_CARD_MEDIA_OVERLAY_WIDTH: i32 = FILE_CARD_WIDTH - 10;
const FILE_CARD_NAME_MAX_WIDTH_CHARS: i32 = 16;
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

    pub fn sort_key(&self) -> u8 {
        match self {
            Self::Folder => 0,
            Self::Image => 1,
            Self::Video => 2,
            Self::Audio => 3,
            Self::Document => 4,
            Self::Text => 5,
            Self::Archive => 6,
            Self::ConfigCode => 7,
            Self::Unknown => 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileItem {
    pub name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub is_dir: bool,
    pub is_openable: bool,
    pub detail: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_unix: Option<i64>,
    pub tags: Vec<TagRecord>,
    /// Original path before trashing, populated for items loaded from trash:///
    pub original_path: Option<PathBuf>,
    /// Tint id from metadata enrichment — drives CSS glow class mark-tint-{id}
    pub mark_tint_id: i64,
    /// Tint color from metadata enrichment — drives the Shape badge fill.
    pub mark_tint_color: Option<String>,
    /// Shape from metadata enrichment — drives badge and CSS class mark-shape-{shape}
    pub mark_shape: Shape,
}

impl FileItem {
    pub fn from_info(parent: &gio::File, info: &gio::FileInfo, show_hidden: bool) -> Option<Self> {
        if !show_hidden && info.is_hidden() {
            return None;
        }

        let child = parent.child(info.name());
        // For GVfs-mounted remotes, path() may be None when GVfs FUSE is not bridging
        // to the local filesystem. Fall back to the URI so the item is still browsable.
        let path = match child.path() {
            Some(p) => p,
            None => PathBuf::from(child.uri().as_str()),
        };
        let kind = FileKind::from_path(&path, info.file_type(), info.content_type().as_deref());

        Some(Self {
            name: info.display_name().to_string(),
            path,
            is_dir: info.file_type() == FileType::Directory,
            is_openable: true,
            detail: None,
            kind,
            size_bytes: (info.size() >= 0).then_some(info.size() as u64),
            modified_unix: info
                .modification_date_time().map(|value| value.to_unix()),
            tags: Vec::new(),
            original_path: None,
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
        })
    }
}

#[derive(Clone)]
pub struct FileGrid {
    pub root: Overlay,
    pub icon_scroll: ScrolledWindow,
    pub list_scroll: ScrolledWindow,
    pub flow: FlowBox,
    pub list_box: ListBox,
    content_stack: Stack,
    empty_state: Label,
    marquee: DrawingArea,
    marquee_rect: Rc<Cell<Option<(f64, f64, f64, f64)>>>,
    pub view_mode: Cell<ViewMode>,
    show_shape_badges: Cell<bool>,
    thumb_targets: RefCell<Vec<ThumbnailTarget>>,
    exit_timer: Rc<RefCell<Option<SourceId>>>,
    mark_refs: Rc<RefCell<Vec<MarkRef>>>,
}

impl FileGrid {
    pub fn build() -> Self {
        let icon_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        icon_scroll.add_css_class("file-grid-scroll");

        let flow = FlowBox::new();
        flow.add_css_class("file-grid");
        flow.set_selection_mode(gtk::SelectionMode::Multiple);
        flow.set_activate_on_single_click(false);
        flow.set_can_focus(true);
        flow.set_homogeneous(true);
        flow.set_column_spacing(FILE_GRID_COLUMN_SPACING);
        flow.set_row_spacing(FILE_GRID_ROW_SPACING);
        // Fill the viewport vertically so the empty area below the last row still
        // belongs to the FlowBox — the custom marquee gesture (in connect_pane) can
        // then start a rubber-band selection from that dead space. Rows still
        // top-pack because children keep their own valign.
        flow.set_valign(Align::Fill);
        flow.set_vexpand(true);
        flow.set_margin_top(FILE_GRID_MARGIN);
        flow.set_margin_bottom(FILE_GRID_MARGIN);
        flow.set_margin_start(FILE_GRID_MARGIN);
        flow.set_margin_end(FILE_GRID_MARGIN);
        icon_scroll.set_child(Some(&flow));

        let list_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        list_scroll.add_css_class("file-list-scroll");

        let list_box = ListBox::new();
        list_box.add_css_class("file-list");
        list_box.set_selection_mode(gtk::SelectionMode::Multiple);
        list_box.set_activate_on_single_click(false);
        list_box.set_can_focus(true);
        // Fill the viewport so marquee drags can begin below the last row.
        list_box.set_valign(Align::Fill);
        list_box.set_vexpand(true);
        list_scroll.set_child(Some(&list_box));

        let content_stack = Stack::new();
        content_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        content_stack.set_transition_duration(150);
        content_stack.add_named(&icon_scroll, Some("icons"));
        content_stack.add_named(&list_scroll, Some("list"));
        content_stack.set_visible_child_name("icons");

        let empty_state = Label::new(Some("Loading folder…"));
        empty_state.add_css_class("file-grid-empty");
        empty_state.set_halign(Align::Center);
        empty_state.set_valign(Align::Center);

        // Transparent overlay used only to paint the marquee selection rectangle.
        // It never handles input (can_target=false) so drags/clicks pass through
        // to the FlowBox/ListBox underneath.
        let marquee = DrawingArea::new();
        marquee.set_can_target(false);
        marquee.add_css_class("marquee-overlay");
        let marquee_rect: Rc<Cell<Option<(f64, f64, f64, f64)>>> = Rc::new(Cell::new(None));
        {
            let marquee_rect = marquee_rect.clone();
            marquee.set_draw_func(move |_, cr, _, _| {
                let Some((x, y, w, h)) = marquee_rect.get() else {
                    return;
                };
                cr.rectangle(x, y, w, h);
                cr.set_source_rgba(0.30, 0.55, 0.95, 0.20);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(0.30, 0.55, 0.95, 0.90);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            });
        }

        let root = Overlay::new();
        root.set_child(Some(&content_stack));
        root.add_overlay(&empty_state);
        root.add_overlay(&marquee);

        Self {
            root,
            icon_scroll,
            list_scroll,
            flow,
            list_box,
            content_stack,
            empty_state,
            marquee,
            marquee_rect,
            view_mode: Cell::new(ViewMode::Icons),
            show_shape_badges: Cell::new(true),
            thumb_targets: RefCell::new(Vec::new()),
            exit_timer: Rc::new(RefCell::new(None)),
            mark_refs: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn set_view_mode(&self, mode: ViewMode) {
        self.view_mode.set(mode);
        self.content_stack.set_visible_child_name(match mode {
            ViewMode::Icons => "icons",
            ViewMode::List => "list",
        });
    }

    pub fn set_selection_enabled(&self, enabled: bool) {
        let mode = if enabled {
            gtk::SelectionMode::Multiple
        } else {
            gtk::SelectionMode::None
        };
        self.flow.set_selection_mode(mode);
        self.list_box.set_selection_mode(mode);
    }

    pub fn set_shape_badges_visible(&self, visible: bool) {
        if self.show_shape_badges.get() == visible {
            return;
        }
        self.show_shape_badges.set(visible);
        for mark_ref in self.mark_refs.borrow().iter() {
            set_icon_badge_visible(
                &mark_ref.icon_overlay,
                mark_ref.shape,
                mark_ref.tint_color.as_deref(),
                visible,
            );
            set_list_badge_visible(
                &mark_ref.list_overlay,
                mark_ref.shape,
                mark_ref.tint_color.as_deref(),
                visible,
            );
        }
    }

    pub fn grab_focus_on_active(&self) {
        match self.view_mode.get() {
            ViewMode::Icons => {
                self.flow.grab_focus();
            }
            ViewMode::List => {
                self.list_box.grab_focus();
            }
        }
    }

    fn cancel_exit_timer(&self) {
        if let Some(id) = self.exit_timer.borrow_mut().take() {
            id.remove();
        }
    }

    pub fn set_loading(&self) {
        self.cancel_exit_timer();
        self.thumb_targets.borrow_mut().clear();

        let has_icon_children = self.flow.child_at_index(0).is_some();
        let has_list_children = self.list_box.row_at_index(0).is_some();

        if has_icon_children || has_list_children {
            let mut idx = 0;
            while let Some(child) = self.flow.child_at_index(idx) {
                child.add_css_class("card-exit");
                idx += 1;
            }
            let mut idx = 0;
            while let Some(row) = self.list_box.row_at_index(idx) {
                row.add_css_class("card-exit");
                idx += 1;
            }
            let grid = self.clone();
            let id = glib::timeout_add_local_once(Duration::from_millis(80), move || {
                *grid.exit_timer.borrow_mut() = None;
                grid.clear();
                grid.empty_state.set_label("Loading folder…");
                grid.empty_state.set_visible(true);
            });
            *self.exit_timer.borrow_mut() = Some(id);
        } else {
            self.clear();
            self.empty_state.set_label("Loading folder…");
            self.empty_state.set_visible(true);
        }
    }

    pub fn set_empty_message(&self, message: &str) {
        self.cancel_exit_timer();
        self.clear();
        self.empty_state.set_label(message);
        self.empty_state.set_visible(true);
    }

    pub fn set_items(&self, items: &[FileItem]) {
        self.cancel_exit_timer();
        self.clear();
        self.empty_state.set_visible(items.is_empty());
        if items.is_empty() {
            self.empty_state.set_label("This folder is empty");
            return;
        }

        let mut targets = Vec::new();
        let mut refs: Vec<MarkRef> = Vec::with_capacity(items.len());
        let show_shape_badges = self.show_shape_badges.get();

        for (index, item) in items.iter().enumerate() {
            let (card, icon_overlay, icon_tags, target) = build_card(item, show_shape_badges);
            card.add_css_class("card-anim");
            card.add_css_class(&format!("card-delay-{}", index.min(15)));
            // card is the shell; the actual .file-card box is its first child
            let icon_card = card
                .first_child()
                .and_then(|w| w.downcast::<Box>().ok())
                .unwrap_or_else(|| card.clone());
            self.flow.append(&card);
            if let Some(t) = target {
                targets.push(t);
            }
            // Build the list row here too so we can store both refs together
            let (row, list_overlay, list_tags) = build_list_row(item, show_shape_badges);
            row.add_css_class("list-row-anim");
            row.add_css_class(&format!("card-delay-{}", index.min(15)));
            self.list_box.append(&row);
            refs.push(MarkRef {
                icon_card,
                icon_overlay,
                icon_tags,
                list_row: row,
                list_overlay,
                list_tags,
                shape: item.mark_shape,
                tint_color: item.mark_tint_color.clone(),
            });
        }
        *self.thumb_targets.borrow_mut() = targets;
        *self.mark_refs.borrow_mut() = refs;
    }

    /// Update the mark CSS classes and shape badge for a single item in place.
    pub fn update_item_mark(
        &self,
        index: usize,
        tint_id: i64,
        tint_color: Option<&str>,
        shape: Shape,
    ) {
        let mut refs = self.mark_refs.borrow_mut();
        let Some(r) = refs.get_mut(index) else { return };
        update_mark_css(&r.icon_card, tint_id, shape);
        update_mark_css(&r.list_row, tint_id, shape);
        r.shape = shape;
        r.tint_color = tint_color.map(ToOwned::to_owned);
        if self.show_shape_badges.get() {
            replace_icon_badge(&r.icon_overlay, shape, tint_color);
            replace_list_badge(&r.list_overlay, shape, tint_color);
        } else {
            remove_shape_badge(&r.icon_overlay);
            remove_shape_badge(&r.list_overlay);
        }
    }

    /// Update the tag chips for a single item in place.
    pub fn update_item_tags(&self, index: usize, tags: &[TagRecord]) {
        let refs = self.mark_refs.borrow();
        let Some(r) = refs.get(index) else { return };
        rebuild_tag_chips(&r.icon_tags, tags, 1);
        rebuild_tag_chips(&r.list_tags, tags, 2);
    }

    /// Take all pending thumbnail targets out of this grid so the caller can
    /// submit them to a ThumbnailLoader. Leaves the internal list empty.
    pub fn drain_thumb_targets(&self) -> Vec<ThumbnailTarget> {
        self.thumb_targets.borrow_mut().drain(..).collect()
    }

    pub fn clear_selection(&self) {
        self.flow.unselect_all();
        self.list_box.unselect_all();
    }

    pub fn select_only_index(&self, index: i32) {
        self.flow.unselect_all();
        self.list_box.unselect_all();
        match self.view_mode.get() {
            ViewMode::Icons => {
                if let Some(child) = self.flow.child_at_index(index) {
                    self.flow.select_child(&child);
                }
            }
            ViewMode::List => {
                if let Some(row) = self.list_box.row_at_index(index) {
                    self.list_box.select_row(Some(&row));
                }
            }
        }
    }

    pub fn select_range(&self, start: i32, end: i32, clear_first: bool) {
        if clear_first {
            self.flow.unselect_all();
            self.list_box.unselect_all();
        }

        let (from, to) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };

        match self.view_mode.get() {
            ViewMode::Icons => {
                for index in from..=to {
                    if let Some(child) = self.flow.child_at_index(index) {
                        self.flow.select_child(&child);
                    }
                }
            }
            ViewMode::List => {
                for index in from..=to {
                    if let Some(row) = self.list_box.row_at_index(index) {
                        self.list_box.select_row(Some(&row));
                    }
                }
            }
        }
    }

    pub fn toggle_index(&self, index: i32) {
        match self.view_mode.get() {
            ViewMode::Icons => {
                if let Some(child) = self.flow.child_at_index(index) {
                    if child.is_selected() {
                        self.flow.unselect_child(&child);
                    } else {
                        self.flow.select_child(&child);
                    }
                }
            }
            ViewMode::List => {
                if let Some(row) = self.list_box.row_at_index(index) {
                    if row.is_selected() {
                        self.list_box.unselect_row(&row);
                    } else {
                        self.list_box.select_row(Some(&row));
                    }
                }
            }
        }
    }

    /// Replace the current selection with exactly the given indices.
    /// Used by the marquee gesture to apply rubber-band hits each drag update.
    pub fn set_selected_indices(&self, indices: &[i32]) {
        match self.view_mode.get() {
            ViewMode::Icons => {
                self.flow.unselect_all();
                for &index in indices {
                    if let Some(child) = self.flow.child_at_index(index) {
                        self.flow.select_child(&child);
                    }
                }
            }
            ViewMode::List => {
                self.list_box.unselect_all();
                for &index in indices {
                    if let Some(row) = self.list_box.row_at_index(index) {
                        self.list_box.select_row(Some(&row));
                    }
                }
            }
        }
    }

    /// Return the indices of items whose bounds intersect `rect`, expressed in the
    /// active container's (FlowBox/ListBox) coordinate space.
    pub fn children_in_rect(&self, rect: &gtk::graphene::Rect) -> Vec<i32> {
        let mut hits = Vec::new();
        match self.view_mode.get() {
            ViewMode::Icons => {
                let container: gtk::Widget = self.flow.clone().upcast();
                let mut idx = 0;
                while let Some(child) = self.flow.child_at_index(idx) {
                    if let Some(bounds) = child.compute_bounds(&container) {
                        if bounds.intersection(rect).is_some() {
                            hits.push(idx);
                        }
                    }
                    idx += 1;
                }
            }
            ViewMode::List => {
                let container: gtk::Widget = self.list_box.clone().upcast();
                let mut idx = 0;
                while let Some(row) = self.list_box.row_at_index(idx) {
                    if let Some(bounds) = row.compute_bounds(&container) {
                        if bounds.intersection(rect).is_some() {
                            hits.push(idx);
                        }
                    }
                    idx += 1;
                }
            }
        }
        hits
    }

    /// The widget of the active view's scrollable container, so callers can map
    /// gesture coordinates to overlay coordinates via `compute_point`.
    pub fn active_container(&self) -> gtk::Widget {
        match self.view_mode.get() {
            ViewMode::Icons => self.flow.clone().upcast(),
            ViewMode::List => self.list_box.clone().upcast(),
        }
    }

    /// Set (or clear) the marquee rectangle to paint, in `root`/overlay coordinates.
    pub fn set_marquee_rect(&self, rect: Option<(f64, f64, f64, f64)>) {
        self.marquee_rect.set(rect);
        self.marquee.queue_draw();
    }

    pub fn focus_index(&self, index: i32) {
        match self.view_mode.get() {
            ViewMode::Icons => {
                if let Some(child) = self.flow.child_at_index(index) {
                    child.grab_focus();
                } else {
                    self.flow.grab_focus();
                }
            }
            ViewMode::List => {
                if let Some(row) = self.list_box.row_at_index(index) {
                    row.grab_focus();
                } else {
                    self.list_box.grab_focus();
                }
            }
        }
    }

    pub fn child_count(&self) -> i32 {
        match self.view_mode.get() {
            ViewMode::Icons => self.flow.observe_children().n_items() as i32,
            ViewMode::List => self.list_box.observe_children().n_items() as i32,
        }
    }

    pub fn estimated_columns(&self) -> i32 {
        if self.view_mode.get() == ViewMode::List {
            return 1;
        }
        let width = self.flow.width();
        if width <= 0 {
            return 1;
        }

        let usable_width = (width - FILE_GRID_MARGIN * 2).max(FILE_CARD_WIDTH);
        let spacing = FILE_GRID_COLUMN_SPACING as i32;
        ((usable_width + spacing) / (FILE_CARD_WIDTH + spacing)).max(1)
    }

    pub fn selected_indices(&self) -> Vec<i32> {
        match self.view_mode.get() {
            ViewMode::Icons => self
                .flow
                .selected_children()
                .into_iter()
                .map(|child| child.index())
                .filter(|index| *index >= 0)
                .collect(),
            ViewMode::List => self
                .list_box
                .selected_rows()
                .into_iter()
                .map(|row| row.index())
                .filter(|index| *index >= 0)
                .collect(),
        }
    }

    fn clear(&self) {
        while let Some(child) = self.flow.child_at_index(0) {
            self.flow.remove(&child);
        }
        while let Some(row) = self.list_box.row_at_index(0) {
            self.list_box.remove(&row);
        }
        self.mark_refs.borrow_mut().clear();
    }
}

fn build_card(
    file: &FileItem,
    show_shape_badge: bool,
) -> (Box, Overlay, Box, Option<ThumbnailTarget>) {
    let shell = Box::new(Orientation::Vertical, 0);
    shell.add_css_class("file-card-shell");
    shell.set_size_request(FILE_CARD_WIDTH, FILE_CARD_HEIGHT);
    shell.set_hexpand(false);
    shell.set_halign(Align::Center);
    shell.set_valign(Align::Start);

    let card = Box::new(Orientation::Vertical, 2);
    card.add_css_class("file-card");
    card.add_css_class(file.kind.css_class());
    card.add_css_class(&format!("mark-tint-{}", file.mark_tint_id));
    card.add_css_class(&format!("mark-shape-{}", file.mark_shape.as_str()));
    card.set_size_request(FILE_CARD_WIDTH, FILE_CARD_HEIGHT);
    card.set_hexpand(false);
    card.set_vexpand(true);
    card.set_halign(Align::Center);
    card.set_valign(Align::Start);
    shell.append(&card);

    let media = Box::new(Orientation::Vertical, 0);
    media.add_css_class("file-card-media");
    media.set_size_request(FILE_CARD_MEDIA_SIZE, FILE_CARD_MEDIA_SIZE);
    media.set_halign(Align::Fill);
    media.set_valign(Align::Start);

    // For image, video, and audio files slot a Stack so we can crossfade in
    // the real thumbnail once it loads. Other kinds use a plain emoji badge.
    let thumb_kind = match file.kind {
        FileKind::Image => Some(ThumbnailKind::Image),
        FileKind::Video => Some(ThumbnailKind::Video),
        FileKind::Audio => Some(ThumbnailKind::Audio),
        _ => None,
    };

    let thumb_target = if let Some(kind) = thumb_kind {
        let badge = Label::new(Some(file.kind.badge()));
        badge.add_css_class("file-card-icon");
        badge.set_halign(Align::Center);
        badge.set_valign(Align::Center);
        badge.set_size_request(FILE_CARD_MEDIA_SIZE, FILE_CARD_MEDIA_SIZE);

        let picture = Picture::new();
        picture.add_css_class("file-thumb");
        picture.set_size_request(FILE_CARD_THUMB_SIZE, FILE_CARD_THUMB_SIZE);
        picture.set_content_fit(gtk::ContentFit::Cover);
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
            kind,
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

    // Use the full card content width so the shape badge can occupy the
    // upper-right card space instead of covering the icon/thumbnail.
    let media_overlay = Overlay::new();
    media_overlay.set_size_request(FILE_CARD_MEDIA_OVERLAY_WIDTH, FILE_CARD_MEDIA_SIZE);
    media_overlay.set_halign(Align::Center);
    media_overlay.set_valign(Align::Start);
    media_overlay.set_child(Some(&media));

    if show_shape_badge {
        add_icon_badge(
            &media_overlay,
            file.mark_shape,
            file.mark_tint_color.as_deref(),
        );
    }

    card.append(&media_overlay);

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

    if let Some(detail_text) = &file.detail {
        let detail = Label::new(Some(detail_text));
        detail.add_css_class("file-card-detail");
        detail.set_halign(gtk::Align::Center);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.set_single_line_mode(true);
        detail.set_width_chars(FILE_CARD_NAME_MAX_WIDTH_CHARS);
        detail.set_max_width_chars(FILE_CARD_NAME_MAX_WIDTH_CHARS);
        detail.set_size_request(FILE_CARD_WIDTH - 10, -1);
        detail.set_justify(gtk::Justification::Center);
        card.append(&detail);
    }

    let tags = Box::new(Orientation::Horizontal, 4);
    tags.add_css_class("file-card-tags");
    tags.set_halign(gtk::Align::Center);
    tags.set_size_request(-1, FILE_CARD_TAGS_HEIGHT);
    rebuild_tag_chips(&tags, &file.tags, 1);
    card.append(&tags);

    (shell, media_overlay, tags, thumb_target)
}

fn build_list_row(file: &FileItem, show_shape_badge: bool) -> (ListBoxRow, Overlay, Box) {
    let row = ListBoxRow::new();
    row.add_css_class("file-list-row");
    row.add_css_class(file.kind.css_class());
    row.add_css_class(&format!("mark-tint-{}", file.mark_tint_id));
    row.add_css_class(&format!("mark-shape-{}", file.mark_shape.as_str()));

    let inner = Box::new(Orientation::Horizontal, 0);
    inner.add_css_class("file-list-row-inner");
    inner.set_hexpand(true);

    let icon = Label::new(Some(file.kind.badge()));
    icon.add_css_class("file-list-icon");
    icon.set_halign(Align::Center);
    icon.set_valign(Align::Center);
    icon.set_size_request(36, -1);
    icon.set_xalign(0.5);

    let icon_overlay = Overlay::new();
    icon_overlay.set_size_request(36, -1);
    icon_overlay.set_valign(Align::Fill);
    icon_overlay.set_child(Some(&icon));

    if show_shape_badge {
        add_list_badge(
            &icon_overlay,
            file.mark_shape,
            file.mark_tint_color.as_deref(),
        );
    }

    inner.append(&icon_overlay);

    let name_box = Box::new(Orientation::Vertical, 1);
    name_box.set_hexpand(true);
    name_box.set_halign(Align::Fill);
    name_box.set_valign(Align::Center);
    name_box.set_size_request(1, -1);

    let name = Label::new(Some(&file.name));
    name.add_css_class("file-list-name");
    name.set_halign(Align::Start);
    name.set_valign(Align::Center);
    name.set_size_request(1, -1);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_single_line_mode(true);
    name.set_xalign(0.0);
    name_box.append(&name);

    if let Some(detail_text) = &file.detail {
        let detail = Label::new(Some(detail_text));
        detail.add_css_class("file-list-detail");
        detail.set_halign(Align::Start);
        detail.set_valign(Align::Center);
        detail.set_size_request(1, -1);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.set_single_line_mode(true);
        detail.set_xalign(0.0);
        name_box.append(&detail);
    }

    inner.append(&name_box);

    let size_text = if file.is_dir {
        String::new()
    } else {
        file.size_bytes
            .map(format_file_size_list)
            .unwrap_or_default()
    };
    let size = Label::new(Some(&size_text));
    size.add_css_class("file-list-size");
    size.set_halign(Align::End);
    size.set_valign(Align::Center);
    size.set_size_request(64, -1);
    size.set_ellipsize(gtk::pango::EllipsizeMode::End);
    size.set_single_line_mode(true);
    size.set_xalign(1.0);
    inner.append(&size);

    let date_text = file
        .modified_unix
        .and_then(|unix| {
            glib::DateTime::from_unix_local(unix)
                .ok()
                .and_then(|dt| dt.format("%Y-%m-%d %H:%M").ok())
                .map(|gs| gs.to_string())
        })
        .unwrap_or_default();
    let date = Label::new(Some(&date_text));
    date.add_css_class("file-list-date");
    date.set_halign(Align::End);
    date.set_valign(Align::Center);
    date.set_size_request(116, -1);
    date.set_ellipsize(gtk::pango::EllipsizeMode::End);
    date.set_single_line_mode(true);
    date.set_xalign(1.0);
    inner.append(&date);

    let list_tags = Box::new(Orientation::Horizontal, 4);
    list_tags.add_css_class("file-list-tags");
    rebuild_tag_chips(&list_tags, &file.tags, 2);
    name_box.append(&list_tags);

    row.set_child(Some(&inner));
    (row, icon_overlay, list_tags)
}

// ── Tag chip helpers ───────────────────────────────────────────────────────────

fn rebuild_tag_chips(container: &Box, tags: &[TagRecord], max_shown: usize) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    if tags.is_empty() {
        container.set_visible(false);
        return;
    }
    container.set_visible(true);
    for tag in tags.iter().take(max_shown) {
        let chip = Label::new(Some(&tag.name));
        chip.add_css_class("file-tag-chip");
        container.append(&chip);
    }
    if tags.len() > max_shown {
        let overflow = Label::new(Some(&format!("+{}", tags.len() - max_shown)));
        overflow.add_css_class("file-tag-chip");
        overflow.add_css_class("file-tag-chip-muted");
        container.append(&overflow);
    }
}

// ── Mark CSS and badge helpers ─────────────────────────────────────────────────

fn update_mark_css(widget: &impl gtk::prelude::WidgetExt, tint_id: i64, shape: Shape) {
    for class in widget.css_classes() {
        let s = class.as_str();
        if s.starts_with("mark-tint-") || s.starts_with("mark-shape-") {
            widget.remove_css_class(s);
        }
    }
    widget.add_css_class(&format!("mark-tint-{tint_id}"));
    widget.add_css_class(&format!("mark-shape-{}", shape.as_str()));
}

fn replace_icon_badge(overlay: &Overlay, shape: Shape, tint_color: Option<&str>) {
    remove_shape_badge(overlay);
    add_icon_badge(overlay, shape, tint_color);
}

fn replace_list_badge(overlay: &Overlay, shape: Shape, tint_color: Option<&str>) {
    remove_shape_badge(overlay);
    add_list_badge(overlay, shape, tint_color);
}

fn set_icon_badge_visible(
    overlay: &Overlay,
    shape: Shape,
    tint_color: Option<&str>,
    visible: bool,
) {
    if visible {
        replace_icon_badge(overlay, shape, tint_color);
    } else {
        remove_shape_badge(overlay);
    }
}

fn set_list_badge_visible(
    overlay: &Overlay,
    shape: Shape,
    tint_color: Option<&str>,
    visible: bool,
) {
    if visible {
        replace_list_badge(overlay, shape, tint_color);
    } else {
        remove_shape_badge(overlay);
    }
}

fn remove_shape_badge(overlay: &Overlay) {
    let mut child = overlay.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        if w.widget_name() == "shape-badge" {
            overlay.remove_overlay(&w);
            break;
        }
        child = next;
    }
}

fn add_icon_badge(overlay: &Overlay, shape: Shape, tint_color: Option<&str>) {
    let badge = make_shape_badge(shape, MARK_BADGE_SIZE_CARD, tint_color);
    badge.set_widget_name("shape-badge");
    badge.set_halign(Align::End);
    badge.set_valign(Align::Start);
    badge.set_margin_end(3);
    badge.set_margin_top(3);
    overlay.add_overlay(&badge);
}

fn add_list_badge(overlay: &Overlay, shape: Shape, tint_color: Option<&str>) {
    let badge = make_shape_badge(shape, MARK_BADGE_SIZE_LIST, tint_color);
    badge.set_widget_name("shape-badge");
    badge.set_halign(Align::End);
    badge.set_valign(Align::End);
    badge.set_margin_end(2);
    badge.set_margin_bottom(2);
    overlay.add_overlay(&badge);
}

// ── Mark badge constants ───────────────────────────────────────────────────────

const MARK_BADGE_SIZE_CARD: i32 = 13;
const MARK_BADGE_SIZE_LIST: i32 = 10;

fn format_file_size_list(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else {
        format!("{size:.1}{}", UNITS[unit])
    }
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
