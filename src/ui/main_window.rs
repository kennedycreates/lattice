use crate::config::{AppConfig, CustomActionConfig};
use crate::metadata::{MetadataStore, ProjectRecord, TagRecord};
use crate::ui::{
    bulk_rename,
    file_grid::{FileGrid, FileItem, FileKind},
    modal_host::{
        build_modal_actions, build_modal_button, build_modal_prompt, ButtonKind, ModalHost,
    },
    ops_panel::{OpId, OpsPanel},
    preview_pane::PreviewPane,
    search_panel::{SearchAgeFilter, SearchKindFilter, SearchPanel, SearchQuery, SearchSizeFilter},
    sidebar::{Sidebar, SidebarTarget},
    status_bar::StatusBar,
    tab_strip::TabStrip,
    tag_filter::{TagFilterPanel, TagFilterSpec},
    toolbar::Toolbar,
};
use gdk_pixbuf::Pixbuf;
use gio::prelude::*;
use glib::UserDirectory;
use gtk::gdk;
use gtk::prelude::*;
use gtk::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, Entry, FlowBox, HeaderBar, Image,
    Label, Orientation, Paned, Popover, Separator,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const DIRECTORY_ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::is-hidden,standard::size,time::modified";
const TRASH_ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::is-hidden,standard::size,time::modified,trash::orig-path,standard::target-uri";
const PREVIEW_ATTRIBUTES: &str =
    "standard::display-name,standard::type,standard::content-type,standard::size,time::modified";
const TERMINAL_ENV_VAR: &str = "LATTICE_TERMINAL";
const TEXT_PREVIEW_LIMIT_BYTES: usize = 64 * 1024;
const TEXT_PREVIEW_DISPLAY_CHARS: usize = 4_000;
const TRIAGE_LARGE_FILE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneSlot {
    Primary,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectTransferKind {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardMode {
    Copy,
    Cut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileClipboardState {
    paths: Vec<PathBuf>,
    mode: ClipboardMode,
}

impl FileClipboardState {
    fn new(paths: Vec<PathBuf>, mode: ClipboardMode) -> Option<Self> {
        (!paths.is_empty()).then_some(Self { paths, mode })
    }

    fn is_copy(&self) -> bool {
        self.mode == ClipboardMode::Copy
    }

    fn after_completed_paste(&self, moved_sources: &[PathBuf]) -> Option<Self> {
        if self.mode == ClipboardMode::Copy {
            return Some(self.clone());
        }

        let remaining = self
            .paths
            .iter()
            .filter(|path| !moved_sources.iter().any(|moved| moved == *path))
            .cloned()
            .collect::<Vec<_>>();
        Self::new(remaining, self.mode)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DownloadsTriageFilter {
    All,
    Today,
    ThisWeek,
    ThisMonth,
    OlderThanOneMonth,
    Images,
    Videos,
    Archives,
    Documents,
    LargeFiles,
}

impl DownloadsTriageFilter {
    const ALL: [Self; 10] = [
        Self::All,
        Self::Today,
        Self::ThisWeek,
        Self::ThisMonth,
        Self::OlderThanOneMonth,
        Self::Images,
        Self::Videos,
        Self::Archives,
        Self::Documents,
        Self::LargeFiles,
    ];

    fn label(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Today => "Today",
            Self::ThisWeek => "This Week",
            Self::ThisMonth => "This Month",
            Self::OlderThanOneMonth => "Older Than 1 Month",
            Self::Images => "Images",
            Self::Videos => "Videos",
            Self::Archives => "Archives",
            Self::Documents => "Documents",
            Self::LargeFiles => "Large Files",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusedContext {
    PathEntry,
    SearchEntry,
    EditableText,
    Sidebar,
    FileGrid,
    TabStrip,
    Window,
}

impl FocusedContext {
    fn is_editable(self) -> bool {
        matches!(
            self,
            Self::PathEntry | Self::SearchEntry | Self::EditableText
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WindowCommand {
    CopySelection,
    CutSelection,
    PasteClipboard,
    CopyPathText,
    NewFolder,
    NewTextDocument,
    RenameSelection,
    TrashSelection,
    OpenSearch,
    ToggleFilter,
    FocusPath,
    Refresh,
    ToggleHidden,
    ToggleSidebar,
    TogglePreview,
    NewTab,
    CloseTab,
    ToggleSplit,
    PreviousTab,
    NextTab,
    GoBack,
    GoUp,
    CyclePane,
    Escape,
    CustomAction(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActionAvailability {
    can_copy_files: bool,
    can_cut_files: bool,
    can_paste_files: bool,
    can_copy_paths: bool,
    can_rename: bool,
    can_trash: bool,
    can_new_folder: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PaneView {
    Directory(PathBuf),
    Tag(TagRecord),
    DownloadsTriage(DownloadsTriageFilter),
    SystemDrives,
    Recent,
    Trash,
    Search(SearchQuery),
}

#[derive(Clone, Debug)]
struct TabState {
    title: String,
    primary_dir: PathBuf,
    primary_back_history: Vec<PathBuf>,
    primary_view: PaneView,
    secondary_dir: PathBuf,
    secondary_back_history: Vec<PathBuf>,
    secondary_view: PaneView,
    split_enabled: bool,
    active_pane: PaneSlot,
}

struct LaunchResolution {
    primary_dir: PathBuf,
    primary_view: PaneView,
    secondary_dir: PathBuf,
    split_enabled: bool,
    notice: Option<String>,
}

impl TabState {
    fn new(path: PathBuf) -> Self {
        let primary_view = PaneView::Directory(path.clone());
        let secondary_view = PaneView::Directory(path.clone());
        let title = tab_title_for_view(&primary_view, &path);
        Self {
            title,
            primary_dir: path.clone(),
            primary_back_history: Vec::new(),
            primary_view,
            secondary_dir: path,
            secondary_back_history: Vec::new(),
            secondary_view,
            split_enabled: false,
            active_pane: PaneSlot::Primary,
        }
    }

    fn for_launch(
        launch: &crate::launch::LaunchConfig,
        places: &Places,
        metadata: &crate::metadata::MetadataStore,
    ) -> (Self, Option<String>) {
        let LaunchResolution {
            primary_dir,
            primary_view,
            secondary_dir,
            split_enabled,
            notice,
        } = resolve_launch(launch, places, metadata);

        let secondary_view = PaneView::Directory(secondary_dir.clone());
        let title = tab_title_for_view(&primary_view, &primary_dir);
        (
            Self {
                title,
                primary_dir,
                primary_back_history: Vec::new(),
                primary_view,
                secondary_dir,
                secondary_back_history: Vec::new(),
                secondary_view,
                split_enabled,
                active_pane: PaneSlot::Primary,
            },
            notice,
        )
    }
}

#[derive(Clone)]
struct PaneWidgets {
    root: GtkBox,
    path_label: Label,
    view_strip: GtkBox,
    view_title: Label,
    triage_filters: Vec<(DownloadsTriageFilter, Button)>,
    search_panel: SearchPanel,
    tag_filter: TagFilterPanel,
    file_grid: FileGrid,
}

impl PaneWidgets {
    fn build(slot: PaneSlot) -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("browser-pane");
        match slot {
            PaneSlot::Primary => root.add_css_class("browser-pane-primary"),
            PaneSlot::Secondary => root.add_css_class("browser-pane-secondary"),
        }

        let header = GtkBox::new(Orientation::Horizontal, 0);
        header.add_css_class("browser-pane-header");

        let path_label = Label::new(Some(""));
        path_label.add_css_class("browser-pane-path");
        path_label.set_halign(Align::Start);
        path_label.set_hexpand(true);
        path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path_label.set_margin_start(10);
        path_label.set_margin_end(10);
        path_label.set_margin_top(6);
        path_label.set_margin_bottom(6);
        header.append(&path_label);

        let view_strip = GtkBox::new(Orientation::Vertical, 8);
        view_strip.add_css_class("pane-view-strip");
        view_strip.set_visible(false);
        view_strip.set_margin_start(10);
        view_strip.set_margin_end(10);
        view_strip.set_margin_top(8);
        view_strip.set_margin_bottom(8);

        let view_title = Label::new(None);
        view_title.add_css_class("pane-view-title");
        view_title.set_halign(Align::Start);
        view_strip.append(&view_title);

        let filter_row = FlowBox::new();
        filter_row.add_css_class("pane-filter-row");
        filter_row.set_selection_mode(gtk::SelectionMode::None);
        filter_row.set_homogeneous(false);
        filter_row.set_column_spacing(6);
        filter_row.set_row_spacing(6);
        filter_row.set_max_children_per_line(64);
        let mut triage_filters = Vec::with_capacity(DownloadsTriageFilter::ALL.len());
        for filter in DownloadsTriageFilter::ALL {
            let button = Button::with_label(filter.label());
            button.add_css_class("pane-filter-button");
            filter_row.append(&button);
            triage_filters.push((filter, button));
        }
        view_strip.append(&filter_row);

        let search_panel = SearchPanel::build();
        let tag_filter = TagFilterPanel::build();

        let file_grid = FileGrid::build();
        file_grid.root.set_vexpand(true);
        file_grid.root.set_hexpand(true);

        root.append(&header);
        root.append(&tag_filter.root);
        root.append(&view_strip);
        root.append(&search_panel.root);
        root.append(&file_grid.root);

        Self {
            root,
            path_label,
            view_strip,
            view_title,
            triage_filters,
            search_panel,
            tag_filter,
            file_grid,
        }
    }
}

pub struct MainWindow;

impl MainWindow {
    pub fn new(
        app: &Application,
        launch: &crate::launch::LaunchConfig,
        config: AppConfig,
    ) -> ApplicationWindow {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Lattice")
            .default_width(1280)
            .default_height(800)
            .build();

        window.add_css_class("app-window");
        window.set_titlebar(Some(&build_titlebar()));

        let places = Places::discover();
        let toolbar = Toolbar::build();
        let sidebar = Sidebar::build();
        let tab_strip = TabStrip::build();
        let primary_pane = PaneWidgets::build(PaneSlot::Primary);
        let secondary_pane = PaneWidgets::build(PaneSlot::Secondary);
        let preview = PreviewPane::build();
        let ops_panel = OpsPanel::build();
        let status = StatusBar::build();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&toolbar.root);

        let body = build_body(
            &sidebar,
            &tab_strip,
            &primary_pane,
            &secondary_pane,
            &preview,
        );
        body.root.set_vexpand(true);
        root.append(&body.root);
        root.append(&ops_panel.root);
        root.append(&status.root);

        // Wrap the entire UI in the in-window modal overlay.
        // ModalHost eliminates separate popup windows that caused squashed first-frame rendering.
        let modal_host = ModalHost::new();
        modal_host.overlay.set_child(Some(&root));
        window.set_child(Some(&modal_host.overlay));

        let controller = BrowserController::new(
            window.clone(),
            places,
            launch,
            toolbar.clone(),
            sidebar.clone(),
            tab_strip.clone(),
            primary_pane.clone(),
            secondary_pane.clone(),
            preview.clone(),
            body.preview_host.clone(),
            status.clone(),
            ops_panel,
            modal_host,
            config,
        );
        controller.bootstrap();

        window
    }
}

fn build_titlebar() -> HeaderBar {
    let titlebar = HeaderBar::new();
    titlebar.set_show_title_buttons(true);

    let title = Label::new(Some("Lattice"));
    title.add_css_class("title");
    title.set_single_line_mode(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::None);
    title.set_width_chars("Lattice".chars().count() as i32);
    titlebar.set_title_widget(Some(&title));

    titlebar
}

#[derive(Clone)]
struct Places {
    home: PathBuf,
    downloads: PathBuf,
    documents: PathBuf,
}

impl Places {
    fn discover() -> Self {
        let home = glib::home_dir();
        let downloads = glib::user_special_dir(UserDirectory::Downloads)
            .unwrap_or_else(|| home.join("Downloads"));
        let documents = glib::user_special_dir(UserDirectory::Documents)
            .unwrap_or_else(|| home.join("Documents"));
        Self {
            home,
            downloads,
            documents,
        }
    }
}

#[derive(Default)]
struct BatchResult {
    success_count: usize,
    failures: Vec<String>,
}

struct BrowserController {
    window: ApplicationWindow,
    places: Places,
    metadata: RefCell<MetadataStore>,
    projects: RefCell<Vec<ProjectRecord>>,
    tags: RefCell<Vec<TagRecord>>,
    terminal_command: Option<Vec<OsString>>,
    toolbar: Toolbar,
    sidebar: Sidebar,
    tab_strip: TabStrip,
    primary_pane: PaneWidgets,
    secondary_pane: PaneWidgets,
    preview: PreviewPane,
    preview_host: GtkBox,
    status: StatusBar,
    context_popover: RefCell<Option<Popover>>,
    file_clipboard: RefCell<Option<FileClipboardState>>,
    tabs: RefCell<Vec<TabState>>,
    active_tab: Cell<usize>,
    active_pane: Cell<PaneSlot>,
    current_dir: RefCell<PathBuf>,
    current_view: RefCell<PaneView>,
    back_history: RefCell<Vec<PathBuf>>,
    items: RefCell<Vec<FileItem>>,
    primary_all_items: RefCell<Vec<FileItem>>,
    secondary_current_dir: RefCell<PathBuf>,
    secondary_view: RefCell<PaneView>,
    secondary_back_history: RefCell<Vec<PathBuf>>,
    secondary_items: RefCell<Vec<FileItem>>,
    secondary_all_items: RefCell<Vec<FileItem>>,
    pending_reveal_path: RefCell<Option<PathBuf>>,
    secondary_pending_reveal_path: RefCell<Option<PathBuf>>,
    pending_status_message: RefCell<Option<String>>,
    show_hidden: Cell<bool>,
    sidebar_visible: Cell<bool>,
    preview_visible: Cell<bool>,
    suppress_panel_toggle_handlers: Cell<bool>,
    split_enabled: Cell<bool>,
    load_generation: Cell<u64>,
    load_cancellable: RefCell<Option<gio::Cancellable>>,
    secondary_load_generation: Cell<u64>,
    secondary_load_cancellable: RefCell<Option<gio::Cancellable>>,
    primary_keyboard_anchor: Cell<Option<i32>>,
    primary_keyboard_current: Cell<Option<i32>>,
    secondary_keyboard_anchor: Cell<Option<i32>>,
    secondary_keyboard_current: Cell<Option<i32>>,
    preview_generation: Cell<u64>,
    preview_cancellable: RefCell<Option<gio::Cancellable>>,
    primary_thumb_loader: crate::thumbnail::ThumbnailLoader,
    secondary_thumb_loader: crate::thumbnail::ThumbnailLoader,
    search_debounce: RefCell<Option<glib::SourceId>>,
    ops_panel: OpsPanel,
    modal_host: ModalHost,
    config: AppConfig,
}

impl BrowserController {
    fn new(
        window: ApplicationWindow,
        places: Places,
        launch: &crate::launch::LaunchConfig,
        toolbar: Toolbar,
        sidebar: Sidebar,
        tab_strip: TabStrip,
        primary_pane: PaneWidgets,
        secondary_pane: PaneWidgets,
        preview: PreviewPane,
        preview_host: GtkBox,
        status: StatusBar,
        ops_panel: OpsPanel,
        modal_host: ModalHost,
        config: AppConfig,
    ) -> Rc<Self> {
        let metadata = MetadataStore::open().or_else(|error| {
            eprintln!("Lattice metadata fallback: {error}");
            MetadataStore::open_in_memory()
        });
        let metadata = metadata.expect("Lattice could not initialize metadata storage.");
        let (initial_tab, launch_notice) = TabState::for_launch(launch, &places, &metadata);
        Rc::new(Self {
            window,
            metadata: RefCell::new(metadata),
            projects: RefCell::new(Vec::new()),
            tags: RefCell::new(Vec::new()),
            tabs: RefCell::new(vec![initial_tab]),
            active_tab: Cell::new(0),
            active_pane: Cell::new(PaneSlot::Primary),
            current_dir: RefCell::new(places.home.clone()),
            current_view: RefCell::new(PaneView::Directory(places.home.clone())),
            secondary_current_dir: RefCell::new(places.home.clone()),
            secondary_view: RefCell::new(PaneView::Directory(places.home.clone())),
            terminal_command: detect_terminal_command(),
            places,
            toolbar,
            sidebar,
            tab_strip,
            primary_pane,
            secondary_pane,
            preview,
            preview_host,
            status,
            context_popover: RefCell::new(None),
            file_clipboard: RefCell::new(None),
            back_history: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            primary_all_items: RefCell::new(Vec::new()),
            secondary_back_history: RefCell::new(Vec::new()),
            secondary_items: RefCell::new(Vec::new()),
            secondary_all_items: RefCell::new(Vec::new()),
            pending_reveal_path: RefCell::new(None),
            secondary_pending_reveal_path: RefCell::new(None),
            pending_status_message: RefCell::new(launch_notice),
            show_hidden: Cell::new(false),
            sidebar_visible: Cell::new(true),
            preview_visible: Cell::new(true),
            suppress_panel_toggle_handlers: Cell::new(false),
            split_enabled: Cell::new(false),
            load_generation: Cell::new(0),
            load_cancellable: RefCell::new(None),
            secondary_load_generation: Cell::new(0),
            secondary_load_cancellable: RefCell::new(None),
            primary_keyboard_anchor: Cell::new(None),
            primary_keyboard_current: Cell::new(None),
            secondary_keyboard_anchor: Cell::new(None),
            secondary_keyboard_current: Cell::new(None),
            preview_generation: Cell::new(0),
            preview_cancellable: RefCell::new(None),
            primary_thumb_loader: crate::thumbnail::ThumbnailLoader::new(),
            secondary_thumb_loader: crate::thumbnail::ThumbnailLoader::new(),
            search_debounce: RefCell::new(None),
            ops_panel,
            modal_host,
            config,
        })
    }

    fn bootstrap(self: &Rc<Self>) {
        self.connect_navigation();
        self.connect_sidebar();
        self.connect_tab_strip();
        self.connect_panes();
        self.connect_preview_actions();
        self.connect_search_panels();
        self.connect_window_shortcuts();
        self.attach_pane_dnd(PaneSlot::Primary);
        self.attach_pane_dnd(PaneSlot::Secondary);
        self.attach_sidebar_dnd();
        self.wire_tag_filters();
        self.refresh_metadata_sidebar();
        self.update_action_state();
        self.rebuild_tab_strip();
        self.sync_split_visibility();
        self.reload_active_tab();
    }

    fn connect_navigation(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.toolbar
            .back_button
            .connect_clicked(move |_| controller.go_back());

        let controller = Rc::clone(self);
        self.toolbar
            .up_button
            .connect_clicked(move |_| controller.go_up());

        let controller = Rc::clone(self);
        self.toolbar
            .refresh_button
            .connect_clicked(move |_| controller.refresh());

        let controller = Rc::clone(self);
        self.toolbar.sidebar_toggle.connect_toggled(move |toggle| {
            if controller.suppress_panel_toggle_handlers.get() {
                return;
            }
            controller.set_sidebar_visible(toggle.is_active());
        });

        let controller = Rc::clone(self);
        self.toolbar
            .split_toggle
            .connect_toggled(move |toggle| controller.set_split_enabled(toggle.is_active()));

        let controller = Rc::clone(self);
        self.toolbar
            .show_hidden_toggle
            .connect_toggled(move |toggle| controller.set_show_hidden(toggle.is_active()));

        let controller = Rc::clone(self);
        self.toolbar.preview_toggle.connect_toggled(move |toggle| {
            if controller.suppress_panel_toggle_handlers.get() {
                return;
            }
            controller.set_preview_visible(toggle.is_active());
        });

        let controller = Rc::clone(self);
        self.toolbar
            .new_folder_button
            .connect_clicked(move |_| controller.create_new_folder());

        let controller = Rc::clone(self);
        self.toolbar
            .new_text_document_button
            .connect_clicked(move |_| controller.create_new_text_document());

        let controller = Rc::clone(self);
        self.toolbar
            .rename_button
            .connect_clicked(move |_| controller.rename_selected());

        let controller = Rc::clone(self);
        self.toolbar
            .trash_button
            .connect_clicked(move |_| controller.trash_selected());

        let controller = Rc::clone(self);
        self.toolbar
            .path_button
            .connect_clicked(move |_| controller.begin_path_entry_editing());

        let controller = Rc::clone(self);
        self.toolbar
            .path_entry
            .connect_activate(move |_| controller.navigate_from_path_entry());

        let controller = Rc::clone(self);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(move |_| controller.begin_path_entry_editing());

        let controller = Rc::clone(self);
        focus.connect_leave(move |_| controller.finish_path_entry_editing());
        self.toolbar.path_entry.add_controller(focus);

        let controller = Rc::clone(self);
        let key_controller = gtk::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gdk::Key::Escape {
                controller.cancel_path_entry_editing();
                return glib::Propagation::Stop;
            }

            if key == gdk::Key::l && modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                controller.begin_path_entry_editing();
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });
        self.toolbar.path_entry.add_controller(key_controller);

        let controller = Rc::clone(self);
        self.toolbar
            .search_button
            .connect_clicked(move |_| controller.open_search_in_current_dir());

        let controller = Rc::clone(self);
        self.toolbar
            .filter_toggle
            .connect_toggled(move |btn| controller.set_filter_panel_open(btn.is_active()));
    }

    fn connect_window_shortcuts(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        let win_keys = gtk::EventControllerKey::new();
        win_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        win_keys.connect_key_pressed(move |_, key, _, modifiers| {
            controller.handle_window_key(key, modifiers)
        });
        self.window.add_controller(win_keys);
    }

    fn handle_window_key(
        self: &Rc<Self>,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> glib::Propagation {
        let focus = self.focused_context();

        if focus.is_editable() {
            return if matches!(
                self.window_command_from_key(key, modifiers),
                Some(WindowCommand::Escape)
            ) && self.handle_escape(focus)
            {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            };
        }

        if self.handle_sidebar_navigation(focus, key, modifiers)
            || self.handle_file_grid_key(focus, key, modifiers)
        {
            return glib::Propagation::Stop;
        }

        if let Some(command) = self.window_command_from_key(key, modifiers) {
            if self.handle_window_command(command) {
                return glib::Propagation::Stop;
            }
        }

        glib::Propagation::Proceed
    }

    fn window_command_from_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<WindowCommand> {
        configured_window_command_from_key(&self.config, key, modifiers)
    }

    fn focused_context(&self) -> FocusedContext {
        let Some(focus) = gtk::prelude::RootExt::focus(&self.window) else {
            return FocusedContext::Window;
        };

        let path_entry: gtk::Widget = self.toolbar.path_entry.clone().upcast();
        if focus == path_entry {
            return FocusedContext::PathEntry;
        }

        for entry in self.search_entries() {
            let widget: gtk::Widget = entry.upcast();
            if focus == widget {
                return FocusedContext::SearchEntry;
            }
        }

        if focus.is::<Entry>() {
            return FocusedContext::EditableText;
        }

        if focus.has_css_class("sidebar-button") {
            return FocusedContext::Sidebar;
        }

        if focus.has_css_class("tab-button")
            || focus.has_css_class("tab-close-button")
            || focus.has_css_class("tab-add-button")
        {
            return FocusedContext::TabStrip;
        }

        if focus.is::<gtk::FlowBox>() || focus.is::<gtk::FlowBoxChild>() {
            return FocusedContext::FileGrid;
        }

        FocusedContext::Window
    }

    fn search_entries(&self) -> [Entry; 2] {
        [
            self.primary_pane.search_panel.name_entry.clone(),
            self.secondary_pane.search_panel.name_entry.clone(),
        ]
    }

    fn sidebar_buttons(&self) -> Vec<Button> {
        let mut buttons = vec![
            self.sidebar.home_button.clone(),
            self.sidebar.downloads_button.clone(),
            self.sidebar.documents_button.clone(),
            self.sidebar.downloads_triage_button.clone(),
            self.sidebar.drives_button.clone(),
            self.sidebar.recent_button.clone(),
            self.sidebar.trash_button.clone(),
        ];
        buttons.extend(
            self.sidebar
                .project_buttons()
                .into_iter()
                .map(|(_, button)| button),
        );
        buttons.extend(
            self.sidebar
                .tag_buttons()
                .into_iter()
                .map(|(_, button)| button),
        );
        buttons
    }

    fn handle_sidebar_navigation(
        &self,
        focus: FocusedContext,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> bool {
        if focus != FocusedContext::Sidebar || !relevant_modifiers(modifiers).is_empty() {
            return false;
        }

        let offset = match key {
            gdk::Key::Up => -1,
            gdk::Key::Down => 1,
            _ => return false,
        };

        let Some(current_focus) = gtk::prelude::RootExt::focus(&self.window) else {
            return false;
        };
        let buttons = self.sidebar_buttons();
        let Some(current_index) = buttons.iter().position(|button| {
            let widget: gtk::Widget = button.clone().upcast();
            current_focus == widget
        }) else {
            return false;
        };

        let target = (current_index as i32 + offset)
            .clamp(0, buttons.len().saturating_sub(1) as i32) as usize;
        buttons[target].grab_focus();
        true
    }

    fn connect_sidebar(self: &Rc<Self>) {
        connect_directory_button(self, &self.sidebar.home_button, self.places.home.clone());
        connect_directory_button(
            self,
            &self.sidebar.downloads_button,
            self.places.downloads.clone(),
        );
        connect_directory_button(
            self,
            &self.sidebar.documents_button,
            self.places.documents.clone(),
        );
        let controller = Rc::clone(self);
        self.sidebar
            .downloads_triage_button
            .connect_clicked(move |_| controller.open_downloads_triage(DownloadsTriageFilter::All));
        let controller = Rc::clone(self);
        self.sidebar
            .drives_button
            .connect_clicked(move |_| controller.open_system_drives());
        let controller = Rc::clone(self);
        self.sidebar
            .recent_button
            .connect_clicked(move |_| controller.open_recent());
        let controller = Rc::clone(self);
        self.sidebar
            .trash_button
            .connect_clicked(move |_| controller.open_trash());
    }

    fn refresh_metadata_sidebar(self: &Rc<Self>) {
        let (projects, tags) = {
            let metadata = self.metadata.borrow();
            let projects = metadata.list_projects().unwrap_or_default();
            let tags = metadata.list_tags().unwrap_or_default();
            (projects, tags)
        };

        self.projects.replace(projects.clone());
        self.tags.replace(tags.clone());
        self.sidebar.set_projects(&projects);
        self.sidebar.set_tags(&tags);

        for (project_id, button) in self.sidebar.project_buttons() {
            let controller = Rc::clone(self);
            button.connect_clicked(move |_| controller.open_project(project_id));
        }

        for (tag_id, button) in self.sidebar.tag_buttons() {
            let controller = Rc::clone(self);
            button.connect_clicked(move |_| controller.open_tag(tag_id));
        }

        self.refresh_search_tag_buttons(PaneSlot::Primary);
        self.refresh_search_tag_buttons(PaneSlot::Secondary);
        self.primary_pane.tag_filter.set_tags(&tags);
        self.secondary_pane.tag_filter.set_tags(&tags);
        self.update_sidebar_state();
    }

    fn open_project(self: &Rc<Self>, project_id: i64) {
        let Some(project) = self
            .projects
            .borrow()
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
        else {
            self.status.set_message("Project not found.");
            return;
        };

        self.navigate_to(self.active_slot(), project.root_path, true);
    }

    fn open_tag(self: &Rc<Self>, tag_id: i64) {
        let Some(tag) = self
            .tags
            .borrow()
            .iter()
            .find(|tag| tag.id == tag_id)
            .cloned()
        else {
            self.status.set_message("Tag not found.");
            return;
        };

        let slot = self.active_slot();
        self.current_view_cell(slot)
            .replace(PaneView::Tag(tag.clone()));
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_tag_view(slot, tag);
    }

    fn open_downloads_triage(self: &Rc<Self>, filter: DownloadsTriageFilter) {
        let slot = self.active_slot();
        self.current_dir_cell(slot)
            .replace(self.places.downloads.clone());
        self.current_view_cell(slot)
            .replace(PaneView::DownloadsTriage(filter));
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_downloads_triage(slot);
    }

    fn set_triage_filter(self: &Rc<Self>, slot: PaneSlot, filter: DownloadsTriageFilter) {
        if !matches!(self.current_view_for(slot), PaneView::DownloadsTriage(_)) {
            return;
        }

        self.current_view_cell(slot)
            .replace(PaneView::DownloadsTriage(filter));
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_downloads_triage(slot);
    }

    fn update_view_strip(&self, slot: PaneSlot) {
        let pane = self.pane_widgets(slot);
        let is_search = matches!(self.current_view_for(slot), PaneView::Search(_));
        pane.search_panel.root.set_visible(is_search);

        match self.current_view_for(slot) {
            PaneView::Search(query) => {
                pane.view_strip.set_visible(false);
                let scope = format_path(&query.scope_dir, &self.places.home);
                pane.search_panel
                    .scope_label
                    .set_label(&format!("in {scope}"));
                pane.search_panel.sync_from_query(&query);
            }
            PaneView::Directory(_) | PaneView::Trash => {
                pane.view_strip.set_visible(false);
            }
            PaneView::Tag(tag) => {
                pane.view_strip.set_visible(true);
                pane.view_title.set_visible(true);
                pane.view_title
                    .set_label(&format!("Tagged Files · #{}", tag.name));
                for (_, button) in &pane.triage_filters {
                    button.set_visible(false);
                }
            }
            PaneView::DownloadsTriage(active_filter) => {
                pane.view_strip.set_visible(true);
                pane.view_title.set_visible(false);
                for (filter, button) in &pane.triage_filters {
                    button.set_visible(true);
                    if *filter == active_filter {
                        button.add_css_class("active");
                    } else {
                        button.remove_css_class("active");
                    }
                }
            }
            PaneView::SystemDrives => {
                pane.view_strip.set_visible(true);
                pane.view_title.set_visible(true);
                pane.view_title.set_label("Mounted Volumes");
                for (_, button) in &pane.triage_filters {
                    button.set_visible(false);
                }
            }
            PaneView::Recent => {
                pane.view_strip.set_visible(true);
                pane.view_title.set_visible(true);
                pane.view_title.set_label("Recent Folders");
                for (_, button) in &pane.triage_filters {
                    button.set_visible(false);
                }
            }
        }
    }

    fn load_current_view(self: &Rc<Self>, slot: PaneSlot) {
        match self.current_view_for(slot) {
            PaneView::Directory(path) => self.load_directory(slot, path),
            PaneView::Tag(tag) => self.load_tag_view(slot, tag),
            PaneView::DownloadsTriage(_) => self.load_downloads_triage(slot),
            PaneView::SystemDrives => self.load_system_drives_view(slot),
            PaneView::Recent => self.load_recent_view(slot),
            PaneView::Trash => self.load_trash_view(slot),
            PaneView::Search(query) => self.load_search_view(slot, query),
        }
    }

    fn connect_tab_strip(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.tab_strip
            .new_tab_button
            .connect_clicked(move |_| controller.open_new_tab(None));
    }

    fn connect_preview_actions(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.preview
            .open_button
            .connect_clicked(move |_| controller.open_preview_target());

        let controller = Rc::clone(self);
        self.preview
            .copy_path_button
            .connect_clicked(move |_| controller.copy_preview_target_path());

        let controller = Rc::clone(self);
        self.preview
            .open_parent_button
            .connect_clicked(move |_| controller.open_preview_parent());
    }

    fn connect_panes(self: &Rc<Self>) {
        self.connect_pane(PaneSlot::Primary);
        self.connect_pane(PaneSlot::Secondary);
    }

    fn connect_pane(self: &Rc<Self>, slot: PaneSlot) {
        let pane = self.pane_widgets(slot).clone();
        let controller = Rc::clone(self);
        pane.file_grid
            .flow
            .connect_selected_children_changed(move |_| controller.update_selection_for(slot));

        let controller = Rc::clone(self);
        pane.file_grid
            .flow
            .connect_child_activated(move |_, child| {
                controller.set_active_pane(slot);
                controller.activate_index(slot, child.index())
            });

        let controller = Rc::clone(self);
        let flow = pane.file_grid.flow.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        gesture.connect_pressed(move |_, _, x, y| {
            controller.set_active_pane(slot);
            if flow.child_at_pos(x as i32, y as i32).is_none() {
                controller.show_current_folder_menu(slot, x, y);
            }
        });
        pane.file_grid.flow.add_controller(gesture);

        let controller = Rc::clone(self);
        let flow_click = gtk::GestureClick::new();
        flow_click.set_button(0);
        flow_click.connect_pressed(move |_, _, _, _| controller.set_active_pane(slot));
        pane.file_grid.flow.add_controller(flow_click);

        let controller = Rc::clone(self);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.connect_pressed(move |_, _, _, _| controller.set_active_pane(slot));
        pane.root.add_controller(click);

        for (filter, button) in pane.triage_filters.iter() {
            let controller = Rc::clone(self);
            let filter = *filter;
            button.connect_clicked(move |_| controller.set_triage_filter(slot, filter));
        }
    }

    fn handle_window_command(self: &Rc<Self>, command: WindowCommand) -> bool {
        match command {
            WindowCommand::CopySelection => {
                self.copy_selected_to_file_clipboard(ClipboardMode::Copy);
                true
            }
            WindowCommand::CutSelection => {
                self.copy_selected_to_file_clipboard(ClipboardMode::Cut);
                true
            }
            WindowCommand::PasteClipboard => {
                self.paste_file_clipboard_into_active_pane();
                true
            }
            WindowCommand::CopyPathText => {
                self.copy_path_text_for_active_context();
                true
            }
            WindowCommand::NewFolder => {
                self.create_new_folder();
                true
            }
            WindowCommand::NewTextDocument => {
                self.create_new_text_document();
                true
            }
            WindowCommand::RenameSelection => {
                self.rename_selected();
                true
            }
            WindowCommand::TrashSelection => {
                self.trash_selected();
                true
            }
            WindowCommand::OpenSearch => {
                self.open_search_in_current_dir();
                true
            }
            WindowCommand::ToggleFilter => {
                let is_open = self
                    .pane_widgets(self.active_slot())
                    .tag_filter
                    .root
                    .is_visible();
                self.set_filter_panel_open(!is_open);
                true
            }
            WindowCommand::FocusPath => {
                self.begin_path_entry_editing();
                true
            }
            WindowCommand::Refresh => {
                self.refresh();
                true
            }
            WindowCommand::ToggleHidden => {
                self.set_show_hidden(!self.show_hidden.get());
                true
            }
            WindowCommand::ToggleSidebar => {
                self.set_sidebar_visible(!self.sidebar_visible.get());
                true
            }
            WindowCommand::TogglePreview => {
                self.set_preview_visible(!self.preview_visible.get());
                true
            }
            WindowCommand::NewTab => {
                self.open_new_tab(None);
                true
            }
            WindowCommand::CloseTab => {
                self.close_tab(self.active_tab.get());
                true
            }
            WindowCommand::ToggleSplit => {
                self.set_split_enabled(!self.split_enabled.get());
                true
            }
            WindowCommand::PreviousTab => {
                self.switch_tab_relative(-1);
                true
            }
            WindowCommand::NextTab => {
                self.switch_tab_relative(1);
                true
            }
            WindowCommand::GoBack => {
                self.go_back();
                true
            }
            WindowCommand::GoUp => {
                self.go_up();
                true
            }
            WindowCommand::CyclePane => {
                self.cycle_active_pane();
                true
            }
            WindowCommand::Escape => self.handle_escape(self.focused_context()),
            WindowCommand::CustomAction(id) => {
                self.run_custom_action_by_id(&id);
                true
            }
        }
    }

    fn handle_escape(self: &Rc<Self>, focus: FocusedContext) -> bool {
        match focus {
            FocusedContext::PathEntry => {
                self.cancel_path_entry_editing();
                true
            }
            FocusedContext::SearchEntry => self.exit_search_if_empty(self.active_slot()),
            FocusedContext::EditableText => false,
            _ => {
                if self.dismiss_context_menu() {
                    return true;
                }
                if self.exit_search_if_empty(self.active_slot()) {
                    return true;
                }
                let slot = self.active_slot();
                if self.pane_widgets(slot).tag_filter.root.is_visible()
                    && !self.pane_widgets(slot).tag_filter.is_filtering()
                {
                    self.set_filter_panel_open(false);
                    return true;
                }
                if !self.selected_paths().is_empty() {
                    self.clear_selection_in_active_pane();
                    return true;
                }
                false
            }
        }
    }

    fn handle_file_grid_key(
        self: &Rc<Self>,
        focus: FocusedContext,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> bool {
        if focus != FocusedContext::FileGrid {
            return false;
        }

        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        if alt {
            return false;
        }

        match key {
            gdk::Key::Return | gdk::Key::KP_Enter => {
                self.activate_keyboard_target();
                true
            }
            gdk::Key::Left => {
                self.move_grid_keyboard_selection(-1, ctrl, shift);
                true
            }
            gdk::Key::Right => {
                self.move_grid_keyboard_selection(1, ctrl, shift);
                true
            }
            gdk::Key::Up => {
                let columns = self
                    .pane_widgets(self.active_slot())
                    .file_grid
                    .estimated_columns();
                self.move_grid_keyboard_selection(-columns, ctrl, shift);
                true
            }
            gdk::Key::Down => {
                let columns = self
                    .pane_widgets(self.active_slot())
                    .file_grid
                    .estimated_columns();
                self.move_grid_keyboard_selection(columns, ctrl, shift);
                true
            }
            _ if ctrl && key_char(key) == Some('a') => {
                self.select_all_in_active_pane();
                true
            }
            _ if key_char(key) == Some(' ') => {
                self.apply_space_selection(ctrl, shift);
                true
            }
            _ => false,
        }
    }

    fn activate_keyboard_target(self: &Rc<Self>) {
        let slot = self.active_slot();
        let target = self.keyboard_current_cell(slot).get().or_else(|| {
            self.pane_widgets(slot)
                .file_grid
                .selected_indices()
                .into_iter()
                .next()
        });
        if let Some(index) = target {
            self.activate_index(slot, index);
        }
    }

    fn move_grid_keyboard_selection(self: &Rc<Self>, offset: i32, ctrl: bool, shift: bool) {
        let slot = self.active_slot();
        let count = self.pane_widgets(slot).file_grid.child_count();
        if count <= 0 {
            return;
        }

        let current = self
            .keyboard_current_cell(slot)
            .get()
            .or_else(|| {
                self.pane_widgets(slot)
                    .file_grid
                    .selected_indices()
                    .into_iter()
                    .next()
            })
            .unwrap_or(0);
        let target = if self
            .pane_widgets(slot)
            .file_grid
            .selected_indices()
            .is_empty()
            && self.keyboard_current_cell(slot).get().is_none()
        {
            0
        } else {
            (current + offset).clamp(0, count - 1)
        };

        if ctrl && !shift {
            self.set_keyboard_focus(slot, target, true);
            return;
        }

        if shift {
            let anchor = self.keyboard_anchor_cell(slot).get().unwrap_or(current);
            self.select_range_in_slot(slot, anchor, target, true);
            self.keyboard_anchor_cell(slot).set(Some(anchor));
            self.keyboard_current_cell(slot).set(Some(target));
            self.pane_widgets(slot).file_grid.focus_index(target);
            if slot == self.active_slot() {
                self.update_selection();
            }
            return;
        }

        self.select_only_in_slot(slot, target);
    }

    fn apply_space_selection(self: &Rc<Self>, ctrl: bool, shift: bool) {
        let slot = self.active_slot();
        let count = self.pane_widgets(slot).file_grid.child_count();
        if count <= 0 {
            return;
        }

        let index = self
            .keyboard_current_cell(slot)
            .get()
            .or_else(|| {
                self.pane_widgets(slot)
                    .file_grid
                    .selected_indices()
                    .into_iter()
                    .next()
            })
            .unwrap_or(0);

        if shift {
            let anchor = self.keyboard_anchor_cell(slot).get().unwrap_or(index);
            self.select_range_in_slot(slot, anchor, index, true);
            self.keyboard_anchor_cell(slot).set(Some(anchor));
            self.keyboard_current_cell(slot).set(Some(index));
        } else if ctrl {
            self.pane_widgets(slot).file_grid.toggle_index(index);
            self.keyboard_anchor_cell(slot).set(Some(index));
            self.keyboard_current_cell(slot).set(Some(index));
            self.pane_widgets(slot).file_grid.focus_index(index);
        } else {
            self.select_only_in_slot(slot, index);
        }

        if slot == self.active_slot() {
            self.update_selection();
        }
    }

    fn set_keyboard_focus(&self, slot: PaneSlot, index: i32, update_anchor: bool) {
        self.keyboard_current_cell(slot).set(Some(index));
        if update_anchor {
            self.keyboard_anchor_cell(slot).set(Some(index));
        }
        self.pane_widgets(slot).file_grid.focus_index(index);
    }

    fn select_only_in_slot(self: &Rc<Self>, slot: PaneSlot, index: i32) {
        self.pane_widgets(slot).file_grid.select_only_index(index);
        self.set_keyboard_focus(slot, index, true);
        if slot == self.active_slot() {
            self.update_selection();
        }
    }

    fn select_range_in_slot(&self, slot: PaneSlot, anchor: i32, target: i32, clear_first: bool) {
        self.pane_widgets(slot)
            .file_grid
            .select_range(anchor, target, clear_first);
        self.pane_widgets(slot).file_grid.focus_index(target);
    }

    fn select_all_in_active_pane(self: &Rc<Self>) {
        let slot = self.active_slot();
        let count = self.pane_widgets(slot).file_grid.child_count();
        if count <= 0 {
            return;
        }

        self.pane_widgets(slot)
            .file_grid
            .select_range(0, count - 1, true);
        self.keyboard_anchor_cell(slot).set(Some(0));
        self.keyboard_current_cell(slot).set(Some(count - 1));
        self.pane_widgets(slot).file_grid.focus_index(count - 1);
        self.update_selection();
    }

    fn clear_selection_in_active_pane(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.pane_widgets(slot).file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.update_selection();
        self.pane_widgets(slot).file_grid.flow.grab_focus();
    }

    fn reset_keyboard_state(&self, slot: PaneSlot) {
        self.keyboard_anchor_cell(slot).set(None);
        self.keyboard_current_cell(slot).set(None);
    }

    fn sync_keyboard_state_from_selection(&self, slot: PaneSlot) {
        let selected = self.pane_widgets(slot).file_grid.selected_indices();
        if let Some(index) = selected.first().copied() {
            self.keyboard_current_cell(slot).set(Some(index));
            if selected.len() == 1 {
                self.keyboard_anchor_cell(slot).set(Some(index));
            } else if self.keyboard_anchor_cell(slot).get().is_none() {
                self.keyboard_anchor_cell(slot).set(Some(index));
            }
        } else {
            self.reset_keyboard_state(slot);
        }
    }

    fn cycle_active_pane(self: &Rc<Self>) {
        if !self.split_enabled.get() {
            return;
        }

        let next = if self.active_slot() == PaneSlot::Primary {
            PaneSlot::Secondary
        } else {
            PaneSlot::Primary
        };
        self.set_active_pane(next);
        self.pane_widgets(next).file_grid.flow.grab_focus();
    }

    fn switch_tab_relative(self: &Rc<Self>, offset: i32) {
        let count = self.tabs.borrow().len();
        if count <= 1 {
            return;
        }

        let current = self.active_tab.get() as i32;
        let next = (current + offset).rem_euclid(count as i32) as usize;
        self.switch_to_tab(next);
    }

    fn pane_widgets(&self, slot: PaneSlot) -> &PaneWidgets {
        match slot {
            PaneSlot::Primary => &self.primary_pane,
            PaneSlot::Secondary => &self.secondary_pane,
        }
    }

    fn current_dir_cell(&self, slot: PaneSlot) -> &RefCell<PathBuf> {
        match slot {
            PaneSlot::Primary => &self.current_dir,
            PaneSlot::Secondary => &self.secondary_current_dir,
        }
    }

    fn current_view_cell(&self, slot: PaneSlot) -> &RefCell<PaneView> {
        match slot {
            PaneSlot::Primary => &self.current_view,
            PaneSlot::Secondary => &self.secondary_view,
        }
    }

    fn back_history_cell(&self, slot: PaneSlot) -> &RefCell<Vec<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.back_history,
            PaneSlot::Secondary => &self.secondary_back_history,
        }
    }

    fn items_cell(&self, slot: PaneSlot) -> &RefCell<Vec<FileItem>> {
        match slot {
            PaneSlot::Primary => &self.items,
            PaneSlot::Secondary => &self.secondary_items,
        }
    }

    fn all_items_cell(&self, slot: PaneSlot) -> &RefCell<Vec<FileItem>> {
        match slot {
            PaneSlot::Primary => &self.primary_all_items,
            PaneSlot::Secondary => &self.secondary_all_items,
        }
    }

    fn pending_reveal_cell(&self, slot: PaneSlot) -> &RefCell<Option<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.pending_reveal_path,
            PaneSlot::Secondary => &self.secondary_pending_reveal_path,
        }
    }

    fn load_generation_cell(&self, slot: PaneSlot) -> &Cell<u64> {
        match slot {
            PaneSlot::Primary => &self.load_generation,
            PaneSlot::Secondary => &self.secondary_load_generation,
        }
    }

    fn load_cancellable_cell(&self, slot: PaneSlot) -> &RefCell<Option<gio::Cancellable>> {
        match slot {
            PaneSlot::Primary => &self.load_cancellable,
            PaneSlot::Secondary => &self.secondary_load_cancellable,
        }
    }

    fn keyboard_anchor_cell(&self, slot: PaneSlot) -> &Cell<Option<i32>> {
        match slot {
            PaneSlot::Primary => &self.primary_keyboard_anchor,
            PaneSlot::Secondary => &self.secondary_keyboard_anchor,
        }
    }

    fn keyboard_current_cell(&self, slot: PaneSlot) -> &Cell<Option<i32>> {
        match slot {
            PaneSlot::Primary => &self.primary_keyboard_current,
            PaneSlot::Secondary => &self.secondary_keyboard_current,
        }
    }

    fn current_dir_for(&self, slot: PaneSlot) -> PathBuf {
        self.current_dir_cell(slot).borrow().clone()
    }

    fn current_view_for(&self, slot: PaneSlot) -> PaneView {
        self.current_view_cell(slot).borrow().clone()
    }

    fn is_directory_view(&self, slot: PaneSlot) -> bool {
        matches!(self.current_view_for(slot), PaneView::Directory(_))
    }

    fn display_label_for(&self, slot: PaneSlot) -> String {
        view_display_label(&self.current_view_for(slot), &self.places.home)
    }

    fn current_item_count_for(&self, slot: PaneSlot) -> usize {
        self.items_cell(slot).borrow().len()
    }

    fn active_slot(&self) -> PaneSlot {
        self.active_pane.get()
    }

    fn other_slot(slot: PaneSlot) -> PaneSlot {
        match slot {
            PaneSlot::Primary => PaneSlot::Secondary,
            PaneSlot::Secondary => PaneSlot::Primary,
        }
    }

    fn sync_active_tab_state(&self) {
        let active_index = self.active_tab.get();
        if let Some(tab) = self.tabs.borrow_mut().get_mut(active_index) {
            tab.primary_dir = self.current_dir.borrow().clone();
            tab.primary_back_history = self.back_history.borrow().clone();
            tab.primary_view = self.current_view.borrow().clone();
            tab.secondary_dir = self.secondary_current_dir.borrow().clone();
            tab.secondary_back_history = self.secondary_back_history.borrow().clone();
            tab.secondary_view = self.secondary_view.borrow().clone();
            tab.split_enabled = self.split_enabled.get();
            tab.active_pane = self.active_pane.get();
            tab.title = tab_title_for_view(&tab.primary_view, &tab.primary_dir);
        }
    }

    fn rebuild_tab_strip(self: &Rc<Self>) {
        while let Some(child) = self.tab_strip.tabs_box.first_child() {
            self.tab_strip.tabs_box.remove(&child);
        }

        let tabs = self.tabs.borrow().clone();
        let active_index = self.active_tab.get();
        let can_close = tabs.len() > 1;

        for (index, tab) in tabs.into_iter().enumerate() {
            let tab_chip = GtkBox::new(Orientation::Horizontal, 4);
            tab_chip.add_css_class("tab-chip");
            if index == active_index {
                tab_chip.add_css_class("active");
            }

            let tab_button = Button::with_label(&tab.title);
            tab_button.add_css_class("tab-button");
            if index == active_index {
                tab_button.add_css_class("active");
            }
            crate::ui::attach_tooltip(
                &tab_button,
                view_display_label(&tab.primary_view, &self.places.home),
            );
            tab_button.set_focus_on_click(true);
            let controller = Rc::clone(self);
            tab_button.connect_clicked(move |_| controller.switch_to_tab(index));

            let close_button = Button::builder().icon_name("window-close-symbolic").build();
            close_button.add_css_class("tab-close-button");
            let close_host = crate::ui::tooltip_host(&close_button, "Close tab (Ctrl+W)");
            close_button.set_sensitive(can_close);
            let controller = Rc::clone(self);
            close_button.connect_clicked(move |_| controller.close_tab(index));

            tab_chip.append(&tab_button);
            tab_chip.append(&close_host);
            self.tab_strip.tabs_box.append(&tab_chip);
        }
    }

    fn open_new_tab(self: &Rc<Self>, path: Option<PathBuf>) {
        self.sync_active_tab_state();
        let target = path.unwrap_or_else(|| self.current_dir_for(self.active_slot()));
        self.tabs.borrow_mut().push(TabState::new(target));
        let new_index = self.tabs.borrow().len().saturating_sub(1);
        self.active_tab.set(new_index);
        self.reload_active_tab();
    }

    fn close_tab(self: &Rc<Self>, index: usize) {
        if self.tabs.borrow().len() <= 1 {
            return;
        }

        self.sync_active_tab_state();
        {
            let mut tabs = self.tabs.borrow_mut();
            if index >= tabs.len() {
                return;
            }
            tabs.remove(index);
        }

        let next_active = if self.active_tab.get() > index {
            self.active_tab.get() - 1
        } else if self.active_tab.get() == index {
            index.min(self.tabs.borrow().len().saturating_sub(1))
        } else {
            self.active_tab.get()
        };
        self.active_tab.set(next_active);
        self.reload_active_tab();
    }

    fn switch_to_tab(self: &Rc<Self>, index: usize) {
        if index == self.active_tab.get() || index >= self.tabs.borrow().len() {
            return;
        }

        self.sync_active_tab_state();
        self.active_tab.set(index);
        self.reload_active_tab();
    }

    fn reload_active_tab(self: &Rc<Self>) {
        let Some(tab) = self.tabs.borrow().get(self.active_tab.get()).cloned() else {
            return;
        };

        self.current_dir.replace(tab.primary_dir.clone());
        self.back_history.replace(tab.primary_back_history.clone());
        self.current_view.replace(tab.primary_view.clone());
        self.secondary_current_dir
            .replace(tab.secondary_dir.clone());
        self.secondary_back_history
            .replace(tab.secondary_back_history.clone());
        self.secondary_view.replace(tab.secondary_view.clone());
        self.split_enabled.set(tab.split_enabled);
        self.active_pane.set(if tab.split_enabled {
            tab.active_pane
        } else {
            PaneSlot::Primary
        });

        if self.toolbar.split_toggle.is_active() != self.split_enabled.get() {
            self.toolbar
                .split_toggle
                .set_active(self.split_enabled.get());
        }

        self.rebuild_tab_strip();
        self.sync_split_visibility();
        self.update_active_pane_visuals();
        self.update_view_strip(PaneSlot::Primary);
        self.load_current_view(PaneSlot::Primary);
        if self.split_enabled.get() {
            self.update_view_strip(PaneSlot::Secondary);
            self.load_current_view(PaneSlot::Secondary);
        } else {
            self.secondary_pane.file_grid.clear_selection();
            self.reset_keyboard_state(PaneSlot::Secondary);
        }
    }

    fn update_active_pane_visuals(&self) {
        let active = self.active_slot();
        for slot in [PaneSlot::Primary, PaneSlot::Secondary] {
            let pane = self.pane_widgets(slot);
            if slot == active {
                pane.root.add_css_class("active");
                pane.root.add_css_class("browser-pane-active");
            } else {
                pane.root.remove_css_class("active");
                pane.root.remove_css_class("browser-pane-active");
            }
        }
    }

    fn sync_split_visibility(&self) {
        let enabled = self.split_enabled.get();
        self.secondary_pane.root.set_visible(enabled);
        self.secondary_pane.path_label.set_visible(enabled);
        self.secondary_pane
            .view_strip
            .set_visible(enabled && self.secondary_pane.view_strip.is_visible());
    }

    fn set_split_enabled(self: &Rc<Self>, enabled: bool) {
        if self.split_enabled.get() == enabled {
            return;
        }

        self.split_enabled.set(enabled);
        if self.toolbar.split_toggle.is_active() != enabled {
            self.toolbar.split_toggle.set_active(enabled);
        }
        if !enabled && self.active_slot() == PaneSlot::Secondary {
            self.active_pane.set(PaneSlot::Primary);
        }

        self.sync_active_tab_state();
        self.rebuild_tab_strip();
        self.sync_split_visibility();
        self.update_active_pane_visuals();

        if enabled {
            self.update_view_strip(PaneSlot::Secondary);
            self.load_current_view(PaneSlot::Secondary);
        }

        self.update_navigation_state();
        self.update_sidebar_state();
        self.sync_path_entry_to_display();
        self.update_selection();
    }

    fn set_active_pane(self: &Rc<Self>, slot: PaneSlot) {
        let target = if slot == PaneSlot::Secondary && !self.split_enabled.get() {
            PaneSlot::Primary
        } else {
            slot
        };

        if self.active_slot() == target {
            return;
        }

        self.active_pane.set(target);
        self.sync_active_tab_state();
        self.update_active_pane_visuals();
        self.status.set_path(&self.display_label_for(target));
        self.update_navigation_state();
        self.update_sidebar_state();
        self.sync_path_entry_to_display();
        self.update_selection();
    }

    fn update_selection_for(self: &Rc<Self>, slot: PaneSlot) {
        self.sync_keyboard_state_from_selection(slot);
        if slot == self.active_slot() {
            self.update_selection();
        }
    }

    fn navigate_to(self: &Rc<Self>, slot: PaneSlot, path: PathBuf, remember_current: bool) {
        if remember_current {
            let current = self.current_dir_for(slot);
            if current != path {
                self.back_history_cell(slot).borrow_mut().push(current);
            }
        }

        self.current_dir_cell(slot).replace(path.clone());
        self.current_view_cell(slot)
            .replace(PaneView::Directory(path.clone()));
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_directory(slot, path);
    }

    fn go_back(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !self.is_directory_view(slot) {
            self.status
                .set_message("Back is only available in directory views.");
            return;
        }
        let previous = self.back_history_cell(slot).borrow_mut().pop();
        if let Some(path) = previous {
            self.current_dir_cell(slot).replace(path.clone());
            self.current_view_cell(slot)
                .replace(PaneView::Directory(path.clone()));
            self.sync_active_tab_state();
            self.update_view_strip(slot);
            if slot == PaneSlot::Primary {
                self.rebuild_tab_strip();
            }
            self.load_directory(slot, path);
        }
    }

    fn go_up(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !self.is_directory_view(slot) {
            self.status
                .set_message("Up is only available in directory views.");
            return;
        }
        let current = self.current_dir_for(slot);
        if let Some(parent) = current.parent() {
            self.navigate_to(slot, parent.to_path_buf(), true);
        }
    }

    fn refresh(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.load_current_view(slot);
        if self.split_enabled.get() {
            let other = Self::other_slot(slot);
            self.load_current_view(other);
        }
    }

    fn set_show_hidden(self: &Rc<Self>, show_hidden: bool) {
        if self.toolbar.show_hidden_toggle.is_active() != show_hidden {
            self.toolbar.show_hidden_toggle.set_active(show_hidden);
        }
        if self.show_hidden.get() == show_hidden {
            return;
        }
        self.show_hidden.set(show_hidden);
        self.reload_active_tab();
    }

    fn navigate_from_path_entry(self: &Rc<Self>) {
        let raw_input = self.toolbar.path_entry.text().trim().to_string();
        if raw_input.is_empty() {
            self.sync_path_entry_to_display();
            return;
        }

        let Some(target_file) = self.resolve_path_input(&raw_input) else {
            self.show_error_dialog(
                "Invalid Path",
                "Lattice could not resolve that path input. Use an absolute path, `~`, `~/...`, or a path relative to the current folder.",
            );
            self.sync_path_entry_to_display();
            return;
        };

        let Some(target_path) = target_file.path() else {
            self.show_error_dialog(
                "Unsupported Path",
                "That location could not be resolved to a local filesystem path.",
            );
            self.sync_path_entry_to_display();
            return;
        };

        let file_type =
            target_file.query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>);
        if file_type == gio::FileType::Unknown
            && !target_file.query_exists(None::<&gio::Cancellable>)
        {
            self.show_error_dialog(
                "Path Not Found",
                &format!("No file or folder exists at:\n\n{}", target_path.display()),
            );
            self.sync_path_entry_to_display();
            return;
        }

        if file_type == gio::FileType::Directory {
            self.pending_reveal_cell(self.active_slot())
                .borrow_mut()
                .take();
            self.toolbar
                .path_entry
                .set_text(&target_path.display().to_string());
            self.navigate_to(self.active_slot(), target_path, true);
            return;
        }

        let Some(parent) = target_path.parent().map(Path::to_path_buf) else {
            self.show_error_dialog(
                "Parent Folder Unavailable",
                "Lattice could not open the parent folder for that file path.",
            );
            self.sync_path_entry_to_display();
            return;
        };

        self.pending_reveal_cell(self.active_slot())
            .replace(Some(target_path.clone()));
        self.pending_status_message.replace(Some(format!(
            "Opened parent folder for {}.",
            target_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("typed path")
        )));

        if parent == self.current_dir_for(self.active_slot()) {
            self.reveal_pending_selection(self.active_slot());
            self.refresh_preview();
            self.status
                .set_message("Opened parent folder for the typed file path.");
            self.sync_path_entry_to_display();
        } else {
            self.toolbar
                .path_entry
                .set_text(&parent.display().to_string());
            self.navigate_to(self.active_slot(), parent, true);
        }
    }

    fn begin_path_entry_editing(self: &Rc<Self>) {
        let absolute = self
            .current_dir_for(self.active_slot())
            .display()
            .to_string();
        self.toolbar.show_entry_mode();
        self.toolbar.path_entry.set_text(&absolute);
        let entry = self.toolbar.path_entry.clone();
        glib::idle_add_local_once(move || {
            entry.grab_focus();
            entry.select_region(0, -1);
        });
    }

    fn finish_path_entry_editing(&self) {
        self.sync_path_entry_to_display();
    }

    fn cancel_path_entry_editing(&self) {
        self.sync_path_entry_to_display();
        self.pane_widgets(self.active_slot())
            .file_grid
            .flow
            .grab_focus();
    }

    fn apply_sidebar_visibility(&self, visible: bool) {
        if self.toolbar.sidebar_toggle.is_active() != visible {
            self.suppress_panel_toggle_handlers.set(true);
            self.toolbar.sidebar_toggle.set_active(visible);
            self.suppress_panel_toggle_handlers.set(false);
        }
        if self.sidebar_visible.get() == visible {
            return;
        }
        self.sidebar_visible.set(visible);
        self.sidebar.root.set_visible(visible);
    }

    fn set_sidebar_visible(self: &Rc<Self>, visible: bool) {
        self.apply_sidebar_visibility(visible);
    }

    fn apply_preview_visibility(self: &Rc<Self>, visible: bool) {
        if self.toolbar.preview_toggle.is_active() != visible {
            self.suppress_panel_toggle_handlers.set(true);
            self.toolbar.preview_toggle.set_active(visible);
            self.suppress_panel_toggle_handlers.set(false);
        }
        if self.preview_visible.get() == visible {
            return;
        }
        self.preview_visible.set(visible);
        self.preview_host.set_visible(visible);
        self.cancel_active_preview();

        if visible {
            self.refresh_preview();
        }
    }

    fn set_preview_visible(self: &Rc<Self>, visible: bool) {
        self.apply_preview_visibility(visible);
    }

    fn load_directory(self: &Rc<Self>, slot: PaneSlot, path: PathBuf) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_path = match self.current_view_for(slot) {
            PaneView::Directory(_) => self.display_path(&path),
            _ => self.display_label_for(slot),
        };
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_path);
        pane.file_grid.set_loading();
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_path);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_path);
            self.status.clear_message();
            self.status.set_counts(0, 0);
        }

        if slot == self.active_slot() && self.preview_visible.get() {
            self.preview.show_loading(
                "Current Folder",
                &crate::ui::file_grid::FileKind::Folder,
                "Loading folder preview…",
            );
            self.preview.set_action_state(false, false, false);
        }
        if slot == self.active_slot() {
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }

        let generation = self.load_generation_cell(slot).get() + 1;
        self.load_generation_cell(slot).set(generation);

        let cancellable = gio::Cancellable::new();
        self.load_cancellable_cell(slot)
            .replace(Some(cancellable.clone()));

        let directory = gio::File::for_path(&path);
        let directory_for_callback = directory.clone();
        let cancellable_for_callback = cancellable.clone();
        let controller = Rc::clone(self);
        directory.enumerate_children_async(
            DIRECTORY_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                if !controller.is_current_load(slot, generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                match result {
                    Ok(enumerator) => {
                        let collected = Rc::new(RefCell::new(Vec::new()));
                        controller.read_directory_batch(
                            slot,
                            directory_for_callback.clone(),
                            enumerator,
                            collected,
                            generation,
                            path.clone(),
                            cancellable_for_callback.clone(),
                        );
                    }
                    Err(error) => {
                        controller.finish_load_error(slot, generation, &path, &error);
                    }
                }
            },
        );
    }

    fn read_directory_batch(
        self: &Rc<Self>,
        slot: PaneSlot,
        directory: gio::File,
        enumerator: gio::FileEnumerator,
        collected: Rc<RefCell<Vec<FileItem>>>,
        generation: u64,
        path: PathBuf,
        cancellable: gio::Cancellable,
    ) {
        let controller = Rc::clone(self);
        let enumerator_for_callback = enumerator.clone();
        let cancellable_for_callback = cancellable.clone();
        enumerator.next_files_async(
            64,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                if !controller.is_current_load(slot, generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                match result {
                    Ok(batch) if batch.is_empty() => {
                        let mut items = collected.borrow().clone();
                        sort_items(&mut items);
                        controller.finish_load(slot, generation, &path, items);
                    }
                    Ok(batch) => {
                        let show_hidden = controller.show_hidden.get();
                        {
                            let mut collected_ref = collected.borrow_mut();
                            for info in batch {
                                if let Some(item) =
                                    FileItem::from_info(&directory, &info, show_hidden)
                                {
                                    collected_ref.push(item);
                                }
                            }
                        }

                        controller.read_directory_batch(
                            slot,
                            directory.clone(),
                            enumerator_for_callback.clone(),
                            collected.clone(),
                            generation,
                            path.clone(),
                            cancellable_for_callback.clone(),
                        );
                    }
                    Err(error) => {
                        controller.finish_load_error(slot, generation, &path, &error);
                    }
                }
            },
        );
    }

    fn finish_load(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        path: &Path,
        items: Vec<FileItem>,
    ) {
        if !self.is_current_load(slot, generation) {
            return;
        }

        self.load_cancellable_cell(slot).borrow_mut().take();
        let mut items = self.enrich_items_with_tags(items);
        if let PaneView::DownloadsTriage(filter) = self.current_view_for(slot) {
            items = filter_triage_items(items, filter);
        }
        if matches!(self.current_view_for(slot), PaneView::Directory(_)) {
            if let Err(error) = self.metadata.borrow_mut().record_recent_location(path) {
                eprintln!("Lattice recent-location update failed: {error}");
            }
        }
        // Store the full (unfiltered) item list so tag filter can re-apply
        self.all_items_cell(slot).replace(items.clone());
        // Apply tag filter for directory views
        let spec = self.pane_widgets(slot).tag_filter.spec();
        let displayed =
            if spec.is_empty() || !matches!(self.current_view_for(slot), PaneView::Directory(_)) {
                items
            } else {
                items
                    .into_iter()
                    .filter(|item| spec.matches(item))
                    .collect()
            };
        self.items_cell(slot).replace(displayed.clone());
        self.pane_widgets(slot).file_grid.set_items(&displayed);
        let thumb_targets = self.pane_widgets(slot).file_grid.drain_thumb_targets();
        if !thumb_targets.is_empty() {
            self.thumb_loader_for(slot).submit(thumb_targets);
        }
        self.attach_context_handlers(slot);
        self.attach_item_dnd(slot);
        let revealed = self.reveal_pending_selection(slot);

        let display_path = match self.current_view_for(slot) {
            PaneView::Directory(_) => self.display_path(path),
            _ => self.display_label_for(slot),
        };
        self.pane_widgets(slot).path_label.set_label(&display_path);
        if slot == self.active_slot() {
            self.status.set_path(&display_path);
            self.toolbar.set_breadcrumb_path(&display_path);
            self.toolbar.show_breadcrumb_mode();
            if let Some(message) = self.pending_status_message.borrow_mut().take() {
                self.status.set_message(&message);
            }
            self.update_navigation_state();
            self.update_action_state();
            if revealed {
                self.update_selection();
            } else {
                self.show_empty_selection_preview(slot, &display_path, displayed.len());
                self.status.set_counts(displayed.len(), 0);
                self.refresh_preview();
            }
        }
    }

    fn finish_load_error(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        path: &Path,
        error: &glib::Error,
    ) {
        if !self.is_current_load(slot, generation) {
            return;
        }

        self.load_cancellable_cell(slot).borrow_mut().take();
        self.items_cell(slot).borrow_mut().clear();
        self.pane_widgets(slot)
            .file_grid
            .set_empty_message(match self.current_view_for(slot) {
                PaneView::Directory(_) => "Unable to read this folder.",
                PaneView::Tag(_) => "Unable to read tagged files.",
                PaneView::DownloadsTriage(_) => "Unable to read Downloads.",
                PaneView::SystemDrives => "Unable to read mounted volumes.",
                PaneView::Recent => "Unable to read recent folders.",
                PaneView::Trash => "Unable to read Trash.",
                PaneView::Search(_) => "Search failed.",
            });

        let display_path = match self.current_view_for(slot) {
            PaneView::Directory(_) => self.display_path(path),
            _ => self.display_label_for(slot),
        };
        self.pane_widgets(slot).path_label.set_label(&display_path);
        if slot == self.active_slot() {
            self.preview
                .show_error(&display_path, &friendly_error_detail(error));
            self.status.set_counts(0, 0);
            self.status.set_message("Unable to read this folder");
            self.status.set_path(&display_path);
            self.toolbar.set_breadcrumb_path(&display_path);
            self.toolbar.show_breadcrumb_mode();
            self.update_navigation_state();
            self.update_action_state();
            self.preview.set_action_state(false, false, false);
        }
    }

    fn load_downloads_triage(self: &Rc<Self>, slot: PaneSlot) {
        self.current_dir_cell(slot)
            .replace(self.places.downloads.clone());
        self.load_directory(slot, self.places.downloads.clone());
    }

    fn open_system_drives(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.current_dir_cell(slot)
            .replace(self.places.home.clone());
        self.current_view_cell(slot).replace(PaneView::SystemDrives);
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_system_drives_view(slot);
    }

    fn load_system_drives_view(self: &Rc<Self>, slot: PaneSlot) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_label = self.display_label_for(slot);
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.set_loading();
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }

        if slot == self.active_slot() && self.preview_visible.get() {
            self.preview
                .show_loading("System Drives", &FileKind::Folder, "Loading volumes…");
            self.preview.set_action_state(false, false, false);
        }

        let listing = collect_mounted_volume_items();
        let items = listing.items;
        self.items_cell(slot).replace(items.clone());

        if items.is_empty() {
            let empty_message = if listing.skipped_non_local > 0 {
                "No mounted local drives are available to browse."
            } else {
                "No mounted drives or volumes are available."
            };
            self.pane_widgets(slot)
                .file_grid
                .set_empty_message(empty_message);
        } else {
            self.pane_widgets(slot).file_grid.set_items(&items);
            self.attach_context_handlers(slot);
        }

        self.pane_widgets(slot).path_label.set_label(&display_label);
        if slot == self.active_slot() {
            self.status.set_path(&display_label);
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.update_navigation_state();
            self.update_action_state();
            self.show_empty_selection_preview(slot, &display_label, items.len());
            self.status.set_counts(items.len(), 0);
            self.refresh_preview();
        }
    }

    fn open_recent(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.current_dir_cell(slot)
            .replace(self.places.home.clone());
        self.current_view_cell(slot).replace(PaneView::Recent);
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_recent_view(slot);
    }

    fn load_recent_view(self: &Rc<Self>, slot: PaneSlot) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_label = self.display_label_for(slot);
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.set_loading();
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }

        if slot == self.active_slot() && self.preview_visible.get() {
            self.preview.show_loading(
                "Recent Folders",
                &FileKind::Folder,
                "Loading recent folders…",
            );
            self.preview.set_action_state(false, false, false);
        }

        let listing = {
            let mut metadata = self.metadata.borrow_mut();
            collect_recent_folder_items(&mut metadata)
        };

        let items = match listing {
            Ok(listing) => {
                if listing.skipped_missing > 0 && slot == self.active_slot() {
                    self.pending_status_message.replace(Some(format!(
                        "{} missing recent folder(s) were removed.",
                        listing.skipped_missing
                    )));
                }
                listing.items
            }
            Err(error) => {
                self.pane_widgets(slot)
                    .file_grid
                    .set_empty_message("Recent folders are unavailable.");
                if slot == self.active_slot() {
                    self.preview.show_error("Recent Folders", &error);
                    self.status.set_counts(0, 0);
                    self.status.set_message("Recent folders are unavailable");
                    self.status.set_path(&display_label);
                    self.update_action_state();
                    self.preview.set_action_state(false, false, false);
                }
                return;
            }
        };

        self.items_cell(slot).replace(items.clone());
        if items.is_empty() {
            self.pane_widgets(slot)
                .file_grid
                .set_empty_message("No recent folders yet.");
        } else {
            self.pane_widgets(slot).file_grid.set_items(&items);
            self.attach_context_handlers(slot);
        }

        self.pane_widgets(slot).path_label.set_label(&display_label);
        if slot == self.active_slot() {
            self.status.set_path(&display_label);
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            if let Some(message) = self.pending_status_message.borrow_mut().take() {
                self.status.set_message(&message);
            }
            self.update_navigation_state();
            self.update_action_state();
            self.show_empty_selection_preview(slot, &display_label, items.len());
            self.status.set_counts(items.len(), 0);
            self.refresh_preview();
        }
    }

    fn open_trash(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.current_dir_cell(slot)
            .replace(self.places.home.clone());
        self.current_view_cell(slot).replace(PaneView::Trash);
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_trash_view(slot);
    }

    fn load_trash_view(self: &Rc<Self>, slot: PaneSlot) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_label = self.display_label_for(slot);
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.set_loading();
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }

        if slot == self.active_slot() && self.preview_visible.get() {
            self.preview.show_loading(
                "Trash",
                &crate::ui::file_grid::FileKind::Unknown,
                "Loading trash…",
            );
            self.preview.set_action_state(false, false, false);
        }

        let generation = self.load_generation_cell(slot).get() + 1;
        self.load_generation_cell(slot).set(generation);

        let cancellable = gio::Cancellable::new();
        self.load_cancellable_cell(slot)
            .replace(Some(cancellable.clone()));

        let directory = gio::File::for_uri("trash:///");
        let directory_for_callback = directory.clone();
        let cancellable_for_callback = cancellable.clone();
        let controller = Rc::clone(self);
        directory.enumerate_children_async(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                if !controller.is_current_load(slot, generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                match result {
                    Ok(enumerator) => {
                        controller.read_trash_batch(
                            slot,
                            directory_for_callback,
                            enumerator,
                            Rc::new(RefCell::new(Vec::new())),
                            generation,
                            cancellable_for_callback,
                        );
                    }
                    Err(error) => {
                        controller.finish_trash_load_error(slot, generation, &error);
                    }
                }
            },
        );
    }

    fn read_trash_batch(
        self: &Rc<Self>,
        slot: PaneSlot,
        directory: gio::File,
        enumerator: gio::FileEnumerator,
        collected: Rc<RefCell<Vec<FileItem>>>,
        generation: u64,
        cancellable: gio::Cancellable,
    ) {
        let controller = Rc::clone(self);
        let enumerator_for_callback = enumerator.clone();
        let cancellable_for_callback = cancellable.clone();
        enumerator.next_files_async(
            64,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                if !controller.is_current_load(slot, generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                match result {
                    Ok(batch) if batch.is_empty() => {
                        let mut items = collected.borrow().clone();
                        sort_items(&mut items);
                        controller.finish_trash_load(slot, generation, items);
                    }
                    Ok(batch) => {
                        {
                            let mut collected_ref = collected.borrow_mut();
                            for info in &batch {
                                // trash:/// is a GVfs virtual filesystem — child.path()
                                // returns None for every item. Resolve the real path via
                                // standard::target-uri (set by the GVFS trash backend),
                                // then fall back to the FreeDesktop trash files directory.
                                let path = info
                                    .attribute_string("standard::target-uri")
                                    .and_then(|uri| gio::File::for_uri(uri.as_str()).path())
                                    .or_else(|| {
                                        let trash_files =
                                            glib::home_dir().join(".local/share/Trash/files");
                                        Some(trash_files.join(info.name()))
                                    });

                                let Some(path) = path else { continue };

                                let kind = FileKind::from_path(
                                    &path,
                                    info.file_type(),
                                    info.content_type().as_deref(),
                                );
                                let original_path = info
                                    .attribute_byte_string("trash::orig-path")
                                    .map(|s| PathBuf::from(s.as_str()));

                                collected_ref.push(FileItem {
                                    name: info.display_name().to_string(),
                                    path,
                                    is_dir: info.file_type() == gio::FileType::Directory,
                                    kind,
                                    size_bytes: (info.size() >= 0).then_some(info.size() as u64),
                                    modified_unix: info
                                        .modification_date_time()
                                        .map(|dt| dt.to_unix()),
                                    tags: Vec::new(),
                                    original_path,
                                });
                            }
                        }
                        controller.read_trash_batch(
                            slot,
                            directory.clone(),
                            enumerator_for_callback.clone(),
                            collected.clone(),
                            generation,
                            cancellable_for_callback.clone(),
                        );
                    }
                    Err(error) => {
                        controller.finish_trash_load_error(slot, generation, &error);
                    }
                }
            },
        );
    }

    fn finish_trash_load(self: &Rc<Self>, slot: PaneSlot, generation: u64, items: Vec<FileItem>) {
        if !self.is_current_load(slot, generation) {
            return;
        }

        self.load_cancellable_cell(slot).borrow_mut().take();
        self.items_cell(slot).replace(items.clone());

        if items.is_empty() {
            self.pane_widgets(slot)
                .file_grid
                .set_empty_message("Trash is empty");
        } else {
            self.pane_widgets(slot).file_grid.set_items(&items);
            self.attach_context_handlers(slot);
            self.attach_item_dnd(slot);
        }

        let display_label = self.display_label_for(slot);
        self.pane_widgets(slot).path_label.set_label(&display_label);
        if slot == self.active_slot() {
            self.status.set_path(&display_label);
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.update_navigation_state();
            self.update_action_state();
            self.show_empty_selection_preview(slot, &display_label, items.len());
            self.status.set_counts(items.len(), 0);
            self.refresh_preview();
        }
    }

    fn finish_trash_load_error(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        error: &glib::Error,
    ) {
        if !self.is_current_load(slot, generation) {
            return;
        }

        self.load_cancellable_cell(slot).borrow_mut().take();
        self.items_cell(slot).borrow_mut().clear();
        self.pane_widgets(slot)
            .file_grid
            .set_empty_message("Trash is unavailable on this system.");

        let display_label = self.display_label_for(slot);
        self.pane_widgets(slot).path_label.set_label(&display_label);
        if slot == self.active_slot() {
            self.preview
                .show_error("Trash", &friendly_error_detail(error));
            self.status.set_counts(0, 0);
            self.status.set_message("Trash is unavailable");
            self.status.set_path(&display_label);
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.update_navigation_state();
            self.update_action_state();
            self.preview.set_action_state(false, false, false);
        }
    }

    fn restore_items_from_trash(self: &Rc<Self>, items: Vec<FileItem>) {
        if items.is_empty() {
            return;
        }
        let label = if items.len() == 1 {
            format!("Restore: {}", items[0].name)
        } else {
            format!("Restore: {} items from Trash", items.len())
        };
        let cancellable = gio::Cancellable::new();
        let op_id = self.ops_panel.add_op(&label, Some(cancellable.clone()));
        self.run_restore_batch(
            op_id,
            Rc::new(items),
            0,
            cancellable,
            Rc::new(RefCell::new(Vec::new())),
        );
    }

    fn run_restore_batch(
        self: &Rc<Self>,
        op_id: OpId,
        items: Rc<Vec<FileItem>>,
        index: usize,
        cancellable: gio::Cancellable,
        errors: Rc<RefCell<Vec<String>>>,
    ) {
        let total = items.len();

        if index >= total {
            let errs = errors.borrow().clone();
            self.ops_panel.finish_op(op_id, &errs);
            self.load_current_view(PaneSlot::Primary);
            if self.split_enabled.get() {
                self.load_current_view(PaneSlot::Secondary);
            }
            return;
        }

        if cancellable.is_cancelled() {
            self.ops_panel.finish_op(op_id, &["Cancelled.".to_string()]);
            return;
        }

        let item = &items[index];
        let fname = item.name.clone();
        self.ops_panel
            .update_progress(op_id, index as f64 / total as f64, &fname);

        let Some(orig_path) = item.original_path.clone() else {
            errors
                .borrow_mut()
                .push(format!("{fname}: original path not recorded"));
            let controller = Rc::clone(self);
            let items_clone = Rc::clone(&items);
            let errors_clone = Rc::clone(&errors);
            let cancellable_clone = cancellable.clone();
            glib::idle_add_local_once(move || {
                controller.run_restore_batch(
                    op_id,
                    items_clone,
                    index + 1,
                    cancellable_clone,
                    errors_clone,
                );
            });
            return;
        };

        let src_file = gio::File::for_path(&item.path);
        let dst_file = gio::File::for_path(&orig_path);

        let controller = Rc::clone(self);
        let items_clone = Rc::clone(&items);
        let errors_clone = Rc::clone(&errors);
        let cancellable_clone = cancellable.clone();
        let ops_panel = self.ops_panel.clone();
        let base_frac = index as f64 / total as f64;
        let fname_for_cb = fname.clone();

        let progress_cb: Box<dyn FnMut(i64, i64)> = Box::new(move |done: i64, all: i64| {
            let ff = if all > 0 {
                done as f64 / all as f64
            } else {
                0.0
            };
            let overall = (base_frac + ff / total as f64).min(1.0);
            let detail = if all > 0 {
                format!(
                    "{}  {} / {}",
                    fname_for_cb,
                    fmt_bytes(done as u64),
                    fmt_bytes(all as u64)
                )
            } else {
                fname_for_cb.clone()
            };
            ops_panel.update_progress(op_id, overall, &detail);
        });

        src_file.move_async(
            &dst_file,
            gio::FileCopyFlags::ALL_METADATA,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            Some(progress_cb),
            move |result| {
                if let Err(ref e) = result {
                    if e.kind::<gio::IOErrorEnum>() != Some(gio::IOErrorEnum::Cancelled) {
                        errors_clone
                            .borrow_mut()
                            .push(format!("{fname}: {}", e.message()));
                    }
                }
                controller.run_restore_batch(
                    op_id,
                    items_clone,
                    index + 1,
                    cancellable_clone,
                    errors_clone,
                );
            },
        );
    }

    // ── Search ───────────────────────────────────────────────────────────

    fn exit_search_if_empty(self: &Rc<Self>, slot: PaneSlot) -> bool {
        let PaneView::Search(query) = self.current_view_for(slot) else {
            return false;
        };
        if !query.name.trim().is_empty() {
            return false;
        }

        self.current_dir_cell(slot).replace(query.scope_dir.clone());
        self.current_view_cell(slot)
            .replace(PaneView::Directory(query.scope_dir.clone()));
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_directory(slot, query.scope_dir);
        self.pane_widgets(slot).file_grid.flow.grab_focus();
        true
    }

    fn open_search_in_current_dir(self: &Rc<Self>) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::Search(_)) {
            let entry = self.pane_widgets(slot).search_panel.name_entry.clone();
            glib::idle_add_local_once(move || {
                entry.grab_focus();
                entry.select_region(0, -1);
            });
            return;
        }
        if !matches!(
            self.current_view_for(slot),
            PaneView::Directory(_) | PaneView::DownloadsTriage(_)
        ) {
            self.status
                .set_message("Search is only available in folder views.");
            return;
        }
        let scope = self.current_dir_for(slot);
        let query = SearchQuery::new(scope);
        self.open_search(slot, query);
    }

    fn open_search(self: &Rc<Self>, slot: PaneSlot, query: SearchQuery) {
        self.current_dir_cell(slot).replace(query.scope_dir.clone());
        self.current_view_cell(slot)
            .replace(PaneView::Search(query.clone()));
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        // Build tag row first so update_view_strip can sync chip state correctly
        let tags = self.tags.borrow().clone();
        self.pane_widgets(slot).search_panel.set_tags(&tags);
        self.wire_search_tag_buttons(slot);
        // update_view_strip calls sync_from_query — that's the only call we need
        self.update_view_strip(slot);
        self.load_search_view(slot, query);
        self.pane_widgets(slot).search_panel.name_entry.grab_focus();
    }

    fn rerun_search(self: &Rc<Self>, slot: PaneSlot) {
        let PaneView::Search(mut query) = self.current_view_for(slot) else {
            return;
        };
        let panel = &self.pane_widgets(slot).search_panel;
        query.name = panel.name_entry.text().to_string();
        query.recursive = panel.recursive_toggle.is_active();

        for (filter, btn) in &panel.kind_buttons {
            if btn.has_css_class("active") {
                query.kind = filter.clone();
                break;
            }
        }
        for (filter, btn) in &panel.age_buttons {
            if btn.has_css_class("active") {
                query.age = filter.clone();
                break;
            }
        }
        for (filter, btn) in &panel.size_buttons {
            if btn.has_css_class("active") {
                query.size = filter.clone();
                break;
            }
        }
        query.tag_id = None;
        for (id, btn) in panel.tag_buttons() {
            if id != -1 && btn.has_css_class("active") {
                query.tag_id = Some(id);
                break;
            }
        }

        self.current_view_cell(slot)
            .replace(PaneView::Search(query.clone()));
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_search_view(slot, query);
    }

    fn load_search_view(self: &Rc<Self>, slot: PaneSlot, query: SearchQuery) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_label = self.display_label_for(slot);
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.set_loading();
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }

        if slot == self.active_slot() && self.preview_visible.get() {
            self.preview
                .show_loading("Search Results", &FileKind::Unknown, "Searching…");
            self.preview.set_action_state(false, false, false);
        }

        let generation = self.load_generation_cell(slot).get() + 1;
        self.load_generation_cell(slot).set(generation);

        let controller = Rc::clone(self);
        let show_hidden = self.show_hidden.get();
        glib::MainContext::default().spawn_local(async move {
            let query_clone = query.clone();
            let raw = gio::spawn_blocking(move || {
                let mut results = Vec::new();
                search_directory_blocking(
                    &query_clone.scope_dir,
                    &query_clone,
                    show_hidden,
                    0,
                    &mut results,
                );
                results
            })
            .await
            .unwrap_or_default();

            if !controller.is_current_load(slot, generation) {
                return;
            }

            // Enrich with tags then apply optional tag filter
            let enriched = controller.enrich_items_with_tags(raw);
            let items: Vec<FileItem> = if let Some(tag_id) = query.tag_id {
                enriched
                    .into_iter()
                    .filter(|item| item.tags.iter().any(|t| t.id == tag_id))
                    .collect()
            } else {
                enriched
            };

            controller.finish_search_load(slot, generation, display_label, items);
        });
    }

    fn finish_search_load(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        display_label: String,
        items: Vec<FileItem>,
    ) {
        if !self.is_current_load(slot, generation) {
            return;
        }

        self.load_cancellable_cell(slot).borrow_mut().take();
        self.items_cell(slot).replace(items.clone());

        if items.is_empty() {
            self.pane_widgets(slot)
                .file_grid
                .set_empty_message("No files match your search.");
        } else {
            self.pane_widgets(slot).file_grid.set_items(&items);
            let thumb_targets = self.pane_widgets(slot).file_grid.drain_thumb_targets();
            if !thumb_targets.is_empty() {
                self.thumb_loader_for(slot).submit(thumb_targets);
            }
            self.attach_context_handlers(slot);
            self.attach_item_dnd(slot);
        }

        self.pane_widgets(slot).path_label.set_label(&display_label);
        if slot == self.active_slot() {
            self.status.set_path(&display_label);
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.update_navigation_state();
            self.update_action_state();
            self.show_empty_selection_preview(slot, &display_label, items.len());
            self.status.set_counts(items.len(), 0);
            if items.len() >= MAX_SEARCH_RESULTS {
                self.status.set_message(&format!(
                    "Showing first {MAX_SEARCH_RESULTS} results — refine your search to see more."
                ));
            }
            self.refresh_preview();
        }
    }

    fn connect_search_panels(self: &Rc<Self>) {
        self.connect_search_panel(PaneSlot::Primary);
        self.connect_search_panel(PaneSlot::Secondary);
    }

    fn connect_search_panel(self: &Rc<Self>, slot: PaneSlot) {
        let panel = self.pane_widgets(slot).search_panel.clone();

        // Name entry: debounce 280 ms
        // Access the panel through controller so is_updating() reads the live flag,
        // not a stale copy in a cloned struct.
        let controller = Rc::clone(self);
        panel.name_entry.connect_changed(move |_| {
            if controller.pane_widgets(slot).search_panel.is_updating() {
                return;
            }
            if let Some(id) = controller.search_debounce.borrow_mut().take() {
                id.remove();
            }
            if !matches!(controller.current_view_for(slot), PaneView::Search(_)) {
                return;
            }
            let c1 = Rc::clone(&controller);
            let c2 = Rc::clone(&controller);
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(280), move || {
                    c1.search_debounce.borrow_mut().take();
                    c2.rerun_search(slot);
                });
            *controller.search_debounce.borrow_mut() = Some(id);
        });

        // Recursive toggle — same guard; sync_from_query calls set_active which
        // fires toggled, which would otherwise cancel the search we just started.
        let controller = Rc::clone(self);
        panel.recursive_toggle.connect_toggled(move |_| {
            if controller.pane_widgets(slot).search_panel.is_updating() {
                return;
            }
            if matches!(controller.current_view_for(slot), PaneView::Search(_)) {
                controller.rerun_search(slot);
            }
        });

        // Kind chips — exclusive selection
        for (filter, btn) in panel.kind_buttons.clone() {
            let controller = Rc::clone(self);
            let all_kind: Vec<Button> = panel.kind_buttons.iter().map(|(_, b)| b.clone()).collect();
            btn.connect_clicked(move |clicked| {
                for b in &all_kind {
                    b.remove_css_class("active");
                }
                clicked.add_css_class("active");
                drop(filter.clone()); // keep capture alive
                if matches!(controller.current_view_for(slot), PaneView::Search(_)) {
                    controller.rerun_search(slot);
                }
            });
        }

        // Age chips — exclusive
        for (filter, btn) in panel.age_buttons.clone() {
            let controller = Rc::clone(self);
            let all: Vec<Button> = panel.age_buttons.iter().map(|(_, b)| b.clone()).collect();
            btn.connect_clicked(move |clicked| {
                for b in &all {
                    b.remove_css_class("active");
                }
                clicked.add_css_class("active");
                drop(filter.clone());
                if matches!(controller.current_view_for(slot), PaneView::Search(_)) {
                    controller.rerun_search(slot);
                }
            });
        }

        // Size chips — exclusive
        for (filter, btn) in panel.size_buttons.clone() {
            let controller = Rc::clone(self);
            let all: Vec<Button> = panel.size_buttons.iter().map(|(_, b)| b.clone()).collect();
            btn.connect_clicked(move |clicked| {
                for b in &all {
                    b.remove_css_class("active");
                }
                clicked.add_css_class("active");
                drop(filter.clone());
                if matches!(controller.current_view_for(slot), PaneView::Search(_)) {
                    controller.rerun_search(slot);
                }
            });
        }
    }

    /// Called after set_tags() — connects click handlers to the current tag chips.
    fn wire_search_tag_buttons(self: &Rc<Self>, slot: PaneSlot) {
        let panel = self.pane_widgets(slot).search_panel.clone();
        for (id, btn) in panel.tag_buttons() {
            let controller = Rc::clone(self);
            let all_tag: Vec<(i64, Button)> = panel.tag_buttons();
            btn.connect_clicked(move |clicked| {
                for (_, b) in &all_tag {
                    b.remove_css_class("active");
                }
                clicked.add_css_class("active");
                if matches!(controller.current_view_for(slot), PaneView::Search(_)) {
                    controller.rerun_search(slot);
                }
                let _ = id;
            });
        }
    }

    /// Called when tags change globally; updates the search panel only when it is
    /// currently visible (search view is active for that slot).
    fn refresh_search_tag_buttons(self: &Rc<Self>, slot: PaneSlot) {
        if !matches!(self.current_view_for(slot), PaneView::Search(_)) {
            return;
        }
        let tags = self.tags.borrow().clone();
        self.pane_widgets(slot).search_panel.set_tags(&tags);
        self.wire_search_tag_buttons(slot);
    }

    fn load_tag_view(self: &Rc<Self>, slot: PaneSlot, tag: TagRecord) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_label = self.display_label_for(slot);
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.set_loading();
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }

        if slot == self.active_slot() && self.preview_visible.get() {
            self.preview.show_loading(
                &format!("#{}", tag.name),
                &crate::ui::file_grid::FileKind::Unknown,
                "Loading tagged files…",
            );
            self.preview.set_action_state(false, false, false);
        }

        let generation = self.load_generation_cell(slot).get() + 1;
        self.load_generation_cell(slot).set(generation);

        let paths = match self.metadata.borrow().list_paths_for_tag(tag.id) {
            Ok(paths) => paths,
            Err(error) => {
                self.show_error_dialog("Tag Load Failed", &error);
                self.finish_virtual_load(
                    slot,
                    generation,
                    display_label,
                    Vec::new(),
                    "No tagged files available.",
                );
                return;
            }
        };

        let cancellable = gio::Cancellable::new();
        self.load_cancellable_cell(slot)
            .replace(Some(cancellable.clone()));

        let controller = Rc::clone(self);
        self.query_tagged_paths(
            slot,
            generation,
            paths,
            0,
            Rc::new(RefCell::new(Vec::new())),
            display_label,
            cancellable,
            controller,
        );
    }

    fn query_tagged_paths(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        paths: Vec<PathBuf>,
        index: usize,
        collected: Rc<RefCell<Vec<FileItem>>>,
        display_label: String,
        cancellable: gio::Cancellable,
        controller: Rc<Self>,
    ) {
        if !self.is_current_load(slot, generation) || cancellable.is_cancelled() {
            return;
        }

        if index >= paths.len() {
            let mut items = collected.borrow().clone();
            sort_items(&mut items);
            self.finish_virtual_load(
                slot,
                generation,
                display_label,
                items,
                "No files are using this tag yet.",
            );
            return;
        }

        let target_path = paths[index].clone();
        let file = gio::File::for_path(&target_path);
        let file_for_callback = file.clone();
        let cancellable_for_callback = cancellable.clone();
        file.query_info_async(
            DIRECTORY_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                if !controller.is_current_load(slot, generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                if let Ok(info) = result {
                    if controller.show_hidden.get() || !info.is_hidden() {
                        if let Some(path) = file_for_callback.path() {
                            collected.borrow_mut().push(FileItem {
                                name: info.display_name().to_string(),
                                kind: crate::ui::file_grid::FileKind::from_path(
                                    &path,
                                    info.file_type(),
                                    info.content_type().as_deref(),
                                ),
                                path,
                                is_dir: info.file_type() == gio::FileType::Directory,
                                size_bytes: (info.size() >= 0).then_some(info.size() as u64),
                                modified_unix: info
                                    .modification_date_time()
                                    .and_then(|value| Some(value.to_unix())),
                                tags: Vec::new(),
                                original_path: None,
                            });
                        }
                    }
                }

                controller.query_tagged_paths(
                    slot,
                    generation,
                    paths.clone(),
                    index + 1,
                    collected.clone(),
                    display_label.clone(),
                    cancellable_for_callback.clone(),
                    Rc::clone(&controller),
                );
            },
        );
    }

    fn finish_virtual_load(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        display_label: String,
        items: Vec<FileItem>,
        empty_message: &str,
    ) {
        if !self.is_current_load(slot, generation) {
            return;
        }

        self.load_cancellable_cell(slot).borrow_mut().take();
        let items = self.enrich_items_with_tags(items);
        self.items_cell(slot).replace(items.clone());
        if items.is_empty() {
            self.pane_widgets(slot)
                .file_grid
                .set_empty_message(empty_message);
        } else {
            self.pane_widgets(slot).file_grid.set_items(&items);
            let thumb_targets = self.pane_widgets(slot).file_grid.drain_thumb_targets();
            if !thumb_targets.is_empty() {
                self.thumb_loader_for(slot).submit(thumb_targets);
            }
            self.attach_context_handlers(slot);
            self.attach_item_dnd(slot);
        }

        self.pane_widgets(slot).path_label.set_label(&display_label);
        if slot == self.active_slot() {
            self.status.set_path(&display_label);
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.update_navigation_state();
            self.update_action_state();
            self.show_empty_selection_preview(slot, &display_label, items.len());
            self.status.set_counts(items.len(), 0);
            self.refresh_preview();
        }
    }

    fn enrich_items_with_tags(&self, mut items: Vec<FileItem>) -> Vec<FileItem> {
        let paths = items
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();
        let tags_by_path = self
            .metadata
            .borrow()
            .tags_for_paths(&paths)
            .unwrap_or_else(|_| HashMap::new());

        for item in &mut items {
            item.tags = tags_by_path.get(&item.path).cloned().unwrap_or_default();
        }
        items
    }

    fn show_empty_selection_preview(&self, slot: PaneSlot, display_label: &str, item_count: usize) {
        match self.current_view_for(slot) {
            PaneView::Directory(_) => self.preview.show_current_folder(display_label, item_count),
            PaneView::Tag(tag) => {
                self.preview.show_folder(
                    &format!("#{}", tag.name),
                    display_label,
                    None,
                    Some(item_count),
                    "Tag View",
                );
                self.preview.set_action_state(false, false, false);
            }
            PaneView::DownloadsTriage(filter) => {
                self.preview.show_folder(
                    filter.label(),
                    display_label,
                    None,
                    Some(item_count),
                    "Downloads Triage",
                );
                self.preview.set_action_state(false, false, false);
            }
            PaneView::SystemDrives => {
                self.preview.show_folder(
                    "System Drives",
                    display_label,
                    None,
                    Some(item_count),
                    "Mounted Volumes",
                );
                self.preview.set_action_state(false, false, false);
            }
            PaneView::Recent => {
                self.preview.show_folder(
                    "Recent Folders",
                    display_label,
                    None,
                    Some(item_count),
                    "Recent Locations",
                );
                self.preview.set_action_state(false, false, false);
            }
            PaneView::Trash => {
                self.preview
                    .show_folder("Trash", display_label, None, Some(item_count), "Trash");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::Search(query) => {
                let title = if query.name.is_empty() {
                    "Search Results".to_string()
                } else {
                    format!("Search \u{201c}{}\u{201d}", query.name)
                };
                self.preview
                    .show_folder(&title, display_label, None, Some(item_count), "Search");
                self.preview.set_action_state(false, false, false);
            }
        }
    }

    fn attach_context_handlers(self: &Rc<Self>, slot: PaneSlot) {
        let pane = self.pane_widgets(slot).clone();
        for index in 0..self.items_cell(slot).borrow().len() {
            if let Some(child) = pane.file_grid.flow.child_at_index(index as i32) {
                let gesture = gtk::GestureClick::new();
                gesture.set_button(3);

                let controller = Rc::clone(self);
                let anchor: gtk::Widget = child.clone().upcast();
                gesture.connect_pressed(move |_, _, x, y| {
                    controller.set_active_pane(slot);
                    controller.show_context_menu(slot, index as i32, anchor.clone(), x, y);
                });

                child.add_controller(gesture);
            }
        }
    }

    fn show_context_menu(
        self: &Rc<Self>,
        slot: PaneSlot,
        index: i32,
        anchor: gtk::Widget,
        x: f64,
        y: f64,
    ) {
        self.dismiss_context_menu();
        self.set_active_pane(slot);
        self.pane_widgets(slot).file_grid.select_only_index(index);
        self.set_keyboard_focus(slot, index, true);

        let item = match self.item_for_index(slot, index) {
            Some(item) => item,
            None => return,
        };

        let popover = Popover::new();
        popover.add_css_class("context-menu");
        popover.set_has_arrow(true);
        popover.set_autohide(true);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.set_parent(&anchor);

        let menu_box = GtkBox::new(Orientation::Vertical, 2);
        menu_box.set_margin_top(6);
        menu_box.set_margin_bottom(6);
        menu_box.set_margin_start(6);
        menu_box.set_margin_end(6);
        menu_box.set_size_request(200, -1);

        if matches!(self.current_view_for(slot), PaneView::Trash) {
            // Trash view: restore + copy path
            if item.original_path.is_some() {
                append_menu_button(
                    &menu_box,
                    "Restore from Trash",
                    Some("document-revert-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || {
                            controller.restore_items_from_trash(vec![item.clone()]);
                        }
                    },
                );
            } else {
                let note = Label::new(Some("Restore unavailable (no original path)"));
                note.set_margin_start(8);
                note.set_margin_end(8);
                note.set_halign(gtk::Align::Start);
                note.add_css_class("context-note");
                menu_box.append(&note);
            }
            append_menu_sep(&menu_box);
            append_menu_button(&menu_box, "Copy Path", Some("edit-copy-symbolic"), false, {
                let controller = Rc::clone(self);
                let item = item.clone();
                move || controller.copy_paths_to_clipboard(vec![item.path.clone()])
            });
        } else if matches!(
            self.current_view_for(slot),
            PaneView::SystemDrives | PaneView::Recent
        ) {
            // Drives / Recent: open group
            append_menu_button(&menu_box, "Open", Some("document-open-symbolic"), false, {
                let controller = Rc::clone(self);
                let item = item.clone();
                move || controller.open_item_in_slot(slot, &item)
            });
            append_menu_button(
                &menu_box,
                "Open in New Tab",
                Some("tab-new-symbolic"),
                false,
                {
                    let controller = Rc::clone(self);
                    let item = item.clone();
                    move || controller.open_new_tab(Some(item.path.clone()))
                },
            );
            if self.split_enabled.get() {
                append_menu_button(
                    &menu_box,
                    "Open in Other Pane",
                    Some("window-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_folder_in_other_pane(slot, item.path.clone())
                    },
                );
            } else {
                append_menu_button(
                    &menu_box,
                    "Open in Split Pane",
                    Some("window-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_folder_in_split(item.path.clone())
                    },
                );
            }
            // File / project group
            append_menu_sep(&menu_box);
            append_menu_button(
                &menu_box,
                "Pin as Project",
                Some("starred-symbolic"),
                false,
                {
                    let controller = Rc::clone(self);
                    let item = item.clone();
                    move || controller.show_pin_project_dialog(item.path.clone())
                },
            );
            append_menu_button(&menu_box, "Copy Path", Some("edit-copy-symbolic"), false, {
                let controller = Rc::clone(self);
                let item = item.clone();
                move || controller.copy_paths_to_clipboard(vec![item.path.clone()])
            });
            append_menu_button(
                &menu_box,
                "Terminal Here",
                Some("utilities-terminal-symbolic"),
                false,
                {
                    let controller = Rc::clone(self);
                    let item = item.clone();
                    move || controller.open_terminal_for_path(item.path.clone(), true)
                },
            );
        } else {
            // Normal directory view
            let entries = self.item_context_entries(item.is_dir);
            self.append_item_context_menu_entries(&menu_box, slot, &item, &entries);
        }

        popover.set_child(Some(&menu_box));

        let controller = Rc::clone(self);
        let popover_for_signal = popover.clone();
        popover.connect_closed(move |_| {
            let should_clear = controller
                .context_popover
                .borrow()
                .as_ref()
                .map(|current| current == &popover_for_signal)
                .unwrap_or(false);

            if should_clear {
                controller.context_popover.borrow_mut().take();
            }
            popover_for_signal.unparent();
        });

        self.context_popover.replace(Some(popover.clone()));
        popover.popup();
    }

    fn show_current_folder_menu(self: &Rc<Self>, slot: PaneSlot, x: f64, y: f64) {
        if !matches!(
            self.current_view_for(slot),
            PaneView::Directory(_) | PaneView::DownloadsTriage(_)
        ) {
            return;
        }

        self.dismiss_context_menu();
        self.set_active_pane(slot);
        self.pane_widgets(slot).file_grid.clear_selection();
        self.reset_keyboard_state(slot);

        let popover = Popover::new();
        popover.add_css_class("context-menu");
        popover.set_has_arrow(true);
        popover.set_autohide(true);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.set_parent(&self.pane_widgets(slot).file_grid.flow);

        let menu_box = GtkBox::new(Orientation::Vertical, 2);
        menu_box.set_margin_top(6);
        menu_box.set_margin_bottom(6);
        menu_box.set_margin_start(6);
        menu_box.set_margin_end(6);
        menu_box.set_size_request(190, -1);

        let entries = self.background_context_entries();
        self.append_background_context_menu_entries(&menu_box, slot, &entries);

        popover.set_child(Some(&menu_box));

        let controller = Rc::clone(self);
        let popover_for_signal = popover.clone();
        popover.connect_closed(move |_| {
            let should_clear = controller
                .context_popover
                .borrow()
                .as_ref()
                .map(|current| current == &popover_for_signal)
                .unwrap_or(false);

            if should_clear {
                controller.context_popover.borrow_mut().take();
            }
            popover_for_signal.unparent();
        });

        self.context_popover.replace(Some(popover.clone()));
        popover.popup();
    }

    fn item_context_entries(&self, is_dir: bool) -> Vec<String> {
        let configured = if is_dir {
            self.config.context_menu.folder.as_ref()
        } else {
            self.config.context_menu.file.as_ref()
        };
        configured.cloned().unwrap_or_else(|| {
            if is_dir {
                vec![
                    "open",
                    "open_new_tab",
                    "open_in_pane",
                    "separator",
                    "rename",
                    "copy_path",
                    "terminal_here",
                    "separator",
                    "pin_project",
                    "send_to_project",
                    "add_tag",
                    "remove_tag",
                    "separator",
                    "move_to_trash",
                    "delete_permanently",
                ]
            } else {
                vec![
                    "open",
                    "open_with",
                    "separator",
                    "rename",
                    "copy_path",
                    "terminal_here",
                    "separator",
                    "send_to_project",
                    "add_tag",
                    "remove_tag",
                    "separator",
                    "move_to_trash",
                    "delete_permanently",
                ]
            }
            .into_iter()
            .map(str::to_string)
            .collect()
        })
    }

    fn background_context_entries(&self) -> Vec<String> {
        self.config
            .context_menu
            .background
            .clone()
            .unwrap_or_else(|| {
                [
                    "new_folder",
                    "new_text_document",
                    "separator",
                    "pin_project",
                    "terminal_here",
                    "copy_path",
                ]
                .into_iter()
                .map(str::to_string)
                .collect()
            })
    }

    fn append_item_context_menu_entries(
        self: &Rc<Self>,
        menu_box: &GtkBox,
        slot: PaneSlot,
        item: &FileItem,
        entries: &[String],
    ) {
        for entry in entries {
            match entry.as_str() {
                "separator" => append_menu_sep(menu_box),
                "open" => {
                    append_menu_button(menu_box, "Open", Some("document-open-symbolic"), false, {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_item_in_slot(slot, &item)
                    })
                }
                "open_with" if !item.is_dir => append_menu_button(
                    menu_box,
                    "Open With\u{2026}",
                    Some("applications-other-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.show_open_with_dialog(item.path.clone())
                    },
                ),
                "open_new_tab" if item.is_dir => append_menu_button(
                    menu_box,
                    "Open in New Tab",
                    Some("tab-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_new_tab(Some(item.path.clone()))
                    },
                ),
                "open_in_pane" if item.is_dir && self.split_enabled.get() => append_menu_button(
                    menu_box,
                    "Open in Other Pane",
                    Some("window-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_folder_in_other_pane(slot, item.path.clone())
                    },
                ),
                "open_in_pane" if item.is_dir => append_menu_button(
                    menu_box,
                    "Open in Split Pane",
                    Some("window-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_folder_in_split(item.path.clone())
                    },
                ),
                "rename" => {
                    append_menu_button(menu_box, "Rename", Some("document-edit-symbolic"), false, {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.show_rename_dialog(item.path.clone(), item.name.clone())
                    })
                }
                "copy_path" => {
                    append_menu_button(menu_box, "Copy Path", Some("edit-copy-symbolic"), false, {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.copy_paths_to_clipboard(vec![item.path.clone()])
                    })
                }
                "terminal_here" => append_menu_button(
                    menu_box,
                    "Terminal Here",
                    Some("utilities-terminal-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_terminal_for_path(item.path.clone(), item.is_dir)
                    },
                ),
                "pin_project" if item.is_dir => append_menu_button(
                    menu_box,
                    "Pin as Project",
                    Some("starred-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.show_pin_project_dialog(item.path.clone())
                    },
                ),
                "send_to_project" => append_menu_button(
                    menu_box,
                    "Send to Project",
                    Some("go-next-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || {
                            controller
                                .show_send_to_project_dialog(controller.selected_paths_for(slot))
                        }
                    },
                ),
                "add_tag" => {
                    append_menu_button(menu_box, "Add Tag", Some("list-add-symbolic"), false, {
                        let controller = Rc::clone(self);
                        move || controller.show_add_tag_dialog(controller.selected_paths_for(slot))
                    })
                }
                "remove_tag" => append_menu_button(
                    menu_box,
                    "Remove Tag",
                    Some("list-remove-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || {
                            controller.show_remove_tag_dialog(controller.selected_paths_for(slot))
                        }
                    },
                ),
                "move_to_trash" => append_menu_button(
                    menu_box,
                    "Move to Trash",
                    Some("user-trash-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.move_paths_to_trash(vec![item.path.clone()])
                    },
                ),
                "delete_permanently" => append_menu_button(
                    menu_box,
                    "Delete Permanently",
                    Some("edit-delete-symbolic"),
                    true,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.confirm_permanent_delete(vec![item.path.clone()])
                    },
                ),
                custom if custom.starts_with("custom.") => {
                    self.append_custom_context_action(
                        menu_box,
                        slot,
                        item,
                        &custom["custom.".len()..],
                    );
                }
                _ => {}
            }
        }
    }

    fn append_background_context_menu_entries(
        self: &Rc<Self>,
        menu_box: &GtkBox,
        slot: PaneSlot,
        entries: &[String],
    ) {
        for entry in entries {
            match entry.as_str() {
                "separator" => append_menu_sep(menu_box),
                "new_folder" => append_menu_button(
                    menu_box,
                    "New Folder",
                    Some("folder-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.create_new_folder()
                    },
                ),
                "new_text_document" => append_menu_button(
                    menu_box,
                    "New Text Document",
                    Some("document-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.create_new_text_document()
                    },
                ),
                "pin_project" => append_menu_button(
                    menu_box,
                    "Pin as Project",
                    Some("starred-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.show_pin_project_dialog(controller.current_dir_for(slot))
                    },
                ),
                "terminal_here" => append_menu_button(
                    menu_box,
                    "Terminal Here",
                    Some("utilities-terminal-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.open_current_folder_terminal()
                    },
                ),
                "copy_path" => {
                    append_menu_button(menu_box, "Copy Path", Some("edit-copy-symbolic"), false, {
                        let controller = Rc::clone(self);
                        move || {
                            controller
                                .copy_paths_to_clipboard(vec![controller.current_dir_for(slot)])
                        }
                    })
                }
                custom if custom.starts_with("custom.") => {
                    self.append_custom_background_action(
                        menu_box,
                        slot,
                        &custom["custom.".len()..],
                    );
                }
                _ => {}
            }
        }
    }

    fn append_custom_context_action(
        self: &Rc<Self>,
        menu_box: &GtkBox,
        slot: PaneSlot,
        item: &FileItem,
        action_id: &str,
    ) {
        let Some(action) = self.config.custom_action(action_id).cloned() else {
            return;
        };
        let context = if item.is_dir { "folder" } else { "file" };
        if !action.contexts.iter().any(|value| value == context) {
            return;
        }

        let label = action.label.clone();
        append_menu_button(menu_box, &label, Some("system-run-symbolic"), false, {
            let controller = Rc::clone(self);
            move || {
                controller
                    .run_custom_action(&action, custom_action_paths_for_context(&controller, slot))
            }
        });
    }

    fn append_custom_background_action(
        self: &Rc<Self>,
        menu_box: &GtkBox,
        slot: PaneSlot,
        action_id: &str,
    ) {
        let Some(action) = self.config.custom_action(action_id).cloned() else {
            return;
        };
        if !action.contexts.iter().any(|value| value == "background") {
            return;
        }

        let label = action.label.clone();
        append_menu_button(menu_box, &label, Some("system-run-symbolic"), false, {
            let controller = Rc::clone(self);
            move || controller.run_custom_action(&action, vec![controller.current_dir_for(slot)])
        });
    }

    fn dismiss_context_menu(&self) -> bool {
        if let Some(popover) = self.context_popover.borrow_mut().take() {
            if popover.parent().is_some() {
                popover.unparent();
            }
            return true;
        }
        false
    }

    fn update_selection(self: &Rc<Self>) {
        let slot = self.active_slot();
        let selected_items = self.selected_items_for(slot);
        let item_count = self.current_item_count_for(slot);
        let selected_count = selected_items.len();

        self.status.set_counts(item_count, selected_count);
        self.status.set_path(&self.display_label_for(slot));
        self.update_action_state();
        self.refresh_preview();
    }

    fn update_action_state(&self) {
        let slot = self.active_slot();
        let selected_count = self.pane_widgets(slot).file_grid.selected_indices().len();
        let actions = action_availability(
            &self.current_view_for(slot),
            selected_count,
            self.file_clipboard.borrow().is_some(),
        );
        self.toolbar.rename_button.set_sensitive(actions.can_rename);
        self.toolbar.trash_button.set_sensitive(actions.can_trash);
        self.toolbar
            .new_folder_button
            .set_sensitive(actions.can_new_folder);
        self.toolbar
            .new_text_document_button
            .set_sensitive(actions.can_new_folder);
    }

    fn refresh_preview(self: &Rc<Self>) {
        self.cancel_active_preview();

        if !self.preview_visible.get() {
            return;
        }

        let selected_items = self.selected_items();
        match selected_items.len() {
            0 => {
                let slot = self.active_slot();
                if matches!(self.current_view_for(slot), PaneView::Directory(_)) {
                    let current = self.current_dir_for(slot);
                    let path_label = self.display_path(&current);
                    self.preview.show_loading(
                        "Current Folder",
                        &crate::ui::file_grid::FileKind::Folder,
                        "Loading folder metadata…",
                    );
                    self.preview
                        .set_action_state(false, true, current.parent().is_some());
                    self.load_current_directory_preview(
                        current,
                        path_label,
                        self.current_item_count_for(slot),
                    );
                } else {
                    let display_label = self.display_label_for(slot);
                    self.show_empty_selection_preview(
                        slot,
                        &display_label,
                        self.current_item_count_for(slot),
                    );
                }
            }
            1 => {
                let item = selected_items.into_iter().next().unwrap();
                self.preview
                    .show_loading(&item.name, &item.kind, "Loading preview…");
                self.preview.set_action_state(true, true, true);
                self.load_item_preview(item);
            }
            count => {
                self.preview.show_multi_selection(count);
                self.preview.set_action_state(false, false, false);
            }
        }
    }

    fn load_current_directory_preview(
        self: &Rc<Self>,
        path: PathBuf,
        display_path: String,
        item_count: usize,
    ) {
        let generation = self.preview_generation.get() + 1;
        self.preview_generation.set(generation);
        let cancellable = gio::Cancellable::new();
        self.preview_cancellable.replace(Some(cancellable.clone()));
        let cancellable_for_callback = cancellable.clone();

        let controller = Rc::clone(self);
        gio::File::for_path(&path).query_info_async(
            PREVIEW_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                if !controller.is_current_preview(generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                controller.preview_cancellable.borrow_mut().take();
                match result {
                    Ok(info) => {
                        let modified = format_modified_time(info.modification_date_time());
                        controller.preview.show_folder(
                            "Current Folder",
                            &display_path,
                            modified.as_deref(),
                            Some(item_count),
                            "Folder",
                        );
                        controller
                            .preview
                            .set_action_state(false, true, path.parent().is_some());
                    }
                    Err(error) => {
                        controller
                            .preview
                            .show_error(&display_path, &friendly_error_detail(&error));
                        controller.preview.set_action_state(false, false, false);
                    }
                }
            },
        );
    }

    fn load_item_preview(self: &Rc<Self>, item: FileItem) {
        let generation = self.preview_generation.get() + 1;
        self.preview_generation.set(generation);
        let cancellable = gio::Cancellable::new();
        self.preview_cancellable.replace(Some(cancellable.clone()));
        let cancellable_for_callback = cancellable.clone();
        let cancellable_for_render = cancellable.clone();

        let controller = Rc::clone(self);
        let display_path = self.display_path(&item.path);
        let file = gio::File::for_path(&item.path);
        file.query_info_async(
            PREVIEW_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                if !controller.is_current_preview(generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                match result {
                    Ok(info) => {
                        controller.render_item_preview(
                            generation,
                            item.clone(),
                            display_path.clone(),
                            info,
                            cancellable_for_render.clone(),
                        );
                    }
                    Err(error) => {
                        controller.preview_cancellable.borrow_mut().take();
                        controller
                            .preview
                            .show_error(&display_path, &friendly_error_detail(&error));
                        controller.preview.set_action_state(false, false, false);
                    }
                }
            },
        );
    }

    fn render_item_preview(
        self: &Rc<Self>,
        generation: u64,
        item: FileItem,
        display_path: String,
        info: gio::FileInfo,
        cancellable: gio::Cancellable,
    ) {
        let modified = format_modified_time(info.modification_date_time());
        let size = (info.size() >= 0).then(|| format_file_size(info.size() as u64));
        let content_type = info.content_type();
        let mime = content_type.as_deref().unwrap_or("");
        let type_label = preview_type_label(&item, mime);

        if item.is_dir {
            self.preview_cancellable.borrow_mut().take();
            self.preview.show_folder(
                &item.name,
                &display_path,
                modified.as_deref(),
                None,
                &type_label,
            );
            self.preview.set_action_state(true, true, true);
            return;
        }

        if item.kind == FileKind::Image {
            self.preview_cancellable.borrow_mut().take();
            let dimensions = probe_image_dimensions(&item.path)
                .or_else(|| self.preview.image_dimensions())
                .map(|(w, h)| format!("{w} × {h}"));
            self.preview.show_image(
                &item.kind,
                &item.name,
                &display_path,
                size.as_deref(),
                modified.as_deref(),
                dimensions.as_deref(),
            );
            self.preview
                .set_image_file(Some(&gio::File::for_path(&item.path)));
            self.preview.set_mime_type(Some(mime));
            self.preview.set_action_state(true, true, true);
            return;
        }

        if matches!(item.kind, FileKind::Text | FileKind::ConfigCode) {
            self.load_text_preview(
                generation,
                item,
                display_path,
                type_label,
                mime.to_string(),
                size,
                modified,
                cancellable,
            );
            return;
        }

        self.preview_cancellable.borrow_mut().take();

        let note = match item.kind {
            FileKind::Video | FileKind::Audio => None,
            _ => Some("No preview available for this file type."),
        };
        self.preview.show_basic_file(
            &item.kind,
            &type_label,
            &item.name,
            &display_path,
            size.as_deref(),
            modified.as_deref(),
            note,
        );
        self.preview.set_mime_type(Some(mime));
        self.preview.set_action_state(true, true, true);
    }

    fn load_text_preview(
        self: &Rc<Self>,
        generation: u64,
        item: FileItem,
        display_path: String,
        type_label: String,
        mime: String,
        size: Option<String>,
        modified: Option<String>,
        cancellable: gio::Cancellable,
    ) {
        let file = gio::File::for_path(&item.path);
        let limit = TEXT_PREVIEW_LIMIT_BYTES;
        let total_read = Rc::new(Cell::new(0usize));
        let total_read_for_callback = total_read.clone();
        let controller = Rc::clone(self);
        let cancellable_for_callback = cancellable.clone();

        file.load_partial_contents_async(
            Some(&cancellable),
            move |chunk| {
                let seen = total_read.get().saturating_add(chunk.len());
                total_read.set(seen);
                seen < limit
            },
            move |result| {
                if !controller.is_current_preview(generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                controller.preview_cancellable.borrow_mut().take();
                match result {
                    Ok((contents, _etag)) => {
                        let mut text = String::from_utf8_lossy(contents.as_ref()).to_string();
                        let truncated = total_read_for_callback.get() >= limit;
                        if text.chars().count() > TEXT_PREVIEW_DISPLAY_CHARS {
                            text = text.chars().take(TEXT_PREVIEW_DISPLAY_CHARS).collect();
                        }
                        let note = if truncated {
                            Some(format!(
                                "Preview truncated to the first {} KB.",
                                TEXT_PREVIEW_LIMIT_BYTES / 1024
                            ))
                        } else {
                            None
                        };
                        controller.preview.show_text_preview(
                            &item.kind,
                            &type_label,
                            &item.name,
                            &display_path,
                            size.as_deref(),
                            modified.as_deref(),
                            &text,
                            note.as_deref(),
                        );
                        controller.preview.set_mime_type(Some(&mime));
                        controller.preview.set_action_state(true, true, true);
                    }
                    Err(error) => {
                        controller.preview.show_basic_file(
                            &item.kind,
                            &type_label,
                            &item.name,
                            &display_path,
                            size.as_deref(),
                            modified.as_deref(),
                            Some(&friendly_error_detail(&error)),
                        );
                        controller.preview.set_mime_type(Some(&mime));
                        controller.preview.set_action_state(true, true, true);
                    }
                }
            },
        );
    }

    fn activate_index(self: &Rc<Self>, slot: PaneSlot, index: i32) {
        if let Some(item) = self.item_for_index(slot, index) {
            self.open_item_in_slot(slot, &item);
        }
    }

    fn open_item(self: &Rc<Self>, item: &FileItem) {
        self.open_item_in_slot(self.active_slot(), item);
    }

    fn open_item_in_slot(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
        if item.is_dir {
            self.navigate_to(slot, item.path.clone(), true);
        } else {
            self.open_file(&item.path);
        }
    }

    fn open_folder_in_split(self: &Rc<Self>, path: PathBuf) {
        self.set_split_enabled(true);
        self.set_active_pane(PaneSlot::Secondary);
        self.navigate_to(PaneSlot::Secondary, path, true);
    }

    fn open_folder_in_other_pane(self: &Rc<Self>, slot: PaneSlot, path: PathBuf) {
        let target = Self::other_slot(slot);
        if !self.split_enabled.get() {
            self.open_folder_in_split(path);
            return;
        }

        self.set_active_pane(target);
        self.navigate_to(target, path, true);
    }

    fn open_file(self: &Rc<Self>, path: &Path) {
        let file = gio::File::for_path(path);
        let uri = file.uri().to_string();
        let uri_for_callback = uri.clone();
        let controller = Rc::clone(self);
        let launch_context = gio::AppLaunchContext::new();

        gio::AppInfo::launch_default_for_uri_async(
            &uri,
            Some(&launch_context),
            None::<&gio::Cancellable>,
            move |result| {
                if let Err(error) = result {
                    let (title, detail) = friendly_error(&error);
                    controller.show_error_dialog(
                        title,
                        &format!("{detail}\n\nTarget: {uri_for_callback}"),
                    );
                }
            },
        );
    }

    fn show_open_with_dialog(self: &Rc<Self>, path: PathBuf) {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let (content_type, _) = gio::content_type_guess(Some(file_name.as_str()), &[]);
        let apps: Vec<gio::AppInfo> = gio::AppInfo::all_for_type(&content_type)
            .into_iter()
            .collect();

        if apps.is_empty() {
            self.show_error_dialog("Open With", "No applications found for this file type.");
            return;
        }

        let content = GtkBox::new(Orientation::Vertical, 2);
        content.add_css_class("open-with-list");

        for app in apps {
            let display_name = app.display_name().to_string();
            let app_clone = app.clone();
            let path_clone = path.clone();
            let controller = Rc::clone(self);

            let btn = Button::with_label(&display_name);
            btn.add_css_class("context-menu-button");
            btn.set_halign(Align::Fill);

            btn.connect_clicked(move |_| {
                let file = gio::File::for_path(&path_clone);
                let uri = file.uri().to_string();
                let ctx = gio::AppLaunchContext::new();
                if let Err(e) = app_clone.launch_uris(&[uri.as_str()], Some(&ctx)) {
                    controller.show_error_dialog("Open With Failed", &e.message());
                }
                controller.modal_host.hide();
            });

            content.append(&btn);
        }

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel);

        let host = self.modal_host.clone();
        self.modal_host.show_with_custom_ui(
            "Open With",
            &content,
            &actions,
            true,
            Some(Box::new(move || host.hide())),
        );
    }

    fn open_created_text_document(self: &Rc<Self>, path: PathBuf) {
        let file = gio::File::for_path(&path);
        let uri = file.uri().to_string();
        let uri_for_callback = uri.clone();
        let controller = Rc::clone(self);
        let launch_context = gio::AppLaunchContext::new();

        gio::AppInfo::launch_default_for_uri_async(
            &uri,
            Some(&launch_context),
            None::<&gio::Cancellable>,
            move |result| {
                if let Err(error) = result {
                    let (title, detail) = friendly_error(&error);
                    controller.show_error_dialog(
                        title,
                        &format!(
                            "The text document was created, but Lattice could not open it with the default app.\n\n{detail}\n\nTarget: {uri_for_callback}"
                        ),
                    );
                }
            },
        );
    }

    // ── Tag filter ────────────────────────────────────────────────────────────

    fn wire_tag_filters(self: &Rc<Self>) {
        self.wire_tag_filter_for_slot(PaneSlot::Primary);
        self.wire_tag_filter_for_slot(PaneSlot::Secondary);
    }

    fn wire_tag_filter_for_slot(self: &Rc<Self>, slot: PaneSlot) {
        let controller = Rc::clone(self);
        self.pane_widgets(slot)
            .tag_filter
            .connect_changed(move |spec| {
                controller.apply_tag_filter(slot, &spec);
                controller.sync_filter_button_state(slot);
            });
    }

    fn set_filter_panel_open(self: &Rc<Self>, open: bool) {
        let slot = self.active_slot();
        self.pane_widgets(slot).tag_filter.root.set_visible(open);
        self.toolbar.filter_toggle.set_active(open);
        self.sync_filter_button_state(slot);
    }

    fn sync_filter_button_state(&self, slot: PaneSlot) {
        let pane = self.pane_widgets(slot);
        let count = pane.tag_filter.active_count();
        if count > 0 {
            self.toolbar
                .filter_toggle
                .add_css_class("toolbar-filter-active");
        } else {
            self.toolbar
                .filter_toggle
                .remove_css_class("toolbar-filter-active");
        }
    }

    fn apply_tag_filter(self: &Rc<Self>, slot: PaneSlot, spec: &TagFilterSpec) {
        if !matches!(self.current_view_for(slot), PaneView::Directory(_)) {
            return;
        }
        let all = self.all_items_cell(slot).borrow().clone();
        let filtered: Vec<FileItem> = if spec.is_empty() {
            all
        } else {
            all.into_iter().filter(|item| spec.matches(item)).collect()
        };
        let count = filtered.len();
        self.items_cell(slot).replace(filtered.clone());
        self.pane_widgets(slot).file_grid.set_items(&filtered);
        let thumb_targets = self.pane_widgets(slot).file_grid.drain_thumb_targets();
        if !thumb_targets.is_empty() {
            self.thumb_loader_for(slot).submit(thumb_targets);
        }
        self.attach_context_handlers(slot);
        self.attach_item_dnd(slot);
        if slot == self.active_slot() {
            self.update_action_state();
            let display = self.display_label_for(slot);
            self.show_empty_selection_preview(slot, &display, count);
            self.status.set_counts(count, 0);
            self.refresh_preview();
        }
    }

    // ── Rename ────────────────────────────────────────────────────────────────

    fn rename_selected(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !action_availability(
            &self.current_view_for(slot),
            self.selected_paths_for(slot).len(),
            self.file_clipboard.borrow().is_some(),
        )
        .can_rename
        {
            self.status
                .set_message("Rename is not available in this view.");
            return;
        }

        let items = self.selected_items();
        match items.len() {
            0 => {}
            1 => {
                let item = items.into_iter().next().unwrap();
                self.show_rename_dialog(item.path, item.name);
            }
            _ => self.show_bulk_rename_dialog(items),
        }
    }

    fn show_bulk_rename_dialog(self: &Rc<Self>, selected: Vec<FileItem>) {
        let selected_paths: HashSet<PathBuf> = selected.iter().map(|i| i.path.clone()).collect();
        let existing_names: HashSet<String> = self
            .items
            .borrow()
            .iter()
            .filter(|item| !selected_paths.contains(&item.path))
            .map(|item| item.name.clone())
            .collect();

        let controller = Rc::clone(self);
        bulk_rename::show(&self.modal_host, selected, existing_names, move |renames| {
            controller.apply_bulk_rename(renames);
        });
    }

    fn apply_bulk_rename(self: &Rc<Self>, renames: Vec<(PathBuf, String)>) {
        if renames.is_empty() {
            return;
        }
        let label = format!(
            "Renaming {} file{}…",
            renames.len(),
            if renames.len() == 1 { "" } else { "s" }
        );
        let op_id = self.ops_panel.add_op(&label, None);
        self.run_bulk_rename_step(
            Rc::new(renames),
            0,
            op_id,
            Rc::new(RefCell::new(BatchResult::default())),
        );
    }

    fn run_bulk_rename_step(
        self: &Rc<Self>,
        renames: Rc<Vec<(PathBuf, String)>>,
        index: usize,
        op_id: OpId,
        result: Rc<RefCell<BatchResult>>,
    ) {
        if index >= renames.len() {
            let failures = result.borrow().failures.clone();
            self.ops_panel.finish_op(op_id, &failures);
            let n = result.borrow().success_count;
            if n > 0 {
                self.pending_status_message.replace(Some(format!(
                    "Renamed {} file{}.",
                    n,
                    if n == 1 { "" } else { "s" }
                )));
            }
            self.refresh();
            return;
        }

        let total = renames.len();
        let (old_path, new_name) = {
            let (p, n) = &renames[index];
            (p.clone(), n.clone())
        };
        let new_path = old_path
            .parent()
            .map(|p| p.join(&new_name))
            .unwrap_or_else(|| PathBuf::from(&new_name));

        self.ops_panel
            .update_progress(op_id, index as f64 / total as f64, &new_name);

        let file = gio::File::for_path(&old_path);
        let controller = Rc::clone(self);
        let new_name_for_err = new_name.clone();

        file.set_display_name_async(
            &new_name,
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |res| {
                match res {
                    Ok(_) => {
                        result.borrow_mut().success_count += 1;
                        if let Err(e) = controller
                            .metadata
                            .borrow_mut()
                            .move_tagged_path_prefix(&old_path, &new_path)
                        {
                            eprintln!("bulk rename metadata: {e}");
                        }
                    }
                    Err(e) => {
                        let (_, detail) = friendly_error(&e);
                        result
                            .borrow_mut()
                            .failures
                            .push(format!("{new_name_for_err}: {detail}"));
                    }
                }
                controller.run_bulk_rename_step(renames, index + 1, op_id, result);
            },
        );
    }

    fn show_rename_dialog(self: &Rc<Self>, path: PathBuf, current_name: String) {
        let controller = Rc::clone(self);
        self.modal_host.show_input(
            "Rename",
            "Choose a new name for the selected item.",
            &current_name,
            "Rename",
            move |name| controller.rename_path(path.clone(), name),
        );
    }

    fn rename_path(self: &Rc<Self>, path: PathBuf, new_name: String) {
        if new_name.is_empty() {
            self.show_error_dialog("Invalid Name", "Names cannot be empty.");
            return;
        }

        if path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value == new_name)
            .unwrap_or(false)
        {
            return;
        }

        self.status.set_message("Renaming item…");
        let controller = Rc::clone(self);
        let file = gio::File::for_path(&path);
        let new_path = path
            .parent()
            .map(|parent| parent.join(&new_name))
            .unwrap_or_else(|| PathBuf::from(&new_name));
        let new_name_for_callback = new_name.clone();
        file.set_display_name_async(
            &new_name,
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(_) => {
                    if let Err(error) = controller
                        .metadata
                        .borrow_mut()
                        .move_tagged_path_prefix(&path, &new_path)
                    {
                        controller.show_error_dialog("Tag Update Failed", &error);
                    }
                    controller
                        .pending_status_message
                        .replace(Some("Rename complete.".to_string()));
                    controller.refresh();
                }
                Err(error) => {
                    let (title, detail) = friendly_error(&error);
                    controller.show_error_dialog(
                        title,
                        &format!("{detail}\n\nRequested name: {new_name_for_callback}"),
                    );
                }
            },
        );
    }

    fn create_new_folder(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !action_availability(
            &self.current_view_for(slot),
            self.selected_paths_for(slot).len(),
            self.file_clipboard.borrow().is_some(),
        )
        .can_new_folder
        {
            self.status
                .set_message("New Folder is not available in this view.");
            return;
        }

        let suggested_name = next_new_folder_path(&self.current_dir_for(self.active_slot()))
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("New Folder")
            .to_string();

        let controller = Rc::clone(self);
        self.modal_host.show_input(
            "New Folder",
            "Choose a name for the new folder.",
            &suggested_name,
            "Create",
            move |name| controller.create_folder_named(name),
        );
    }

    fn create_new_text_document(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !action_availability(
            &self.current_view_for(slot),
            self.selected_paths_for(slot).len(),
            self.file_clipboard.borrow().is_some(),
        )
        .can_new_folder
        {
            self.status
                .set_message("New Text Document is not available in this view.");
            return;
        }

        let suggested_name = next_new_text_document_path(&self.current_dir_for(slot))
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let controller = Rc::clone(self);
        self.modal_host.show_input(
            "New Text Document",
            "Choose a name for the new text document.",
            &suggested_name,
            "Create",
            move |name| controller.create_text_document_named(name),
        );
    }

    fn create_folder_named(self: &Rc<Self>, folder_name: String) {
        if folder_name.is_empty() {
            self.show_error_dialog("Invalid Name", "Folder names cannot be empty.");
            return;
        }

        let current_dir = self.current_dir_for(self.active_slot());
        let folder_path = current_dir.join(&folder_name);
        let file = gio::File::for_path(&folder_path);

        self.status.set_message("Creating folder…");
        let controller = Rc::clone(self);
        file.make_directory_async(
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(_) => {
                    controller
                        .pending_status_message
                        .replace(Some("Folder created.".to_string()));
                    controller.refresh();
                }
                Err(error) => {
                    let (title, detail) = friendly_error(&error);
                    controller.show_error_dialog(
                        title,
                        &format!("{detail}\n\nRequested name: {folder_name}"),
                    );
                }
            },
        );
    }

    fn create_text_document_named(self: &Rc<Self>, document_name: String) {
        if document_name.is_empty() {
            self.show_error_dialog("Invalid Name", "Document names cannot be empty.");
            return;
        }

        let slot = self.active_slot();
        let current_dir = self.current_dir_for(slot);
        let document_path = current_dir.join(&document_name);
        let file = gio::File::for_path(&document_path);

        self.status.set_message("Creating text document…");
        let controller = Rc::clone(self);
        file.create_async(
            gio::FileCreateFlags::NONE,
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(stream) => {
                    let controller = Rc::clone(&controller);
                    let document_name_for_close = document_name.clone();
                    let document_path_for_close = document_path.clone();
                    stream.close_async(
                        glib::Priority::DEFAULT,
                        None::<&gio::Cancellable>,
                        move |close_result| match close_result {
                            Ok(_) => {
                                controller
                                    .pending_reveal_cell(slot)
                                    .replace(Some(document_path_for_close.clone()));
                                controller
                                    .pending_status_message
                                    .replace(Some("Text document created.".to_string()));
                                controller.refresh();
                                controller
                                    .open_created_text_document(document_path_for_close.clone());
                            }
                            Err(error) => {
                                controller.refresh();
                                let (title, detail) = friendly_error(&error);
                                controller.show_error_dialog(
                                    title,
                                    &format!(
                                        "{detail}\n\nRequested name: {document_name_for_close}"
                                    ),
                                );
                            }
                        },
                    );
                }
                Err(error) => {
                    let (title, detail) = friendly_error(&error);
                    controller.show_error_dialog(
                        title,
                        &format!("{detail}\n\nRequested name: {document_name}"),
                    );
                }
            },
        );
    }

    fn show_pin_project_dialog(self: &Rc<Self>, path: PathBuf) {
        if !gio::File::for_path(&path).query_exists(None::<&gio::Cancellable>) {
            self.modal_host
                .show_error("Folder Missing", "That folder no longer exists.");
            return;
        }

        let initial_name = suggested_project_name(&path);
        let controller = Rc::clone(self);
        self.modal_host.show_input(
            "Pin as Project",
            "Choose a project name for this folder.",
            &initial_name,
            "Pin",
            move |name| controller.pin_project(path.clone(), name),
        );
    }

    fn pin_project(self: &Rc<Self>, path: PathBuf, name: String) {
        if name.trim().is_empty() {
            self.show_error_dialog("Invalid Name", "Project names cannot be empty.");
            return;
        }

        match self.metadata.borrow_mut().create_project(&name, &path) {
            Ok(project) => {
                self.refresh_metadata_sidebar();
                self.status
                    .set_message(&format!("Pinned project: {}.", project.name));
            }
            Err(error) => self.show_error_dialog("Project Save Failed", &error),
        }
    }

    fn show_add_tag_dialog(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            self.status
                .set_message("Select an item before adding a tag.");
            return;
        }

        let controller = Rc::clone(self);
        self.modal_host.show_input(
            "Add Tag",
            "Type a tag name to apply to the selected item(s). Existing names will be reused.",
            "",
            "Apply",
            move |name| controller.apply_tag_to_paths(paths.clone(), name),
        );
    }

    fn apply_tag_to_paths(self: &Rc<Self>, paths: Vec<PathBuf>, tag_name: String) {
        if tag_name.trim().is_empty() {
            self.show_error_dialog("Invalid Tag", "Tag names cannot be empty.");
            return;
        }

        let result = (|| {
            let mut metadata = self.metadata.borrow_mut();
            let tag = metadata.ensure_tag(&tag_name)?;
            metadata.add_tag_to_paths(tag.id, &paths)?;
            Ok::<TagRecord, String>(tag)
        })();

        match result {
            Ok(tag) => {
                self.refresh_metadata_sidebar();
                self.status
                    .set_message(&format!("Applied tag #{}.", tag.name));
                self.refresh();
            }
            Err(error) => self.show_error_dialog("Tag Apply Failed", &error),
        }
    }

    fn show_remove_tag_dialog(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            self.status
                .set_message("Select an item before removing tags.");
            return;
        }

        let tags = match self.metadata.borrow().tags_for_selection(&paths) {
            Ok(tags) => tags,
            Err(error) => {
                self.show_error_dialog("Tag Lookup Failed", &error);
                return;
            }
        };
        if tags.is_empty() {
            self.status
                .set_message("The selected item does not have any tags.");
            return;
        }

        let content = gtk::Box::new(Orientation::Vertical, 8);
        content.append(&build_modal_prompt(
            "Choose which tags to remove from the selected item(s).",
        ));

        let checks = tags
            .into_iter()
            .map(|tag| {
                let check = gtk::CheckButton::with_label(&format!("#{}", tag.name));
                check.set_active(true);
                check.set_halign(Align::Start);
                content.append(&check);
                (tag.id, check)
            })
            .collect::<Vec<_>>();

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let controller = Rc::clone(self);
        let remove_btn = build_modal_button("Remove", ButtonKind::Primary, move || {
            let selected = checks
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(tag_id, _)| *tag_id)
                .collect::<Vec<_>>();
            controller.remove_tags_from_paths(paths.clone(), selected);
            host.hide();
        });
        actions.append(&remove_btn);

        self.modal_host
            .show_with_custom_ui("Remove Tag", &content, &actions, false, None);
    }

    fn remove_tags_from_paths(self: &Rc<Self>, paths: Vec<PathBuf>, tag_ids: Vec<i64>) {
        if tag_ids.is_empty() {
            return;
        }

        let result = {
            let mut metadata = self.metadata.borrow_mut();
            let mut last = Ok(());
            for tag_id in tag_ids {
                last = metadata.remove_tag_from_paths(tag_id, &paths);
                if last.is_err() {
                    break;
                }
            }
            last
        };

        match result {
            Ok(()) => {
                self.refresh_metadata_sidebar();
                self.status.set_message("Removed selected tags.");
                self.refresh();
            }
            Err(error) => self.show_error_dialog("Tag Remove Failed", &error),
        }
    }

    fn show_send_to_project_dialog(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            self.status
                .set_message("Select an item before sending it to a project.");
            return;
        }

        let projects = self.projects.borrow().clone();
        if projects.is_empty() {
            self.modal_host.show_error(
                "No Projects Yet",
                "Pin a folder as a project first, then send files to it.",
            );
            return;
        }

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.append(&build_modal_prompt(
            "Choose a destination project and whether to copy or move.",
        ));

        let project_box = GtkBox::new(Orientation::Vertical, 8);
        project_box.set_halign(Align::Fill);
        project_box.set_hexpand(true);
        content.append(&project_box);

        let mut first_project_button: Option<gtk::CheckButton> = None;
        let project_buttons = projects
            .iter()
            .map(|project| {
                let button = gtk::CheckButton::with_label(&format!(
                    "{}  {}",
                    format_path(&project.root_path, &self.places.home),
                    project.name
                ));
                button.set_halign(Align::Start);
                if let Some(first) = &first_project_button {
                    button.set_group(Some(first));
                } else {
                    button.set_active(true);
                    first_project_button = Some(button.clone());
                }
                project_box.append(&button);
                (project.id, button)
            })
            .collect::<Vec<_>>();

        let action_row = GtkBox::new(Orientation::Horizontal, 12);
        content.append(&action_row);
        let copy_button = gtk::CheckButton::with_label("Copy to project");
        copy_button.set_active(true);
        copy_button.set_halign(Align::Start);
        action_row.append(&copy_button);

        let move_button = gtk::CheckButton::with_label("Move to project");
        move_button.set_group(Some(&copy_button));
        move_button.set_halign(Align::Start);
        action_row.append(&move_button);

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let controller = Rc::clone(self);
        let send_btn = build_modal_button("Send", ButtonKind::Primary, move || {
            let project_id = project_buttons
                .iter()
                .find(|(_, button)| button.is_active())
                .map(|(project_id, _)| *project_id);
            if let Some(project_id) = project_id {
                let kind = if move_button.is_active() {
                    ProjectTransferKind::Move
                } else {
                    ProjectTransferKind::Copy
                };
                controller.send_paths_to_project(paths.clone(), project_id, kind);
            }
            host.hide();
        });
        actions.append(&send_btn);

        self.modal_host
            .show_with_custom_ui("Send to Project", &content, &actions, false, None);
    }

    fn send_paths_to_project(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        project_id: i64,
        kind: ProjectTransferKind,
    ) {
        let Some(project) = self
            .projects
            .borrow()
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
        else {
            self.show_error_dialog("Project Missing", "That project no longer exists.");
            return;
        };

        let destination_root = match self
            .metadata
            .borrow()
            .list_project_destinations(project.id)
            .ok()
            .and_then(|destinations| {
                destinations
                    .into_iter()
                    .find(|destination| destination.relative_path.is_empty())
            }) {
            Some(destination) => project.root_path.join(destination.relative_path),
            None => project.root_path.clone(),
        };

        let verb = match kind {
            ProjectTransferKind::Copy => "Copy",
            ProjectTransferKind::Move => "Move",
        };
        let dest_name = destination_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&project.name);
        let label = format!("{verb} {} item(s) → {dest_name}", paths.len());
        let op_id = self.ops_panel.add_op(&label, None);
        self.run_project_transfer(
            paths,
            0,
            destination_root,
            kind,
            op_id,
            Rc::new(RefCell::new(BatchResult::default())),
        );
    }

    fn run_project_transfer(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        index: usize,
        destination_root: PathBuf,
        kind: ProjectTransferKind,
        op_id: OpId,
        result: Rc<RefCell<BatchResult>>,
    ) {
        if index >= paths.len() {
            let errs: Vec<String> = result.borrow().failures.clone();
            self.ops_panel.finish_op(op_id, &errs);
            self.refresh();
            return;
        }

        let total = paths.len();
        let source_path = paths[index].clone();
        let fname = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        self.ops_panel
            .update_progress(op_id, index as f64 / total as f64, &fname);

        let Some(file_name) = source_path.file_name() else {
            result
                .borrow_mut()
                .failures
                .push(format!("{}: Missing file name.", source_path.display()));
            self.run_project_transfer(paths, index + 1, destination_root, kind, op_id, result);
            return;
        };

        let destination_path = destination_root.join(file_name);
        if source_path == destination_path {
            result.borrow_mut().failures.push(format!(
                "{}: Source and destination are the same.",
                source_path.display()
            ));
            self.run_project_transfer(paths, index + 1, destination_root, kind, op_id, result);
            return;
        }

        if gio::File::for_path(&destination_path).query_exists(None::<&gio::Cancellable>) {
            self.show_project_conflict_dialog(
                paths,
                index,
                destination_root,
                kind,
                op_id,
                result,
                source_path,
                destination_path,
            );
            return;
        }

        self.perform_project_transfer(
            paths,
            index,
            destination_root,
            kind,
            op_id,
            result,
            source_path,
            destination_path,
            false,
        );
    }

    fn show_project_conflict_dialog(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        index: usize,
        destination_root: PathBuf,
        kind: ProjectTransferKind,
        op_id: OpId,
        result: Rc<RefCell<BatchResult>>,
        source_path: PathBuf,
        destination_path: PathBuf,
    ) {
        let prompt_text = format!(
            "A project item already exists named:\n{}\n\nChoose how to continue.",
            destination_path.display()
        );
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.append(&build_modal_prompt(&prompt_text));

        let next_dest = next_copy_path(&destination_path);
        let actions = build_modal_actions();

        // Cancel: abort the whole transfer operation
        let host = self.modal_host.clone();
        let ctrl = Rc::clone(self);
        let result_cancel = Rc::clone(&result);
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || {
            let errs: Vec<String> = result_cancel.borrow().failures.clone();
            ctrl.ops_panel.finish_op(op_id, &errs);
            ctrl.refresh();
            host.hide();
        });
        actions.append(&cancel_btn);

        // Rename Copy: transfer to a non-conflicting path
        let host = self.modal_host.clone();
        let ctrl = Rc::clone(self);
        let paths_rename = paths.clone();
        let dest_root_rename = destination_root.clone();
        let src_rename = source_path.clone();
        let result_rename = Rc::clone(&result);
        let rename_btn = build_modal_button("Rename Copy", ButtonKind::Primary, move || {
            ctrl.perform_project_transfer(
                paths_rename.clone(),
                index,
                dest_root_rename.clone(),
                kind,
                op_id,
                Rc::clone(&result_rename),
                src_rename.clone(),
                next_dest.clone(),
                false,
            );
            host.hide();
        });
        actions.append(&rename_btn);

        // Replace: overwrite the existing file
        let host = self.modal_host.clone();
        let ctrl = Rc::clone(self);
        let paths_replace = paths.clone();
        let dest_root_replace = destination_root.clone();
        let src_replace = source_path.clone();
        let dest_replace = destination_path.clone();
        let result_replace = Rc::clone(&result);
        let replace_btn = build_modal_button("Replace", ButtonKind::Danger, move || {
            ctrl.perform_project_transfer(
                paths_replace.clone(),
                index,
                dest_root_replace.clone(),
                kind,
                op_id,
                Rc::clone(&result_replace),
                src_replace.clone(),
                dest_replace.clone(),
                true,
            );
            host.hide();
        });
        actions.append(&replace_btn);

        // Dangerous action: scrim must not dismiss this dialog
        self.modal_host
            .show_with_custom_ui("Name Conflict", &content, &actions, false, None);
    }

    fn perform_project_transfer(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        index: usize,
        destination_root: PathBuf,
        kind: ProjectTransferKind,
        op_id: OpId,
        result: Rc<RefCell<BatchResult>>,
        source_path: PathBuf,
        destination_path: PathBuf,
        overwrite: bool,
    ) {
        let source = gio::File::for_path(&source_path);
        let destination = gio::File::for_path(&destination_path);
        let flags = if overwrite {
            gio::FileCopyFlags::OVERWRITE
        } else {
            gio::FileCopyFlags::NONE
        };

        let controller = Rc::clone(self);
        let src_disp = source_path.display().to_string();
        let dst_disp = destination_path.display().to_string();

        match kind {
            ProjectTransferKind::Copy => {
                let source_type = source
                    .query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>);
                if source_type == gio::FileType::Directory {
                    match copy_path_recursively(&source, &destination, overwrite) {
                        Ok(()) => result.borrow_mut().success_count += 1,
                        Err(error) => result.borrow_mut().failures.push(format!(
                            "{src_disp} → {dst_disp}: {}",
                            friendly_error_detail(&error)
                        )),
                    }
                    controller.run_project_transfer(
                        paths,
                        index + 1,
                        destination_root,
                        kind,
                        op_id,
                        result,
                    );
                } else {
                    let ops_panel = self.ops_panel.clone();
                    let fname = source_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    let total = paths.len();
                    let base_frac = index as f64 / total as f64;
                    let progress_cb: Box<dyn FnMut(i64, i64)> =
                        Box::new(move |done: i64, all: i64| {
                            let ff = if all > 0 {
                                done as f64 / all as f64
                            } else {
                                0.0
                            };
                            let overall = (base_frac + ff / total as f64).min(1.0);
                            let detail = if all > 0 {
                                format!(
                                    "{}  {} / {}",
                                    fname,
                                    fmt_bytes(done as u64),
                                    fmt_bytes(all as u64)
                                )
                            } else {
                                fname.clone()
                            };
                            ops_panel.update_progress(op_id, overall, &detail);
                        });
                    source.copy_async(
                        &destination,
                        flags,
                        glib::Priority::DEFAULT,
                        None::<&gio::Cancellable>,
                        Some(progress_cb),
                        move |operation| {
                            match operation {
                                Ok(_) => result.borrow_mut().success_count += 1,
                                Err(error) => result.borrow_mut().failures.push(format!(
                                    "{src_disp} → {dst_disp}: {}",
                                    friendly_error_detail(&error)
                                )),
                            }
                            controller.run_project_transfer(
                                paths,
                                index + 1,
                                destination_root,
                                kind,
                                op_id,
                                result,
                            );
                        },
                    );
                }
            }
            ProjectTransferKind::Move => {
                source.move_async(
                    &destination,
                    flags,
                    glib::Priority::DEFAULT,
                    None::<&gio::Cancellable>,
                    None,
                    move |operation| {
                        match operation {
                            Ok(()) => {
                                result.borrow_mut().success_count += 1;
                                let _ = controller
                                    .metadata
                                    .borrow_mut()
                                    .move_tagged_path_prefix(&source_path, &destination_path);
                            }
                            Err(error) => result.borrow_mut().failures.push(format!(
                                "{src_disp} → {dst_disp}: {}",
                                friendly_error_detail(&error)
                            )),
                        }
                        controller.run_project_transfer(
                            paths,
                            index + 1,
                            destination_root,
                            kind,
                            op_id,
                            result,
                        );
                    },
                );
            }
        }
    }

    fn copy_selected_to_file_clipboard(&self, mode: ClipboardMode) {
        let slot = self.active_slot();
        let selected = self.selected_paths_for(slot);
        if selected.is_empty() {
            self.status
                .set_message("Select one or more items before using that shortcut.");
            return;
        }

        if mode == ClipboardMode::Cut
            && !action_availability(
                &self.current_view_for(slot),
                selected.len(),
                self.file_clipboard.borrow().is_some(),
            )
            .can_cut_files
        {
            self.status
                .set_message("Cut is not available in this view.");
            return;
        }

        self.file_clipboard
            .replace(FileClipboardState::new(selected.clone(), mode));
        let verb = match mode {
            ClipboardMode::Copy => "Copied",
            ClipboardMode::Cut => "Ready to move",
        };
        self.status.set_message(&format!(
            "{verb} {} item{}.",
            selected.len(),
            if selected.len() == 1 { "" } else { "s" }
        ));
    }

    fn paste_file_clipboard_into_active_pane(self: &Rc<Self>) {
        let slot = self.active_slot();
        let Some(clipboard) = self.file_clipboard.borrow().clone() else {
            self.status.set_message("Nothing is waiting to be pasted.");
            return;
        };

        let Some(destination) = self.paste_destination_for_slot(slot) else {
            self.status
                .set_message("Paste is not available in this view.");
            return;
        };

        let is_copy = clipboard.is_copy();
        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            controller
                .handle_dnd_drop(
                    clipboard.paths.clone(),
                    destination,
                    is_copy,
                    Some(clipboard),
                )
                .await;
        });
    }

    fn paste_destination_for_slot(&self, slot: PaneSlot) -> Option<PathBuf> {
        match self.current_view_for(slot) {
            PaneView::Directory(path) => Some(path),
            PaneView::DownloadsTriage(_) => Some(self.current_dir_for(slot)),
            PaneView::Tag(_)
            | PaneView::Trash
            | PaneView::SystemDrives
            | PaneView::Recent
            | PaneView::Search(_) => None,
        }
    }

    fn update_file_clipboard_after_batch(
        &self,
        snapshot: Option<&FileClipboardState>,
        moved_sources: &[PathBuf],
    ) {
        let Some(snapshot) = snapshot else {
            return;
        };

        let current = self.file_clipboard.borrow().clone();
        if current.as_ref() != Some(snapshot) {
            return;
        }

        self.file_clipboard
            .replace(snapshot.after_completed_paste(moved_sources));
    }

    fn trash_selected(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !action_availability(
            &self.current_view_for(slot),
            self.selected_paths_for(slot).len(),
            self.file_clipboard.borrow().is_some(),
        )
        .can_trash
        {
            self.status
                .set_message("Move to Trash is not available in this view.");
            return;
        }

        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }

        self.move_paths_to_trash(paths);
    }

    fn move_paths_to_trash(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let label = if paths.len() == 1 {
            format!(
                "Trash: {}",
                paths[0]
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("item")
            )
        } else {
            format!("Trash: {} items", paths.len())
        };
        let cancellable = gio::Cancellable::new();
        let op_id = self.ops_panel.add_op(&label, Some(cancellable.clone()));
        self.run_trash_op(
            op_id,
            Rc::new(paths),
            0,
            cancellable,
            Rc::new(RefCell::new(Vec::new())),
        );
    }

    fn run_trash_op(
        self: &Rc<Self>,
        op_id: OpId,
        paths: Rc<Vec<PathBuf>>,
        index: usize,
        cancellable: gio::Cancellable,
        errors: Rc<RefCell<Vec<String>>>,
    ) {
        let total = paths.len();

        if index >= total {
            let errs = errors.borrow().clone();
            self.ops_panel.finish_op(op_id, &errs);
            self.refresh();
            return;
        }

        if cancellable.is_cancelled() {
            self.ops_panel.finish_op(op_id, &["Cancelled.".to_string()]);
            self.refresh();
            return;
        }

        let current_path = paths[index].clone();
        let fname = current_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        self.ops_panel
            .update_progress(op_id, index as f64 / total as f64, &fname);

        let controller = Rc::clone(self);
        let errors_clone = Rc::clone(&errors);
        let paths_clone = Rc::clone(&paths);
        let cancellable_clone = cancellable.clone();

        gio::File::for_path(&current_path).trash_async(
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                match result {
                    Ok(_) => {
                        let _ = controller
                            .metadata
                            .borrow_mut()
                            .delete_tagged_path_prefix(&current_path);
                    }
                    Err(ref e)
                        if e.kind::<gio::IOErrorEnum>() != Some(gio::IOErrorEnum::Cancelled) =>
                    {
                        errors_clone.borrow_mut().push(format!(
                            "{}: {}",
                            current_path.display(),
                            e.message()
                        ));
                    }
                    _ => {}
                }
                controller.run_trash_op(
                    op_id,
                    paths_clone,
                    index + 1,
                    cancellable_clone,
                    errors_clone,
                );
            },
        );
    }

    fn confirm_permanent_delete(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let display = if paths.len() == 1 {
            format!(
                "\u{201c}{}\u{201d}",
                paths[0]
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("item")
            )
        } else {
            format!("{} items", paths.len())
        };
        let prompt = format!(
            "Permanently delete {display}?\n\nThis cannot be undone. The file will be gone forever."
        );
        let controller = Rc::clone(self);
        self.modal_host.show_confirm(
            "Delete Permanently",
            &prompt,
            "Delete Forever",
            true,
            true,
            move || controller.delete_items_permanently(paths.clone()),
        );
    }

    fn delete_items_permanently(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let label = if paths.len() == 1 {
            format!(
                "Delete: {}",
                paths[0]
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("item")
            )
        } else {
            format!("Delete: {} items", paths.len())
        };
        let cancellable = gio::Cancellable::new();
        let op_id = self.ops_panel.add_op(&label, Some(cancellable.clone()));
        self.run_delete_op(
            op_id,
            Rc::new(paths),
            0,
            cancellable,
            Rc::new(RefCell::new(Vec::new())),
        );
    }

    fn run_delete_op(
        self: &Rc<Self>,
        op_id: OpId,
        paths: Rc<Vec<PathBuf>>,
        index: usize,
        cancellable: gio::Cancellable,
        errors: Rc<RefCell<Vec<String>>>,
    ) {
        let total = paths.len();

        if index >= total {
            let errs = errors.borrow().clone();
            self.ops_panel.finish_op(op_id, &errs);
            self.refresh();
            return;
        }

        if cancellable.is_cancelled() {
            self.ops_panel.finish_op(op_id, &["Cancelled.".to_string()]);
            self.refresh();
            return;
        }

        let current_path = paths[index].clone();
        let fname = current_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        self.ops_panel
            .update_progress(op_id, index as f64 / total as f64, &fname);

        let controller = Rc::clone(self);
        let errors_clone = Rc::clone(&errors);
        let paths_clone = Rc::clone(&paths);
        let cancellable_clone = cancellable.clone();

        gio::File::for_path(&current_path).delete_async(
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                match result {
                    Ok(_) => {
                        let _ = controller
                            .metadata
                            .borrow_mut()
                            .delete_tagged_path_prefix(&current_path);
                    }
                    Err(ref e)
                        if e.kind::<gio::IOErrorEnum>() != Some(gio::IOErrorEnum::Cancelled) =>
                    {
                        errors_clone.borrow_mut().push(format!(
                            "{}: {}",
                            current_path.display(),
                            e.message()
                        ));
                    }
                    _ => {}
                }
                controller.run_delete_op(
                    op_id,
                    paths_clone,
                    index + 1,
                    cancellable_clone,
                    errors_clone,
                );
            },
        );
    }

    fn copy_paths_to_clipboard(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        let text = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        self.window.clipboard().set_text(&text);
        self.status.set_message(&format!(
            "Copied {} path{} to the clipboard.",
            paths.len(),
            if paths.len() == 1 { "" } else { "s" }
        ));
    }

    fn copy_path_text_for_active_context(self: &Rc<Self>) {
        let slot = self.active_slot();
        let paths = {
            let selected = self.selected_paths_for(slot);
            if selected.is_empty() {
                match self.current_view_for(slot) {
                    PaneView::Directory(path) => vec![path],
                    PaneView::DownloadsTriage(_) => vec![self.current_dir_for(slot)],
                    PaneView::Search(query) => vec![query.scope_dir],
                    _ => {
                        self.status
                            .set_message("Select an item to copy its path in this view.");
                        return;
                    }
                }
            } else {
                selected
            }
        };
        self.copy_paths_to_clipboard(paths);
    }

    fn open_terminal_for_path(self: &Rc<Self>, path: PathBuf, is_dir: bool) {
        let Some(command) = self.terminal_command.clone() else {
            self.status
                .set_message("No terminal command is configured.");
            self.show_error_dialog(
                "Terminal Unavailable",
                "No terminal command was found. Set LATTICE_TERMINAL or install kitty or x-terminal-emulator.",
            );
            return;
        };

        let directory = if is_dir {
            path
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.current_dir_for(self.active_slot()))
        };

        let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
        launcher.set_cwd(&directory);
        let args = command
            .iter()
            .map(|value| value.as_os_str())
            .collect::<Vec<&OsStr>>();

        match launcher.spawn(&args) {
            Ok(_) => self
                .status
                .set_message("Opened terminal for this location."),
            Err(error) => {
                let (title, detail) = friendly_error(&error);
                self.show_error_dialog(title, &detail);
            }
        }
    }

    fn open_current_folder_terminal(self: &Rc<Self>) {
        self.open_terminal_for_path(self.current_dir_for(self.active_slot()), true);
    }

    fn run_custom_action_by_id(self: &Rc<Self>, action_id: &str) {
        let Some(action) = self.config.custom_action(action_id).cloned() else {
            let message = format!("Custom action '{action_id}' is not configured.");
            self.status.set_message(&message);
            return;
        };
        let paths = custom_action_paths_for_context(self, self.active_slot());
        self.run_custom_action(&action, paths);
    }

    fn run_custom_action(self: &Rc<Self>, action: &CustomActionConfig, paths: Vec<PathBuf>) {
        if action.needs_selection && paths.is_empty() {
            let message = format!("Select one or more items before using {}.", action.label);
            self.status.set_message(&message);
            return;
        }

        let cwd = self.current_dir_for(self.active_slot());
        let Some(argv) = expand_custom_action_argv(action, &paths, &cwd) else {
            self.show_error_dialog(
                "Custom Action Unavailable",
                &format!("{} has no command to run.", action.label),
            );
            return;
        };

        let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
        launcher.set_cwd(&cwd);
        let args = argv
            .iter()
            .map(|value| value.as_os_str())
            .collect::<Vec<&OsStr>>();

        match launcher.spawn(&args) {
            Ok(_) => {
                let message = format!("Started custom action: {}.", action.label);
                self.status.set_message(&message);
            }
            Err(error) => {
                let (title, detail) = friendly_error(&error);
                self.show_error_dialog(title, &detail);
            }
        }
    }

    fn open_preview_target(self: &Rc<Self>) {
        if let Some(item) = self.selected_single_item() {
            self.open_item(&item);
        }
    }

    fn copy_preview_target_path(self: &Rc<Self>) {
        self.copy_path_text_for_active_context();
    }

    fn open_preview_parent(self: &Rc<Self>) {
        let target = if let Some(item) = self.selected_single_item() {
            item.path.parent().map(Path::to_path_buf)
        } else {
            self.current_dir_for(self.active_slot())
                .parent()
                .map(Path::to_path_buf)
        };

        let Some(target) = target else {
            self.status
                .set_message("No parent folder is available here.");
            return;
        };

        if target == self.current_dir_for(self.active_slot()) {
            self.status
                .set_message("Already showing the parent folder.");
            return;
        }

        self.navigate_to(self.active_slot(), target, true);
    }

    fn selected_items(&self) -> Vec<FileItem> {
        self.selected_items_for(self.active_slot())
    }

    fn selected_items_for(&self, slot: PaneSlot) -> Vec<FileItem> {
        self.pane_widgets(slot)
            .file_grid
            .selected_indices()
            .into_iter()
            .filter_map(|index| self.item_for_index(slot, index))
            .collect()
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_paths_for(self.active_slot())
    }

    fn selected_paths_for(&self, slot: PaneSlot) -> Vec<PathBuf> {
        self.selected_items_for(slot)
            .into_iter()
            .map(|item| item.path)
            .collect()
    }

    fn selected_single_item(&self) -> Option<FileItem> {
        self.selected_single_item_for(self.active_slot())
    }

    fn selected_single_item_for(&self, slot: PaneSlot) -> Option<FileItem> {
        let items = self.selected_items_for(slot);
        if items.len() == 1 {
            items.into_iter().next()
        } else {
            None
        }
    }

    fn update_navigation_state(&self) {
        let is_directory = self.is_directory_view(self.active_slot());
        self.toolbar.back_button.set_sensitive(
            is_directory
                && !self
                    .back_history_cell(self.active_slot())
                    .borrow()
                    .is_empty(),
        );
        self.toolbar.up_button.set_sensitive(
            is_directory
                && self
                    .current_dir_for(self.active_slot())
                    .parent()
                    .map(Path::exists)
                    .unwrap_or(false),
        );
        self.toolbar.refresh_button.set_sensitive(true);
    }

    fn update_sidebar_state(&self) {
        let active = match self.current_view_for(self.active_slot()) {
            PaneView::Tag(tag) => Some(SidebarTarget::Tag(tag.id)),
            PaneView::DownloadsTriage(_) => Some(SidebarTarget::DownloadsTriage),
            PaneView::SystemDrives => Some(SidebarTarget::SystemDrives),
            PaneView::Recent => Some(SidebarTarget::Recent),
            PaneView::Trash => Some(SidebarTarget::Trash),
            PaneView::Search(_) => None,
            PaneView::Directory(_) => {
                let current = self.current_dir_for(self.active_slot());
                self.projects
                    .borrow()
                    .iter()
                    .find(|project| current.starts_with(&project.root_path))
                    .map(|project| SidebarTarget::Project(project.id))
                    .or_else(|| {
                        current
                            .starts_with(&self.places.downloads)
                            .then_some(SidebarTarget::Downloads)
                    })
                    .or_else(|| {
                        current
                            .starts_with(&self.places.documents)
                            .then_some(SidebarTarget::Documents)
                    })
                    .or_else(|| {
                        current
                            .starts_with(&self.places.home)
                            .then_some(SidebarTarget::Home)
                    })
            }
        };

        self.sidebar.set_active(active.as_ref());
    }

    fn item_for_index(&self, slot: PaneSlot, index: i32) -> Option<FileItem> {
        if index < 0 {
            return None;
        }

        self.items_cell(slot).borrow().get(index as usize).cloned()
    }

    fn reveal_pending_selection(&self, slot: PaneSlot) -> bool {
        let Some(target_path) = self.pending_reveal_cell(slot).borrow_mut().take() else {
            return false;
        };

        let Some(index) = self
            .items_cell(slot)
            .borrow()
            .iter()
            .position(|item| item.path == target_path)
        else {
            return false;
        };

        self.pane_widgets(slot)
            .file_grid
            .select_only_index(index as i32);
        self.set_keyboard_focus(slot, index as i32, true);
        true
    }

    fn display_path(&self, path: &Path) -> String {
        format_path(path, &self.places.home)
    }

    fn sync_path_entry_to_display(&self) {
        let display_path = self.display_label_for(self.active_slot());
        self.toolbar.path_entry.set_text(&display_path);
        self.toolbar.set_breadcrumb_path(&display_path);
        self.toolbar.show_breadcrumb_mode();
    }

    fn resolve_path_input(&self, input: &str) -> Option<gio::File> {
        if input.starts_with("file://") {
            return Some(gio::File::for_uri(input));
        }

        if input == "~" {
            return Some(gio::File::for_path(&self.places.home));
        }

        if let Some(relative_home) = input.strip_prefix("~/") {
            return Some(gio::File::for_path(self.places.home.join(relative_home)));
        }

        let candidate = PathBuf::from(input);
        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            self.current_dir_for(self.active_slot()).join(candidate)
        };

        Some(gio::File::for_path(resolved))
    }

    fn thumb_loader_for(&self, slot: PaneSlot) -> &crate::thumbnail::ThumbnailLoader {
        match slot {
            PaneSlot::Primary => &self.primary_thumb_loader,
            PaneSlot::Secondary => &self.secondary_thumb_loader,
        }
    }

    fn cancel_active_load(&self, slot: PaneSlot) {
        if let Some(cancellable) = self.load_cancellable_cell(slot).borrow_mut().take() {
            cancellable.cancel();
        }
        self.thumb_loader_for(slot).cancel();
    }

    fn cancel_active_preview(&self) {
        if let Some(cancellable) = self.preview_cancellable.borrow_mut().take() {
            cancellable.cancel();
        }
    }

    fn is_current_load(&self, slot: PaneSlot, generation: u64) -> bool {
        self.load_generation_cell(slot).get() == generation
    }

    fn is_current_preview(&self, generation: u64) -> bool {
        self.preview_generation.get() == generation
    }

    fn show_error_dialog(&self, title: &str, detail: &str) {
        self.modal_host.show_error(title, detail);
    }

    // ── Drag-and-drop ────────────────────────────────────────────────

    /// Called once from bootstrap. Adds a pane-level DropTarget that catches
    /// drops anywhere in the pane and deposits files into the current folder.
    fn attach_pane_dnd(self: &Rc<Self>, slot: PaneSlot) {
        let pane_root = self.pane_widgets(slot).root.clone();
        let last_action = Rc::new(Cell::new(gdk::DragAction::MOVE));
        let la_motion = last_action.clone();

        let drop_target = gtk::DropTarget::new(
            glib::Type::STRING,
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );

        let root_enter = pane_root.clone();
        drop_target.connect_enter(move |_, _, _| {
            root_enter.add_css_class("drop-active");
            gdk::DragAction::MOVE
        });

        drop_target.connect_motion(move |_, _, _| {
            let a = if ctrl_held() {
                gdk::DragAction::COPY
            } else {
                gdk::DragAction::MOVE
            };
            la_motion.set(a);
            a
        });

        let root_leave = pane_root.clone();
        drop_target.connect_leave(move |_| {
            root_leave.remove_css_class("drop-active");
        });

        let controller = Rc::clone(self);
        let root_drop = pane_root.clone();
        drop_target.connect_drop(move |_, value, _, _| {
            root_drop.remove_css_class("drop-active");
            // Don't accept background drops in virtual views with no real folder target.
            if matches!(
                controller.current_view_for(slot),
                PaneView::Trash | PaneView::SystemDrives | PaneView::Recent | PaneView::Search(_)
            ) {
                return false;
            }
            let paths = parse_dropped_uris(value);
            if paths.is_empty() {
                return false;
            }
            let dest = controller.current_dir_for(slot);
            let is_copy = last_action.get() == gdk::DragAction::COPY;
            let c = Rc::clone(&controller);
            glib::MainContext::default().spawn_local(async move {
                c.handle_dnd_drop(paths, dest, is_copy, None).await;
            });
            true
        });

        pane_root.add_controller(drop_target);
    }

    /// Called once from bootstrap. Adds DropTargets to the static sidebar
    /// buttons (Home, Downloads, Documents) so files can be dragged into them.
    fn attach_sidebar_dnd(self: &Rc<Self>) {
        let buttons: &[(gtk::Button, PathBuf)] = &[
            (self.sidebar.home_button.clone(), self.places.home.clone()),
            (
                self.sidebar.downloads_button.clone(),
                self.places.downloads.clone(),
            ),
            (
                self.sidebar.documents_button.clone(),
                self.places.documents.clone(),
            ),
        ];

        for (button, dest_path) in buttons {
            let last_action = Rc::new(Cell::new(gdk::DragAction::MOVE));
            let la_motion = last_action.clone();

            let drop_target = gtk::DropTarget::new(
                glib::Type::STRING,
                gdk::DragAction::COPY | gdk::DragAction::MOVE,
            );

            let btn_enter = button.clone();
            drop_target.connect_enter(move |_, _, _| {
                btn_enter.add_css_class("drop-hover");
                gdk::DragAction::MOVE
            });

            drop_target.connect_motion(move |_, _, _| {
                let a = if ctrl_held() {
                    gdk::DragAction::COPY
                } else {
                    gdk::DragAction::MOVE
                };
                la_motion.set(a);
                a
            });

            let btn_leave = button.clone();
            drop_target.connect_leave(move |_| {
                btn_leave.remove_css_class("drop-hover");
            });

            let controller = Rc::clone(self);
            let btn_drop = button.clone();
            let dest = dest_path.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                btn_drop.remove_css_class("drop-hover");
                let paths = parse_dropped_uris(value);
                if paths.is_empty() {
                    return false;
                }
                let is_copy = last_action.get() == gdk::DragAction::COPY;
                let c = Rc::clone(&controller);
                let d = dest.clone();
                glib::MainContext::default().spawn_local(async move {
                    c.handle_dnd_drop(paths, d, is_copy, None).await;
                });
                true
            });

            button.add_controller(drop_target);
        }
    }

    /// Called after every set_items(). Attaches DragSources to every card
    /// and DropTargets to every folder card.
    fn attach_item_dnd(self: &Rc<Self>, slot: PaneSlot) {
        let pane = self.pane_widgets(slot).clone();
        let count = self.items_cell(slot).borrow().len();

        for idx in 0..count {
            let Some(flow_child) = pane.file_grid.flow.child_at_index(idx as i32) else {
                continue;
            };
            let Some(item) = self.items_cell(slot).borrow().get(idx).cloned() else {
                continue;
            };

            // ── Drag source on every card ──────────────────────────
            let drag_source = gtk::DragSource::new();
            drag_source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);

            let ctrl = Rc::clone(self);
            drag_source.connect_prepare(move |_, _, _| {
                let all = ctrl.items_cell(slot).borrow();
                let grid = ctrl.pane_widgets(slot).file_grid.clone();
                let selected = grid.selected_indices();

                let paths: Vec<PathBuf> = if selected.contains(&(idx as i32)) {
                    selected
                        .iter()
                        .filter_map(|&i| all.get(i as usize).map(|it| it.path.clone()))
                        .collect()
                } else {
                    all.get(idx)
                        .map(|it| vec![it.path.clone()])
                        .unwrap_or_default()
                };
                drop(all);

                if paths.is_empty() {
                    return None;
                }

                let uri_list = paths
                    .iter()
                    .map(|p| gio::File::for_path(p).uri().to_string())
                    .collect::<Vec<_>>()
                    .join("\r\n");

                Some(gdk::ContentProvider::for_value(&uri_list.to_value()))
            });

            flow_child.add_controller(drag_source);

            // ── Drop target on folder cards ────────────────────────
            // Skip folder drop targets in virtual views — they are not normal folder listings.
            if !item.is_dir
                || matches!(
                    self.current_view_for(slot),
                    PaneView::Trash
                        | PaneView::SystemDrives
                        | PaneView::Recent
                        | PaneView::Search(_)
                )
            {
                continue;
            }

            let last_action = Rc::new(Cell::new(gdk::DragAction::MOVE));
            let la_motion = last_action.clone();

            let drop_target = gtk::DropTarget::new(
                glib::Type::STRING,
                gdk::DragAction::COPY | gdk::DragAction::MOVE,
            );

            let fc_enter = flow_child.clone();
            drop_target.connect_enter(move |_, _, _| {
                if let Some(card) = fc_enter.first_child() {
                    card.add_css_class("drop-hover");
                }
                gdk::DragAction::MOVE
            });

            drop_target.connect_motion(move |_, _, _| {
                let a = if ctrl_held() {
                    gdk::DragAction::COPY
                } else {
                    gdk::DragAction::MOVE
                };
                la_motion.set(a);
                a
            });

            let fc_leave = flow_child.clone();
            drop_target.connect_leave(move |_| {
                if let Some(card) = fc_leave.first_child() {
                    card.remove_css_class("drop-hover");
                }
            });

            let controller = Rc::clone(self);
            let folder_path = item.path.clone();
            let fc_drop = flow_child.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                if let Some(card) = fc_drop.first_child() {
                    card.remove_css_class("drop-hover");
                }
                let paths = parse_dropped_uris(value);
                if paths.is_empty() {
                    return false;
                }
                let is_copy = last_action.get() == gdk::DragAction::COPY;
                let dest = folder_path.clone();
                let c = Rc::clone(&controller);
                glib::MainContext::default().spawn_local(async move {
                    c.handle_dnd_drop(paths, dest, is_copy, None).await;
                });
                true
            });

            flow_child.add_controller(drop_target);
        }
    }

    /// Core async drop handler. Resolves conflicts, performs copy/move via GIO,
    /// then reloads affected panes.
    async fn handle_dnd_drop(
        self: Rc<Self>,
        src_paths: Vec<PathBuf>,
        dest_dir: PathBuf,
        is_copy: bool,
        clipboard_state: Option<FileClipboardState>,
    ) {
        let src_paths: Vec<PathBuf> = src_paths
            .into_iter()
            .filter(|src| {
                let already_there = src.parent().map(|p| p == dest_dir).unwrap_or(false);
                let into_self = dest_dir.starts_with(src);
                !already_there && !into_self
            })
            .collect();

        if src_paths.is_empty() {
            return;
        }

        let flags_overwrite = gio::FileCopyFlags::OVERWRITE | gio::FileCopyFlags::ALL_METADATA;
        let flags_plain = gio::FileCopyFlags::ALL_METADATA;

        // Phase 1: resolve conflicts (async, shows dialogs)
        let mut resolved: Vec<(PathBuf, PathBuf, gio::FileCopyFlags)> = Vec::new();
        for src in &src_paths {
            let filename = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let tentative = dest_dir.join(&filename);
            let (dst, flags) = if tentative.exists() {
                match self.show_conflict_dialog(&filename).await {
                    ConflictChoice::Skip => continue,
                    ConflictChoice::Replace => (tentative, flags_overwrite),
                    ConflictChoice::KeepBoth => (free_name_in(&dest_dir, &filename), flags_plain),
                }
            } else {
                (tentative, flags_plain)
            };
            resolved.push((src.clone(), dst, flags));
        }

        if resolved.is_empty() {
            return;
        }

        // Phase 2: hand off to the queued op runner
        let dest_name = dest_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("destination")
            .to_string();
        let verb = if is_copy { "Copy" } else { "Move" };
        let label = format!("{verb} {} item(s) → {dest_name}", resolved.len());
        self.start_copy_move_op(resolved, is_copy, &label, clipboard_state);
    }

    fn start_copy_move_op(
        self: &Rc<Self>,
        items: Vec<(PathBuf, PathBuf, gio::FileCopyFlags)>,
        is_copy: bool,
        label: &str,
        clipboard_state: Option<FileClipboardState>,
    ) {
        let cancellable = gio::Cancellable::new();
        let op_id = self.ops_panel.add_op(label, Some(cancellable.clone()));
        self.run_copy_move_batch(
            op_id,
            Rc::new(items),
            0,
            is_copy,
            cancellable,
            Rc::new(RefCell::new(Vec::new())),
            Rc::new(RefCell::new(Vec::new())),
            clipboard_state,
        );
    }

    fn run_copy_move_batch(
        self: &Rc<Self>,
        op_id: OpId,
        items: Rc<Vec<(PathBuf, PathBuf, gio::FileCopyFlags)>>,
        index: usize,
        is_copy: bool,
        cancellable: gio::Cancellable,
        errors: Rc<RefCell<Vec<String>>>,
        moved_sources: Rc<RefCell<Vec<PathBuf>>>,
        clipboard_state: Option<FileClipboardState>,
    ) {
        let total = items.len();

        if index >= total {
            let errs = errors.borrow().clone();
            self.update_file_clipboard_after_batch(
                clipboard_state.as_ref(),
                &moved_sources.borrow(),
            );
            self.ops_panel.finish_op(op_id, &errs);
            self.load_current_view(PaneSlot::Primary);
            if self.split_enabled.get() {
                self.load_current_view(PaneSlot::Secondary);
            }
            return;
        }

        if cancellable.is_cancelled() {
            self.update_file_clipboard_after_batch(
                clipboard_state.as_ref(),
                &moved_sources.borrow(),
            );
            self.ops_panel.finish_op(op_id, &["Cancelled.".to_string()]);
            self.load_current_view(PaneSlot::Primary);
            if self.split_enabled.get() {
                self.load_current_view(PaneSlot::Secondary);
            }
            return;
        }

        let (src, dst, flags) = &items[index];
        let fname = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let base_frac = index as f64 / total as f64;
        let flags = *flags;

        self.ops_panel.update_progress(op_id, base_frac, &fname);

        let ops_panel = self.ops_panel.clone();
        let fname_for_cb = fname.clone();
        let progress_cb: Box<dyn FnMut(i64, i64)> = Box::new(move |done: i64, all: i64| {
            let file_frac = if all > 0 {
                done as f64 / all as f64
            } else {
                0.0
            };
            let overall = (base_frac + file_frac / total as f64).min(1.0);
            let detail = if all > 0 {
                format!(
                    "{}  {} / {}",
                    fname_for_cb,
                    fmt_bytes(done as u64),
                    fmt_bytes(all as u64)
                )
            } else {
                fname_for_cb.clone()
            };
            ops_panel.update_progress(op_id, overall, &detail);
        });

        let src_file = gio::File::for_path(src);
        let dst_file = gio::File::for_path(dst);
        let src_display = src.display().to_string();
        let src_path = src.clone();
        let dst_path = dst.clone();

        let controller = Rc::clone(self);
        let items_clone = Rc::clone(&items);
        let errors_clone = Rc::clone(&errors);
        let moved_sources_clone = Rc::clone(&moved_sources);
        let cancellable_clone = cancellable.clone();
        let clipboard_state_clone = clipboard_state.clone();
        let src_display_for_completion = src_display.clone();

        let completion = move |result: Result<(), glib::Error>| {
            match result {
                Ok(()) => {
                    if !is_copy {
                        moved_sources_clone.borrow_mut().push(src_path.clone());
                        if let Err(error) = controller
                            .metadata
                            .borrow_mut()
                            .move_tagged_path_prefix(&src_path, &dst_path)
                        {
                            eprintln!("Lattice metadata move update failed: {error}");
                        }
                    }
                }
                Err(ref e) => {
                    if e.kind::<gio::IOErrorEnum>() != Some(gio::IOErrorEnum::Cancelled) {
                        errors_clone
                            .borrow_mut()
                            .push(format!("{src_display_for_completion}: {}", e.message()));
                    }
                }
            }
            controller.run_copy_move_batch(
                op_id,
                items_clone,
                index + 1,
                is_copy,
                cancellable_clone,
                errors_clone,
                moved_sources_clone,
                clipboard_state_clone,
            );
        };

        if is_copy
            && src_file.query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>)
                == gio::FileType::Directory
        {
            let controller = Rc::clone(self);
            let items_clone = Rc::clone(&items);
            let errors_clone = Rc::clone(&errors);
            let moved_sources_clone = Rc::clone(&moved_sources);
            let cancellable_clone = cancellable.clone();
            let clipboard_state_clone = clipboard_state.clone();
            let src_path = src.clone();
            let dst_path = dst.clone();
            let src_display = src_display.clone();
            glib::MainContext::default().spawn_local(async move {
                let src_file = gio::File::for_path(&src_path);
                let dst_file = gio::File::for_path(&dst_path);
                let result = match gio::spawn_blocking(move || {
                    copy_path_recursively(
                        &src_file,
                        &dst_file,
                        flags.contains(gio::FileCopyFlags::OVERWRITE),
                    )
                })
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(glib::Error::new(
                        gio::IOErrorEnum::Failed,
                        "Directory copy task failed.",
                    )),
                };

                if let Err(ref e) = result {
                    if e.kind::<gio::IOErrorEnum>() != Some(gio::IOErrorEnum::Cancelled) {
                        errors_clone
                            .borrow_mut()
                            .push(format!("{src_display}: {}", e.message()));
                    }
                }

                controller.run_copy_move_batch(
                    op_id,
                    items_clone,
                    index + 1,
                    is_copy,
                    cancellable_clone,
                    errors_clone,
                    moved_sources_clone,
                    clipboard_state_clone,
                );
            });
        } else if is_copy {
            src_file.copy_async(
                &dst_file,
                flags,
                glib::Priority::DEFAULT,
                Some(&cancellable),
                Some(progress_cb),
                completion,
            );
        } else {
            src_file.move_async(
                &dst_file,
                flags,
                glib::Priority::DEFAULT,
                Some(&cancellable),
                Some(progress_cb),
                completion,
            );
        }
    }

    /// Show a conflict resolution dialog and return the user's choice.
    async fn show_conflict_dialog(self: &Rc<Self>, name: &str) -> ConflictChoice {
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(format!("\u{201c}{name}\u{201d} already exists"))
            .detail(
                "A file with this name already exists at the destination. \
                 What would you like to do?",
            )
            .buttons(["Skip", "Keep Both", "Replace"])
            .cancel_button(0)
            .default_button(1)
            .build();

        match dialog.choose_future(Some(&self.window)).await {
            Ok(1) => ConflictChoice::KeepBoth,
            Ok(2) => ConflictChoice::Replace,
            _ => ConflictChoice::Skip,
        }
    }
}

// ── DnD helpers ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum ConflictChoice {
    Skip,
    KeepBoth,
    Replace,
}

fn relevant_modifiers(modifiers: gdk::ModifierType) -> gdk::ModifierType {
    modifiers
        & (gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK)
}

fn key_char(key: gdk::Key) -> Option<char> {
    key.to_unicode().map(|value| value.to_ascii_lowercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyBinding {
    ctrl: bool,
    shift: bool,
    alt: bool,
    key: BindingKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKey {
    Char(char),
    Named(&'static str),
}

#[cfg(test)]
fn window_command_from_key(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<WindowCommand> {
    configured_window_command_from_key(&AppConfig::default(), key, modifiers)
}

fn configured_window_command_from_key(
    config: &AppConfig,
    key: gdk::Key,
    modifiers: gdk::ModifierType,
) -> Option<WindowCommand> {
    for (action_id, shortcut) in &config.shortcuts {
        let Some(binding) = parse_key_binding(shortcut) else {
            continue;
        };
        if binding.matches(key, modifiers) {
            if let Some(custom_id) = action_id.strip_prefix("custom.") {
                return Some(WindowCommand::CustomAction(custom_id.to_string()));
            }
            if let Some(command) = builtin_command(action_id) {
                return Some(command);
            }
        }
    }

    for action in &config.custom_actions {
        let Some(shortcut) = &action.shortcut else {
            continue;
        };
        let Some(binding) = parse_key_binding(shortcut) else {
            continue;
        };
        if binding.matches(key, modifiers) {
            return Some(WindowCommand::CustomAction(action.id.clone()));
        }
    }

    None
}

fn builtin_command(action_id: &str) -> Option<WindowCommand> {
    match action_id {
        "copy_selection" => Some(WindowCommand::CopySelection),
        "cut_selection" => Some(WindowCommand::CutSelection),
        "paste_clipboard" => Some(WindowCommand::PasteClipboard),
        "copy_path" => Some(WindowCommand::CopyPathText),
        "new_folder" => Some(WindowCommand::NewFolder),
        "new_text_document" => Some(WindowCommand::NewTextDocument),
        "rename" => Some(WindowCommand::RenameSelection),
        "trash" => Some(WindowCommand::TrashSelection),
        "search" => Some(WindowCommand::OpenSearch),
        "filter_tags" => Some(WindowCommand::ToggleFilter),
        "focus_path" => Some(WindowCommand::FocusPath),
        "refresh" => Some(WindowCommand::Refresh),
        "show_hidden" => Some(WindowCommand::ToggleHidden),
        "toggle_sidebar" => Some(WindowCommand::ToggleSidebar),
        "toggle_preview" => Some(WindowCommand::TogglePreview),
        "new_tab" => Some(WindowCommand::NewTab),
        "close_tab" => Some(WindowCommand::CloseTab),
        "toggle_split" => Some(WindowCommand::ToggleSplit),
        "previous_tab" => Some(WindowCommand::PreviousTab),
        "next_tab" => Some(WindowCommand::NextTab),
        "back" => Some(WindowCommand::GoBack),
        "up" => Some(WindowCommand::GoUp),
        "cycle_pane" => Some(WindowCommand::CyclePane),
        "escape" => Some(WindowCommand::Escape),
        _ => None,
    }
}

fn parse_key_binding(shortcut: &str) -> Option<KeyBinding> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut key = None;

    for part in shortcut.split('+') {
        let part = part.trim();
        let normalized = part.to_ascii_lowercase().replace('_', "");
        match normalized.as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            "esc" | "escape" => key = Some(BindingKey::Named("escape")),
            "delete" | "del" => key = Some(BindingKey::Named("delete")),
            "left" => key = Some(BindingKey::Named("left")),
            "right" => key = Some(BindingKey::Named("right")),
            "up" => key = Some(BindingKey::Named("up")),
            "down" => key = Some(BindingKey::Named("down")),
            "pageup" => key = Some(BindingKey::Named("pageup")),
            "pagedown" => key = Some(BindingKey::Named("pagedown")),
            "backslash" => key = Some(BindingKey::Char('\\')),
            f if f.len() > 1 && f.starts_with('f') => match f {
                "f1" => key = Some(BindingKey::Named("f1")),
                "f2" => key = Some(BindingKey::Named("f2")),
                "f3" => key = Some(BindingKey::Named("f3")),
                "f4" => key = Some(BindingKey::Named("f4")),
                "f5" => key = Some(BindingKey::Named("f5")),
                "f6" => key = Some(BindingKey::Named("f6")),
                "f7" => key = Some(BindingKey::Named("f7")),
                "f8" => key = Some(BindingKey::Named("f8")),
                "f9" => key = Some(BindingKey::Named("f9")),
                "f10" => key = Some(BindingKey::Named("f10")),
                "f11" => key = Some(BindingKey::Named("f11")),
                "f12" => key = Some(BindingKey::Named("f12")),
                _ => return None,
            },
            value => {
                let mut chars = value.chars();
                let ch = chars.next()?;
                if chars.next().is_none() {
                    key = Some(BindingKey::Char(ch.to_ascii_lowercase()));
                } else {
                    return None;
                }
            }
        }
    }

    Some(KeyBinding {
        ctrl,
        shift,
        alt,
        key: key?,
    })
}

impl KeyBinding {
    fn matches(self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        let modifiers = relevant_modifiers(modifiers);
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        self.ctrl == ctrl && self.shift == shift && self.alt == alt && self.key.matches(key)
    }
}

impl BindingKey {
    fn matches(self, key: gdk::Key) -> bool {
        match self {
            Self::Char(ch) => key_char(key) == Some(ch),
            Self::Named(name) => matches!(
                (name, key),
                ("escape", gdk::Key::Escape)
                    | ("delete", gdk::Key::Delete)
                    | ("left", gdk::Key::Left)
                    | ("right", gdk::Key::Right)
                    | ("up", gdk::Key::Up)
                    | ("down", gdk::Key::Down)
                    | ("pageup", gdk::Key::Page_Up)
                    | ("pagedown", gdk::Key::Page_Down)
                    | ("f1", gdk::Key::F1)
                    | ("f2", gdk::Key::F2)
                    | ("f3", gdk::Key::F3)
                    | ("f4", gdk::Key::F4)
                    | ("f5", gdk::Key::F5)
                    | ("f6", gdk::Key::F6)
                    | ("f7", gdk::Key::F7)
                    | ("f8", gdk::Key::F8)
                    | ("f9", gdk::Key::F9)
                    | ("f10", gdk::Key::F10)
                    | ("f11", gdk::Key::F11)
                    | ("f12", gdk::Key::F12)
            ),
        }
    }
}

fn action_availability(
    view: &PaneView,
    selected_count: usize,
    has_file_clipboard: bool,
) -> ActionAvailability {
    let read_only_mutation_view = matches!(
        view,
        PaneView::Trash | PaneView::SystemDrives | PaneView::Recent | PaneView::Search(_)
    );
    let can_paste_files =
        has_file_clipboard && matches!(view, PaneView::Directory(_) | PaneView::DownloadsTriage(_));

    ActionAvailability {
        can_copy_files: selected_count > 0,
        can_cut_files: selected_count > 0 && !read_only_mutation_view,
        can_paste_files,
        can_copy_paths: selected_count > 0 || !matches!(view, PaneView::SystemDrives),
        can_rename: selected_count > 0 && !read_only_mutation_view,
        can_trash: selected_count > 0 && !read_only_mutation_view,
        can_new_folder: matches!(view, PaneView::Directory(_) | PaneView::DownloadsTriage(_)),
    }
}

/// True when the user is holding Ctrl during the current drag.
fn ctrl_held() -> bool {
    gdk::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|s| s.pointer())
        .map(|p| p.modifier_state().contains(gdk::ModifierType::CONTROL_MASK))
        .unwrap_or(false)
}

/// Parse a newline-separated URI list from a drop value into local paths.
fn parse_dropped_uris(value: &glib::Value) -> Vec<PathBuf> {
    value
        .get::<String>()
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let uri = line.trim_end_matches('\r').trim();
            if uri.starts_with("file://") || uri.starts_with("file:///") {
                gio::File::for_uri(uri).path()
            } else {
                None
            }
        })
        .collect()
}

// ── Search ───────────────────────────────────────────────────────────────────

const MAX_SEARCH_DEPTH: u32 = 8;
const MAX_SEARCH_RESULTS: usize = 2_000;

struct SearchEntry {
    path: PathBuf,
    fname: String,
    is_dir: bool,
    size: u64,
    modified_secs: i64,
}

fn search_directory_blocking(
    dir: &Path,
    query: &SearchQuery,
    show_hidden: bool,
    depth: u32,
    results: &mut Vec<FileItem>,
) {
    if depth > MAX_SEARCH_DEPTH || results.len() >= MAX_SEARCH_RESULTS {
        return;
    }
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Collect all entries with metadata up-front so we can do two passes:
    // files first, then subdirectories. Without this, a large subdirectory that
    // appears early in inode order (e.g. target/, node_modules/) can exhaust
    // MAX_SEARCH_RESULTS before files in the same parent directory are reached.
    let mut files: Vec<SearchEntry> = Vec::new();
    let mut subdirs: Vec<SearchEntry> = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !show_hidden && fname.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let modified_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let e = SearchEntry {
            path,
            fname,
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified_secs,
        };
        if e.is_dir {
            subdirs.push(e);
        } else {
            files.push(e);
        }
    }

    // Pass 1: files in this directory
    for e in &files {
        if results.len() >= MAX_SEARCH_RESULTS {
            return;
        }
        if let Some(item) = match_entry(e, query, now_secs) {
            results.push(item);
        }
    }

    // Pass 2: subdirectories — add matching ones then recurse
    for e in &subdirs {
        if results.len() >= MAX_SEARCH_RESULTS {
            return;
        }
        if let Some(item) = match_entry(e, query, now_secs) {
            results.push(item);
        }
        if query.recursive {
            search_directory_blocking(&e.path, query, show_hidden, depth + 1, results);
        }
    }
}

fn match_entry(e: &SearchEntry, query: &SearchQuery, now_secs: i64) -> Option<FileItem> {
    // Name filter
    if !query.name.is_empty() && !e.fname.to_lowercase().contains(&query.name.to_lowercase()) {
        return None;
    }

    // Kind filter
    let gio_type = if e.is_dir {
        gio::FileType::Directory
    } else {
        gio::FileType::Regular
    };
    let kind = search_entry_kind(e, gio_type);

    let kind_ok = match &query.kind {
        SearchKindFilter::All => true,
        SearchKindFilter::Folders => e.is_dir,
        SearchKindFilter::Images => matches!(kind, FileKind::Image),
        SearchKindFilter::Videos => matches!(kind, FileKind::Video),
        SearchKindFilter::Text => matches!(kind, FileKind::Text),
        SearchKindFilter::Archives => matches!(kind, FileKind::Archive),
        SearchKindFilter::Code => matches!(kind, FileKind::ConfigCode),
    };
    if !kind_ok {
        return None;
    }

    // Size filter (skip for directories)
    let size_ok = e.is_dir
        || match &query.size {
            SearchSizeFilter::Any => true,
            SearchSizeFilter::Small => e.size < 1_000_000,
            SearchSizeFilter::Medium => e.size >= 1_000_000 && e.size < 50_000_000,
            SearchSizeFilter::Large => e.size >= 50_000_000,
        };
    if !size_ok {
        return None;
    }

    // Age filter
    let age_ok = match &query.age {
        SearchAgeFilter::Any => true,
        SearchAgeFilter::Today => now_secs - e.modified_secs < 86_400,
        SearchAgeFilter::ThisWeek => now_secs - e.modified_secs < 7 * 86_400,
        SearchAgeFilter::ThisMonth => now_secs - e.modified_secs < 30 * 86_400,
        SearchAgeFilter::Older => now_secs - e.modified_secs >= 30 * 86_400,
    };
    if !age_ok {
        return None;
    }

    Some(FileItem {
        name: e.fname.clone(),
        path: e.path.clone(),
        is_dir: e.is_dir,
        kind,
        size_bytes: if e.is_dir { None } else { Some(e.size) },
        modified_unix: Some(e.modified_secs),
        tags: Vec::new(),
        original_path: None,
    })
}

fn search_entry_kind(e: &SearchEntry, gio_type: gio::FileType) -> FileKind {
    let guessed_content_type = gio::content_type_guess(Some(e.fname.as_str()), &[])
        .0
        .to_string();
    let mut kind = FileKind::from_path(&e.path, gio_type, Some(&guessed_content_type));

    if kind == FileKind::Unknown && gio_type != gio::FileType::Directory {
        let file = gio::File::for_path(&e.path);
        if let Ok(info) = file.query_info(
            "standard::content-type",
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        ) {
            kind = FileKind::from_path(&e.path, gio_type, info.content_type().as_deref());
        }
    }

    kind
}

// ─────────────────────────────────────────────────────────────────────────────

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

/// Return a path in `dir` that does not yet exist by appending " (2)", " (3)"…
fn free_name_in(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) => (&name[..dot], Some(&name[dot..])),
        None => (name, None),
    };
    let mut n = 2u32;
    loop {
        let candidate = match ext {
            Some(e) => format!("{stem} ({n}){e}"),
            None => format!("{stem} ({n})"),
        };
        let path = dir.join(&candidate);
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

struct BodyLayout {
    root: Paned,
    preview_host: GtkBox,
}

struct CenterLayout {
    root: GtkBox,
}

fn build_body(
    sidebar: &Sidebar,
    tab_strip: &TabStrip,
    primary_pane: &PaneWidgets,
    secondary_pane: &PaneWidgets,
    preview: &PreviewPane,
) -> BodyLayout {
    let outer = Paned::new(Orientation::Horizontal);
    outer.set_wide_handle(false);
    outer.set_start_child(Some(&sidebar.root));
    outer.set_position(220);
    outer.set_resize_start_child(false);
    outer.set_shrink_start_child(false);
    outer.set_resize_end_child(true);
    outer.set_shrink_end_child(true);

    let center_and_preview = Paned::new(Orientation::Horizontal);
    center_and_preview.set_wide_handle(false);
    center_and_preview.set_resize_start_child(true);
    center_and_preview.set_shrink_start_child(true);
    center_and_preview.set_resize_end_child(false);
    center_and_preview.set_shrink_end_child(false);

    let center = build_center(tab_strip, primary_pane, secondary_pane);
    let preview_host = GtkBox::new(Orientation::Vertical, 0);
    preview_host.add_css_class("preview-host");
    preview_host.append(&preview.root);
    center_and_preview.set_start_child(Some(&center.root));
    center_and_preview.set_end_child(Some(&preview_host));
    center_and_preview.set_position(820);

    outer.set_end_child(Some(&center_and_preview));
    BodyLayout {
        root: outer,
        preview_host,
    }
}

fn build_center(
    tab_strip: &TabStrip,
    primary_pane: &PaneWidgets,
    secondary_pane: &PaneWidgets,
) -> CenterLayout {
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&tab_strip.root);

    let panes = Paned::new(Orientation::Horizontal);
    panes.add_css_class("split-panes");
    panes.set_wide_handle(false);
    panes.set_resize_start_child(true);
    panes.set_shrink_start_child(true);
    panes.set_resize_end_child(true);
    panes.set_shrink_end_child(true);
    panes.set_start_child(Some(&primary_pane.root));
    panes.set_end_child(Some(&secondary_pane.root));
    panes.set_position(540);
    panes.set_vexpand(true);
    panes.set_hexpand(true);

    vbox.append(&panes);
    CenterLayout { root: vbox }
}

fn connect_directory_button(controller: &Rc<BrowserController>, button: &Button, path: PathBuf) {
    let controller = Rc::clone(controller);
    button.connect_clicked(move |_| {
        controller.navigate_to(controller.active_slot(), path.clone(), true)
    });
}

fn custom_action_paths_for_context(controller: &BrowserController, slot: PaneSlot) -> Vec<PathBuf> {
    let selected = controller.selected_paths_for(slot);
    if selected.is_empty() {
        Vec::new()
    } else {
        selected
    }
}

fn expand_custom_action_argv(
    action: &CustomActionConfig,
    paths: &[PathBuf],
    cwd: &Path,
) -> Option<Vec<OsString>> {
    let mut argv = Vec::new();
    for arg in &action.argv {
        match arg.as_str() {
            "{paths}" => {
                argv.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
            }
            "{path}" => {
                if let Some(path) = paths.first() {
                    argv.push(path.as_os_str().to_os_string());
                }
            }
            "{cwd}" => {
                argv.push(cwd.as_os_str().to_os_string());
            }
            _ => {
                let replaced = arg.replace("{cwd}", &cwd.to_string_lossy());
                let replaced = if let Some(path) = paths.first() {
                    replaced.replace("{path}", &path.to_string_lossy())
                } else {
                    replaced
                };
                argv.push(OsString::from(replaced));
            }
        }
    }
    (!argv.is_empty()).then_some(argv)
}

fn append_menu_button<F: Fn() + 'static>(
    menu_box: &GtkBox,
    label: &str,
    icon_name: Option<&str>,
    dangerous: bool,
    action: F,
) {
    let button = Button::new();
    button.add_css_class("context-menu-button");
    if dangerous {
        button.add_css_class("context-menu-danger");
    }
    button.set_halign(Align::Fill);

    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.set_halign(Align::Start);
    row.set_valign(gtk::Align::Center);

    if let Some(icon) = icon_name {
        let img = Image::from_icon_name(icon);
        img.add_css_class("context-menu-icon");
        img.set_pixel_size(14);
        row.append(&img);
    }

    let lbl = Label::new(Some(label));
    lbl.set_halign(Align::Start);
    row.append(&lbl);

    button.set_child(Some(&row));
    button.connect_clicked(move |_| action());
    menu_box.append(&button);
}

fn append_menu_sep(menu_box: &GtkBox) {
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("context-menu-sep");
    menu_box.append(&sep);
}

fn sort_items(items: &mut [FileItem]) {
    items.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

struct MountedVolumeListing {
    items: Vec<FileItem>,
    skipped_non_local: usize,
}

struct RecentFolderListing {
    items: Vec<FileItem>,
    skipped_missing: usize,
}

fn collect_mounted_volume_items() -> MountedVolumeListing {
    let monitor = gio::VolumeMonitor::get();
    let mut items = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut skipped_non_local = 0usize;

    for mount in monitor.mounts() {
        let root = mount.root();
        let Some(path) = root.path() else {
            skipped_non_local += 1;
            continue;
        };

        if !seen_paths.insert(path.clone()) {
            continue;
        }

        items.push(FileItem {
            name: mount.name().to_string(),
            path,
            kind: FileKind::Folder,
            is_dir: true,
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            original_path: None,
        });
    }

    sort_items(&mut items);

    MountedVolumeListing {
        items,
        skipped_non_local,
    }
}

fn collect_recent_folder_items(
    metadata: &mut MetadataStore,
) -> Result<RecentFolderListing, String> {
    let recent_paths = metadata.list_recent_locations(50)?;
    let mut items = Vec::with_capacity(recent_paths.len());
    let mut stale_paths = Vec::new();

    for path in recent_paths {
        if !path.is_dir() {
            stale_paths.push(path);
            continue;
        }

        items.push(FileItem {
            name: tab_title_for_path(&path),
            path,
            kind: FileKind::Folder,
            is_dir: true,
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            original_path: None,
        });
    }

    let skipped_missing = stale_paths.len();
    if !stale_paths.is_empty() {
        metadata.remove_recent_locations(&stale_paths)?;
    }

    Ok(RecentFolderListing {
        items,
        skipped_missing,
    })
}

fn tab_title_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if path == Path::new("/") {
                "/".to_string()
            } else {
                path.display().to_string()
            }
        })
}

fn tab_title_for_view(view: &PaneView, path: &Path) -> String {
    match view {
        PaneView::Directory(_) => tab_title_for_path(path),
        PaneView::Tag(tag) => format!("#{}", tag.name),
        PaneView::DownloadsTriage(filter) => format!("Triage {}", filter.label()),
        PaneView::SystemDrives => "Drives".to_string(),
        PaneView::Recent => "Recent".to_string(),
        PaneView::Trash => "Trash".to_string(),
        PaneView::Search(q) => {
            if q.name.is_empty() {
                "Search".to_string()
            } else {
                format!("Search \u{201c}{}\u{201d}", q.name)
            }
        }
    }
}

fn view_display_label(view: &PaneView, home: &Path) -> String {
    match view {
        PaneView::Directory(path) => format_path(path, home),
        PaneView::Tag(tag) => format!("Tag / #{}", tag.name),
        PaneView::DownloadsTriage(filter) => format!("Downloads Triage / {}", filter.label()),
        PaneView::SystemDrives => "System Drives".to_string(),
        PaneView::Recent => "Recent".to_string(),
        PaneView::Trash => "Trash".to_string(),
        PaneView::Search(q) => {
            let scope = format_path(&q.scope_dir, home);
            if q.name.is_empty() {
                format!("Search in {scope}")
            } else {
                format!("Search \u{201c}{}\u{201d} in {scope}", q.name)
            }
        }
    }
}

fn format_path(path: &Path, home: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(home) {
        if relative.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", relative.display())
        }
    } else {
        path.display().to_string()
    }
}

fn next_new_folder_path(current_dir: &Path) -> PathBuf {
    let mut attempt = 1;
    loop {
        let name = if attempt == 1 {
            "New Folder".to_string()
        } else {
            format!("New Folder {attempt}")
        };
        let candidate = current_dir.join(name);
        if !gio::File::for_path(&candidate).query_exists(None::<&gio::Cancellable>) {
            return candidate;
        }
        attempt += 1;
    }
}

fn next_new_text_document_path(current_dir: &Path) -> PathBuf {
    let mut attempt = 1;
    loop {
        let name = if attempt == 1 {
            "Untitled".to_string()
        } else {
            format!("Untitled {attempt}")
        };
        let candidate = current_dir.join(name);
        if !gio::File::for_path(&candidate).query_exists(None::<&gio::Cancellable>) {
            return candidate;
        }
        attempt += 1;
    }
}

fn suggested_project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format_path(path, &glib::home_dir()))
}

fn resolve_launch(
    launch: &crate::launch::LaunchConfig,
    places: &Places,
    metadata: &crate::metadata::MetadataStore,
) -> LaunchResolution {
    if let Some((left, right)) = &launch.split {
        let (left_path, left_notice) = validate_launch_directory(left, places, "Left split path");
        let (right_path, right_notice) =
            validate_launch_directory(right, places, "Right split path");
        return LaunchResolution {
            primary_dir: left_path.clone(),
            primary_view: PaneView::Directory(left_path),
            secondary_dir: right_path,
            split_enabled: true,
            notice: combine_launch_notices(left_notice, right_notice),
        };
    }

    if let Some(path) = &launch.path {
        let (resolved_path, notice) = validate_launch_directory(path, places, "Launch path");
        return LaunchResolution {
            primary_dir: resolved_path.clone(),
            primary_view: PaneView::Directory(resolved_path),
            secondary_dir: places.home.clone(),
            split_enabled: false,
            notice,
        };
    }

    if launch.downloads {
        let downloads_path = &places.downloads;
        let notice = if is_launchable_directory(downloads_path) {
            None
        } else {
            Some("Downloads folder is unavailable. Opened Home instead.".to_string())
        };
        let primary_dir = if notice.is_some() {
            places.home.clone()
        } else {
            downloads_path.clone()
        };
        let primary_view = if notice.is_some() {
            PaneView::Directory(primary_dir.clone())
        } else {
            PaneView::DownloadsTriage(DownloadsTriageFilter::All)
        };
        return LaunchResolution {
            primary_dir,
            primary_view,
            secondary_dir: places.home.clone(),
            split_enabled: false,
            notice,
        };
    }

    if let Some(project_name) = &launch.project {
        let project = metadata.list_projects().ok().and_then(|projects| {
            projects
                .into_iter()
                .find(|project| project.name.eq_ignore_ascii_case(project_name))
        });
        return match project {
            Some(project) if is_launchable_directory(&project.root_path) => LaunchResolution {
                primary_dir: project.root_path.clone(),
                primary_view: PaneView::Directory(project.root_path),
                secondary_dir: places.home.clone(),
                split_enabled: false,
                notice: None,
            },
            Some(project) => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::Directory(places.home.clone()),
                secondary_dir: places.home.clone(),
                split_enabled: false,
                notice: Some(format!(
                    "Project '{}' points to a missing folder. Opened Home instead.",
                    project.name
                )),
            },
            None => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::Directory(places.home.clone()),
                secondary_dir: places.home.clone(),
                split_enabled: false,
                notice: Some(format!(
                    "Project '{}' was not found. Opened Home instead.",
                    project_name
                )),
            },
        };
    }

    LaunchResolution {
        primary_dir: places.home.clone(),
        primary_view: PaneView::Directory(places.home.clone()),
        secondary_dir: places.home.clone(),
        split_enabled: false,
        notice: None,
    }
}

fn validate_launch_directory(
    candidate: &Path,
    places: &Places,
    label: &str,
) -> (PathBuf, Option<String>) {
    if is_launchable_directory(candidate) {
        (candidate.to_path_buf(), None)
    } else {
        (
            places.home.clone(),
            Some(format!(
                "{label} '{}' is not a readable folder. Opened Home instead.",
                candidate.display()
            )),
        )
    }
}

fn combine_launch_notices(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(format!("{left} {right}")),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn is_launchable_directory(path: &Path) -> bool {
    gio::File::for_path(path)
        .query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>)
        == gio::FileType::Directory
}

fn next_copy_path(destination: &Path) -> PathBuf {
    let stem = destination
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Copy");
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let parent = destination.parent().unwrap_or_else(|| Path::new("/"));

    let mut attempt = 1;
    loop {
        let suffix = if attempt == 1 {
            " copy".to_string()
        } else {
            format!(" copy {attempt}")
        };
        let candidate_name = if extension.is_empty() {
            format!("{stem}{suffix}")
        } else {
            format!("{stem}{suffix}.{extension}")
        };
        let candidate = parent.join(candidate_name);
        if !gio::File::for_path(&candidate).query_exists(None::<&gio::Cancellable>) {
            return candidate;
        }
        attempt += 1;
    }
}

fn copy_path_recursively(
    source: &gio::File,
    destination: &gio::File,
    overwrite: bool,
) -> Result<(), glib::Error> {
    let source_type =
        source.query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>);
    if source_type != gio::FileType::Directory {
        return source.copy(
            destination,
            if overwrite {
                gio::FileCopyFlags::OVERWRITE
            } else {
                gio::FileCopyFlags::NONE
            },
            None::<&gio::Cancellable>,
            None::<&mut dyn FnMut(i64, i64)>,
        );
    }

    if destination.query_exists(None::<&gio::Cancellable>) {
        let destination_type =
            destination.query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>);
        if destination_type != gio::FileType::Directory {
            if overwrite {
                destination.delete(None::<&gio::Cancellable>)?;
                destination.make_directory_with_parents(None::<&gio::Cancellable>)?;
            } else {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "Destination already exists.",
                ));
            }
        }
    } else {
        destination.make_directory_with_parents(None::<&gio::Cancellable>)?;
    }

    let enumerator = source.enumerate_children(
        DIRECTORY_ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    )?;

    while let Some(info) = enumerator.next_file(None::<&gio::Cancellable>)? {
        let child_source = source.child(&info.name());
        let child_destination = destination.child(&info.name());
        copy_path_recursively(&child_source, &child_destination, overwrite)?;
    }

    Ok(())
}

fn filter_triage_items(items: Vec<FileItem>, filter: DownloadsTriageFilter) -> Vec<FileItem> {
    items
        .into_iter()
        .filter(|item| matches_triage_filter(item, filter))
        .collect()
}

fn matches_triage_filter(item: &FileItem, filter: DownloadsTriageFilter) -> bool {
    match filter {
        DownloadsTriageFilter::All => true,
        DownloadsTriageFilter::Today => item
            .modified_unix
            .and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok())
            .and_then(|value| value.format("%Y-%m-%d").ok())
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| value.format("%Y-%m-%d").ok()),
            )
            .map(|(left, right)| left.as_str() == right.as_str())
            .unwrap_or(false),
        DownloadsTriageFilter::ThisWeek => item
            .modified_unix
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| Some(value.to_unix())),
            )
            .map(|(modified, now)| now.saturating_sub(modified) <= 7 * 24 * 60 * 60)
            .unwrap_or(false),
        DownloadsTriageFilter::ThisMonth => item
            .modified_unix
            .and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok())
            .and_then(|value| value.format("%Y-%m").ok())
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| value.format("%Y-%m").ok()),
            )
            .map(|(left, right)| left.as_str() == right.as_str())
            .unwrap_or(false),
        DownloadsTriageFilter::OlderThanOneMonth => item
            .modified_unix
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| Some(value.to_unix())),
            )
            .map(|(modified, now)| now.saturating_sub(modified) > 30 * 24 * 60 * 60)
            .unwrap_or(false),
        DownloadsTriageFilter::Images => item.kind == crate::ui::file_grid::FileKind::Image,
        DownloadsTriageFilter::Videos => item.kind == crate::ui::file_grid::FileKind::Video,
        DownloadsTriageFilter::Archives => item.kind == crate::ui::file_grid::FileKind::Archive,
        DownloadsTriageFilter::Documents => {
            matches!(
                item.kind,
                crate::ui::file_grid::FileKind::Document
                    | crate::ui::file_grid::FileKind::Text
                    | crate::ui::file_grid::FileKind::ConfigCode
            )
        }
        DownloadsTriageFilter::LargeFiles => {
            item.size_bytes.unwrap_or(0) >= TRIAGE_LARGE_FILE_BYTES
        }
    }
}

fn detect_terminal_command() -> Option<Vec<OsString>> {
    if let Ok(value) = std::env::var(TERMINAL_ENV_VAR) {
        let parts = value
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .map(OsString::from)
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Some(parts);
        }
    }

    for candidate in [
        "kitty",
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "alacritty",
        "wezterm",
    ] {
        if command_exists(candidate) {
            return Some(vec![OsString::from(candidate)]);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::LaunchConfig;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lattice-{label}-{unique}"))
    }

    fn test_places() -> Places {
        let root = temp_test_dir("places");
        let home = root.join("home");
        let downloads = root.join("downloads");
        let documents = root.join("documents");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&downloads).unwrap();
        fs::create_dir_all(&documents).unwrap();
        Places {
            home,
            downloads,
            documents,
        }
    }

    fn test_tag() -> TagRecord {
        TagRecord {
            id: 7,
            name: "Focus".to_string(),
            color: None,
        }
    }

    #[test]
    fn clipboard_cut_clears_moved_sources_and_keeps_failures() {
        let clipboard = FileClipboardState::new(
            vec![
                PathBuf::from("/tmp/one"),
                PathBuf::from("/tmp/two"),
                PathBuf::from("/tmp/three"),
            ],
            ClipboardMode::Cut,
        )
        .unwrap();

        let updated = clipboard
            .after_completed_paste(&[PathBuf::from("/tmp/one"), PathBuf::from("/tmp/three")])
            .unwrap();

        assert_eq!(updated.mode, ClipboardMode::Cut);
        assert_eq!(updated.paths, vec![PathBuf::from("/tmp/two")]);
    }

    #[test]
    fn clipboard_copy_survives_paste() {
        let clipboard =
            FileClipboardState::new(vec![PathBuf::from("/tmp/alpha")], ClipboardMode::Copy)
                .unwrap();

        let updated = clipboard
            .after_completed_paste(&[PathBuf::from("/tmp/alpha")])
            .unwrap();

        assert_eq!(updated, clipboard);
    }

    #[test]
    fn action_availability_blocks_paste_in_virtual_views() {
        let tag = test_tag();

        let search_actions = action_availability(
            &PaneView::Search(SearchQuery::new(PathBuf::from("/tmp"))),
            1,
            true,
        );
        assert!(!search_actions.can_paste_files);
        assert!(!search_actions.can_trash);
        assert!(!search_actions.can_new_folder);

        let recent_actions = action_availability(&PaneView::Recent, 1, true);
        assert!(!recent_actions.can_paste_files);
        assert!(!recent_actions.can_cut_files);
        assert!(!recent_actions.can_new_folder);

        let tag_actions = action_availability(&PaneView::Tag(tag), 1, true);
        assert!(!tag_actions.can_paste_files);
        assert!(tag_actions.can_trash);
        assert!(!tag_actions.can_new_folder);

        let triage_actions = action_availability(
            &PaneView::DownloadsTriage(DownloadsTriageFilter::All),
            0,
            true,
        );
        assert!(triage_actions.can_new_folder);
        assert!(triage_actions.can_paste_files);
    }

    #[test]
    fn next_new_text_document_path_skips_existing_extensionless_names() {
        let root = temp_test_dir("new-text-document");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Untitled"), "").unwrap();
        fs::write(root.join("Untitled 2"), "").unwrap();

        assert_eq!(next_new_text_document_path(&root), root.join("Untitled 3"));
    }

    #[test]
    fn window_shortcuts_dispatch_standard_commands() {
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let ctrl_shift = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;

        assert_eq!(
            window_command_from_key(gdk::Key::c, ctrl),
            Some(WindowCommand::CopySelection)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::x, ctrl),
            Some(WindowCommand::CutSelection)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::v, ctrl),
            Some(WindowCommand::PasteClipboard)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::c, ctrl_shift),
            Some(WindowCommand::CopyPathText)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::n, ctrl),
            Some(WindowCommand::NewFolder)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::n, ctrl_shift),
            Some(WindowCommand::NewTextDocument)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::b, ctrl),
            Some(WindowCommand::ToggleSidebar)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::t, ctrl),
            Some(WindowCommand::NewTab)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::w, ctrl),
            Some(WindowCommand::CloseTab)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::Page_Up, ctrl),
            Some(WindowCommand::PreviousTab)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::Page_Down, ctrl),
            Some(WindowCommand::NextTab)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::F6, gdk::ModifierType::empty()),
            Some(WindowCommand::CyclePane)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::Delete, gdk::ModifierType::empty()),
            Some(WindowCommand::TrashSelection)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::F2, gdk::ModifierType::empty()),
            Some(WindowCommand::RenameSelection)
        );
    }

    #[test]
    fn configured_shortcuts_override_builtin_and_dispatch_custom_actions() {
        let mut config = AppConfig::default();
        config
            .shortcuts
            .insert("new_folder".to_string(), "Ctrl+Alt+N".to_string());
        config
            .shortcuts
            .insert("custom.open_in_gimp".to_string(), "Ctrl+Alt+G".to_string());

        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let ctrl_alt = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK;

        assert_eq!(
            configured_window_command_from_key(&config, gdk::Key::n, ctrl),
            None
        );
        assert_eq!(
            configured_window_command_from_key(&config, gdk::Key::n, ctrl_alt),
            Some(WindowCommand::NewFolder)
        );
        assert_eq!(
            configured_window_command_from_key(&config, gdk::Key::g, ctrl_alt),
            Some(WindowCommand::CustomAction("open_in_gimp".to_string()))
        );
    }

    #[test]
    fn resolve_launch_falls_back_for_invalid_path() {
        let places = test_places();
        let metadata = MetadataStore::open_in_memory().unwrap();
        let invalid = places.home.join("missing-folder");
        let launch = LaunchConfig {
            path: Some(invalid.clone()),
            ..LaunchConfig::default()
        };

        let resolution = resolve_launch(&launch, &places, &metadata);
        assert_eq!(resolution.primary_dir, places.home);
        assert!(matches!(resolution.primary_view, PaneView::Directory(_)));
        assert!(resolution.notice.unwrap().contains("Opened Home instead."));
    }

    #[test]
    fn resolve_launch_uses_existing_project_case_insensitively() {
        let places = test_places();
        let mut metadata = MetadataStore::open_in_memory().unwrap();
        let project_root = places.documents.join("workspace");
        fs::create_dir_all(&project_root).unwrap();
        metadata.create_project("Alpha", &project_root).unwrap();

        let launch = LaunchConfig {
            project: Some("alpha".to_string()),
            ..LaunchConfig::default()
        };

        let resolution = resolve_launch(&launch, &places, &metadata);
        assert_eq!(resolution.primary_dir, project_root);
        assert!(resolution.notice.is_none());
    }

    #[test]
    fn resolve_launch_split_preserves_valid_side_and_falls_back_invalid_side() {
        let places = test_places();
        let metadata = MetadataStore::open_in_memory().unwrap();
        let valid_left = places.downloads.clone();
        let invalid_right = places.home.join("missing-right");
        let launch = LaunchConfig {
            split: Some((valid_left.clone(), invalid_right)),
            ..LaunchConfig::default()
        };

        let resolution = resolve_launch(&launch, &places, &metadata);
        assert!(resolution.split_enabled);
        assert_eq!(resolution.primary_dir, valid_left);
        assert_eq!(resolution.secondary_dir, places.home);
        assert!(resolution.notice.unwrap().contains("Right split path"));
    }
}

fn command_exists(command: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path_var).any(|directory| directory.join(command).is_file())
}

fn friendly_error(error: &glib::Error) -> (&'static str, String) {
    match error.kind::<gio::IOErrorEnum>() {
        Some(gio::IOErrorEnum::PermissionDenied) => (
            "Permission Denied",
            "Lattice does not have permission to complete this operation.".to_string(),
        ),
        Some(gio::IOErrorEnum::NotFound) => (
            "File Not Found",
            "The selected file or folder no longer exists.".to_string(),
        ),
        Some(gio::IOErrorEnum::Exists) => (
            "Name Conflict",
            "An item with that name already exists in this location.".to_string(),
        ),
        _ => ("Operation Failed", error.message().to_string()),
    }
}

fn friendly_error_detail(error: &glib::Error) -> String {
    let (title, detail) = friendly_error(error);
    format!("{title}: {detail}")
}

fn format_modified_time(time: Option<glib::DateTime>) -> Option<String> {
    time.and_then(|value| value.format("%Y-%m-%d %H:%M").ok())
        .map(|value| value.to_string())
}

fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0usize;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn preview_type_label(item: &FileItem, mime: &str) -> String {
    let mime_lower = mime.to_ascii_lowercase();
    if let Some(friendly) = mime_to_friendly_type(&mime_lower) {
        return friendly.to_string();
    }
    // Broad fallback for unrecognised audio files classified as Unknown
    if item.kind == FileKind::Unknown && mime_lower.starts_with("audio/") {
        return "Audio".to_string();
    }
    item.kind.label().to_string()
}

fn mime_to_friendly_type(mime: &str) -> Option<&'static str> {
    match mime {
        // Images
        "image/jpeg" | "image/jpg" => Some("JPEG Image"),
        "image/png" => Some("PNG Image"),
        "image/gif" => Some("GIF Image"),
        "image/webp" => Some("WebP Image"),
        "image/bmp" | "image/x-bmp" => Some("Bitmap Image"),
        "image/tiff" | "image/x-tiff" => Some("TIFF Image"),
        "image/svg+xml" => Some("SVG Image"),
        "image/avif" => Some("AVIF Image"),
        "image/heic" | "image/heif" => Some("HEIC Image"),
        "image/vnd.microsoft.icon" | "image/ico" | "image/x-icon" => Some("ICO Image"),
        // Video
        "video/mp4" | "video/x-m4v" => Some("MPEG-4 Video"),
        "video/x-matroska" => Some("Matroska Video"),
        "video/quicktime" => Some("QuickTime Video"),
        "video/x-msvideo" => Some("AVI Video"),
        "video/webm" => Some("WebM Video"),
        "video/mpeg" | "video/x-mpeg" => Some("MPEG Video"),
        "video/ogg" | "video/x-ogm+ogg" => Some("OGG Video"),
        "video/x-ms-wmv" => Some("WMV Video"),
        // Audio
        "audio/mpeg" | "audio/mp3" | "audio/x-mp3" => Some("MP3 Audio"),
        "audio/flac" | "audio/x-flac" => Some("FLAC Audio"),
        "audio/wav" | "audio/x-wav" => Some("WAV Audio"),
        "audio/ogg" | "audio/vorbis" | "audio/x-vorbis+ogg" => Some("OGG Audio"),
        "audio/opus" | "audio/x-opus+ogg" => Some("Opus Audio"),
        "audio/aac" | "audio/x-aac" | "audio/mp4" => Some("AAC Audio"),
        "audio/x-m4a" => Some("M4A Audio"),
        "audio/x-ms-wma" => Some("WMA Audio"),
        // Documents
        "application/pdf" => Some("PDF Document"),
        "application/msword" => Some("Word Document"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            Some("Word Document")
        }
        "application/vnd.oasis.opendocument.text" => Some("ODF Text"),
        "application/vnd.ms-excel" => Some("Excel Spreadsheet"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            Some("Excel Spreadsheet")
        }
        "application/epub+zip" => Some("EPUB E-Book"),
        // Archives
        "application/zip" | "application/x-zip-compressed" => Some("ZIP Archive"),
        "application/x-tar" => Some("TAR Archive"),
        "application/gzip" | "application/x-gzip" => Some("GZip Archive"),
        "application/x-bzip2" => Some("BZip2 Archive"),
        "application/x-7z-compressed" => Some("7-Zip Archive"),
        "application/vnd.rar" | "application/x-rar-compressed" => Some("RAR Archive"),
        "application/java-archive" => Some("JAR Archive"),
        // Text / code (broad enough to be useful)
        "text/plain" => Some("Plain Text"),
        "text/html" | "text/xhtml" | "application/xhtml+xml" => Some("HTML Document"),
        "text/css" => Some("CSS Stylesheet"),
        "text/javascript" | "application/javascript" => Some("JavaScript Source"),
        "application/json" => Some("JSON Data"),
        "application/xml" | "text/xml" => Some("XML Document"),
        "text/x-python" | "application/x-python" => Some("Python Source"),
        "text/x-rust" | "application/x-rust" => Some("Rust Source"),
        "text/x-csrc" | "text/x-c" => Some("C Source"),
        "text/x-c++src" => Some("C++ Source"),
        "text/x-shellscript" | "application/x-shellscript" => Some("Shell Script"),
        "text/markdown" => Some("Markdown Text"),
        _ => None,
    }
}

fn probe_image_dimensions(path: &std::path::Path) -> Option<(i32, i32)> {
    let (_fmt, w, h) = Pixbuf::file_info(path)?;
    (w > 0 && h > 0).then_some((w, h))
}
