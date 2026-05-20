// Lattice Picker — stripped-down internal file/folder picker.
//
// Two usage modes:
//   1. In-window modal via `show_picker_modal` — used by internal Lattice features.
//   2. Standalone window via `launch_picker_window` — used by `lattice --picker` CLI mode.
//
// The picker intentionally omits destructive operations (delete, trash, rename),
// painting mode, palette editing, and the full file browser toolbar. It is safe
// to surface externally and is groundwork for future xdg-desktop-portal support.
use crate::metadata::{CloudRecord, PlaceRecord};
use crate::ui::file_grid::FileKind;
use crate::ui::modal_host::{build_modal_actions, build_modal_button, ButtonKind, ModalHost};
use gio::FileType;
use gtk::prelude::*;
use gtk::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox,
    ListBoxRow, Orientation, ScrolledWindow, SelectionMode, Separator,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ── Public types ──────────────────────────────────────────────────────────────

/// What the picker is being used for.
#[derive(Clone, Debug)]
pub enum PickerMode {
    /// Select one existing folder. Folders-first listing; confirm uses selected dir or current dir.
    OpenFolder,
    /// Select one existing file.
    OpenFile,
    /// Select one or more existing files.
    OpenFiles,
    /// Choose a destination folder and filename for saving.
    SaveFile { suggested_name: String },
}

/// Full configuration for a picker instance.
#[derive(Clone, Debug)]
pub struct PickerConfig {
    pub mode: PickerMode,
    /// Starting directory (defaults to $HOME when not specified).
    pub initial_dir: PathBuf,
    pub show_hidden: bool,
}

impl PickerConfig {
    pub fn open_folder(initial_dir: PathBuf) -> Self {
        Self {
            mode: PickerMode::OpenFolder,
            initial_dir,
            show_hidden: false,
        }
    }

    pub fn open_file(initial_dir: PathBuf) -> Self {
        Self {
            mode: PickerMode::OpenFile,
            initial_dir,
            show_hidden: false,
        }
    }

    pub fn open_files(initial_dir: PathBuf) -> Self {
        Self {
            mode: PickerMode::OpenFiles,
            initial_dir,
            show_hidden: false,
        }
    }

    pub fn save_file(initial_dir: PathBuf, suggested_name: &str) -> Self {
        Self {
            mode: PickerMode::SaveFile {
                suggested_name: suggested_name.to_string(),
            },
            initial_dir,
            show_hidden: false,
        }
    }
}

/// Result returned to the caller on confirmation.
#[derive(Clone, Debug)]
pub enum PickerResult {
    Single(PathBuf),
    Multiple(Vec<PathBuf>),
}

// ── Internal types ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct PickerEntry {
    name: String,
    path: PathBuf,
    kind: FileKind,
    is_dir: bool,
    size_bytes: Option<u64>,
}

struct PickerState {
    current_dir: RefCell<PathBuf>,
    back_stack: RefCell<Vec<PathBuf>>,
    forward_stack: RefCell<Vec<PathBuf>>,
    items: RefCell<Vec<PickerEntry>>,
    selected_indices: RefCell<Vec<usize>>,
}

struct PickerRefs {
    list_box: ListBox,
    back_btn: Button,
    fwd_btn: Button,
    up_btn: Button,
    path_label: Label,
    sidebar_btns: Vec<(PathBuf, Button)>,
}

struct PickerBuilt {
    root: GtkBox,
    state: Rc<PickerState>,
    refs: Rc<PickerRefs>,
    save_entry: Entry,
    config: PickerConfig,
}

// ── File listing ─────────────────────────────────────────────────────────────

fn list_dir(dir: &Path, show_hidden: bool) -> Vec<PickerEntry> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut entries: Vec<PickerEntry> = read_dir
        .flatten()
        .filter_map(|de| {
            let name = de.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                return None;
            }
            let path = de.path();
            // Follow symlinks for is_dir detection
            let meta = std::fs::metadata(&path).ok()?;
            let is_dir = meta.is_dir();
            let file_type = if is_dir {
                FileType::Directory
            } else {
                FileType::Regular
            };
            let kind = FileKind::from_path(&path, file_type, None);
            Some(PickerEntry {
                name,
                path,
                kind,
                is_dir,
                size_bytes: if is_dir { None } else { Some(meta.len()) },
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });

    entries
}

fn format_size(bytes: Option<u64>) -> String {
    match bytes {
        None => String::new(),
        Some(b) if b < 1_024 => format!("{b} B"),
        Some(b) if b < 1_024 * 1_024 => format!("{:.1} KB", b as f64 / 1_024.0),
        Some(b) if b < 1_024 * 1_024 * 1_024 => {
            format!("{:.1} MB", b as f64 / (1_024.0 * 1_024.0))
        }
        Some(b) => format!("{:.1} GB", b as f64 / (1_024.0 * 1_024.0 * 1_024.0)),
    }
}

// ── Row builder ───────────────────────────────────────────────────────────────

fn build_picker_row(entry: &PickerEntry) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.add_css_class("picker-row");
    if entry.is_dir {
        row.add_css_class("picker-row-dir");
    } else {
        row.add_css_class("picker-row-file");
    }

    let hbox = GtkBox::new(Orientation::Horizontal, 6);
    hbox.set_margin_start(6);
    hbox.set_margin_end(8);
    hbox.set_margin_top(2);
    hbox.set_margin_bottom(2);

    let icon = Label::new(Some(entry.kind.badge()));
    icon.add_css_class("picker-row-icon");
    icon.set_width_chars(2);
    icon.set_halign(Align::Center);

    let name = Label::new(Some(&entry.name));
    name.add_css_class("picker-row-name");
    name.set_hexpand(true);
    name.set_halign(Align::Start);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(50);

    let size_str = format_size(entry.size_bytes);
    let size = Label::new(Some(&size_str));
    size.add_css_class("picker-row-size");
    size.set_halign(Align::End);
    size.set_width_chars(7);

    hbox.append(&icon);
    hbox.append(&name);
    hbox.append(&size);
    row.set_child(Some(&hbox));
    row
}

// ── Navigation helpers ────────────────────────────────────────────────────────

fn do_reload(state: &Rc<PickerState>, refs: &Rc<PickerRefs>, config: &PickerConfig) {
    let dir = state.current_dir.borrow().clone();
    let entries = list_dir(&dir, config.show_hidden);
    *state.items.borrow_mut() = entries.clone();
    state.selected_indices.borrow_mut().clear();

    while let Some(child) = refs.list_box.first_child() {
        refs.list_box.remove(&child);
    }
    for entry in &entries {
        refs.list_box.append(&build_picker_row(entry));
    }
}

fn update_nav_ui(state: &Rc<PickerState>, refs: &Rc<PickerRefs>) {
    let dir = state.current_dir.borrow().clone();
    refs.back_btn
        .set_sensitive(!state.back_stack.borrow().is_empty());
    refs.fwd_btn
        .set_sensitive(!state.forward_stack.borrow().is_empty());
    refs.up_btn
        .set_sensitive(dir.parent().map(|p| p != dir).unwrap_or(false));
    refs.path_label.set_text(&dir.to_string_lossy());

    for (path, btn) in &refs.sidebar_btns {
        if path == &dir {
            btn.add_css_class("active");
        } else {
            btn.remove_css_class("active");
        }
    }
}

fn do_navigate(
    state: &Rc<PickerState>,
    refs: &Rc<PickerRefs>,
    config: &PickerConfig,
    new_dir: PathBuf,
) {
    let cur = state.current_dir.borrow().clone();
    if cur == new_dir {
        return;
    }
    state.back_stack.borrow_mut().push(cur);
    state.forward_stack.borrow_mut().clear();
    *state.current_dir.borrow_mut() = new_dir;
    do_reload(state, refs, config);
    update_nav_ui(state, refs);
}

fn do_go_back(state: &Rc<PickerState>, refs: &Rc<PickerRefs>, config: &PickerConfig) {
    let prev = state.back_stack.borrow_mut().pop();
    if let Some(prev) = prev {
        let cur = state.current_dir.borrow().clone();
        state.forward_stack.borrow_mut().push(cur);
        *state.current_dir.borrow_mut() = prev;
        do_reload(state, refs, config);
        update_nav_ui(state, refs);
    }
}

fn do_go_forward(state: &Rc<PickerState>, refs: &Rc<PickerRefs>, config: &PickerConfig) {
    let next = state.forward_stack.borrow_mut().pop();
    if let Some(next) = next {
        let cur = state.current_dir.borrow().clone();
        state.back_stack.borrow_mut().push(cur);
        *state.current_dir.borrow_mut() = next;
        do_reload(state, refs, config);
        update_nav_ui(state, refs);
    }
}

// ── Confirm helpers ───────────────────────────────────────────────────────────

fn update_confirm_sensitivity(
    state: &Rc<PickerState>,
    config: &PickerConfig,
    confirm_btn: &Button,
    save_entry: &Entry,
) {
    let indices = state.selected_indices.borrow();
    let items = state.items.borrow();

    let sensitive = match &config.mode {
        PickerMode::OpenFolder => true,
        PickerMode::OpenFile => indices
            .iter()
            .any(|&i| items.get(i).map(|e| !e.is_dir).unwrap_or(false)),
        PickerMode::OpenFiles => indices
            .iter()
            .any(|&i| items.get(i).map(|e| !e.is_dir).unwrap_or(false)),
        PickerMode::SaveFile { .. } => !save_entry.text().trim().is_empty(),
    };
    confirm_btn.set_sensitive(sensitive);
}

fn fire_confirm(
    state: &Rc<PickerState>,
    config: &PickerConfig,
    save_entry: &Entry,
) -> Option<PickerResult> {
    let dir = state.current_dir.borrow().clone();
    let indices = state.selected_indices.borrow().clone();
    let items = state.items.borrow();

    match &config.mode {
        PickerMode::OpenFolder => {
            let path = indices
                .iter()
                .find_map(|&i| items.get(i).filter(|e| e.is_dir).map(|e| e.path.clone()))
                .unwrap_or(dir);
            Some(PickerResult::Single(path))
        }
        PickerMode::OpenFile => {
            let path = indices
                .iter()
                .find_map(|&i| items.get(i).filter(|e| !e.is_dir).map(|e| e.path.clone()))?;
            Some(PickerResult::Single(path))
        }
        PickerMode::OpenFiles => {
            let paths: Vec<PathBuf> = indices
                .iter()
                .filter_map(|&i| items.get(i).filter(|e| !e.is_dir).map(|e| e.path.clone()))
                .collect();
            if paths.is_empty() {
                None
            } else {
                Some(PickerResult::Multiple(paths))
            }
        }
        PickerMode::SaveFile { .. } => {
            let name = save_entry.text().trim().to_string();
            if name.is_empty() {
                return None;
            }
            Some(PickerResult::Single(dir.join(&name)))
        }
    }
}

fn initial_confirm_sensitive(mode: &PickerMode) -> bool {
    match mode {
        PickerMode::OpenFolder => true,
        PickerMode::SaveFile { suggested_name } => !suggested_name.is_empty(),
        _ => false,
    }
}

fn picker_title_str(mode: &PickerMode) -> &'static str {
    match mode {
        PickerMode::OpenFolder => "Choose Folder",
        PickerMode::OpenFile => "Open File",
        PickerMode::OpenFiles => "Open Files",
        PickerMode::SaveFile { .. } => "Save File",
    }
}

fn confirm_label_str(mode: &PickerMode) -> &'static str {
    match mode {
        PickerMode::OpenFolder => "Select Folder",
        PickerMode::OpenFile => "Open",
        PickerMode::OpenFiles => "Open",
        PickerMode::SaveFile { .. } => "Save",
    }
}

// ── Widget build ──────────────────────────────────────────────────────────────

fn build_picker_sidebar_btn(label: &str) -> Button {
    let btn = Button::with_label(label);
    btn.add_css_class("picker-sidebar-btn");
    btn.set_halign(Align::Fill);
    btn
}

fn build_sidebar_section_label(text: &str) -> Label {
    let lbl = Label::new(Some(text));
    lbl.add_css_class("picker-sidebar-section");
    lbl.set_halign(Align::Start);
    lbl
}

fn build_picker_content(
    config: PickerConfig,
    places: &[PlaceRecord],
    cloud_locs: &[CloudRecord],
    recent_dirs: &[PathBuf],
) -> PickerBuilt {
    // ── State ─────────────────────────────────────────────────────────────────
    let state = Rc::new(PickerState {
        current_dir: RefCell::new(config.initial_dir.clone()),
        back_stack: RefCell::new(Vec::new()),
        forward_stack: RefCell::new(Vec::new()),
        items: RefCell::new(Vec::new()),
        selected_indices: RefCell::new(Vec::new()),
    });

    // ── Nav bar ───────────────────────────────────────────────────────────────
    let nav_bar = GtkBox::new(Orientation::Horizontal, 4);
    nav_bar.add_css_class("picker-nav-bar");
    nav_bar.set_margin_start(6);
    nav_bar.set_margin_end(6);
    nav_bar.set_margin_top(4);
    nav_bar.set_margin_bottom(4);

    let back_btn = Button::with_label("←");
    back_btn.add_css_class("picker-nav-btn");
    back_btn.set_sensitive(false);

    let fwd_btn = Button::with_label("→");
    fwd_btn.add_css_class("picker-nav-btn");
    fwd_btn.set_sensitive(false);

    let up_btn = Button::with_label("↑");
    up_btn.add_css_class("picker-nav-btn");

    let path_label = Label::new(Some(&config.initial_dir.to_string_lossy()));
    path_label.add_css_class("picker-path");
    path_label.set_hexpand(true);
    path_label.set_halign(Align::Start);
    path_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);

    nav_bar.append(&back_btn);
    nav_bar.append(&fwd_btn);
    nav_bar.append(&up_btn);
    nav_bar.append(&path_label);

    // ── Sidebar ───────────────────────────────────────────────────────────────
    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.add_css_class("picker-sidebar");
    sidebar.set_width_request(160);
    sidebar.set_hexpand(false);

    let mut sidebar_btns: Vec<(PathBuf, Button)> = Vec::new();
    let mut sidebar_btns_for_wiring: Vec<(PathBuf, Button)> = Vec::new();

    let home_dir = glib::home_dir();

    sidebar.append(&build_sidebar_section_label("PLACES"));

    let home_btn = build_picker_sidebar_btn("🏠  Home");
    sidebar.append(&home_btn);
    sidebar_btns_for_wiring.push((home_dir.clone(), home_btn.clone()));
    sidebar_btns.push((home_dir, home_btn));

    for place in places {
        let path = place.folder_path.clone();
        let btn = build_picker_sidebar_btn(&format!("📌  {}", place.name));
        sidebar.append(&btn);
        sidebar_btns_for_wiring.push((path.clone(), btn.clone()));
        sidebar_btns.push((path, btn));
    }

    if !cloud_locs.is_empty() {
        sidebar.append(&build_sidebar_section_label("CLOUD"));
        for cloud in cloud_locs {
            let path = PathBuf::from(&cloud.path);
            let btn = build_picker_sidebar_btn(&format!("☁  {}", cloud.name));
            sidebar.append(&btn);
            sidebar_btns_for_wiring.push((path.clone(), btn.clone()));
            sidebar_btns.push((path, btn));
        }
    }

    let visible_recent: Vec<PathBuf> = recent_dirs
        .iter()
        .filter(|p| p.is_dir())
        .take(8)
        .cloned()
        .collect();
    if !visible_recent.is_empty() {
        sidebar.append(&build_sidebar_section_label("RECENT"));
        for rpath in &visible_recent {
            let name = rpath
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| rpath.to_string_lossy().to_string());
            let btn = build_picker_sidebar_btn(&format!("🕐  {}", name));
            sidebar.append(&btn);
            sidebar_btns_for_wiring.push((rpath.clone(), btn.clone()));
            sidebar_btns.push((rpath.clone(), btn));
        }
    }

    // ── File list ─────────────────────────────────────────────────────────────
    let list_box = ListBox::new();
    list_box.add_css_class("picker-list");
    list_box.set_selection_mode(match config.mode {
        PickerMode::OpenFiles => SelectionMode::Multiple,
        _ => SelectionMode::Single,
    });

    let list_scroll = ScrolledWindow::new();
    list_scroll.set_child(Some(&list_box));
    list_scroll.set_vexpand(true);
    list_scroll.set_hexpand(true);

    // ── Save row (hidden unless SaveFile mode) ────────────────────────────────
    let save_entry = Entry::new();
    save_entry.add_css_class("picker-save-entry");
    save_entry.set_hexpand(true);
    if let PickerMode::SaveFile { ref suggested_name } = config.mode {
        save_entry.set_text(suggested_name);
    }

    let save_lbl = Label::new(Some("Save as:"));
    save_lbl.add_css_class("picker-save-label");

    let save_row = GtkBox::new(Orientation::Horizontal, 8);
    save_row.add_css_class("picker-save-row");
    save_row.append(&save_lbl);
    save_row.append(&save_entry);
    save_row.set_visible(matches!(config.mode, PickerMode::SaveFile { .. }));

    // ── Picker pane ───────────────────────────────────────────────────────────
    let picker_pane = GtkBox::new(Orientation::Vertical, 0);
    picker_pane.set_hexpand(true);
    picker_pane.set_vexpand(true);
    picker_pane.append(&list_scroll);
    picker_pane.append(&save_row);

    // ── Body ─────────────────────────────────────────────────────────────────
    let body = GtkBox::new(Orientation::Horizontal, 0);
    body.add_css_class("picker-body");
    body.set_vexpand(true);

    body.append(&sidebar);
    let body_sep = Separator::new(Orientation::Vertical);
    body_sep.add_css_class("picker-body-sep");
    body_sep.set_size_request(1, -1);
    body.append(&body_sep);
    body.append(&picker_pane);

    // ── Root ──────────────────────────────────────────────────────────────────
    let root = GtkBox::new(Orientation::Vertical, 0);
    root.add_css_class("picker-root");
    root.set_size_request(600, 400);
    root.append(&nav_bar);
    root.append(&Separator::new(Orientation::Horizontal));
    root.append(&body);

    // ── Build refs ────────────────────────────────────────────────────────────
    let refs = Rc::new(PickerRefs {
        list_box: list_box.clone(),
        back_btn: back_btn.clone(),
        fwd_btn: fwd_btn.clone(),
        up_btn: up_btn.clone(),
        path_label: path_label.clone(),
        sidebar_btns,
    });

    // ── Initial load ──────────────────────────────────────────────────────────
    do_reload(&state, &refs, &config);
    update_nav_ui(&state, &refs);

    // ── Wire sidebar buttons ──────────────────────────────────────────────────
    for (path, btn) in sidebar_btns_for_wiring {
        let state = Rc::clone(&state);
        let refs = Rc::clone(&refs);
        let config = config.clone();
        btn.connect_clicked(move |_| do_navigate(&state, &refs, &config, path.clone()));
    }

    // ── Wire nav buttons ──────────────────────────────────────────────────────
    {
        let state = Rc::clone(&state);
        let refs = Rc::clone(&refs);
        let config = config.clone();
        back_btn.connect_clicked(move |_| do_go_back(&state, &refs, &config));
    }
    {
        let state = Rc::clone(&state);
        let refs = Rc::clone(&refs);
        let config = config.clone();
        fwd_btn.connect_clicked(move |_| do_go_forward(&state, &refs, &config));
    }
    {
        let state = Rc::clone(&state);
        let refs = Rc::clone(&refs);
        let config = config.clone();
        up_btn.connect_clicked(move |_| {
            let cur = state.current_dir.borrow().clone();
            if let Some(parent) = cur.parent().map(PathBuf::from) {
                if parent != cur {
                    do_navigate(&state, &refs, &config, parent);
                }
            }
        });
    }

    PickerBuilt {
        root,
        state,
        refs,
        save_entry,
        config,
    }
}

// ── Selection / confirm wiring (shared by both public fns) ───────────────────

fn wire_confirm_and_rows(built: &PickerBuilt, confirm_btn: &Button, do_confirm: Rc<dyn Fn()>) {
    // Selection changes → sensitivity
    {
        let confirm_btn = confirm_btn.clone();
        let state = Rc::clone(&built.state);
        let config = built.config.clone();
        let save_entry = built.save_entry.clone();
        built
            .refs
            .list_box
            .connect_selected_rows_changed(move |lb| {
                let indices: Vec<usize> = lb
                    .selected_rows()
                    .iter()
                    .map(|r| r.index() as usize)
                    .collect();
                *state.selected_indices.borrow_mut() = indices;
                update_confirm_sensitivity(&state, &config, &confirm_btn, &save_entry);
            });
    }

    // Save entry changes → sensitivity (SaveFile mode only)
    if matches!(built.config.mode, PickerMode::SaveFile { .. }) {
        let confirm_btn = confirm_btn.clone();
        let state = Rc::clone(&built.state);
        let config = built.config.clone();
        let save_entry_c = built.save_entry.clone();
        built.save_entry.connect_changed(move |_| {
            update_confirm_sensitivity(&state, &config, &confirm_btn, &save_entry_c);
        });
    }

    // Row activation (double-click)
    {
        let state = Rc::clone(&built.state);
        let refs = Rc::clone(&built.refs);
        let config = built.config.clone();
        let save_entry = built.save_entry.clone();
        built.refs.list_box.connect_row_activated(move |_, row| {
            let idx = row.index() as usize;
            let is_dir = {
                let items = state.items.borrow();
                items.get(idx).map(|e| e.is_dir)
            };
            match is_dir {
                Some(true) => {
                    let new_dir = state.items.borrow()[idx].path.clone();
                    do_navigate(&state, &refs, &config, new_dir);
                }
                Some(false) => match &config.mode {
                    PickerMode::OpenFile | PickerMode::OpenFiles => do_confirm(),
                    PickerMode::SaveFile { .. } => {
                        let name = state.items.borrow()[idx].name.clone();
                        save_entry.set_text(&name);
                    }
                    PickerMode::OpenFolder => {}
                },
                None => {}
            }
        });
    }
}

// ── Public: show as in-window modal ──────────────────────────────────────────

/// Show the picker as a Lattice in-window modal overlay.
///
/// `on_confirm` is called with the chosen path(s); `on_cancel` is called
/// when the user cancels or dismisses via Escape / scrim click.
/// The modal closes itself before either callback fires.
pub fn show_picker_modal(
    modal_host: &ModalHost,
    config: PickerConfig,
    places: &[PlaceRecord],
    cloud_locs: &[CloudRecord],
    recent_dirs: &[PathBuf],
    on_confirm: impl Fn(PickerResult) + 'static,
    on_cancel: impl Fn() + 'static,
) {
    let built = build_picker_content(config, places, cloud_locs, recent_dirs);

    let confirm_label = confirm_label_str(&built.config.mode);
    let initial_sensitive = initial_confirm_sensitive(&built.config.mode);
    let title = picker_title_str(&built.config.mode);

    let on_confirm = Rc::new(on_confirm);
    let on_cancel = Rc::new(on_cancel);

    let do_confirm: Rc<dyn Fn()> = {
        let host = modal_host.clone();
        let state = Rc::clone(&built.state);
        let config = built.config.clone();
        let save_entry = built.save_entry.clone();
        let on_confirm = Rc::clone(&on_confirm);
        Rc::new(move || {
            if let Some(result) = fire_confirm(&state, &config, &save_entry) {
                host.hide();
                on_confirm(result);
            }
        })
    };

    let confirm_btn = build_modal_button(confirm_label, ButtonKind::Primary, {
        let dc = Rc::clone(&do_confirm);
        move || dc()
    });
    confirm_btn.set_sensitive(initial_sensitive);

    let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, {
        let host = modal_host.clone();
        let cb = Rc::clone(&on_cancel);
        move || {
            host.hide();
            cb();
        }
    });

    wire_confirm_and_rows(&built, &confirm_btn, do_confirm);

    let actions = build_modal_actions();
    actions.append(&cancel_btn);
    actions.append(&confirm_btn);

    let dismiss: Box<dyn Fn()> = {
        let host = modal_host.clone();
        let cb = Rc::clone(&on_cancel);
        Box::new(move || {
            host.hide();
            cb();
        })
    };

    modal_host.show_with_custom_ui(title, &built.root, &actions, true, Some(dismiss));
}

// ── Public: launch as standalone window (CLI mode) ────────────────────────────

/// Launch the picker as a standalone `ApplicationWindow`.
///
/// On confirm: prints the chosen path(s) to stdout and exits 0.
/// On cancel or window close: exits 1.
///
/// Call this from `app.connect_activate` when `--picker` is detected.
pub fn launch_picker_window(
    app: &Application,
    config: PickerConfig,
    places: &[PlaceRecord],
    cloud_locs: &[CloudRecord],
    recent_dirs: &[PathBuf],
) {
    let built = build_picker_content(config, places, cloud_locs, recent_dirs);

    let confirm_label = confirm_label_str(&built.config.mode);
    let initial_sensitive = initial_confirm_sensitive(&built.config.mode);
    let title = picker_title_str(&built.config.mode);

    let window = ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(720)
        .default_height(540)
        .build();

    let do_confirm: Rc<dyn Fn()> = {
        let state = Rc::clone(&built.state);
        let config = built.config.clone();
        let save_entry = built.save_entry.clone();
        Rc::new(move || {
            if let Some(result) = fire_confirm(&state, &config, &save_entry) {
                match &result {
                    PickerResult::Single(p) => println!("{}", p.display()),
                    PickerResult::Multiple(ps) => {
                        for p in ps {
                            println!("{}", p.display());
                        }
                    }
                }
                std::process::exit(0);
            }
        })
    };

    let confirm_btn = Button::with_label(confirm_label);
    confirm_btn.add_css_class("modal-primary-button");
    confirm_btn.set_sensitive(initial_sensitive);
    confirm_btn.connect_clicked({
        let dc = Rc::clone(&do_confirm);
        move |_| dc()
    });

    let cancel_btn = Button::with_label("Cancel");
    cancel_btn.add_css_class("modal-secondary-button");
    cancel_btn.connect_clicked(|_| std::process::exit(1));

    wire_confirm_and_rows(&built, &confirm_btn, do_confirm);

    // Actions row at bottom of window
    let actions_sep = Separator::new(Orientation::Horizontal);

    let actions_row = GtkBox::new(Orientation::Horizontal, 8);
    actions_row.add_css_class("picker-window-actions");
    actions_row.set_halign(Align::End);
    actions_row.set_margin_top(8);
    actions_row.set_margin_bottom(10);
    actions_row.set_margin_end(12);
    actions_row.append(&cancel_btn);
    actions_row.append(&confirm_btn);

    let outer = GtkBox::new(Orientation::Vertical, 0);
    outer.append(&built.root);
    outer.append(&actions_sep);
    outer.append(&actions_row);

    window.set_child(Some(&outer));
    window.connect_close_request(|_| {
        std::process::exit(1);
    });
    window.present();
}
