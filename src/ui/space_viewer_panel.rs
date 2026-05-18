use crate::ui::file_grid::FileKind;
use gio::FileType;
use gtk::cairo;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, DrawingArea, EventControllerMotion, GestureClick, Label,
    Orientation, Popover, ProgressBar, ScrolledWindow, Spinner, Stack,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ─── theme colors ─────────────────────────────────────────────────────────────

const KIND_COLORS: &[(FileKind, (f64, f64, f64))] = &[
    (FileKind::Folder, (0.788, 0.588, 0.180)),
    (FileKind::Image, (0.784, 0.251, 0.439)),
    (FileKind::Video, (0.502, 0.251, 0.627)),
    (FileKind::Audio, (0.553, 0.357, 0.753)),
    (FileKind::Document, (0.616, 0.451, 0.322)),
    (FileKind::Text, (0.314, 0.502, 0.722)),
    (FileKind::Archive, (0.722, 0.537, 0.125)),
    (FileKind::ConfigCode, (0.239, 0.502, 0.376)),
    (FileKind::Unknown, (0.502, 0.376, 0.251)),
];

fn kind_color(kind: &FileKind) -> (f64, f64, f64) {
    KIND_COLORS
        .iter()
        .find(|(k, _)| k == kind)
        .map(|(_, c)| *c)
        .unwrap_or((0.502, 0.376, 0.251))
}

// ─── views ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpaceView {
    Breakdown,
    TopItems,
    Categories,
}

impl SpaceView {
    const ALL: &'static [SpaceView] = &[Self::Breakdown, Self::TopItems, Self::Categories];

    fn label(self) -> &'static str {
        match self {
            Self::Breakdown => "BREAKDOWN",
            Self::TopItems => "TOP ITEMS",
            Self::Categories => "CATEGORIES",
        }
    }

    fn stack_name(self) -> &'static str {
        match self {
            Self::Breakdown => "breakdown",
            Self::TopItems => "top_items",
            Self::Categories => "categories",
        }
    }
}

// ─── data types ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanDepth {
    FolderOnly,
    OneLevelDeep,
    FullRecursive,
}

#[derive(Clone, Debug)]
pub struct CategoryStat {
    pub kind: FileKind,
    pub bytes: u64,
    pub count: u64,
}

#[derive(Clone, Debug)]
pub struct FileStat {
    pub path: PathBuf,
    pub bytes: u64,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ScanResult {
    pub root: PathBuf,
    pub depth: ScanDepth,
    pub total_bytes: u64,
    pub file_count: u64,
    pub folder_count: u64,
    pub by_category: Vec<CategoryStat>,
    pub largest_files: Vec<FileStat>,
    pub largest_dirs: Vec<FileStat>,
    pub error_count: u32,
    pub cancelled: bool,
}

// ─── scanning ─────────────────────────────────────────────────────────────────

pub fn scan(root: PathBuf, depth: ScanDepth, cancel: Arc<AtomicBool>) -> ScanResult {
    let mut total_bytes: u64 = 0;
    let mut file_count: u64 = 0;
    let mut folder_count: u64 = 0;
    let mut error_count: u32 = 0;
    let mut cat_map: HashMap<u8, (FileKind, u64, u64)> = HashMap::new();
    let mut largest_files: Vec<FileStat> = Vec::new();
    let mut largest_dirs: Vec<FileStat> = Vec::new();

    match depth {
        ScanDepth::FolderOnly => scan_one_level(
            &root,
            false,
            &mut total_bytes,
            &mut file_count,
            &mut folder_count,
            &mut error_count,
            &mut cat_map,
            &mut largest_files,
            &mut largest_dirs,
        ),
        ScanDepth::OneLevelDeep => scan_one_level(
            &root,
            true,
            &mut total_bytes,
            &mut file_count,
            &mut folder_count,
            &mut error_count,
            &mut cat_map,
            &mut largest_files,
            &mut largest_dirs,
        ),
        ScanDepth::FullRecursive => scan_recursive(
            &root,
            0,
            &cancel,
            &mut total_bytes,
            &mut file_count,
            &mut folder_count,
            &mut error_count,
            &mut cat_map,
            &mut largest_files,
            &mut largest_dirs,
        ),
    }

    let cancelled = cancel.load(Ordering::Relaxed);

    let mut by_category: Vec<CategoryStat> = cat_map
        .into_values()
        .map(|(kind, bytes, count)| CategoryStat { kind, bytes, count })
        .collect();
    by_category.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    largest_files.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    largest_files.truncate(15);
    largest_dirs.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    largest_dirs.truncate(15);

    ScanResult {
        root,
        depth,
        total_bytes,
        file_count,
        folder_count,
        by_category,
        largest_files,
        largest_dirs,
        error_count,
        cancelled,
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_one_level(
    root: &Path,
    enumerate_subdirs: bool,
    total_bytes: &mut u64,
    file_count: &mut u64,
    folder_count: &mut u64,
    error_count: &mut u32,
    cat_map: &mut HashMap<u8, (FileKind, u64, u64)>,
    largest_files: &mut Vec<FileStat>,
    largest_dirs: &mut Vec<FileStat>,
) {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => {
            *error_count += 1;
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                *error_count += 1;
                continue;
            }
        };
        if meta.is_symlink() {
            if let Ok(tm) = std::fs::metadata(&path) {
                if tm.is_file() {
                    let sz = tm.len();
                    *total_bytes += sz;
                    *file_count += 1;
                    let kind = classify(&path);
                    accum_cat(cat_map, &kind, sz);
                    push_file(largest_files, &path, sz);
                }
            }
        } else if meta.is_dir() {
            *folder_count += 1;
            let dir_size = if enumerate_subdirs {
                sum_dir_one_level(&path, error_count, cat_map, largest_files, file_count)
            } else {
                0
            };
            *total_bytes += dir_size;
            push_dir(largest_dirs, &path, dir_size);
        } else {
            let sz = meta.len();
            *total_bytes += sz;
            *file_count += 1;
            let kind = classify(&path);
            accum_cat(cat_map, &kind, sz);
            push_file(largest_files, &path, sz);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sum_dir_one_level(
    dir: &Path,
    error_count: &mut u32,
    cat_map: &mut HashMap<u8, (FileKind, u64, u64)>,
    largest_files: &mut Vec<FileStat>,
    file_count: &mut u64,
) -> u64 {
    let mut total = 0u64;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => {
            *error_count += 1;
            return 0;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                *error_count += 1;
                continue;
            }
        };
        if meta.is_file() {
            let sz = meta.len();
            total += sz;
            *file_count += 1;
            let kind = classify(&path);
            accum_cat(cat_map, &kind, sz);
            push_file(largest_files, &path, sz);
        } else if meta.is_symlink() {
            if let Ok(tm) = std::fs::metadata(&path) {
                if tm.is_file() {
                    let sz = tm.len();
                    total += sz;
                    *file_count += 1;
                    let kind = classify(&path);
                    accum_cat(cat_map, &kind, sz);
                    push_file(largest_files, &path, sz);
                }
            }
        }
    }
    total
}

#[allow(clippy::too_many_arguments)]
fn scan_recursive(
    dir: &Path,
    depth: u32,
    cancel: &Arc<AtomicBool>,
    total_bytes: &mut u64,
    file_count: &mut u64,
    folder_count: &mut u64,
    error_count: &mut u32,
    cat_map: &mut HashMap<u8, (FileKind, u64, u64)>,
    largest_files: &mut Vec<FileStat>,
    largest_dirs: &mut Vec<FileStat>,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => {
            *error_count += 1;
            return;
        }
    };
    for entry in entries.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                *error_count += 1;
                continue;
            }
        };
        if meta.is_symlink() {
            if let Ok(tm) = std::fs::metadata(&path) {
                if tm.is_file() {
                    let sz = tm.len();
                    *total_bytes += sz;
                    *file_count += 1;
                    let kind = classify(&path);
                    accum_cat(cat_map, &kind, sz);
                    push_file(largest_files, &path, sz);
                }
            }
        } else if meta.is_dir() {
            *folder_count += 1;
            let before = *total_bytes;
            scan_recursive(
                &path,
                depth + 1,
                cancel,
                total_bytes,
                file_count,
                folder_count,
                error_count,
                cat_map,
                largest_files,
                largest_dirs,
            );
            let dir_size = total_bytes.saturating_sub(before);
            if depth == 0 {
                push_dir(largest_dirs, &path, dir_size);
            }
        } else {
            let sz = meta.len();
            *total_bytes += sz;
            *file_count += 1;
            let kind = classify(&path);
            accum_cat(cat_map, &kind, sz);
            push_file(largest_files, &path, sz);
        }
    }
}

fn classify(path: &Path) -> FileKind {
    FileKind::from_path(path, gio_file_type(path), None)
}

fn gio_file_type(path: &Path) -> FileType {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => FileType::Directory,
        Ok(m) if m.is_file() => FileType::Regular,
        _ => FileType::Unknown,
    }
}

fn accum_cat(map: &mut HashMap<u8, (FileKind, u64, u64)>, kind: &FileKind, sz: u64) {
    let entry = map
        .entry(kind.sort_key())
        .or_insert_with(|| (kind.clone(), 0, 0));
    entry.1 += sz;
    entry.2 += 1;
}

fn push_file(list: &mut Vec<FileStat>, path: &Path, bytes: u64) {
    if list.len() < 20 || list.iter().any(|f| f.bytes < bytes) {
        list.push(FileStat {
            path: path.to_path_buf(),
            bytes,
        });
        if list.len() > 20 {
            list.sort_by(|a, b| b.bytes.cmp(&a.bytes));
            list.truncate(20);
        }
    }
}

fn push_dir(list: &mut Vec<FileStat>, path: &Path, bytes: u64) {
    list.push(FileStat {
        path: path.to_path_buf(),
        bytes,
    });
}

// ─── formatting ───────────────────────────────────────────────────────────────

pub fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut idx = 0;
    while v >= 1024.0 && idx + 1 < UNITS.len() {
        v /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", v, UNITS[idx])
    }
}

// ─── panel ────────────────────────────────────────────────────────────────────

struct Callbacks {
    on_open: RefCell<Option<Box<dyn Fn(PathBuf)>>>,
    on_reveal: RefCell<Option<Box<dyn Fn(PathBuf)>>>,
    on_add_to_tray: RefCell<Option<Box<dyn Fn(Vec<PathBuf>)>>>,
    on_copy_path: RefCell<Option<Box<dyn Fn(PathBuf)>>>,
    on_trash: RefCell<Option<Box<dyn Fn(PathBuf)>>>,
}

struct Inner {
    // scan state
    current_root: RefCell<PathBuf>,
    current_depth: Cell<ScanDepth>,
    generation: Cell<u32>,
    cancel_flag: Rc<RefCell<Arc<AtomicBool>>>,

    // header
    path_label: Label,
    rescan_btn: Button,
    spinner: Spinner,

    // stats strip
    stats_box: GtkBox,
    total_lbl: Label,
    items_lbl: Label,
    files_lbl: Label,
    dirs_lbl: Label,
    error_lbl: Label,

    // depth buttons
    depth_folder_btn: Button,
    depth_level_btn: Button,
    depth_full_btn: Button,

    // view cycling
    current_view: Cell<SpaceView>,
    view_name_lbl: Label,
    view_prev_btn: Button,
    view_next_btn: Button,
    view_stack: Stack,

    // breakdown (pie)
    chart_area: DrawingArea,
    chart_segments: RefCell<Vec<(FileKind, f64)>>,
    chart_hover_info: RefCell<Vec<(FileKind, u64)>>,
    chart_total_bytes: Cell<u64>,
    hovered_segment: Cell<Option<usize>>,
    hover_name_lbl: Label,
    hover_size_lbl: Label,
    hover_hint_lbl: Label,
    legend_box: GtkBox,

    // top items
    files_list: GtkBox,
    dirs_list: GtkBox,

    // categories
    categories_box: GtkBox,

    cbs: Callbacks,
}

#[derive(Clone)]
pub struct SpaceViewerPanel {
    pub root: GtkBox,
    inner: Rc<Inner>,
}

impl SpaceViewerPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("sv-panel");
        root.set_visible(false);
        root.set_vexpand(true);

        // ── header ──────────────────────────────────────────────────────────
        let header = GtkBox::new(Orientation::Horizontal, 0);
        header.add_css_class("sv-header");
        let title_col = GtkBox::new(Orientation::Vertical, 2);
        title_col.set_hexpand(true);
        let title = Label::new(Some("SPACE VIEWER"));
        title.add_css_class("sv-title");
        title.set_halign(Align::Start);
        let path_label = Label::new(Some(""));
        path_label.add_css_class("sv-path-label");
        path_label.set_halign(Align::Start);
        path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title_col.append(&title);
        title_col.append(&path_label);
        header.append(&title_col);
        let spinner = Spinner::new();
        spinner.add_css_class("sv-spinner");
        spinner.set_visible(false);
        spinner.set_valign(Align::Center);
        header.append(&spinner);
        let rescan_btn = Button::from_icon_name("view-refresh-symbolic");
        rescan_btn.add_css_class("sv-rescan-btn");
        rescan_btn.add_css_class("pane-view-btn");
        rescan_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(&rescan_btn, "Re-scan folder");
        header.append(&rescan_btn);
        root.append(&header);

        // ── stats strip ─────────────────────────────────────────────────────
        let stats_box = GtkBox::new(Orientation::Vertical, 4);
        stats_box.add_css_class("sv-stat-strip");
        stats_box.set_visible(false);
        let total_lbl = Label::new(None);
        total_lbl.add_css_class("sv-stat-value");
        total_lbl.set_halign(Align::Start);
        let row2 = GtkBox::new(Orientation::Horizontal, 8);
        let items_lbl = Label::new(None);
        items_lbl.add_css_class("sv-stat-label");
        items_lbl.set_halign(Align::Start);
        let files_lbl = Label::new(None);
        files_lbl.add_css_class("sv-stat-label");
        files_lbl.set_halign(Align::Start);
        let dirs_lbl = Label::new(None);
        dirs_lbl.add_css_class("sv-stat-label");
        dirs_lbl.set_halign(Align::Start);
        row2.append(&items_lbl);
        row2.append(&files_lbl);
        row2.append(&dirs_lbl);
        let error_lbl = Label::new(None);
        error_lbl.add_css_class("sv-error-note");
        error_lbl.set_halign(Align::Start);
        error_lbl.set_visible(false);
        stats_box.append(&total_lbl);
        stats_box.append(&row2);
        stats_box.append(&error_lbl);
        root.append(&stats_box);

        // ── depth buttons ───────────────────────────────────────────────────
        let depth_row = GtkBox::new(Orientation::Horizontal, 6);
        depth_row.add_css_class("sv-depth-row");
        let depth_folder_btn = Button::with_label("Folder only");
        depth_folder_btn.add_css_class("sv-depth-btn");
        let depth_level_btn = Button::with_label("One level");
        depth_level_btn.add_css_class("sv-depth-btn");
        depth_level_btn.add_css_class("active");
        let depth_full_btn = Button::with_label("Full scan");
        depth_full_btn.add_css_class("sv-depth-btn");
        depth_row.append(&depth_folder_btn);
        depth_row.append(&depth_level_btn);
        depth_row.append(&depth_full_btn);
        root.append(&depth_row);

        // ── view nav ─────────────────────────────────────────────────────────
        let view_nav = GtkBox::new(Orientation::Horizontal, 0);
        view_nav.add_css_class("sv-view-nav");
        let view_prev_btn = Button::with_label("‹");
        view_prev_btn.add_css_class("sv-view-nav-btn");
        let view_name_lbl = Label::new(Some(SpaceView::Breakdown.label()));
        view_name_lbl.add_css_class("sv-view-nav-label");
        view_name_lbl.set_hexpand(true);
        view_name_lbl.set_halign(Align::Center);
        let view_next_btn = Button::with_label("›");
        view_next_btn.add_css_class("sv-view-nav-btn");
        view_nav.append(&view_prev_btn);
        view_nav.append(&view_name_lbl);
        view_nav.append(&view_next_btn);
        root.append(&view_nav);

        // ── per-view stack ───────────────────────────────────────────────────
        let view_stack = Stack::new();
        view_stack.set_vexpand(true);
        view_stack.set_transition_type(gtk::StackTransitionType::None);

        // BREAKDOWN
        let breakdown_box = GtkBox::new(Orientation::Vertical, 0);
        let chart_row = GtkBox::new(Orientation::Horizontal, 0);
        chart_row.add_css_class("sv-chart-row");
        let chart_area = DrawingArea::new();
        chart_area.set_content_width(180);
        chart_area.set_content_height(180);
        chart_area.set_halign(Align::Start);
        chart_area.set_valign(Align::Center);
        chart_area.set_margin_top(10);
        chart_area.set_margin_bottom(10);
        chart_area.set_margin_start(10);
        chart_row.append(&chart_area);
        let hover_col = GtkBox::new(Orientation::Vertical, 6);
        hover_col.set_hexpand(true);
        hover_col.set_valign(Align::Center);
        hover_col.set_halign(Align::Center);
        hover_col.set_margin_start(4);
        hover_col.set_margin_end(10);
        let hover_name_lbl = Label::new(None);
        hover_name_lbl.add_css_class("sv-hover-name");
        hover_name_lbl.set_halign(Align::Center);
        hover_name_lbl.set_wrap(true);
        hover_name_lbl.set_justify(gtk::Justification::Center);
        let hover_size_lbl = Label::new(None);
        hover_size_lbl.add_css_class("sv-hover-size");
        hover_size_lbl.set_halign(Align::Center);
        let hover_hint_lbl = Label::new(Some("Hover a slice\nfor details"));
        hover_hint_lbl.add_css_class("sv-hover-hint");
        hover_hint_lbl.set_halign(Align::Center);
        hover_hint_lbl.set_justify(gtk::Justification::Center);
        hover_col.append(&hover_name_lbl);
        hover_col.append(&hover_size_lbl);
        hover_col.append(&hover_hint_lbl);
        chart_row.append(&hover_col);
        breakdown_box.append(&chart_row);
        let legend_box = GtkBox::new(Orientation::Vertical, 2);
        legend_box.add_css_class("sv-legend");
        legend_box.set_margin_start(14);
        legend_box.set_margin_end(14);
        legend_box.set_margin_bottom(10);
        breakdown_box.append(&legend_box);
        view_stack.add_named(&scrolled_view(&breakdown_box), Some("breakdown"));

        // TOP ITEMS
        let top_items_box = GtkBox::new(Orientation::Vertical, 0);
        let files_heading = Label::new(Some("LARGEST FILES"));
        files_heading.add_css_class("sv-section-heading");
        files_heading.set_halign(Align::Start);
        top_items_box.append(&files_heading);
        let files_list = GtkBox::new(Orientation::Vertical, 0);
        top_items_box.append(&files_list);
        let dirs_heading = Label::new(Some("LARGEST SUBFOLDERS"));
        dirs_heading.add_css_class("sv-section-heading");
        dirs_heading.set_halign(Align::Start);
        top_items_box.append(&dirs_heading);
        let dirs_list = GtkBox::new(Orientation::Vertical, 0);
        dirs_list.set_margin_bottom(12);
        top_items_box.append(&dirs_list);
        view_stack.add_named(&scrolled_view(&top_items_box), Some("top_items"));

        // CATEGORIES
        let cat_outer = GtkBox::new(Orientation::Vertical, 0);
        let cat_heading = Label::new(Some("BY FILE TYPE"));
        cat_heading.add_css_class("sv-section-heading");
        cat_heading.set_halign(Align::Start);
        cat_outer.append(&cat_heading);
        let categories_box = GtkBox::new(Orientation::Vertical, 0);
        categories_box.set_margin_bottom(12);
        cat_outer.append(&categories_box);
        view_stack.add_named(&scrolled_view(&cat_outer), Some("categories"));

        root.append(&view_stack);

        let inner = Rc::new(Inner {
            current_root: RefCell::new(PathBuf::new()),
            current_depth: Cell::new(ScanDepth::OneLevelDeep),
            generation: Cell::new(0),
            cancel_flag: Rc::new(RefCell::new(Arc::new(AtomicBool::new(false)))),
            path_label,
            rescan_btn,
            spinner,
            stats_box,
            total_lbl,
            items_lbl,
            files_lbl,
            dirs_lbl,
            error_lbl,
            depth_folder_btn,
            depth_level_btn,
            depth_full_btn,
            current_view: Cell::new(SpaceView::Breakdown),
            view_name_lbl,
            view_prev_btn,
            view_next_btn,
            view_stack,
            chart_area,
            chart_segments: RefCell::new(Vec::new()),
            chart_hover_info: RefCell::new(Vec::new()),
            chart_total_bytes: Cell::new(0),
            hovered_segment: Cell::new(None),
            hover_name_lbl,
            hover_size_lbl,
            hover_hint_lbl,
            legend_box,
            files_list,
            dirs_list,
            categories_box,
            cbs: Callbacks {
                on_open: RefCell::new(None),
                on_reveal: RefCell::new(None),
                on_add_to_tray: RefCell::new(None),
                on_copy_path: RefCell::new(None),
                on_trash: RefCell::new(None),
            },
        });

        let panel = SpaceViewerPanel { root, inner };
        panel.wire_depth_buttons();
        panel.wire_rescan();
        panel.wire_view_nav();
        panel.wire_chart_hover();
        panel.wire_chart_draw();
        panel
    }

    // ── wiring ────────────────────────────────────────────────────────────────

    fn wire_depth_buttons(&self) {
        let p = self.clone();
        self.inner.depth_folder_btn.connect_clicked(move |_| {
            p.set_depth(ScanDepth::FolderOnly);
        });
        let p = self.clone();
        self.inner.depth_level_btn.connect_clicked(move |_| {
            p.set_depth(ScanDepth::OneLevelDeep);
        });
        let p = self.clone();
        self.inner.depth_full_btn.connect_clicked(move |_| {
            p.set_depth(ScanDepth::FullRecursive);
        });
    }

    fn wire_rescan(&self) {
        let p = self.clone();
        self.inner.rescan_btn.connect_clicked(move |_| {
            let root = p.inner.current_root.borrow().clone();
            if !root.as_os_str().is_empty() {
                p.start_scan(root);
            }
        });
    }

    fn wire_view_nav(&self) {
        let p = self.clone();
        self.inner.view_prev_btn.connect_clicked(move |_| {
            let all = SpaceView::ALL;
            let idx = all
                .iter()
                .position(|&v| v == p.inner.current_view.get())
                .unwrap_or(0);
            p.switch_view(all[(idx + all.len() - 1) % all.len()]);
        });
        let p = self.clone();
        self.inner.view_next_btn.connect_clicked(move |_| {
            let all = SpaceView::ALL;
            let idx = all
                .iter()
                .position(|&v| v == p.inner.current_view.get())
                .unwrap_or(0);
            p.switch_view(all[(idx + 1) % all.len()]);
        });
    }

    fn switch_view(&self, view: SpaceView) {
        self.inner.current_view.set(view);
        self.inner.view_name_lbl.set_label(view.label());
        self.inner
            .view_stack
            .set_visible_child_name(view.stack_name());
    }

    fn wire_chart_hover(&self) {
        let motion = EventControllerMotion::new();
        let p = self.clone();
        motion.connect_motion(move |_, mx, my| {
            let area = &p.inner.chart_area;
            let w = area.width() as f64;
            let h = area.height() as f64;
            let cx = w / 2.0;
            let cy = h / 2.0;
            let r = w.min(h) * 0.42;
            let dist = ((mx - cx).powi(2) + (my - cy).powi(2)).sqrt();
            if dist > r + 12.0 {
                if p.inner.hovered_segment.get().is_some() {
                    p.inner.hovered_segment.set(None);
                    p.clear_hover_labels();
                    area.queue_draw();
                }
                return;
            }
            let mut angle = (my - cy).atan2(mx - cx);
            if angle < -std::f64::consts::FRAC_PI_2 {
                angle += 2.0 * std::f64::consts::PI;
            }
            let mut a = angle + std::f64::consts::FRAC_PI_2;
            if a < 0.0 {
                a += 2.0 * std::f64::consts::PI;
            }
            let segs = p.inner.chart_segments.borrow();
            let mut prev_end = 0.0_f64;
            let mut found = None;
            for (i, (_, end)) in segs.iter().enumerate() {
                if a >= prev_end && a < *end {
                    found = Some(i);
                    break;
                }
                prev_end = *end;
            }
            let prev = p.inner.hovered_segment.get();
            p.inner.hovered_segment.set(found);
            if prev != found {
                p.update_hover_labels(found);
                area.queue_draw();
            }
        });
        let leave_p = self.clone();
        motion.connect_leave(move |_| {
            if leave_p.inner.hovered_segment.get().is_some() {
                leave_p.inner.hovered_segment.set(None);
                leave_p.clear_hover_labels();
                leave_p.inner.chart_area.queue_draw();
            }
        });
        self.inner.chart_area.add_controller(motion);
    }

    fn wire_chart_draw(&self) {
        let p = self.clone();
        self.inner.chart_area.set_draw_func(move |_, cr, w, h| {
            draw_pie(
                cr,
                w,
                h,
                &p.inner.chart_segments.borrow(),
                p.inner.hovered_segment.get(),
            );
        });
    }

    fn update_hover_labels(&self, idx: Option<usize>) {
        let Some(i) = idx else {
            self.clear_hover_labels();
            return;
        };
        let info = self.inner.chart_hover_info.borrow();
        let Some((kind, bytes)) = info.get(i) else {
            self.clear_hover_labels();
            return;
        };
        let total = self.inner.chart_total_bytes.get();
        let pct = if total > 0 {
            (bytes * 100).saturating_div(total)
        } else {
            0
        };
        self.inner
            .hover_name_lbl
            .set_label(&format!("{}  {}", kind.badge(), kind.label()));
        self.inner
            .hover_size_lbl
            .set_label(&format!("{}  ·  {}%", format_size(*bytes), pct));
        self.inner.hover_hint_lbl.set_visible(false);
    }

    fn clear_hover_labels(&self) {
        self.inner.hover_name_lbl.set_label("");
        self.inner.hover_size_lbl.set_label("");
        self.inner.hover_hint_lbl.set_visible(true);
    }

    fn set_depth(&self, depth: ScanDepth) {
        self.inner.current_depth.set(depth);
        let (f, l, r) = match depth {
            ScanDepth::FolderOnly => (true, false, false),
            ScanDepth::OneLevelDeep => (false, true, false),
            ScanDepth::FullRecursive => (false, false, true),
        };
        set_active_css(&self.inner.depth_folder_btn, f);
        set_active_css(&self.inner.depth_level_btn, l);
        set_active_css(&self.inner.depth_full_btn, r);
        let root = self.inner.current_root.borrow().clone();
        if !root.as_os_str().is_empty() {
            self.start_scan(root);
        }
    }

    pub fn set_folder(&self, path: &Path) {
        *self.inner.current_root.borrow_mut() = path.to_path_buf();
        self.inner.path_label.set_label(&path.display().to_string());
        self.start_scan(path.to_path_buf());
    }

    pub fn cancel_scan(&self) {
        self.inner
            .cancel_flag
            .borrow()
            .store(true, Ordering::Relaxed);
        self.inner
            .generation
            .set(self.inner.generation.get().wrapping_add(1));
        self.inner.spinner.stop();
        self.inner.spinner.set_visible(false);
    }

    fn start_scan(&self, root: PathBuf) {
        self.inner
            .cancel_flag
            .borrow()
            .store(true, Ordering::Relaxed);
        let new_flag = Arc::new(AtomicBool::new(false));
        *self.inner.cancel_flag.borrow_mut() = new_flag.clone();
        let gen = self.inner.generation.get().wrapping_add(1);
        self.inner.generation.set(gen);

        self.inner.spinner.set_visible(true);
        self.inner.spinner.start();
        self.inner.stats_box.set_visible(false);
        clear_box(&self.inner.files_list);
        clear_box(&self.inner.dirs_list);
        clear_box(&self.inner.legend_box);
        clear_box(&self.inner.categories_box);
        self.inner.chart_segments.borrow_mut().clear();
        self.inner.chart_area.queue_draw();

        let depth = self.inner.current_depth.get();
        let panel = self.clone();

        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || scan(root, depth, new_flag))
                .await
                .unwrap_or_else(|_| ScanResult {
                    root: PathBuf::new(),
                    depth,
                    total_bytes: 0,
                    file_count: 0,
                    folder_count: 0,
                    by_category: Vec::new(),
                    largest_files: Vec::new(),
                    largest_dirs: Vec::new(),
                    error_count: 0,
                    cancelled: true,
                });
            if panel.inner.generation.get() != gen {
                return;
            }
            panel.apply_result(result);
        });
    }

    fn apply_result(&self, result: ScanResult) {
        self.inner.spinner.stop();
        self.inner.spinner.set_visible(false);
        self.inner.stats_box.set_visible(true);

        let total = result.total_bytes;
        self.inner
            .total_lbl
            .set_label(&format!("Total: {}", format_size(total)));
        self.inner.items_lbl.set_label(&format!(
            "{}  items",
            result.file_count + result.folder_count
        ));
        self.inner
            .files_lbl
            .set_label(&format!("{}  files", result.file_count));
        self.inner
            .dirs_lbl
            .set_label(&format!("{}  folders", result.folder_count));

        if result.cancelled {
            self.inner.error_lbl.set_label("⚠ Scan cancelled.");
            self.inner.error_lbl.set_visible(true);
        } else if result.error_count > 0 {
            self.inner.error_lbl.set_label(&format!(
                "⚠ {} item{} skipped (permission denied)",
                result.error_count,
                if result.error_count == 1 { "" } else { "s" }
            ));
            self.inner.error_lbl.set_visible(true);
        } else {
            self.inner.error_lbl.set_visible(false);
        }

        // breakdown
        let mut segs: Vec<(FileKind, f64)> = Vec::new();
        let mut hover_info: Vec<(FileKind, u64)> = Vec::new();
        let mut cumulative = 0.0_f64;
        let full = 2.0 * std::f64::consts::PI;
        for cat in &result.by_category {
            if total == 0 || cat.bytes == 0 {
                continue;
            }
            cumulative += cat.bytes as f64 / total as f64 * full;
            segs.push((cat.kind.clone(), cumulative));
            hover_info.push((cat.kind.clone(), cat.bytes));
        }
        *self.inner.chart_segments.borrow_mut() = segs;
        *self.inner.chart_hover_info.borrow_mut() = hover_info;
        self.inner.chart_total_bytes.set(total);
        self.inner.hovered_segment.set(None);
        self.clear_hover_labels();
        self.inner.chart_area.queue_draw();

        clear_box(&self.inner.legend_box);
        for cat in &result.by_category {
            if cat.bytes == 0 {
                continue;
            }
            let pct = if total > 0 {
                cat.bytes * 100 / total
            } else {
                0
            };
            self.inner
                .legend_box
                .append(&build_legend_row(&cat.kind, pct, cat.bytes, cat.count));
        }

        // top items
        clear_box(&self.inner.files_list);
        let max_file = result
            .largest_files
            .first()
            .map(|f| f.bytes)
            .unwrap_or(1)
            .max(1);
        for entry in &result.largest_files {
            self.inner
                .files_list
                .append(&self.build_rank_row(entry, max_file, false));
        }
        if result.largest_files.is_empty() {
            self.inner.files_list.append(&empty_hint("No files found."));
        }
        clear_box(&self.inner.dirs_list);
        let max_dir = result
            .largest_dirs
            .first()
            .map(|f| f.bytes)
            .unwrap_or(1)
            .max(1);
        for entry in &result.largest_dirs {
            self.inner
                .dirs_list
                .append(&self.build_rank_row(entry, max_dir, true));
        }
        if result.largest_dirs.is_empty() {
            self.inner
                .dirs_list
                .append(&empty_hint("No subfolders found."));
        }

        // categories
        clear_box(&self.inner.categories_box);
        let max_cat = result
            .by_category
            .first()
            .map(|c| c.bytes)
            .unwrap_or(1)
            .max(1);
        for cat in &result.by_category {
            if cat.bytes == 0 {
                continue;
            }
            self.inner.categories_box.append(&build_category_row(
                &cat.kind, cat.bytes, cat.count, max_cat,
            ));
        }
        if result.by_category.is_empty() {
            self.inner
                .categories_box
                .append(&empty_hint("No files found."));
        }
    }

    fn build_rank_row(&self, entry: &FileStat, max_bytes: u64, is_dir: bool) -> GtkBox {
        let path = entry.path.clone();
        let bytes = entry.bytes;
        let row = GtkBox::new(Orientation::Vertical, 2);
        row.add_css_class("sv-bar-row");

        let top = GtkBox::new(Orientation::Horizontal, 6);
        let icon = if is_dir {
            "📁"
        } else {
            classify(&path).badge()
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let icon_lbl = Label::new(Some(icon));
        let name_lbl = Label::new(Some(&name));
        name_lbl.add_css_class("sv-bar-name");
        name_lbl.set_halign(Align::Start);
        name_lbl.set_hexpand(true);
        name_lbl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let size_lbl = Label::new(Some(&format_size(bytes)));
        size_lbl.add_css_class("sv-bar-size");
        top.append(&icon_lbl);
        top.append(&name_lbl);
        top.append(&size_lbl);
        row.append(&top);

        let bar = ProgressBar::new();
        bar.add_css_class("sv-bar-progress");
        bar.set_fraction(bytes as f64 / max_bytes as f64);
        bar.set_margin_start(20);
        row.append(&bar);

        let actions = GtkBox::new(Orientation::Horizontal, 0);
        actions.set_margin_start(20);

        macro_rules! act {
            ($label:expr, $cb:ident, $p:expr) => {{
                let btn = action_btn($label);
                let p = self.clone();
                let pp = $p.clone();
                btn.connect_clicked(move |_| {
                    if let Some(f) = &*p.inner.cbs.$cb.borrow() {
                        f(pp.clone());
                    }
                });
                btn
            }};
        }
        actions.append(&act!("Open", on_open, path));
        actions.append(&act!("Reveal", on_reveal, path));
        {
            let btn = action_btn("+ Tray");
            let p = self.clone();
            let pp = path.clone();
            btn.connect_clicked(move |_| {
                if let Some(f) = &*p.inner.cbs.on_add_to_tray.borrow() {
                    f(vec![pp.clone()]);
                }
            });
            actions.append(&btn);
        }
        actions.append(&act!("Copy Path", on_copy_path, path));
        {
            let btn = action_btn("Trash");
            btn.add_css_class("sv-action-btn-danger");
            let p = self.clone();
            let pp = path.clone();
            btn.connect_clicked(move |_| {
                if let Some(f) = &*p.inner.cbs.on_trash.borrow() {
                    f(pp.clone());
                }
            });
            actions.append(&btn);
        }
        row.append(&actions);
        self.attach_rank_context_menu(&row, path);
        row
    }

    fn attach_rank_context_menu(&self, row: &GtkBox, path: PathBuf) {
        let click = GestureClick::new();
        click.set_button(3);

        let panel = self.clone();
        let row_widget = row.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);

            let popover = Popover::new();
            popover.add_css_class("context-menu");
            popover.set_has_arrow(false);
            popover.set_autohide(true);
            popover.set_parent(&row_widget);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

            let menu = GtkBox::new(Orientation::Vertical, 0);
            let reveal_btn = Button::with_label("View in Folder");
            reveal_btn.add_css_class("context-menu-button");
            reveal_btn.set_halign(Align::Fill);

            let panel = panel.clone();
            let path = path.clone();
            let popover_for_action = popover.clone();
            reveal_btn.connect_clicked(move |_| {
                popover_for_action.popdown();
                if let Some(f) = &*panel.inner.cbs.on_reveal.borrow() {
                    f(path.clone());
                }
            });

            menu.append(&reveal_btn);
            popover.set_child(Some(&menu));
            popover.popup();
        });

        row.add_controller(click);
    }

    pub fn connect_open_file<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.inner.cbs.on_open.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_reveal_file<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.inner.cbs.on_reveal.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_add_to_tray<F: Fn(Vec<PathBuf>) + 'static>(&self, f: F) {
        *self.inner.cbs.on_add_to_tray.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_copy_path<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.inner.cbs.on_copy_path.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_trash_file<F: Fn(PathBuf) + 'static>(&self, f: F) {
        *self.inner.cbs.on_trash.borrow_mut() = Some(Box::new(f));
    }
}

// ─── pie chart ────────────────────────────────────────────────────────────────

fn draw_pie(
    cr: &cairo::Context,
    w: i32,
    h: i32,
    segments: &[(FileKind, f64)],
    hovered: Option<usize>,
) {
    let (w, h) = (w as f64, h as f64);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = w.min(h) * 0.42;
    let tau = 2.0 * std::f64::consts::PI;
    let start = -std::f64::consts::FRAC_PI_2;

    cr.set_source_rgb(0.110, 0.106, 0.098);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    if segments.is_empty() {
        cr.set_source_rgb(0.184, 0.176, 0.157);
        cr.arc(cx, cy, r, 0.0, tau);
        let _ = cr.fill();
        cr.set_source_rgb(0.502, 0.376, 0.251);
        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        #[allow(deprecated)]
        cr.set_font_size(11.0);
        let (tw, th) = cr
            .text_extents("No data")
            .map(|e| (e.width(), e.height()))
            .unwrap_or((40.0, 11.0));
        cr.move_to(cx - tw / 2.0, cy + th / 2.0);
        let _ = cr.show_text("No data");
        return;
    }

    let mut prev = start;
    for (i, (kind, end_frac)) in segments.iter().enumerate() {
        let end = start + *end_frac;
        let (rr, gg, bb) = kind_color(kind);
        let offset = if hovered == Some(i) { 8.0 } else { 0.0 };
        let mid = (prev + end) / 2.0;
        let (dcx, dcy) = (cx + offset * mid.cos(), cy + offset * mid.sin());
        cr.set_source_rgb(rr, gg, bb);
        cr.move_to(dcx, dcy);
        cr.arc(dcx, dcy, r, prev, end);
        cr.line_to(dcx, dcy);
        cr.close_path();
        let _ = cr.fill_preserve();
        cr.set_source_rgb(0.110, 0.106, 0.098);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        prev = end;
    }
}

// ─── legend & category rows ───────────────────────────────────────────────────

fn build_legend_row(kind: &FileKind, pct: u64, bytes: u64, _count: u64) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.add_css_class("sv-legend-row");
    let dot = DrawingArea::new();
    dot.set_content_width(10);
    dot.set_content_height(10);
    dot.set_valign(Align::Center);
    let (rr, gg, bb) = kind_color(kind);
    dot.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgb(rr, gg, bb);
        let r = w.min(h) as f64 / 2.0;
        cr.arc(
            w as f64 / 2.0,
            h as f64 / 2.0,
            r,
            0.0,
            2.0 * std::f64::consts::PI,
        );
        let _ = cr.fill();
    });
    let name_lbl = Label::new(Some(kind.label()));
    name_lbl.add_css_class("sv-legend-name");
    name_lbl.set_halign(Align::Start);
    name_lbl.set_hexpand(true);
    let pct_lbl = Label::new(Some(&format!("{pct}%")));
    pct_lbl.add_css_class("sv-legend-pct");
    let size_lbl = Label::new(Some(&format_size(bytes)));
    size_lbl.add_css_class("sv-legend-size");
    row.append(&dot);
    row.append(&name_lbl);
    row.append(&pct_lbl);
    row.append(&size_lbl);
    row
}

fn build_category_row(kind: &FileKind, bytes: u64, count: u64, max_bytes: u64) -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 2);
    row.add_css_class("sv-bar-row");
    let top = GtkBox::new(Orientation::Horizontal, 6);
    let icon_lbl = Label::new(Some(kind.badge()));
    let name_lbl = Label::new(Some(kind.label()));
    name_lbl.add_css_class("sv-bar-name");
    name_lbl.set_halign(Align::Start);
    name_lbl.set_hexpand(true);
    let detail = Label::new(Some(&format!(
        "{}  ·  {}",
        format_size(bytes),
        if count == 1 {
            "1 file".to_string()
        } else {
            format!("{count} files")
        }
    )));
    detail.add_css_class("sv-bar-size");
    top.append(&icon_lbl);
    top.append(&name_lbl);
    top.append(&detail);
    row.append(&top);

    let bar = ProgressBar::new();
    bar.add_css_class("sv-bar-progress");
    bar.set_fraction(bytes as f64 / max_bytes as f64);
    bar.set_margin_start(20);

    row.append(&bar);
    row
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn action_btn(label: &str) -> Button {
    let btn = Button::with_label(label);
    btn.add_css_class("sv-action-btn");
    btn
}

fn set_active_css(btn: &Button, active: bool) {
    if active {
        btn.add_css_class("active");
    } else {
        btn.remove_css_class("active");
    }
}

fn empty_hint(text: &str) -> Label {
    let lbl = Label::new(Some(text));
    lbl.add_css_class("sv-empty-hint");
    lbl.set_halign(Align::Start);
    lbl
}

fn scrolled_view<W>(child: &W) -> ScrolledWindow
where
    W: IsA<gtk::Widget>,
{
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .build();
    scrolled.set_child(Some(child));
    scrolled
}

fn clear_box(b: &GtkBox) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}
