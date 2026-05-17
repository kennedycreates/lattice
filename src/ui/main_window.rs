use crate::action_plan::ActionPlan as FileOpPlan;
use crate::config::{AppConfig, CustomActionConfig};
use crate::metadata::{ActivityLogEntry, MetadataStore, PlaceRecord, ProjectRecord, TagRecord};
use crate::ui::{
    activity_log_panel::{ActivityLogAction, ActivityLogPanel},
    project_landing_panel::ProjectLandingPanel,
    bulk_rename, conflict_resolver,
    file_grid::{FileGrid, FileItem, FileKind, ViewMode},
    holding_tray::HoldingTray,
    modal_host::{
        build_modal_actions, build_modal_button, build_modal_prompt, ButtonKind, ModalHost,
    },
    ops_panel::{OpId, OpsPanel},
    plan_queue_panel::{PlanQueuePanel, QueueAction},
    preview_pane::PreviewPane,
    search_panel::{SearchAgeFilter, SearchKindFilter, SearchPanel, SearchQuery, SearchSizeFilter},
    sidebar::{Sidebar, SidebarTarget},
    status_bar::StatusBar,
    tab_strip::TabStrip,
    tag_filter::{TagFilterPanel, TagFilterSpec},
    tag_panel::TagManagerPanel,
    toolbar::Toolbar,
};
use gdk_pixbuf::Pixbuf;
use gio::prelude::*;
use glib::UserDirectory;
use gtk::gdk;
use gtk::prelude::*;
use gtk::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, Entry, FlowBox, HeaderBar, Image,
    Label, ListBox, ListBoxRow, Orientation, Paned, Popover, Revealer, Separator,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

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
    Tertiary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneLayout {
    Single,
    Two,
    Three,
}

impl PaneLayout {
    fn next(self) -> Self {
        match self {
            Self::Single => Self::Two,
            Self::Two => Self::Three,
            Self::Three => Self::Single,
        }
    }

    fn pane_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }

    fn includes(self, slot: PaneSlot) -> bool {
        match slot {
            PaneSlot::Primary => true,
            PaneSlot::Secondary => self.pane_count() >= 2,
            PaneSlot::Tertiary => self.pane_count() >= 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectTransferKind {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayProjectAction {
    Copy,
    Move,
}

impl TrayProjectAction {
    fn transfer_kind(self) -> ProjectTransferKind {
        match self {
            Self::Copy => ProjectTransferKind::Copy,
            Self::Move => ProjectTransferKind::Move,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Copy => "Copy Tray to Project",
            Self::Move => "Move Tray to Project",
        }
    }
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
enum TriageFilter {
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
    Audio,
    Executables,
    Empty,
    Duplicates,
}

impl TriageFilter {
    const ALL: [Self; 14] = [
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
        Self::Audio,
        Self::Executables,
        Self::Empty,
        Self::Duplicates,
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
            Self::Audio => "Audio",
            Self::Executables => "Executables",
            Self::Empty => "Empty Files",
            Self::Duplicates => "Duplicates",
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
    HoldingTray,
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
    ToggleHoldingTray,
    NewTab,
    CloseTab,
    ToggleSplit,
    PreviousTab,
    NextTab,
    GoBack,
    GoForward,
    GoUp,
    CyclePane,
    Escape,
    SetViewIcons,
    SetViewList,
    TogglePlanMode,
    EmptyTrash,
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
    Triage { root: PathBuf, filter: TriageFilter },
    SystemDrives,
    Recent,
    Trash,
    Search(SearchQuery),
    ActivityLog,
    ProjectLanding(i64),
    TagManager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum SortField {
    #[default]
    Name,
    Modified,
    Size,
    Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

#[derive(Clone, Debug)]
struct TabState {
    title: String,
    primary_dir: PathBuf,
    primary_back_history: Vec<PathBuf>,
    primary_forward_history: Vec<PathBuf>,
    primary_view: PaneView,
    secondary_dir: PathBuf,
    secondary_back_history: Vec<PathBuf>,
    secondary_forward_history: Vec<PathBuf>,
    secondary_view: PaneView,
    tertiary_dir: PathBuf,
    tertiary_back_history: Vec<PathBuf>,
    tertiary_forward_history: Vec<PathBuf>,
    tertiary_view: PaneView,
    pane_layout: PaneLayout,
    active_pane: PaneSlot,
    primary_view_mode: ViewMode,
    secondary_view_mode: ViewMode,
    tertiary_view_mode: ViewMode,
    primary_show_hidden: bool,
    secondary_show_hidden: bool,
    tertiary_show_hidden: bool,
    primary_sort_field: SortField,
    primary_sort_direction: SortDirection,
    secondary_sort_field: SortField,
    secondary_sort_direction: SortDirection,
    tertiary_sort_field: SortField,
    tertiary_sort_direction: SortDirection,
}

struct LaunchResolution {
    primary_dir: PathBuf,
    primary_view: PaneView,
    secondary_dir: PathBuf,
    tertiary_dir: PathBuf,
    pane_layout: PaneLayout,
    notice: Option<String>,
}

impl TabState {
    fn new(path: PathBuf) -> Self {
        let primary_view = PaneView::Directory(path.clone());
        let secondary_view = PaneView::Directory(path.clone());
        let tertiary_view = PaneView::Directory(path.clone());
        let title = tab_title_for_view(&primary_view, &path);
        Self {
            title,
            primary_dir: path.clone(),
            primary_back_history: Vec::new(),
            primary_forward_history: Vec::new(),
            primary_view,
            secondary_dir: path.clone(),
            secondary_back_history: Vec::new(),
            secondary_forward_history: Vec::new(),
            secondary_view,
            tertiary_dir: path,
            tertiary_back_history: Vec::new(),
            tertiary_forward_history: Vec::new(),
            tertiary_view,
            pane_layout: PaneLayout::Single,
            active_pane: PaneSlot::Primary,
            primary_view_mode: ViewMode::Icons,
            secondary_view_mode: ViewMode::Icons,
            tertiary_view_mode: ViewMode::Icons,
            primary_show_hidden: false,
            secondary_show_hidden: false,
            tertiary_show_hidden: false,
            primary_sort_field: SortField::Name,
            primary_sort_direction: SortDirection::Ascending,
            secondary_sort_field: SortField::Name,
            secondary_sort_direction: SortDirection::Ascending,
            tertiary_sort_field: SortField::Name,
            tertiary_sort_direction: SortDirection::Ascending,
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
            tertiary_dir,
            pane_layout,
            notice,
        } = resolve_launch(launch, places, metadata);

        let secondary_view = PaneView::Directory(secondary_dir.clone());
        let tertiary_view = PaneView::Directory(tertiary_dir.clone());
        let title = tab_title_for_view(&primary_view, &primary_dir);
        (
            Self {
                title,
                primary_dir,
                primary_back_history: Vec::new(),
                primary_forward_history: Vec::new(),
                primary_view,
                secondary_dir,
                secondary_back_history: Vec::new(),
                secondary_forward_history: Vec::new(),
                secondary_view,
                tertiary_dir,
                tertiary_back_history: Vec::new(),
                tertiary_forward_history: Vec::new(),
                tertiary_view,
                pane_layout,
                active_pane: PaneSlot::Primary,
                primary_view_mode: ViewMode::Icons,
                secondary_view_mode: ViewMode::Icons,
                tertiary_view_mode: ViewMode::Icons,
                primary_show_hidden: false,
                secondary_show_hidden: false,
                tertiary_show_hidden: false,
                primary_sort_field: SortField::Name,
                primary_sort_direction: SortDirection::Ascending,
                secondary_sort_field: SortField::Name,
                secondary_sort_direction: SortDirection::Ascending,
                tertiary_sort_field: SortField::Name,
                tertiary_sort_direction: SortDirection::Ascending,
            },
            notice,
        )
    }
}

#[derive(Clone)]
struct PaneWidgets {
    root: GtkBox,
    path_label: Label,
    filter_toggle_btn: Button,
    hidden_toggle_btn: Button,
    hidden_toggle_icon: gtk::Image,
    sort_btn: Button,
    sort_icon: gtk::Image,
    view_mode_btn: Button,
    view_mode_icon: gtk::Image,
    view_strip: GtkBox,
    view_title: Label,
    triage_filters: Vec<(TriageFilter, Button)>,
    search_panel: SearchPanel,
    search_revealer: Revealer,
    tag_filter: TagFilterPanel,
    tag_filter_revealer: Revealer,
    file_grid: FileGrid,
    activity_log_panel: ActivityLogPanel,
    project_landing_panel: ProjectLandingPanel,
    tag_manager_panel: TagManagerPanel,
}

impl PaneWidgets {
    fn build(slot: PaneSlot) -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("browser-pane");
        match slot {
            PaneSlot::Primary => root.add_css_class("browser-pane-primary"),
            PaneSlot::Secondary => root.add_css_class("browser-pane-secondary"),
            PaneSlot::Tertiary => root.add_css_class("browser-pane-tertiary"),
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

        let filter_toggle_icon = gtk::Image::from_icon_name("object-select-symbolic");
        let filter_toggle_btn = Button::new();
        filter_toggle_btn.set_child(Some(&filter_toggle_icon));
        filter_toggle_btn.add_css_class("pane-view-btn");
        filter_toggle_btn.add_css_class("pane-filter-btn");
        filter_toggle_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(&filter_toggle_btn, "Tag filter (Ctrl+G)");
        header.append(&filter_toggle_btn);

        let hidden_toggle_icon = gtk::Image::from_icon_name("view-reveal-symbolic");
        let hidden_toggle_btn = Button::new();
        hidden_toggle_btn.set_child(Some(&hidden_toggle_icon));
        hidden_toggle_btn.add_css_class("pane-view-btn");
        hidden_toggle_btn.add_css_class("pane-hidden-btn");
        hidden_toggle_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(&hidden_toggle_btn, "Hidden files (Ctrl+H)");
        header.append(&hidden_toggle_btn);

        let sort_icon = gtk::Image::from_icon_name("view-sort-ascending-symbolic");
        let sort_btn = Button::new();
        sort_btn.set_child(Some(&sort_icon));
        sort_btn.add_css_class("pane-view-btn");
        sort_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(&sort_btn, "Sort order");
        header.append(&sort_btn);

        let view_mode_icon = gtk::Image::from_icon_name("view-grid-symbolic");
        let view_mode_btn = Button::new();
        view_mode_btn.set_child(Some(&view_mode_icon));
        view_mode_btn.add_css_class("pane-view-btn");
        view_mode_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(&view_mode_btn, "Toggle icon/list view (Ctrl+1/Ctrl+2)");
        header.append(&view_mode_btn);

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
        let mut triage_filters = Vec::with_capacity(TriageFilter::ALL.len());
        for filter in TriageFilter::ALL {
            let button = Button::with_label(filter.label());
            button.add_css_class("pane-filter-button");
            filter_row.append(&button);
            triage_filters.push((filter, button));
        }
        view_strip.append(&filter_row);

        let search_panel = SearchPanel::build();
        let search_revealer = Revealer::new();
        search_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        search_revealer.set_transition_duration(180);
        search_revealer.set_child(Some(&search_panel.root));
        search_revealer.set_reveal_child(false);

        let tag_filter = TagFilterPanel::build();
        let tag_filter_revealer = Revealer::new();
        tag_filter_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        tag_filter_revealer.set_transition_duration(180);
        tag_filter_revealer.set_child(Some(&tag_filter.root));
        tag_filter_revealer.set_reveal_child(false);
        tag_filter_revealer.set_visible(false);

        let file_grid = FileGrid::build();
        file_grid.root.set_vexpand(true);
        file_grid.root.set_hexpand(true);

        let activity_log_panel = ActivityLogPanel::build();
        let project_landing_panel = ProjectLandingPanel::build();

        let tag_manager_panel = TagManagerPanel::build();

        root.append(&header);
        root.append(&tag_filter_revealer);
        root.append(&view_strip);
        root.append(&search_revealer);
        root.append(&file_grid.root);
        root.append(&activity_log_panel.root);
        root.append(&project_landing_panel.root);
        root.append(&tag_manager_panel.root);

        Self {
            root,
            path_label,
            filter_toggle_btn,
            hidden_toggle_btn,
            hidden_toggle_icon,
            sort_btn,
            sort_icon,
            view_mode_btn,
            view_mode_icon,
            view_strip,
            view_title,
            triage_filters,
            search_panel,
            search_revealer,
            tag_filter,
            tag_filter_revealer,
            file_grid,
            activity_log_panel,
            project_landing_panel,
            tag_manager_panel,
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
        let tertiary_pane = PaneWidgets::build(PaneSlot::Tertiary);
        let preview = PreviewPane::build();
        let holding_tray = HoldingTray::build();
        let plan_queue_panel = PlanQueuePanel::build();
        let ops_panel = OpsPanel::build();
        let status = StatusBar::build();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&toolbar.root);

        let body = build_body(
            &sidebar,
            &tab_strip,
            &primary_pane,
            &secondary_pane,
            &tertiary_pane,
            &preview,
        );
        body.root.set_vexpand(true);
        root.append(&body.root);
        root.append(&holding_tray.root);
        root.append(&plan_queue_panel.root);
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
            tertiary_pane.clone(),
            preview.clone(),
            holding_tray.clone(),
            plan_queue_panel,
            status.clone(),
            ops_panel,
            body.sidebar_revealer.clone(),
            body.preview_revealer.clone(),
            body.split_paned.clone(),
            body.right_paned.clone(),
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
}

impl Places {
    fn discover() -> Self {
        let home = glib::home_dir();
        let downloads = glib::user_special_dir(UserDirectory::Downloads)
            .unwrap_or_else(|| home.join("Downloads"));
        Self { home, downloads }
    }
}

#[derive(Default)]
struct BatchResult {
    success_count: usize,
    successful_paths: Vec<PathBuf>,
    failures: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActionPlan {
    title: String,
    confirm_label: String,
    destructive: bool,
    lines: Vec<String>,
}

impl ActionPlan {
    fn new(
        title: impl Into<String>,
        confirm_label: impl Into<String>,
        destructive: bool,
        lines: Vec<String>,
    ) -> Self {
        Self {
            title: title.into(),
            confirm_label: confirm_label.into(),
            destructive,
            lines,
        }
    }
}

#[derive(Clone)]
struct TrayCompletion {
    action: String,
    clear_successful_paths: bool,
}

struct BrowserController {
    window: ApplicationWindow,
    places: Places,
    metadata: RefCell<MetadataStore>,
    user_places: RefCell<Vec<PlaceRecord>>,
    projects: RefCell<Vec<ProjectRecord>>,
    tags: RefCell<Vec<TagRecord>>,
    terminal_command: Option<Vec<OsString>>,
    toolbar: Toolbar,
    sidebar: Sidebar,
    tab_strip: TabStrip,
    primary_pane: PaneWidgets,
    secondary_pane: PaneWidgets,
    tertiary_pane: PaneWidgets,
    preview: PreviewPane,
    holding_tray: HoldingTray,
    status: StatusBar,
    context_popover: RefCell<Option<Popover>>,
    file_clipboard: RefCell<Option<FileClipboardState>>,
    holding_tray_items: RefCell<Vec<FileItem>>,
    holding_tray_selection: RefCell<Vec<PathBuf>>,
    tabs: RefCell<Vec<TabState>>,
    active_tab: Cell<usize>,
    active_pane: Cell<PaneSlot>,
    current_dir: RefCell<PathBuf>,
    current_view: RefCell<PaneView>,
    back_history: RefCell<Vec<PathBuf>>,
    forward_history: RefCell<Vec<PathBuf>>,
    items: RefCell<Vec<FileItem>>,
    primary_all_items: RefCell<Vec<FileItem>>,
    secondary_current_dir: RefCell<PathBuf>,
    secondary_view: RefCell<PaneView>,
    secondary_back_history: RefCell<Vec<PathBuf>>,
    secondary_forward_history: RefCell<Vec<PathBuf>>,
    secondary_items: RefCell<Vec<FileItem>>,
    secondary_all_items: RefCell<Vec<FileItem>>,
    tertiary_current_dir: RefCell<PathBuf>,
    tertiary_view: RefCell<PaneView>,
    tertiary_back_history: RefCell<Vec<PathBuf>>,
    tertiary_forward_history: RefCell<Vec<PathBuf>>,
    tertiary_items: RefCell<Vec<FileItem>>,
    tertiary_all_items: RefCell<Vec<FileItem>>,
    pending_reveal_path: RefCell<Option<PathBuf>>,
    secondary_pending_reveal_path: RefCell<Option<PathBuf>>,
    tertiary_pending_reveal_path: RefCell<Option<PathBuf>>,
    pending_status_message: RefCell<Option<String>>,
    primary_show_hidden: Cell<bool>,
    secondary_show_hidden: Cell<bool>,
    tertiary_show_hidden: Cell<bool>,
    sidebar_visible: Cell<bool>,
    preview_visible: Cell<bool>,
    suppress_panel_toggle_handlers: Cell<bool>,
    pane_layout: Cell<PaneLayout>,
    primary_view_mode: Cell<ViewMode>,
    secondary_view_mode: Cell<ViewMode>,
    tertiary_view_mode: Cell<ViewMode>,
    primary_sort_field: Cell<SortField>,
    primary_sort_direction: Cell<SortDirection>,
    secondary_sort_field: Cell<SortField>,
    secondary_sort_direction: Cell<SortDirection>,
    tertiary_sort_field: Cell<SortField>,
    tertiary_sort_direction: Cell<SortDirection>,
    load_generation: Cell<u64>,
    load_cancellable: RefCell<Option<gio::Cancellable>>,
    secondary_load_generation: Cell<u64>,
    secondary_load_cancellable: RefCell<Option<gio::Cancellable>>,
    tertiary_load_generation: Cell<u64>,
    tertiary_load_cancellable: RefCell<Option<gio::Cancellable>>,
    primary_keyboard_anchor: Cell<Option<i32>>,
    primary_keyboard_current: Cell<Option<i32>>,
    secondary_keyboard_anchor: Cell<Option<i32>>,
    secondary_keyboard_current: Cell<Option<i32>>,
    tertiary_keyboard_anchor: Cell<Option<i32>>,
    tertiary_keyboard_current: Cell<Option<i32>>,
    preview_generation: Cell<u64>,
    preview_cancellable: RefCell<Option<gio::Cancellable>>,
    primary_thumb_loader: crate::thumbnail::ThumbnailLoader,
    secondary_thumb_loader: crate::thumbnail::ThumbnailLoader,
    tertiary_thumb_loader: crate::thumbnail::ThumbnailLoader,
    holding_tray_thumb_loader: crate::thumbnail::ThumbnailLoader,
    search_debounce: RefCell<Option<glib::SourceId>>,
    primary_search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    secondary_search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    tertiary_search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    ops_panel: OpsPanel,
    plan_queue_panel: PlanQueuePanel,
    plan_mode_active: Cell<bool>,
    action_queue: RefCell<Vec<crate::action_plan::ActionPlan>>,
    sidebar_revealer: Revealer,
    preview_revealer: Revealer,
    split_paned: Paned,
    right_paned: Paned,
    modal_host: ModalHost,
    config: AppConfig,
    primary_duplicate_set: RefCell<Option<std::collections::HashSet<PathBuf>>>,
    secondary_duplicate_set: RefCell<Option<std::collections::HashSet<PathBuf>>>,
    tertiary_duplicate_set: RefCell<Option<std::collections::HashSet<PathBuf>>>,
    primary_duplicate_scan_pending: Cell<bool>,
    secondary_duplicate_scan_pending: Cell<bool>,
    tertiary_duplicate_scan_pending: Cell<bool>,
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
        tertiary_pane: PaneWidgets,
        preview: PreviewPane,
        holding_tray: HoldingTray,
        plan_queue_panel: PlanQueuePanel,
        status: StatusBar,
        ops_panel: OpsPanel,
        sidebar_revealer: Revealer,
        preview_revealer: Revealer,
        split_paned: Paned,
        right_paned: Paned,
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
            user_places: RefCell::new(Vec::new()),
            projects: RefCell::new(Vec::new()),
            tags: RefCell::new(Vec::new()),
            tabs: RefCell::new(vec![initial_tab]),
            active_tab: Cell::new(0),
            active_pane: Cell::new(PaneSlot::Primary),
            current_dir: RefCell::new(places.home.clone()),
            current_view: RefCell::new(PaneView::Directory(places.home.clone())),
            secondary_current_dir: RefCell::new(places.home.clone()),
            secondary_view: RefCell::new(PaneView::Directory(places.home.clone())),
            tertiary_current_dir: RefCell::new(places.home.clone()),
            tertiary_view: RefCell::new(PaneView::Directory(places.home.clone())),
            terminal_command: detect_terminal_command(),
            places,
            toolbar,
            sidebar,
            tab_strip,
            primary_pane,
            secondary_pane,
            tertiary_pane,
            preview,
            holding_tray,
            status,
            context_popover: RefCell::new(None),
            file_clipboard: RefCell::new(None),
            holding_tray_items: RefCell::new(Vec::new()),
            holding_tray_selection: RefCell::new(Vec::new()),
            back_history: RefCell::new(Vec::new()),
            forward_history: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            primary_all_items: RefCell::new(Vec::new()),
            secondary_back_history: RefCell::new(Vec::new()),
            secondary_forward_history: RefCell::new(Vec::new()),
            secondary_items: RefCell::new(Vec::new()),
            secondary_all_items: RefCell::new(Vec::new()),
            tertiary_back_history: RefCell::new(Vec::new()),
            tertiary_forward_history: RefCell::new(Vec::new()),
            tertiary_items: RefCell::new(Vec::new()),
            tertiary_all_items: RefCell::new(Vec::new()),
            pending_reveal_path: RefCell::new(None),
            secondary_pending_reveal_path: RefCell::new(None),
            tertiary_pending_reveal_path: RefCell::new(None),
            pending_status_message: RefCell::new(launch_notice),
            primary_show_hidden: Cell::new(false),
            secondary_show_hidden: Cell::new(false),
            tertiary_show_hidden: Cell::new(false),
            sidebar_visible: Cell::new(true),
            preview_visible: Cell::new(true),
            suppress_panel_toggle_handlers: Cell::new(false),
            pane_layout: Cell::new(PaneLayout::Single),
            primary_view_mode: Cell::new(ViewMode::Icons),
            secondary_view_mode: Cell::new(ViewMode::Icons),
            tertiary_view_mode: Cell::new(ViewMode::Icons),
            primary_sort_field: Cell::new(SortField::Name),
            primary_sort_direction: Cell::new(SortDirection::Ascending),
            secondary_sort_field: Cell::new(SortField::Name),
            secondary_sort_direction: Cell::new(SortDirection::Ascending),
            tertiary_sort_field: Cell::new(SortField::Name),
            tertiary_sort_direction: Cell::new(SortDirection::Ascending),
            load_generation: Cell::new(0),
            load_cancellable: RefCell::new(None),
            secondary_load_generation: Cell::new(0),
            secondary_load_cancellable: RefCell::new(None),
            tertiary_load_generation: Cell::new(0),
            tertiary_load_cancellable: RefCell::new(None),
            primary_keyboard_anchor: Cell::new(None),
            primary_keyboard_current: Cell::new(None),
            secondary_keyboard_anchor: Cell::new(None),
            secondary_keyboard_current: Cell::new(None),
            tertiary_keyboard_anchor: Cell::new(None),
            tertiary_keyboard_current: Cell::new(None),
            preview_generation: Cell::new(0),
            preview_cancellable: RefCell::new(None),
            primary_thumb_loader: crate::thumbnail::ThumbnailLoader::new(),
            secondary_thumb_loader: crate::thumbnail::ThumbnailLoader::new(),
            tertiary_thumb_loader: crate::thumbnail::ThumbnailLoader::new(),
            holding_tray_thumb_loader: crate::thumbnail::ThumbnailLoader::new(),
            search_debounce: RefCell::new(None),
            primary_search_cancel: RefCell::new(None),
            secondary_search_cancel: RefCell::new(None),
            tertiary_search_cancel: RefCell::new(None),
            ops_panel,
            plan_queue_panel,
            plan_mode_active: Cell::new(false),
            action_queue: RefCell::new(Vec::new()),
            sidebar_revealer,
            preview_revealer,
            split_paned,
            right_paned,
            modal_host,
            config,
            primary_duplicate_set: RefCell::new(None),
            secondary_duplicate_set: RefCell::new(None),
            tertiary_duplicate_set: RefCell::new(None),
            primary_duplicate_scan_pending: Cell::new(false),
            secondary_duplicate_scan_pending: Cell::new(false),
            tertiary_duplicate_scan_pending: Cell::new(false),
        })
    }

    fn bootstrap(self: &Rc<Self>) {
        self.connect_navigation();
        self.connect_sidebar();
        self.connect_tab_strip();
        self.connect_panes();
        self.connect_preview_actions();
        self.connect_holding_tray();
        self.connect_search_panels();
        self.connect_window_shortcuts();
        self.attach_pane_dnd(PaneSlot::Primary);
        self.attach_pane_dnd(PaneSlot::Secondary);
        self.attach_pane_dnd(PaneSlot::Tertiary);
        self.attach_sidebar_place_dnd(self.sidebar.home_button.clone(), self.places.home.clone());
        self.wire_tag_filters();
        self.refresh_metadata_sidebar();
        self.update_action_state();
        self.rebuild_tab_strip();
        self.sync_pane_layout_visibility();
        self.reload_active_tab();
        self.refresh_holding_tray();
    }

    fn connect_navigation(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.toolbar
            .back_button
            .connect_clicked(move |_| controller.go_back());

        let controller = Rc::clone(self);
        self.toolbar
            .forward_button
            .connect_clicked(move |_| controller.go_forward());

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
            .connect_clicked(move |_| controller.cycle_pane_layout());

        let controller = Rc::clone(self);
        self.toolbar.preview_toggle.connect_toggled(move |toggle| {
            if controller.suppress_panel_toggle_handlers.get() {
                return;
            }
            controller.set_preview_visible(toggle.is_active());
        });

        let controller = Rc::clone(self);
        self.toolbar
            .holding_tray_toggle
            .connect_toggled(move |toggle| controller.set_holding_tray_visible(toggle.is_active()));

        let controller = Rc::clone(self);
        self.toolbar
            .plan_mode_toggle
            .connect_toggled(move |toggle| controller.set_plan_mode(toggle.is_active()));

        let controller = Rc::clone(self);
        self.plan_queue_panel
            .execute_btn
            .connect_clicked(move |_| controller.execute_plan_queue());

        let controller = Rc::clone(self);
        self.plan_queue_panel.clear_btn.connect_clicked(move |_| {
            controller.action_queue.borrow_mut().clear();
            controller.refresh_plan_queue_panel();
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
            .empty_trash_button
            .connect_clicked(move |_| controller.empty_trash());

        let controller = Rc::clone(self);
        self.toolbar
            .path_button
            .connect_clicked(move |_| controller.begin_path_entry_editing());

        let controller = Rc::clone(self);
        self.toolbar
            .path_entry
            .connect_activate(move |_| controller.navigate_from_path_entry());

        self.attach_path_completion();

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
    }

    fn connect_holding_tray(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.holding_tray
            .add_selection_button
            .connect_clicked(move |_| {
                controller.add_selection_to_holding_tray(controller.active_slot())
            });

        let controller = Rc::clone(self);
        self.holding_tray
            .move_to_project_button
            .connect_clicked(move |_| controller.show_tray_project_dialog(TrayProjectAction::Move));

        let controller = Rc::clone(self);
        self.holding_tray
            .copy_to_project_button
            .connect_clicked(move |_| controller.show_tray_project_dialog(TrayProjectAction::Copy));

        let controller = Rc::clone(self);
        self.holding_tray
            .tag_button
            .connect_clicked(move |_| controller.show_tray_tag_preview());

        let controller = Rc::clone(self);
        self.holding_tray
            .trash_button
            .connect_clicked(move |_| controller.show_tray_trash_preview());

        let controller = Rc::clone(self);
        self.holding_tray
            .copy_path_button
            .connect_clicked(move |_| controller.copy_holding_tray_paths());

        let controller = Rc::clone(self);
        self.holding_tray
            .clear_button
            .connect_clicked(move |_| controller.clear_holding_tray());

        self.attach_holding_tray_dnd();
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

    #[allow(deprecated)]
    fn attach_path_completion(self: &Rc<Self>) {
        let store = gtk::ListStore::new(&[String::static_type()]);
        let completion = gtk::EntryCompletion::builder()
            .model(&store)
            .text_column(0)
            .minimum_key_length(1)
            .inline_completion(true)
            .inline_selection(true)
            .popup_completion(true)
            .popup_set_width(true)
            .popup_single_match(false)
            .build();
        completion.set_match_func(|_, _, _| true);

        let entry = self.toolbar.path_entry.clone();
        completion.connect_match_selected(move |_, model, iter| {
            let value = model.get::<String>(iter, 0);
            entry.set_text(&value);
            entry.set_position(-1);
            glib::Propagation::Stop
        });

        self.toolbar.path_entry.set_completion(Some(&completion));

        let completer = gio::FilenameCompleter::new();
        let latest_input = Rc::new(RefCell::new(String::new()));

        let controller = Rc::clone(self);
        let store_for_changed = store.clone();
        let completer_for_changed = completer.clone();
        let latest_for_changed = latest_input.clone();
        self.toolbar.path_entry.connect_changed(move |entry| {
            let input = entry.text().to_string();
            latest_for_changed.replace(input.clone());
            update_path_completion_model(
                &store_for_changed,
                &completer_for_changed,
                &input,
                &controller.current_dir_for(controller.active_slot()),
                &controller.places.home,
            );
        });

        let controller = Rc::clone(self);
        let store_for_data = store.clone();
        let latest_for_data = latest_input.clone();
        completer.connect_got_completion_data(move |completer| {
            let input = latest_for_data.borrow().clone();
            update_path_completion_model(
                &store_for_data,
                completer,
                &input,
                &controller.current_dir_for(controller.active_slot()),
                &controller.places.home,
            );
        });

        let entry = self.toolbar.path_entry.clone();
        let store_for_keys = store.clone();
        let completion_for_keys = completion.clone();
        let key_controller = gtk::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            let accept_inline = key == gdk::Key::Tab
                || (key == gdk::Key::Right && entry.position() == entry.text_length() as i32);
            if !accept_inline {
                return glib::Propagation::Proceed;
            }

            if let Some(first) = first_path_completion(&store_for_keys) {
                if first != entry.text() {
                    entry.set_text(&first);
                    entry.set_position(-1);
                    return glib::Propagation::Stop;
                }
            }

            completion_for_keys.insert_prefix();
            glib::Propagation::Stop
        });
        self.toolbar.path_entry.add_controller(key_controller);
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

        if self.handle_holding_tray_key(focus, key, modifiers)
            || self.handle_sidebar_navigation(focus, key, modifiers)
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

        if widget_or_ancestor_has_css(&focus, "holding-tray") {
            return FocusedContext::HoldingTray;
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

        if focus.is::<gtk::FlowBox>()
            || focus.is::<gtk::FlowBoxChild>()
            || focus.is::<ListBox>()
            || focus.is::<ListBoxRow>()
        {
            return FocusedContext::FileGrid;
        }

        FocusedContext::Window
    }

    fn search_entries(&self) -> [Entry; 3] {
        [
            self.primary_pane.search_panel.name_entry.clone(),
            self.secondary_pane.search_panel.name_entry.clone(),
            self.tertiary_pane.search_panel.name_entry.clone(),
        ]
    }

    fn sidebar_buttons(&self) -> Vec<Button> {
        let mut buttons = vec![
            self.sidebar.home_button.clone(),
            self.sidebar.triage_button.clone(),
            self.sidebar.activity_log_button.clone(),
            self.sidebar.tags_button.clone(),
            self.sidebar.drives_button.clone(),
            self.sidebar.recent_button.clone(),
            self.sidebar.trash_button.clone(),
        ];
        buttons.extend(
            self.sidebar
                .place_buttons()
                .into_iter()
                .map(|(_, button)| button),
        );
        buttons.extend(
            self.sidebar
                .project_buttons()
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
        let controller = Rc::clone(self);
        self.sidebar
            .search_button
            .connect_clicked(move |_| controller.open_search_in_current_dir());
        let controller = Rc::clone(self);
        self.sidebar
            .triage_button
            .connect_clicked(move |_| controller.triage_active_folder());
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
        let controller = Rc::clone(self);
        self.sidebar
            .activity_log_button
            .connect_clicked(move |_| controller.open_activity_log());
        let controller = Rc::clone(self);
        self.sidebar
            .tags_button
            .connect_clicked(move |_| controller.open_tag_manager());
    }

    fn open_activity_log(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.current_view_cell(slot).replace(PaneView::ActivityLog);
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_activity_log_view(slot);
    }

    fn open_tag_manager(self: &Rc<Self>) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::TagManager) {
            return;
        }
        self.current_view_cell(slot).replace(PaneView::TagManager);
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_tag_manager_view(slot);
        self.update_navigation_state();
    }

    fn load_tag_manager_view(self: &Rc<Self>, slot: PaneSlot) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let pane = self.pane_widgets(slot);
        let display_label = self.display_label_for(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        let (tags, counts) = {
            let meta = self.metadata.borrow();
            let tags = meta.list_tags().unwrap_or_default();
            let counts = meta.count_files_per_tag().unwrap_or_default();
            (tags, counts)
        };
        let tag_count = tags.len();
        pane.tag_manager_panel.set_tags(&tags, &counts);

        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_clicked(move |tag_id| {
            controller.open_tag(tag_id);
        });
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_created(move |name, color| {
            controller.handle_tag_created(name, color);
        });
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_renamed(move |id, name| {
            controller.handle_tag_renamed(id, name);
        });
        let controller = Rc::clone(self);
        pane.tag_manager_panel
            .connect_tag_recolored(move |id, color| {
                controller.handle_tag_recolored(id, color);
            });
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_deleted(move |id| {
            controller.handle_tag_deleted(id);
        });

        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(tag_count, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }
    }

    fn handle_tag_created(self: &Rc<Self>, name: String, color: String) {
        let tag_result = self.metadata.borrow_mut().ensure_tag(&name);
        let result = tag_result.and_then(|tag| {
            self.metadata.borrow_mut().update_tag_color(tag.id, &color)
        });
        match result {
            Ok(()) => {
                self.refresh_metadata_sidebar();
                self.reload_tag_manager_if_visible();
                self.status
                    .set_message(&format!("Tag \u{2018}{name}\u{2019} created."));
            }
            Err(e) => {
                self.modal_host
                    .show_error("Create Tag Failed", &e);
            }
        }
    }

    fn handle_tag_renamed(self: &Rc<Self>, id: i64, new_name: String) {
        let result = self.metadata.borrow_mut().rename_tag(id, &new_name);
        match result {
            Ok(()) => {
                self.refresh_metadata_sidebar();
                self.reload_tag_manager_if_visible();
            }
            Err(e) => {
                self.modal_host
                    .show_error("Rename Tag Failed", &e);
            }
        }
    }

    fn handle_tag_recolored(self: &Rc<Self>, id: i64, color: String) {
        let result = self.metadata.borrow_mut().update_tag_color(id, &color);
        match result {
            Ok(()) => {
                self.refresh_metadata_sidebar();
                self.reload_tag_manager_if_visible();
            }
            Err(e) => {
                self.modal_host
                    .show_error("Recolor Tag Failed", &e);
            }
        }
    }

    fn handle_tag_deleted(self: &Rc<Self>, id: i64) {
        let count = self
            .metadata
            .borrow()
            .count_files_per_tag()
            .unwrap_or_default()
            .get(&id)
            .copied()
            .unwrap_or(0);
        let tag_name = self
            .tags
            .borrow()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        let prompt = if count == 0 {
            format!(
                "Delete tag \u{2018}{tag_name}\u{2019}? This cannot be undone."
            )
        } else {
            format!(
                "Delete tag \u{2018}{tag_name}\u{2019}? It will be removed from {count} file(s). This cannot be undone."
            )
        };
        let controller = Rc::clone(self);
        self.modal_host.show_confirm(
            "Delete Tag",
            &prompt,
            "Delete",
            true,
            false,
            move || {
                let result = controller.metadata.borrow_mut().delete_tag(id);
                match result {
                    Ok(()) => {
                        controller.refresh_metadata_sidebar();
                        controller.reload_tag_manager_if_visible();
                        controller.status.set_message("Tag deleted.");
                    }
                    Err(e) => {
                        controller
                            .modal_host
                            .show_error("Delete Tag Failed", &e);
                    }
                }
            },
        );
    }

    fn reload_tag_manager_if_visible(self: &Rc<Self>) {
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::TagManager) {
                self.load_tag_manager_view(slot);
            }
        }
    }

    fn load_activity_log_view(self: &Rc<Self>, slot: PaneSlot) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let pane = self.pane_widgets(slot);
        let display_label = self.display_label_for(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        let entries = self.metadata.borrow().list_recent_activity(200);
        let controller = Rc::clone(self);
        pane.activity_log_panel
            .populate(&entries, move |action, entry| {
                controller.handle_activity_log_action(action, entry);
            });
        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(entries.len(), 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
            self.show_empty_selection_preview(slot, &display_label, entries.len());
        }
    }

    fn load_project_landing_view(self: &Rc<Self>, slot: PaneSlot, project_id: i64) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let Some(project) = self
            .projects
            .borrow()
            .iter()
            .find(|p| p.id == project_id)
            .cloned()
        else {
            self.status.set_message("Project not found.");
            return;
        };

        let destinations = self
            .metadata
            .borrow()
            .list_project_destinations(project_id)
            .unwrap_or_default();
        let root_str = project.root_path.to_string_lossy().to_string();
        let activity = self
            .metadata
            .borrow()
            .list_project_activity(&root_str, 10);

        let pane = self.pane_widgets(slot);
        let display_label = project.name.clone();
        pane.path_label.set_label(&display_label);
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        let controller = Rc::clone(self);
        let on_navigate = move |path: PathBuf| {
            controller.navigate_to(slot, path, true);
        };

        let controller = Rc::clone(self);
        let on_remove_destination = move |dest_id: i64| {
            controller.remove_project_destination(dest_id, project_id, slot);
        };

        let controller = Rc::clone(self);
        let on_open_new_tab = move |path: PathBuf| {
            controller.open_new_tab(Some(path));
        };

        let controller = Rc::clone(self);
        let on_open_split = move |path: PathBuf| {
            controller.open_in_split(slot, path);
        };

        let controller = Rc::clone(self);
        let on_view_log = move || {
            controller.open_activity_log();
        };

        let controller = Rc::clone(self);
        let on_add_destination = move || {
            controller.show_add_destination_dialog(project_id, slot);
        };

        let controller = Rc::clone(self);
        let on_send_holding_tray = move || {
            controller.send_holding_tray_to_project(project_id);
        };

        pane.project_landing_panel.populate(
            &project,
            &destinations,
            &activity,
            on_navigate,
            on_remove_destination,
            on_open_new_tab,
            on_open_split,
            on_view_log,
            on_add_destination,
            on_send_holding_tray,
        );

        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.update_navigation_state();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.update_sidebar_state();
            self.update_action_state();
        }
    }

    fn show_add_destination_dialog(self: &Rc<Self>, project_id: i64, slot: PaneSlot) {
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.append(&build_modal_prompt(
            "Enter a name and relative path (e.g. \"Inbox\" and \"inbox\").",
        ));

        let name_entry = Entry::new();
        name_entry.set_placeholder_text(Some("Destination name"));
        content.append(&name_entry);

        let path_entry = Entry::new();
        path_entry.set_placeholder_text(Some("Relative path (e.g. inbox or docs/notes)"));
        content.append(&path_entry);

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn =
            build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let controller = Rc::clone(self);
        let add_btn = build_modal_button("Add", ButtonKind::Primary, move || {
            let name = name_entry.text().trim().to_string();
            let rel_path = path_entry.text().trim().to_string();
            if !name.is_empty() {
                let _ = controller
                    .metadata
                    .borrow_mut()
                    .add_project_destination(project_id, &name, &rel_path);
                controller.load_project_landing_view(slot, project_id);
            }
            host.hide();
        });
        actions.append(&add_btn);

        self.modal_host
            .show_with_custom_ui("Add Destination", &content, &actions, false, None);
    }

    fn remove_project_destination(
        self: &Rc<Self>,
        destination_id: i64,
        project_id: i64,
        slot: PaneSlot,
    ) {
        let _ = self
            .metadata
            .borrow_mut()
            .remove_project_destination(destination_id);
        self.load_project_landing_view(slot, project_id);
    }

    fn send_holding_tray_to_project(self: &Rc<Self>, project_id: i64) {
        let paths: Vec<PathBuf> = self
            .holding_tray_items
            .borrow()
            .iter()
            .map(|item| item.path.clone())
            .collect();
        if paths.is_empty() {
            self.status
                .set_message("No items in the holding tray to send.");
            return;
        }
        self.send_paths_to_project(paths, project_id, ProjectTransferKind::Copy, None);
    }

    fn open_in_split(self: &Rc<Self>, requesting_slot: PaneSlot, path: PathBuf) {
        let target_slot = if requesting_slot == PaneSlot::Primary {
            PaneSlot::Secondary
        } else {
            PaneSlot::Primary
        };
        if !self.pane_layout.get().includes(target_slot) {
            self.cycle_pane_layout();
        }
        self.navigate_to(target_slot, path, false);
    }

    fn handle_activity_log_action(
        self: &Rc<Self>,
        action: ActivityLogAction,
        entry: ActivityLogEntry,
    ) {
        match action {
            ActivityLogAction::Undo => self.undo_activity_entry(entry),
            ActivityLogAction::Repeat => self.repeat_activity_entry(entry),
            ActivityLogAction::Reveal => self.reveal_activity_entry(entry),
            ActivityLogAction::CopyPath => self.copy_activity_entry_paths(entry),
        }
    }

    fn show_place_context_menu(
        self: &Rc<Self>,
        place: PlaceRecord,
        anchor: gtk::Widget,
        x: f64,
        y: f64,
    ) {
        self.dismiss_context_menu();

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
        menu_box.set_size_request(190, -1);

        append_menu_button(&menu_box, "Open", Some("document-open-symbolic"), false, {
            let controller = Rc::clone(self);
            let path = place.folder_path.clone();
            move || controller.navigate_to_active(path.clone())
        });
        append_menu_button(&menu_box, "Copy Path", Some("edit-copy-symbolic"), false, {
            let controller = Rc::clone(self);
            let path = place.folder_path.clone();
            move || controller.copy_paths_to_clipboard(vec![path.clone()])
        });
        append_menu_sep(&menu_box);
        append_menu_button(
            &menu_box,
            "Remove from Places",
            Some("list-remove-symbolic"),
            false,
            {
                let controller = Rc::clone(self);
                move || controller.remove_place(place.id)
            },
        );

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

    fn show_project_context_menu(
        self: &Rc<Self>,
        project: ProjectRecord,
        anchor: gtk::Widget,
        x: f64,
        y: f64,
    ) {
        self.dismiss_context_menu();

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
        menu_box.set_size_request(190, -1);

        append_menu_button(&menu_box, "Open Project", Some("document-open-symbolic"), false, {
            let controller = Rc::clone(self);
            let project_id = project.id;
            move || controller.open_project(project_id)
        });
        append_menu_button(
            &menu_box,
            "Open Root Folder",
            Some("folder-symbolic"),
            false,
            {
                let controller = Rc::clone(self);
                let path = project.root_path.clone();
                move || controller.navigate_to_active(path.clone())
            },
        );
        append_menu_button(&menu_box, "Copy Path", Some("edit-copy-symbolic"), false, {
            let controller = Rc::clone(self);
            let path = project.root_path.clone();
            move || controller.copy_paths_to_clipboard(vec![path.clone()])
        });
        append_menu_sep(&menu_box);
        append_menu_button(
            &menu_box,
            "Remove Project",
            Some("list-remove-symbolic"),
            true,
            {
                let controller = Rc::clone(self);
                let project_id = project.id;
                move || controller.confirm_delete_project(project_id)
            },
        );

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

    fn confirm_delete_project(self: &Rc<Self>, project_id: i64) {
        let Some(project) = self
            .projects
            .borrow()
            .iter()
            .find(|p| p.id == project_id)
            .cloned()
        else {
            return;
        };

        let controller = Rc::clone(self);
        self.modal_host.show_confirm(
            "Remove Project",
            &format!(
                "Remove \u{201c}{}\u{201d} from Projects? The folder itself will not be deleted.",
                project.name
            ),
            "Remove",
            true,
            false,
            move || {
                let _ = controller.metadata.borrow_mut().delete_project(project_id);
                // Navigate away if currently viewing this project's landing page
                for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
                    if matches!(controller.current_view_for(slot), PaneView::ProjectLanding(id) if id == project_id)
                    {
                        controller
                            .current_view_cell(slot)
                            .replace(PaneView::Directory(controller.places.home.clone()));
                        controller.load_current_view(slot);
                    }
                }
                controller.refresh_metadata_sidebar();
            },
        );
    }

    fn show_sort_popover(self: &Rc<Self>, slot: PaneSlot, anchor: gtk::Widget) {
        self.dismiss_context_menu();

        let current_field = self.sort_field_cell(slot).get();
        let current_dir = self.sort_direction_cell(slot).get();

        let popover = Popover::new();
        popover.add_css_class("context-menu");
        popover.set_has_arrow(false);
        popover.set_autohide(true);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_parent(&anchor);

        let menu_box = GtkBox::new(Orientation::Vertical, 2);
        menu_box.set_margin_top(6);
        menu_box.set_margin_bottom(6);
        menu_box.set_margin_start(6);
        menu_box.set_margin_end(6);
        menu_box.set_size_request(160, -1);

        for (label, field) in [
            ("Name", SortField::Name),
            ("Date Modified", SortField::Modified),
            ("Size", SortField::Size),
            ("Kind", SortField::Kind),
        ] {
            let icon = if field == current_field {
                Some("object-select-symbolic")
            } else {
                None
            };
            let controller = Rc::clone(self);
            append_menu_button(&menu_box, label, icon, false, move || {
                let new_field = field;
                let new_dir = if controller.sort_field_cell(slot).get() == new_field {
                    match controller.sort_direction_cell(slot).get() {
                        SortDirection::Ascending => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::Ascending,
                    }
                } else {
                    SortDirection::Ascending
                };
                controller.sort_field_cell(slot).set(new_field);
                controller.sort_direction_cell(slot).set(new_dir);
                controller.apply_sort(slot);
            });
        }

        append_menu_sep(&menu_box);

        for (label, dir) in [
            ("↑  Ascending", SortDirection::Ascending),
            ("↓  Descending", SortDirection::Descending),
        ] {
            let icon = if dir == current_dir {
                Some("object-select-symbolic")
            } else {
                None
            };
            let controller = Rc::clone(self);
            append_menu_button(&menu_box, label, icon, false, move || {
                controller.sort_direction_cell(slot).set(dir);
                controller.apply_sort(slot);
            });
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

    fn apply_sort(self: &Rc<Self>, slot: PaneSlot) {
        let field = self.sort_field_cell(slot).get();
        let direction = self.sort_direction_cell(slot).get();

        let icon_name = match direction {
            SortDirection::Ascending => "view-sort-ascending-symbolic",
            SortDirection::Descending => "view-sort-descending-symbolic",
        };
        self.pane_widgets(slot)
            .sort_icon
            .set_icon_name(Some(icon_name));

        let mut items = self.all_items_cell(slot).borrow().clone();
        if items.is_empty() {
            return;
        }

        if let PaneView::Triage { filter, .. } = self.current_view_for(slot) {
            let dup_set = self.duplicate_set_cell(slot).borrow();
            items = filter_triage_items(items, filter, dup_set.as_ref());
        }

        let spec = self.pane_widgets(slot).tag_filter.spec();
        if !spec.is_empty() && matches!(self.current_view_for(slot), PaneView::Directory(_)) {
            items.retain(|item| spec.matches(item));
        }

        sort_items_with(&mut items, field, direction);
        self.items_cell(slot).replace(items.clone());
        self.pane_widgets(slot).file_grid.set_items(&items);
        self.sync_active_tab_state();
    }

    fn repeat_activity_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
        if entry.status != "success" || entry.items.is_empty() {
            self.status
                .set_message("This activity entry cannot be repeated.");
            return;
        }

        let sources = activity_sources(&entry);
        match entry.operation.as_str() {
            "copy" => {
                let Some(dest_dir) = common_activity_destination_parent(&entry) else {
                    self.status
                        .set_message("This copy entry has no destination to repeat.");
                    return;
                };
                self.start_copy_move_with_conflict_check(
                    sources,
                    dest_dir,
                    true,
                    "Repeat copy".to_string(),
                    None,
                );
            }
            "move" => {
                let Some(dest_dir) = common_activity_destination_parent(&entry) else {
                    self.status
                        .set_message("This move entry has no destination to repeat.");
                    return;
                };
                self.start_copy_move_with_conflict_check(
                    sources,
                    dest_dir,
                    false,
                    "Repeat move".to_string(),
                    None,
                );
            }
            "duplicate" => self.do_duplicate_files(sources),
            "rename" | "bulk_rename" => {
                let renames = activity_renames(&entry);
                if renames.is_empty() {
                    self.status
                        .set_message("This rename entry has no names to repeat.");
                } else if renames.len() == 1 {
                    let (path, name) = renames.into_iter().next().unwrap();
                    self.rename_path(path, name);
                } else {
                    self.apply_bulk_rename(renames);
                }
            }
            "new_folder" => {
                if let Some((parent, name)) = activity_created_parent_and_name(&entry) {
                    self.exec_create_folder(parent, name);
                } else {
                    self.status
                        .set_message("This folder creation entry cannot be repeated.");
                }
            }
            "new_file" => {
                if let Some((parent, name)) = activity_created_parent_and_name(&entry) {
                    self.exec_create_text_document(self.active_slot(), parent, name);
                } else {
                    self.status
                        .set_message("This file creation entry cannot be repeated.");
                }
            }
            _ => self
                .status
                .set_message("Repeat is not available for this activity entry."),
        }
    }

    fn undo_activity_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
        if entry.status != "success" || entry.items.is_empty() {
            self.status
                .set_message("This activity entry cannot be undone.");
            return;
        }

        match entry.operation.as_str() {
            "copy" | "duplicate" | "new_folder" | "new_file" => {
                let created = activity_destinations(&entry);
                if created.is_empty() {
                    self.status
                        .set_message("This entry has no created paths to undo.");
                    return;
                }
                self.move_paths_to_trash(created);
            }
            "move" | "rename" | "bulk_rename" => {
                let undo_items: Vec<_> = entry
                    .items
                    .iter()
                    .filter_map(|item| {
                        let source = PathBuf::from(&item.source_path);
                        let destination = item.destination_path.as_ref().map(PathBuf::from)?;
                        Some((destination, source, gio::FileCopyFlags::NONE))
                    })
                    .collect();
                if undo_items.is_empty() {
                    self.status
                        .set_message("This entry has no move history to undo.");
                    return;
                }
                if let Some(conflict) = undo_items
                    .iter()
                    .map(|(_, destination, _)| destination)
                    .find(|path| path.exists())
                {
                    self.show_error_dialog(
                        "Undo Blocked",
                        &format!(
                            "Undo would overwrite an existing item.\n\nTarget: {}",
                            conflict.display()
                        ),
                    );
                    return;
                }
                self.start_copy_move_op(undo_items, false, "Undo operation", None, None);
            }
            "trash" => self.restore_activity_trash_entry(entry),
            _ => self
                .status
                .set_message("Undo is not available for this activity entry."),
        }
    }

    fn reveal_activity_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
        let Some(path) = activity_relevant_path(&entry) else {
            self.status
                .set_message("This activity entry has no path to reveal.");
            return;
        };
        let folder = if path.is_dir() {
            path
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.current_dir_for(self.active_slot()))
        };
        self.navigate_to(self.active_slot(), folder, true);
    }

    fn copy_activity_entry_paths(self: &Rc<Self>, entry: ActivityLogEntry) {
        let paths = if entry.items.is_empty() {
            activity_relevant_path(&entry).into_iter().collect()
        } else {
            entry
                .items
                .iter()
                .map(|item| {
                    item.destination_path
                        .as_ref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from(&item.source_path))
                })
                .collect()
        };
        self.copy_paths_to_clipboard(paths);
    }

    fn restore_activity_trash_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
        let wanted: HashSet<PathBuf> = entry
            .items
            .iter()
            .map(|item| PathBuf::from(&item.source_path))
            .collect();
        if wanted.is_empty() {
            self.status
                .set_message("This trash entry has no original paths to restore.");
            return;
        }

        let trash = gio::File::for_uri("trash:///");
        let enumerator = match trash.enumerate_children(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        ) {
            Ok(enumerator) => enumerator,
            Err(error) => {
                self.show_error_dialog("Undo Failed", &friendly_error_detail(&error));
                return;
            }
        };

        let mut matches = Vec::new();
        loop {
            let info = match enumerator.next_file(None::<&gio::Cancellable>) {
                Ok(Some(info)) => info,
                Ok(None) => break,
                Err(error) => {
                    self.show_error_dialog("Undo Failed", &friendly_error_detail(&error));
                    return;
                }
            };
            let Some(orig_path) = info
                .attribute_byte_string("trash::orig-path")
                .map(|value| PathBuf::from(value.to_string()))
            else {
                continue;
            };
            if !wanted.contains(&orig_path) {
                continue;
            }
            let trash_path = info
                .attribute_string("standard::target-uri")
                .and_then(|uri| gio::File::for_uri(&uri).path())
                .unwrap_or_else(|| {
                    glib::home_dir()
                        .join(".local/share/Trash/files")
                        .join(info.name())
                });
            let kind = FileKind::from_path(
                &trash_path,
                info.file_type(),
                info.content_type().as_deref(),
            );
            matches.push(FileItem {
                name: info.display_name().to_string(),
                path: trash_path,
                kind,
                is_dir: info.file_type() == gio::FileType::Directory,
                size_bytes: (info.size() >= 0).then_some(info.size() as u64),
                modified_unix: info.modification_date_time().map(|value| value.to_unix()),
                tags: Vec::new(),
                original_path: Some(orig_path),
            });
        }

        if matches.is_empty() {
            self.show_error_dialog(
                "Undo Unavailable",
                "Lattice could not find matching Trash items for this activity entry.",
            );
            return;
        }
        self.restore_items_from_trash(matches);
    }

    fn refresh_metadata_sidebar(self: &Rc<Self>) {
        let (places, projects, tags) = {
            let metadata = self.metadata.borrow();
            let places = metadata.list_places().unwrap_or_default();
            let projects = metadata.list_projects().unwrap_or_default();
            let tags = metadata.list_tags().unwrap_or_default();
            (places, projects, tags)
        };

        self.user_places.replace(places.clone());
        self.projects.replace(projects.clone());
        self.tags.replace(tags.clone());
        self.sidebar.set_places(&places);
        self.sidebar.set_projects(&projects);

        for (place, button) in self.sidebar.place_buttons() {
            let controller = Rc::clone(self);
            let path = place.folder_path.clone();
            button.connect_clicked(move |_| controller.navigate_to_active(path.clone()));
            self.attach_sidebar_place_dnd(button.clone(), place.folder_path.clone());
            let controller = Rc::clone(self);
            let place_for_menu = place.clone();
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            gesture.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(widget) = gesture.widget() else {
                    return;
                };
                controller.show_place_context_menu(place_for_menu.clone(), widget, x, y);
            });
            button.add_controller(gesture);
        }

        for (project_id, button) in self.sidebar.project_buttons() {
            let controller = Rc::clone(self);
            button.connect_clicked(move |_| controller.open_project(project_id));

            let Some(project) = projects.iter().find(|p| p.id == project_id).cloned() else {
                continue;
            };
            let controller = Rc::clone(self);
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            gesture.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(widget) = gesture.widget() else {
                    return;
                };
                controller.show_project_context_menu(project.clone(), widget, x, y);
            });
            button.add_controller(gesture);
        }

        self.refresh_search_tag_buttons(PaneSlot::Primary);
        self.refresh_search_tag_buttons(PaneSlot::Secondary);
        self.refresh_search_tag_buttons(PaneSlot::Tertiary);
        self.primary_pane.tag_filter.set_tags(&tags);
        self.secondary_pane.tag_filter.set_tags(&tags);
        self.tertiary_pane.tag_filter.set_tags(&tags);
        self.update_sidebar_state();
    }

    fn open_project(self: &Rc<Self>, project_id: i64) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::ProjectLanding(id) if id == project_id)
        {
            return;
        }
        let current = self.current_view_for(slot);
        if let PaneView::Directory(path) = current {
            self.back_history_cell(slot).borrow_mut().push(path);
            self.forward_history_cell(slot).borrow_mut().clear();
        }
        self.current_view_cell(slot)
            .replace(PaneView::ProjectLanding(project_id));
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_project_landing_view(slot, project_id);
        self.update_navigation_state();
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

    fn open_triage(self: &Rc<Self>, root: PathBuf, filter: TriageFilter) {
        let slot = self.active_slot();
        // Reset duplicate scan state when changing triage root
        let current_root =
            if let PaneView::Triage { root: ref cur, .. } = self.current_view_for(slot) {
                Some(cur.clone())
            } else {
                None
            };
        if current_root.as_ref() != Some(&root) {
            self.duplicate_set_cell(slot).replace(None);
            self.set_duplicate_scan_pending(slot, false);
        }
        self.current_dir_cell(slot).replace(root.clone());
        self.current_view_cell(slot).replace(PaneView::Triage {
            root: root.clone(),
            filter,
        });
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_triage(slot, &root);
    }

    fn set_triage_filter(self: &Rc<Self>, slot: PaneSlot, filter: TriageFilter) {
        let root = match self.current_view_for(slot) {
            PaneView::Triage { root, .. } => root,
            _ => return,
        };
        self.current_view_cell(slot).replace(PaneView::Triage {
            root: root.clone(),
            filter,
        });
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_triage(slot, &root);
    }

    fn triage_active_folder(self: &Rc<Self>) {
        let slot = self.active_slot();
        let Some(root) = self.tool_scope_dir_for(slot) else {
            self.status
                .set_message("Navigate to a folder first, then click Triage.");
            return;
        };
        self.open_triage(root, TriageFilter::All);
    }

    fn start_duplicate_scan(self: &Rc<Self>, slot: PaneSlot, root: PathBuf) {
        self.set_duplicate_scan_pending(slot, true);
        self.update_view_strip(slot);

        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let set = gio::spawn_blocking(move || compute_duplicate_set_from_dir(&root))
                .await
                .unwrap_or_default();
            controller.duplicate_set_cell(slot).replace(Some(set));
            controller.set_duplicate_scan_pending(slot, false);
            if matches!(
                controller.current_view_for(slot),
                PaneView::Triage {
                    filter: TriageFilter::Duplicates,
                    ..
                }
            ) {
                let all_items = controller.all_items_cell(slot).borrow().clone();
                let dup_set = controller.duplicate_set_cell(slot).borrow();
                let filtered =
                    filter_triage_items(all_items, TriageFilter::Duplicates, dup_set.as_ref());
                drop(dup_set);
                controller.items_cell(slot).replace(filtered.clone());
                controller.pane_widgets(slot).file_grid.set_items(&filtered);
                controller.update_view_strip(slot);
                if slot == controller.active_slot() {
                    controller.status.set_counts(filtered.len(), 0);
                    controller.update_action_state();
                }
            }
        });
    }

    fn update_view_strip(&self, slot: PaneSlot) {
        let pane = self.pane_widgets(slot);
        let is_search = matches!(self.current_view_for(slot), PaneView::Search(_));
        pane.search_panel.root.set_visible(is_search);
        pane.search_revealer.set_reveal_child(is_search);

        let is_activity_log = matches!(self.current_view_for(slot), PaneView::ActivityLog);
        let is_project_landing =
            matches!(self.current_view_for(slot), PaneView::ProjectLanding(_));
        let is_tag_manager = matches!(self.current_view_for(slot), PaneView::TagManager);
        pane.file_grid
            .root
            .set_visible(!is_activity_log && !is_project_landing && !is_tag_manager);
        pane.activity_log_panel.root.set_visible(is_activity_log);
        pane.project_landing_panel.root.set_visible(is_project_landing);
        pane.tag_manager_panel.root.set_visible(is_tag_manager);

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
            PaneView::Triage {
                filter: active_filter,
                ..
            } => {
                pane.view_strip.set_visible(true);
                let scanning = self.duplicate_scan_pending_for(slot)
                    && active_filter == TriageFilter::Duplicates;
                if scanning {
                    pane.view_title.set_visible(true);
                    pane.view_title
                        .set_label("🔍 Scanning for duplicates\u{2026}");
                    pane.view_title.add_css_class("pane-filter-scanning");
                } else {
                    pane.view_title.set_visible(false);
                    pane.view_title.remove_css_class("pane-filter-scanning");
                }
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
                pane.view_strip.set_visible(false);
            }
            PaneView::Recent => {
                pane.view_strip.set_visible(false);
            }
            PaneView::ActivityLog => {
                pane.view_strip.set_visible(false);
                pane.tag_filter_revealer.set_reveal_child(false);
                pane.tag_filter_revealer.set_visible(false);
                self.sync_filter_button_state(slot);
            }
            PaneView::ProjectLanding(_) => {
                pane.view_strip.set_visible(false);
                pane.tag_filter_revealer.set_reveal_child(false);
                pane.tag_filter_revealer.set_visible(false);
                self.sync_filter_button_state(slot);
            }
            PaneView::TagManager => {
                pane.view_strip.set_visible(false);
                pane.tag_filter_revealer.set_reveal_child(false);
                pane.tag_filter_revealer.set_visible(false);
                self.sync_filter_button_state(slot);
            }
        }
    }

    fn load_current_view(self: &Rc<Self>, slot: PaneSlot) {
        match self.current_view_for(slot) {
            PaneView::Directory(path) => self.load_directory(slot, path),
            PaneView::Tag(tag) => self.load_tag_view(slot, tag),
            PaneView::Triage { root, .. } => self.load_triage(slot, &root),
            PaneView::SystemDrives => self.load_system_drives_view(slot),
            PaneView::Recent => self.load_recent_view(slot),
            PaneView::Trash => self.load_trash_view(slot),
            PaneView::Search(query) => self.load_search_view(slot, query),
            PaneView::ActivityLog => self.load_activity_log_view(slot),
            PaneView::ProjectLanding(project_id) => {
                self.load_project_landing_view(slot, project_id)
            }
            PaneView::TagManager => self.load_tag_manager_view(slot),
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
        self.connect_pane(PaneSlot::Tertiary);
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
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        gesture.connect_pressed(move |gesture, _, x, y| {
            controller.set_active_pane(slot);
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(child) = flow.child_at_pos(x as i32, y as i32) {
                let anchor: gtk::Widget = flow.clone().upcast();
                controller.show_context_menu(slot, child.index(), anchor, x, y);
            } else {
                controller.show_current_folder_menu(slot, x, y);
            }
        });
        pane.file_grid.flow.add_controller(gesture);

        let controller = Rc::clone(self);
        let flow_click = gtk::GestureClick::new();
        flow_click.set_button(0);
        flow_click.connect_pressed(move |_, _, _, _| controller.set_active_pane(slot));
        pane.file_grid.flow.add_controller(flow_click);

        // List-mode selection signals
        let controller = Rc::clone(self);
        pane.file_grid
            .list_box
            .connect_selected_rows_changed(move |_| controller.update_selection_for(slot));

        let controller = Rc::clone(self);
        pane.file_grid
            .list_box
            .connect_row_activated(move |_, row| {
                controller.set_active_pane(slot);
                controller.activate_index(slot, row.index())
            });

        let controller = Rc::clone(self);
        let list_box = pane.file_grid.list_box.clone();
        let list_rclick = gtk::GestureClick::new();
        list_rclick.set_button(3);
        list_rclick.set_propagation_phase(gtk::PropagationPhase::Capture);
        list_rclick.connect_pressed(move |gesture, _, x, y| {
            controller.set_active_pane(slot);
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(row) = list_box.row_at_y(y as i32) {
                let anchor: gtk::Widget = list_box.clone().upcast();
                controller.show_context_menu(slot, row.index(), anchor, x, y);
            } else {
                controller.show_current_folder_menu(slot, x, y);
            }
        });
        pane.file_grid.list_box.add_controller(list_rclick);

        let controller = Rc::clone(self);
        let list_click = gtk::GestureClick::new();
        list_click.set_button(0);
        list_click.connect_pressed(move |_, _, _, _| controller.set_active_pane(slot));
        pane.file_grid.list_box.add_controller(list_click);

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

        let controller = Rc::clone(self);
        pane.filter_toggle_btn.connect_clicked(move |_| {
            controller.set_active_pane(slot);
            let is_open = controller
                .pane_widgets(slot)
                .tag_filter_revealer
                .reveals_child();
            controller.set_filter_panel_open_for_slot(slot, !is_open);
        });

        let controller = Rc::clone(self);
        pane.hidden_toggle_btn.connect_clicked(move |_| {
            controller.set_active_pane(slot);
            controller.set_show_hidden_for_slot(slot, !controller.show_hidden_cell(slot).get());
        });

        let controller = Rc::clone(self);
        pane.view_mode_btn.connect_clicked(move |_| {
            controller.set_active_pane(slot);
            let next = match controller.view_mode_cell(slot).get() {
                ViewMode::Icons => ViewMode::List,
                ViewMode::List => ViewMode::Icons,
            };
            controller.set_view_mode(slot, next);
        });

        let controller = Rc::clone(self);
        let sort_btn_clone = pane.sort_btn.clone();
        pane.sort_btn.connect_clicked(move |_| {
            controller.set_active_pane(slot);
            let widget: gtk::Widget = sort_btn_clone.clone().upcast();
            controller.show_sort_popover(slot, widget);
        });
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
                    .tag_filter_revealer
                    .reveals_child();
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
                let slot = self.active_slot();
                self.set_show_hidden_for_slot(slot, !self.show_hidden_cell(slot).get());
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
            WindowCommand::ToggleHoldingTray => {
                self.toolbar
                    .holding_tray_toggle
                    .set_active(!self.holding_tray.root.reveals_child());
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
                self.cycle_pane_layout();
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
            WindowCommand::GoForward => {
                self.go_forward();
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
            WindowCommand::SetViewIcons => {
                self.set_view_mode(self.active_slot(), ViewMode::Icons);
                true
            }
            WindowCommand::SetViewList => {
                self.set_view_mode(self.active_slot(), ViewMode::List);
                true
            }
            WindowCommand::TogglePlanMode => {
                self.set_plan_mode(!self.plan_mode_active.get());
                true
            }
            WindowCommand::EmptyTrash => {
                self.empty_trash();
                true
            }
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
                if !self.holding_tray_selection.borrow().is_empty() {
                    self.clear_holding_tray_selection();
                    return true;
                }
                if self.exit_search_if_empty(self.active_slot()) {
                    return true;
                }
                let slot = self.active_slot();
                if self.pane_widgets(slot).tag_filter_revealer.reveals_child()
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

    fn handle_holding_tray_key(
        self: &Rc<Self>,
        focus: FocusedContext,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> bool {
        if focus != FocusedContext::HoldingTray {
            return false;
        }

        let modifiers = relevant_modifiers(modifiers);
        if modifiers.contains(gdk::ModifierType::CONTROL_MASK) && key_char(key) == Some('c') {
            self.copy_holding_tray_paths();
            return true;
        }

        if modifiers.contains(gdk::ModifierType::CONTROL_MASK) && key_char(key) == Some('v') {
            self.paste_file_clipboard_into_holding_tray();
            return true;
        }

        match key {
            gdk::Key::Delete | gdk::Key::BackSpace => {
                self.remove_selected_holding_tray_items();
                true
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                self.open_selected_holding_tray_item();
                true
            }
            gdk::Key::Escape => {
                self.clear_holding_tray_selection();
                true
            }
            _ => false,
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
        self.pane_widgets(slot).file_grid.grab_focus_on_active();
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
        let visible = self.visible_slots();
        if visible.len() <= 1 {
            return;
        }

        let current = self.active_slot();
        let current_index = visible
            .iter()
            .position(|slot| *slot == current)
            .unwrap_or(0);
        let next = visible[(current_index + 1) % visible.len()];
        self.set_active_pane(next);
        self.pane_widgets(next).file_grid.grab_focus_on_active();
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
            PaneSlot::Tertiary => &self.tertiary_pane,
        }
    }

    fn duplicate_set_cell(
        &self,
        slot: PaneSlot,
    ) -> &RefCell<Option<std::collections::HashSet<PathBuf>>> {
        match slot {
            PaneSlot::Primary => &self.primary_duplicate_set,
            PaneSlot::Secondary => &self.secondary_duplicate_set,
            PaneSlot::Tertiary => &self.tertiary_duplicate_set,
        }
    }

    fn duplicate_scan_pending_for(&self, slot: PaneSlot) -> bool {
        match slot {
            PaneSlot::Primary => self.primary_duplicate_scan_pending.get(),
            PaneSlot::Secondary => self.secondary_duplicate_scan_pending.get(),
            PaneSlot::Tertiary => self.tertiary_duplicate_scan_pending.get(),
        }
    }

    fn set_duplicate_scan_pending(&self, slot: PaneSlot, pending: bool) {
        match slot {
            PaneSlot::Primary => self.primary_duplicate_scan_pending.set(pending),
            PaneSlot::Secondary => self.secondary_duplicate_scan_pending.set(pending),
            PaneSlot::Tertiary => self.tertiary_duplicate_scan_pending.set(pending),
        }
    }

    fn current_dir_cell(&self, slot: PaneSlot) -> &RefCell<PathBuf> {
        match slot {
            PaneSlot::Primary => &self.current_dir,
            PaneSlot::Secondary => &self.secondary_current_dir,
            PaneSlot::Tertiary => &self.tertiary_current_dir,
        }
    }

    fn current_view_cell(&self, slot: PaneSlot) -> &RefCell<PaneView> {
        match slot {
            PaneSlot::Primary => &self.current_view,
            PaneSlot::Secondary => &self.secondary_view,
            PaneSlot::Tertiary => &self.tertiary_view,
        }
    }

    fn back_history_cell(&self, slot: PaneSlot) -> &RefCell<Vec<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.back_history,
            PaneSlot::Secondary => &self.secondary_back_history,
            PaneSlot::Tertiary => &self.tertiary_back_history,
        }
    }

    fn forward_history_cell(&self, slot: PaneSlot) -> &RefCell<Vec<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.forward_history,
            PaneSlot::Secondary => &self.secondary_forward_history,
            PaneSlot::Tertiary => &self.tertiary_forward_history,
        }
    }

    fn items_cell(&self, slot: PaneSlot) -> &RefCell<Vec<FileItem>> {
        match slot {
            PaneSlot::Primary => &self.items,
            PaneSlot::Secondary => &self.secondary_items,
            PaneSlot::Tertiary => &self.tertiary_items,
        }
    }

    fn all_items_cell(&self, slot: PaneSlot) -> &RefCell<Vec<FileItem>> {
        match slot {
            PaneSlot::Primary => &self.primary_all_items,
            PaneSlot::Secondary => &self.secondary_all_items,
            PaneSlot::Tertiary => &self.tertiary_all_items,
        }
    }

    fn pending_reveal_cell(&self, slot: PaneSlot) -> &RefCell<Option<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.pending_reveal_path,
            PaneSlot::Secondary => &self.secondary_pending_reveal_path,
            PaneSlot::Tertiary => &self.tertiary_pending_reveal_path,
        }
    }

    fn load_generation_cell(&self, slot: PaneSlot) -> &Cell<u64> {
        match slot {
            PaneSlot::Primary => &self.load_generation,
            PaneSlot::Secondary => &self.secondary_load_generation,
            PaneSlot::Tertiary => &self.tertiary_load_generation,
        }
    }

    fn load_cancellable_cell(&self, slot: PaneSlot) -> &RefCell<Option<gio::Cancellable>> {
        match slot {
            PaneSlot::Primary => &self.load_cancellable,
            PaneSlot::Secondary => &self.secondary_load_cancellable,
            PaneSlot::Tertiary => &self.tertiary_load_cancellable,
        }
    }

    fn search_cancel_cell(&self, slot: PaneSlot) -> &RefCell<Option<Arc<AtomicBool>>> {
        match slot {
            PaneSlot::Primary => &self.primary_search_cancel,
            PaneSlot::Secondary => &self.secondary_search_cancel,
            PaneSlot::Tertiary => &self.tertiary_search_cancel,
        }
    }

    fn keyboard_anchor_cell(&self, slot: PaneSlot) -> &Cell<Option<i32>> {
        match slot {
            PaneSlot::Primary => &self.primary_keyboard_anchor,
            PaneSlot::Secondary => &self.secondary_keyboard_anchor,
            PaneSlot::Tertiary => &self.tertiary_keyboard_anchor,
        }
    }

    fn keyboard_current_cell(&self, slot: PaneSlot) -> &Cell<Option<i32>> {
        match slot {
            PaneSlot::Primary => &self.primary_keyboard_current,
            PaneSlot::Secondary => &self.secondary_keyboard_current,
            PaneSlot::Tertiary => &self.tertiary_keyboard_current,
        }
    }

    fn current_dir_for(&self, slot: PaneSlot) -> PathBuf {
        self.current_dir_cell(slot).borrow().clone()
    }

    fn current_view_for(&self, slot: PaneSlot) -> PaneView {
        self.current_view_cell(slot).borrow().clone()
    }

    fn tool_scope_dir_for(&self, slot: PaneSlot) -> Option<PathBuf> {
        match self.current_view_for(slot) {
            PaneView::Directory(path) => Some(path),
            PaneView::Triage { root, .. } => Some(root),
            PaneView::Search(query) => Some(query.scope_dir),
            PaneView::ActivityLog => Some(self.current_dir_for(slot)),
            _ => None,
        }
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

    fn view_mode_cell(&self, slot: PaneSlot) -> &Cell<ViewMode> {
        match slot {
            PaneSlot::Primary => &self.primary_view_mode,
            PaneSlot::Secondary => &self.secondary_view_mode,
            PaneSlot::Tertiary => &self.tertiary_view_mode,
        }
    }

    fn show_hidden_cell(&self, slot: PaneSlot) -> &Cell<bool> {
        match slot {
            PaneSlot::Primary => &self.primary_show_hidden,
            PaneSlot::Secondary => &self.secondary_show_hidden,
            PaneSlot::Tertiary => &self.tertiary_show_hidden,
        }
    }

    fn sort_field_cell(&self, slot: PaneSlot) -> &Cell<SortField> {
        match slot {
            PaneSlot::Primary => &self.primary_sort_field,
            PaneSlot::Secondary => &self.secondary_sort_field,
            PaneSlot::Tertiary => &self.tertiary_sort_field,
        }
    }

    fn sort_direction_cell(&self, slot: PaneSlot) -> &Cell<SortDirection> {
        match slot {
            PaneSlot::Primary => &self.primary_sort_direction,
            PaneSlot::Secondary => &self.secondary_sort_direction,
            PaneSlot::Tertiary => &self.tertiary_sort_direction,
        }
    }

    fn set_view_mode(self: &Rc<Self>, slot: PaneSlot, mode: ViewMode) {
        self.view_mode_cell(slot).set(mode);
        let pane = self.pane_widgets(slot);
        pane.file_grid.set_view_mode(mode);
        let icon_name = match mode {
            ViewMode::Icons => "view-grid-symbolic",
            ViewMode::List => "view-list-compact-symbolic",
        };
        pane.view_mode_icon.set_icon_name(Some(icon_name));
    }

    fn set_show_hidden_for_slot(self: &Rc<Self>, slot: PaneSlot, show_hidden: bool) {
        if self.show_hidden_cell(slot).get() == show_hidden {
            self.sync_show_hidden_button_state(slot);
            return;
        }
        self.show_hidden_cell(slot).set(show_hidden);
        self.sync_show_hidden_button_state(slot);
        self.sync_active_tab_state();
        self.load_current_view(slot);
    }

    fn sync_show_hidden_button_state(&self, slot: PaneSlot) {
        let pane = self.pane_widgets(slot);
        let show_hidden = self.show_hidden_cell(slot).get();
        if show_hidden {
            pane.hidden_toggle_btn.add_css_class("pane-control-active");
            pane.hidden_toggle_icon
                .set_icon_name(Some("view-reveal-symbolic"));
        } else {
            pane.hidden_toggle_btn
                .remove_css_class("pane-control-active");
            pane.hidden_toggle_icon
                .set_icon_name(Some("view-reveal-symbolic"));
        }
    }

    fn visible_slots(&self) -> Vec<PaneSlot> {
        [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary]
            .into_iter()
            .filter(|slot| self.pane_layout.get().includes(*slot))
            .collect()
    }

    fn next_visible_slot(&self, slot: PaneSlot) -> PaneSlot {
        let visible = self.visible_slots();
        let current_index = visible
            .iter()
            .position(|candidate| *candidate == slot)
            .unwrap_or(0);
        visible[(current_index + 1) % visible.len()]
    }

    fn sync_active_tab_state(&self) {
        let active_index = self.active_tab.get();
        if let Some(tab) = self.tabs.borrow_mut().get_mut(active_index) {
            tab.primary_dir = self.current_dir.borrow().clone();
            tab.primary_back_history = self.back_history.borrow().clone();
            tab.primary_forward_history = self.forward_history.borrow().clone();
            tab.primary_view = self.current_view.borrow().clone();
            tab.secondary_dir = self.secondary_current_dir.borrow().clone();
            tab.secondary_back_history = self.secondary_back_history.borrow().clone();
            tab.secondary_forward_history = self.secondary_forward_history.borrow().clone();
            tab.secondary_view = self.secondary_view.borrow().clone();
            tab.tertiary_dir = self.tertiary_current_dir.borrow().clone();
            tab.tertiary_back_history = self.tertiary_back_history.borrow().clone();
            tab.tertiary_forward_history = self.tertiary_forward_history.borrow().clone();
            tab.tertiary_view = self.tertiary_view.borrow().clone();
            tab.pane_layout = self.pane_layout.get();
            tab.active_pane = self.active_pane.get();
            tab.primary_view_mode = self.primary_view_mode.get();
            tab.secondary_view_mode = self.secondary_view_mode.get();
            tab.tertiary_view_mode = self.tertiary_view_mode.get();
            tab.primary_show_hidden = self.primary_show_hidden.get();
            tab.secondary_show_hidden = self.secondary_show_hidden.get();
            tab.tertiary_show_hidden = self.tertiary_show_hidden.get();
            tab.primary_sort_field = self.primary_sort_field.get();
            tab.primary_sort_direction = self.primary_sort_direction.get();
            tab.secondary_sort_field = self.secondary_sort_field.get();
            tab.secondary_sort_direction = self.secondary_sort_direction.get();
            tab.tertiary_sort_field = self.tertiary_sort_field.get();
            tab.tertiary_sort_direction = self.tertiary_sort_direction.get();
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
        self.forward_history
            .replace(tab.primary_forward_history.clone());
        self.current_view.replace(tab.primary_view.clone());
        self.secondary_current_dir
            .replace(tab.secondary_dir.clone());
        self.secondary_back_history
            .replace(tab.secondary_back_history.clone());
        self.secondary_forward_history
            .replace(tab.secondary_forward_history.clone());
        self.secondary_view.replace(tab.secondary_view.clone());
        self.tertiary_current_dir.replace(tab.tertiary_dir.clone());
        self.tertiary_back_history
            .replace(tab.tertiary_back_history.clone());
        self.tertiary_forward_history
            .replace(tab.tertiary_forward_history.clone());
        self.tertiary_view.replace(tab.tertiary_view.clone());
        self.pane_layout.set(tab.pane_layout);
        self.active_pane
            .set(if tab.pane_layout.includes(tab.active_pane) {
                tab.active_pane
            } else {
                PaneSlot::Primary
            });

        // Restore per-pane view modes
        self.primary_view_mode.set(tab.primary_view_mode);
        self.secondary_view_mode.set(tab.secondary_view_mode);
        self.tertiary_view_mode.set(tab.tertiary_view_mode);
        self.primary_show_hidden.set(tab.primary_show_hidden);
        self.secondary_show_hidden.set(tab.secondary_show_hidden);
        self.tertiary_show_hidden.set(tab.tertiary_show_hidden);
        self.primary_sort_field.set(tab.primary_sort_field);
        self.primary_sort_direction.set(tab.primary_sort_direction);
        self.secondary_sort_field.set(tab.secondary_sort_field);
        self.secondary_sort_direction.set(tab.secondary_sort_direction);
        self.tertiary_sort_field.set(tab.tertiary_sort_field);
        self.tertiary_sort_direction.set(tab.tertiary_sort_direction);
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            self.set_view_mode(slot, self.view_mode_cell(slot).get());
            self.sync_show_hidden_button_state(slot);
            let icon = match self.sort_direction_cell(slot).get() {
                SortDirection::Ascending => "view-sort-ascending-symbolic",
                SortDirection::Descending => "view-sort-descending-symbolic",
            };
            self.pane_widgets(slot).sort_icon.set_icon_name(Some(icon));
        }

        self.rebuild_tab_strip();
        self.sync_pane_layout_visibility();
        self.update_active_pane_visuals();
        self.update_view_strip(PaneSlot::Primary);
        self.load_current_view(PaneSlot::Primary);
        for slot in [PaneSlot::Secondary, PaneSlot::Tertiary] {
            if self.pane_layout.get().includes(slot) {
                self.update_view_strip(slot);
                self.load_current_view(slot);
            } else {
                self.pane_widgets(slot).file_grid.clear_selection();
                self.reset_keyboard_state(slot);
            }
        }
    }

    fn update_active_pane_visuals(&self) {
        let active = self.active_slot();
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
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

    fn sync_pane_layout_visibility(&self) {
        let layout = self.pane_layout.get();

        // Snapshot old state BEFORE changing anything so we can detect transitions.
        let was_single = !self.right_paned.get_visible();
        let was_two = self.right_paned.get_visible() && !self.tertiary_pane.root.get_visible();

        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            let pane = self.pane_widgets(slot);
            let visible = layout.includes(slot);
            pane.root.set_visible(visible);
            pane.path_label.set_visible(visible);
            pane.view_strip
                .set_visible(visible && pane.view_strip.is_visible());
        }

        let now_split = layout != PaneLayout::Single;
        self.right_paned.set_visible(now_split);

        // After the next layout cycle the Paned's allocated width is known.
        // We use it to set an explicit equal-halves divider so the new pane
        // actually gets space instead of inheriting a stale collapsed position.
        if was_single && now_split {
            let sp = self.split_paned.clone();
            glib::idle_add_local_once(move || {
                let w = sp.width();
                if w > 0 {
                    sp.set_position(w / 2);
                }
            });
        }
        if was_two && layout == PaneLayout::Three {
            let rp = self.right_paned.clone();
            glib::idle_add_local_once(move || {
                let w = rp.width();
                if w > 0 {
                    rp.set_position(w / 2);
                }
            });
        }

        self.update_split_button_state();
    }

    fn update_split_button_state(&self) {
        let (icon, tooltip) = match self.pane_layout.get() {
            PaneLayout::Single => ("view-list-symbolic", "Switch to 2 panels (Ctrl+\\)"),
            PaneLayout::Two => ("view-dual-symbolic", "Switch to 3 panels (Ctrl+\\)"),
            PaneLayout::Three => ("view-grid-symbolic", "Switch to 1 panel (Ctrl+\\)"),
        };
        self.toolbar.set_split_icon_state(icon);
        self.toolbar.split_tooltip_label.set_label(tooltip);
    }

    fn cycle_pane_layout(self: &Rc<Self>) {
        let next = self.pane_layout.get().next();
        self.set_pane_layout(next);
    }

    fn set_pane_layout(self: &Rc<Self>, layout: PaneLayout) {
        if self.pane_layout.get() == layout {
            return;
        }

        let previous = self.pane_layout.get();
        if previous == PaneLayout::Two && layout == PaneLayout::Three {
            let source = self.active_slot();
            self.tertiary_current_dir
                .replace(self.current_dir_for(source));
            self.tertiary_view.replace(self.current_view_for(source));
            self.tertiary_back_history
                .replace(self.back_history_cell(source).borrow().clone());
        }

        self.pane_layout.set(layout);
        if !layout.includes(self.active_slot()) {
            self.active_pane.set(PaneSlot::Primary);
        }

        self.sync_active_tab_state();
        self.rebuild_tab_strip();
        self.sync_pane_layout_visibility();
        self.update_active_pane_visuals();

        for slot in [PaneSlot::Secondary, PaneSlot::Tertiary] {
            if layout.includes(slot) {
                self.update_view_strip(slot);
                self.load_current_view(slot);
            } else {
                self.pane_widgets(slot).file_grid.clear_selection();
                self.reset_keyboard_state(slot);
            }
        }

        self.update_navigation_state();
        self.update_sidebar_state();
        self.sync_path_entry_to_display();
        self.update_selection();
    }

    fn set_active_pane(self: &Rc<Self>, slot: PaneSlot) {
        let target = if self.pane_layout.get().includes(slot) {
            slot
        } else {
            PaneSlot::Primary
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
                self.forward_history_cell(slot).borrow_mut().clear();
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

    fn navigate_to_active(self: &Rc<Self>, path: PathBuf) {
        self.navigate_to(self.active_slot(), path, true);
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
            let current = self.current_dir_for(slot);
            self.forward_history_cell(slot).borrow_mut().push(current);
            self.current_dir_cell(slot).replace(path.clone());
            self.current_view_cell(slot)
                .replace(PaneView::Directory(path.clone()));
            self.sync_active_tab_state();
            self.update_view_strip(slot);
            if slot == PaneSlot::Primary {
                self.rebuild_tab_strip();
            }
            self.load_directory(slot, path);
            self.update_navigation_state();
        }
    }

    fn go_forward(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !self.is_directory_view(slot) {
            self.status
                .set_message("Forward is only available in directory views.");
            return;
        }
        let next = self.forward_history_cell(slot).borrow_mut().pop();
        if let Some(path) = next {
            let current = self.current_dir_for(slot);
            self.back_history_cell(slot).borrow_mut().push(current);
            self.current_dir_cell(slot).replace(path.clone());
            self.current_view_cell(slot)
                .replace(PaneView::Directory(path.clone()));
            self.sync_active_tab_state();
            self.update_view_strip(slot);
            if slot == PaneSlot::Primary {
                self.rebuild_tab_strip();
            }
            self.load_directory(slot, path);
            self.update_navigation_state();
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
        self.reload_visible_panes();
    }

    fn reload_visible_panes(self: &Rc<Self>) {
        for slot in self.visible_slots() {
            self.load_current_view(slot);
        }
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
        if visible {
            self.sidebar_revealer.set_visible(true);
            self.sidebar_revealer.set_reveal_child(true);
        } else {
            self.sidebar_revealer.set_reveal_child(false);
            let revealer = self.sidebar_revealer.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(230), move || {
                if !revealer.reveals_child() {
                    revealer.set_visible(false);
                }
            });
        }
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
        if visible {
            self.preview_revealer.set_visible(true);
            self.preview_revealer.set_reveal_child(true);
            self.refresh_preview();
        } else {
            self.cancel_active_preview();
            self.preview_revealer.set_reveal_child(false);
            let revealer = self.preview_revealer.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(230), move || {
                if !revealer.reveals_child() {
                    revealer.set_visible(false);
                }
            });
        }
    }

    fn set_preview_visible(self: &Rc<Self>, visible: bool) {
        self.apply_preview_visibility(visible);
    }

    fn set_holding_tray_visible(self: &Rc<Self>, visible: bool) {
        if visible {
            self.holding_tray.root.set_visible(true);
            self.holding_tray.root.set_reveal_child(true);
        } else {
            self.holding_tray.root.set_reveal_child(false);
            let revealer = self.holding_tray.root.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(210), move || {
                if !revealer.reveals_child() {
                    revealer.set_visible(false);
                }
            });
        }
    }

    // ── Action Planning Mode ─────────────────────────────────────────────────

    fn set_plan_mode(self: &Rc<Self>, active: bool) {
        self.plan_mode_active.set(active);
        if self.toolbar.plan_mode_toggle.is_active() != active {
            self.toolbar.plan_mode_toggle.set_active(active);
        }
        self.refresh_plan_queue_panel();
        if active {
            self.status
                .set_message("Plan mode ON — operations will queue instead of executing.");
        } else if self.action_queue.borrow().is_empty() {
            self.status.set_message("Plan mode OFF.");
        }
    }

    fn queue_plan(self: &Rc<Self>, plan: crate::action_plan::ActionPlan) {
        self.action_queue.borrow_mut().push(plan);
        self.refresh_plan_queue_panel();
        let n = self.action_queue.borrow().len();
        self.status.set_message(&format!(
            "{n} action{} queued — execute or clear in the plan queue panel.",
            if n == 1 { "" } else { "s" }
        ));
    }

    fn refresh_plan_queue_panel(self: &Rc<Self>) {
        let items = self.action_queue.borrow().clone();
        let show = self.plan_mode_active.get() || !items.is_empty();

        let controller = Rc::clone(self);
        self.plan_queue_panel
            .set_items(&items, move |action| match action {
                QueueAction::MoveUp(i) => {
                    let mut q = controller.action_queue.borrow_mut();
                    if i > 0 {
                        q.swap(i - 1, i);
                    }
                    drop(q);
                    controller.refresh_plan_queue_panel();
                }
                QueueAction::MoveDown(i) => {
                    let mut q = controller.action_queue.borrow_mut();
                    if i + 1 < q.len() {
                        q.swap(i, i + 1);
                    }
                    drop(q);
                    controller.refresh_plan_queue_panel();
                }
                QueueAction::Remove(i) => {
                    controller.action_queue.borrow_mut().remove(i);
                    controller.refresh_plan_queue_panel();
                }
            });

        if show {
            self.plan_queue_panel.root.set_visible(true);
            self.plan_queue_panel.root.set_reveal_child(true);
        } else {
            self.plan_queue_panel.root.set_reveal_child(false);
            let revealer = self.plan_queue_panel.root.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(230), move || {
                if !revealer.reveals_child() {
                    revealer.set_visible(false);
                }
            });
        }
    }

    fn execute_plan_queue(self: &Rc<Self>) {
        let queue: Vec<_> = self.action_queue.borrow_mut().drain(..).collect();
        self.refresh_plan_queue_panel();
        if queue.is_empty() {
            return;
        }
        self.status
            .set_message(&format!("Executing {} queued action(s)…", queue.len()));
        for plan in queue {
            match plan.kind {
                crate::action_plan::OpKind::Trash => {
                    self.move_paths_to_trash(plan.sources);
                }
                crate::action_plan::OpKind::Move => {
                    if let Some(dest) = plan.destination {
                        self.start_copy_move_with_conflict_check(
                            plan.sources,
                            dest,
                            false,
                            plan.summary,
                            None,
                        );
                    }
                }
                crate::action_plan::OpKind::Copy => {
                    if let Some(dest) = plan.destination {
                        self.start_copy_move_with_conflict_check(
                            plan.sources,
                            dest,
                            true,
                            plan.summary,
                            None,
                        );
                    }
                }
                crate::action_plan::OpKind::Rename => {
                    if let (Some(src), Some(new_name)) =
                        (plan.sources.first(), plan.file_list.first())
                    {
                        self.rename_path(src.clone(), new_name.clone());
                    }
                }
                crate::action_plan::OpKind::BulkRename => {
                    let renames: Vec<(PathBuf, String)> = plan
                        .sources
                        .into_iter()
                        .zip(plan.file_list.into_iter())
                        .collect();
                    if !renames.is_empty() {
                        self.apply_bulk_rename(renames);
                    }
                }
                crate::action_plan::OpKind::Duplicate => {
                    self.do_duplicate_files(plan.sources);
                }
                crate::action_plan::OpKind::PermanentDelete => {
                    self.delete_items_permanently(plan.sources);
                }
                crate::action_plan::OpKind::NewFolder => {
                    if let (Some(parent), Some(name)) = (plan.destination, plan.file_list.first()) {
                        self.exec_create_folder(parent, name.clone());
                    }
                }
                crate::action_plan::OpKind::NewFile => {
                    if let (Some(parent), Some(name)) = (plan.destination, plan.file_list.first()) {
                        self.exec_create_text_document(self.active_slot(), parent, name.clone());
                    }
                }
                crate::action_plan::OpKind::SendToProject { is_copy } => {
                    if let Some(dest) = plan.destination {
                        let items = plan_copy_move_items(&plan.sources, &dest);
                        if !items.is_empty() {
                            self.start_copy_move_op(items, is_copy, &plan.summary, None, None);
                        }
                    }
                }
            }
        }
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
                        let items = collected.borrow().clone();
                        controller.finish_load(slot, generation, &path, items);
                    }
                    Ok(batch) => {
                        let show_hidden = controller.show_hidden_cell(slot).get();
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
        sort_items_with(
            &mut items,
            self.sort_field_cell(slot).get(),
            self.sort_direction_cell(slot).get(),
        );
        // Store full item list before any view-specific filtering so filters can re-apply
        self.all_items_cell(slot).replace(items.clone());
        if let PaneView::Triage { filter, .. } = self.current_view_for(slot) {
            {
                let dup_set = self.duplicate_set_cell(slot).borrow();
                items = filter_triage_items(items, filter, dup_set.as_ref());
            }
            if filter == TriageFilter::Duplicates
                && self.duplicate_set_cell(slot).borrow().is_none()
                && !self.duplicate_scan_pending_for(slot)
            {
                self.start_duplicate_scan(slot, path.to_path_buf());
            }
        }
        if matches!(self.current_view_for(slot), PaneView::Directory(_)) {
            if let Err(error) = self.metadata.borrow_mut().record_recent_location(path) {
                eprintln!("Lattice recent-location update failed: {error}");
            }
        }
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
                PaneView::Triage { .. } => "Unable to read this folder.",
                PaneView::SystemDrives => "Unable to read mounted volumes.",
                PaneView::Recent => "Unable to read recent folders.",
                PaneView::Trash => "Unable to read Trash.",
                PaneView::Search(_) => "Search failed.",
                PaneView::ActivityLog => "Unable to load activity log.",
                PaneView::ProjectLanding(_) | PaneView::TagManager => "",
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

    fn load_triage(self: &Rc<Self>, slot: PaneSlot, root: &Path) {
        self.current_dir_cell(slot).replace(root.to_path_buf());
        self.load_directory(slot, root.to_path_buf());
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
                        let items = collected.borrow().clone();
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

    fn finish_trash_load(self: &Rc<Self>, slot: PaneSlot, generation: u64, mut items: Vec<FileItem>) {
        if !self.is_current_load(slot, generation) {
            return;
        }

        self.load_cancellable_cell(slot).borrow_mut().take();
        sort_items_with(
            &mut items,
            self.sort_field_cell(slot).get(),
            self.sort_direction_cell(slot).get(),
        );
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
            self.reload_visible_panes();
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

    fn empty_trash(self: &Rc<Self>) {
        let slot = self.active_slot();
        if !matches!(self.current_view_for(slot), PaneView::Trash) {
            return;
        }
        let item_count = self.items_cell(slot).borrow().len();
        if item_count == 0 {
            return;
        }
        let prompt = if item_count == 1 {
            "Permanently delete 1 item from Trash? This cannot be undone.".to_string()
        } else {
            format!("Permanently delete {item_count} items from Trash? This cannot be undone.")
        };
        let controller = Rc::clone(self);
        self.modal_host.show_confirm(
            "Empty Trash",
            &prompt,
            "Empty Trash",
            true,
            false,
            move || controller.do_empty_trash(),
        );
    }

    fn do_empty_trash(self: &Rc<Self>) {
        let cancellable = gio::Cancellable::new();
        let op_id = self
            .ops_panel
            .add_op("Empty Trash", Some(cancellable.clone()));
        let trash = gio::File::for_uri("trash:///");
        let controller = Rc::clone(self);
        let cancellable_clone = cancellable.clone();
        let trash_for_cb = trash.clone();
        trash.enumerate_children_async(
            "standard::name",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| match result {
                Ok(enumerator) => {
                    controller.collect_trash_children_then_delete(
                        op_id,
                        trash_for_cb,
                        enumerator,
                        cancellable_clone,
                        Rc::new(RefCell::new(Vec::new())),
                    );
                }
                Err(e) => {
                    controller
                        .ops_panel
                        .finish_op(op_id, &[e.message().to_string()]);
                    controller.reload_visible_panes();
                }
            },
        );
    }

    fn collect_trash_children_then_delete(
        self: &Rc<Self>,
        op_id: OpId,
        trash: gio::File,
        enumerator: gio::FileEnumerator,
        cancellable: gio::Cancellable,
        children: Rc<RefCell<Vec<gio::File>>>,
    ) {
        let controller = Rc::clone(self);
        let trash_clone = trash.clone();
        let enumerator_clone = enumerator.clone();
        let cancellable_clone = cancellable.clone();
        let children_clone = Rc::clone(&children);
        enumerator.next_files_async(
            64,
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| match result {
                Ok(batch) if batch.is_empty() => {
                    controller.delete_trash_batch(
                        op_id,
                        children_clone,
                        0,
                        cancellable_clone,
                        Rc::new(RefCell::new(Vec::new())),
                    );
                }
                Ok(batch) => {
                    {
                        let mut c = children_clone.borrow_mut();
                        for info in &batch {
                            c.push(trash_clone.child(info.name()));
                        }
                    }
                    controller.collect_trash_children_then_delete(
                        op_id,
                        trash_clone,
                        enumerator_clone,
                        cancellable_clone,
                        children_clone,
                    );
                }
                Err(e) => {
                    if e.kind::<gio::IOErrorEnum>() != Some(gio::IOErrorEnum::Cancelled) {
                        controller
                            .ops_panel
                            .finish_op(op_id, &[e.message().to_string()]);
                    } else {
                        controller.ops_panel.finish_op(op_id, &[]);
                    }
                    controller.reload_visible_panes();
                }
            },
        );
    }

    fn delete_trash_batch(
        self: &Rc<Self>,
        op_id: OpId,
        files: Rc<RefCell<Vec<gio::File>>>,
        index: usize,
        cancellable: gio::Cancellable,
        errors: Rc<RefCell<Vec<String>>>,
    ) {
        let total = files.borrow().len();
        if index >= total {
            let errs = errors.borrow().clone();
            self.ops_panel.finish_op(op_id, &errs);
            self.reload_visible_panes();
            return;
        }
        if cancellable.is_cancelled() {
            self.ops_panel.finish_op(op_id, &[]);
            self.reload_visible_panes();
            return;
        }
        self.ops_panel
            .update_progress(op_id, index as f64 / total as f64, "");
        let file = files.borrow()[index].clone();
        let controller = Rc::clone(self);
        let files_clone = Rc::clone(&files);
        let errors_clone = Rc::clone(&errors);
        let cancellable_clone = cancellable.clone();
        file.delete_async(glib::Priority::DEFAULT, Some(&cancellable), move |result| {
            if let Err(ref e) = result {
                if e.kind::<gio::IOErrorEnum>() != Some(gio::IOErrorEnum::Cancelled) {
                    errors_clone.borrow_mut().push(e.message().to_string());
                }
            }
            controller.delete_trash_batch(
                op_id,
                files_clone,
                index + 1,
                cancellable_clone,
                errors_clone,
            );
        });
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
        self.pane_widgets(slot).file_grid.grab_focus_on_active();
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
        let Some(scope) = self.tool_scope_dir_for(slot) else {
            self.status
                .set_message("Search is only available in folder views.");
            return;
        };
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

        if query.is_unconstrained() {
            pane.file_grid
                .set_empty_message("Type a name or choose filters to search.");
            if slot == self.active_slot() {
                self.show_empty_selection_preview(slot, &display_label, 0);
                self.status.set_counts(0, 0);
                self.refresh_preview();
            }
            return;
        }

        let generation = self.load_generation_cell(slot).get() + 1;
        self.load_generation_cell(slot).set(generation);
        let search_cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel_cell(slot)
            .replace(Some(Arc::clone(&search_cancel)));

        let controller = Rc::clone(self);
        let show_hidden = self.show_hidden_cell(slot).get();
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
                    &search_cancel,
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
        self.search_cancel_cell(slot).borrow_mut().take();
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
        self.connect_search_panel(PaneSlot::Tertiary);
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
            let items = collected.borrow().clone();
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
                    if controller.show_hidden_cell(slot).get() || !info.is_hidden() {
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
        let mut items = self.enrich_items_with_tags(items);
        sort_items_with(
            &mut items,
            self.sort_field_cell(slot).get(),
            self.sort_direction_cell(slot).get(),
        );
        self.all_items_cell(slot).replace(items.clone());
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
            PaneView::Triage { root, filter } => {
                let folder_name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                self.preview.show_folder(
                    &format!("{} — {}", folder_name, filter.label()),
                    display_label,
                    None,
                    Some(item_count),
                    "Triage",
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
            PaneView::ActivityLog => {
                self.preview
                    .show_folder("Activity Log", display_label, None, None, "File History");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::ProjectLanding(_) => {
                self.preview
                    .show_folder("Project", display_label, None, None, "Project");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::TagManager => {
                self.preview
                    .show_folder("Tags", display_label, None, None, "Tag Manager");
                self.preview.set_action_state(false, false, false);
            }
        }
    }

    fn attach_context_handlers(self: &Rc<Self>, slot: PaneSlot) {
        let _ = slot;
        // Right-click is handled once at the FlowBox/ListBox container level in
        // `connect_pane()`. Keeping it there avoids GTK selection/focus races
        // between row wrappers and child widgets.
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
            if self.pane_layout.get() != PaneLayout::Single {
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
            PaneView::Directory(_) | PaneView::Triage { .. }
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
                    "triage_folder",
                    "separator",
                    "add_to_holding_tray",
                    "separator",
                    "rename",
                    "bulk_rename",
                    "duplicate",
                    "copy_path",
                    "terminal_here",
                    "separator",
                    "pin_place",
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
                    "add_to_holding_tray",
                    "separator",
                    "rename",
                    "bulk_rename",
                    "duplicate",
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
                    "pin_place",
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
                "open_in_pane" if item.is_dir && self.pane_layout.get() != PaneLayout::Single => {
                    append_menu_button(
                        menu_box,
                        "Open in Other Pane",
                        Some("window-new-symbolic"),
                        false,
                        {
                            let controller = Rc::clone(self);
                            let item = item.clone();
                            move || controller.open_folder_in_other_pane(slot, item.path.clone())
                        },
                    )
                }
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
                "triage_folder" if item.is_dir => {
                    append_menu_button(menu_box, "🧹 Triage This Folder", None, false, {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.open_triage(item.path.clone(), TriageFilter::All)
                    })
                }
                "rename" => {
                    append_menu_button(menu_box, "Rename", Some("document-edit-symbolic"), false, {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.show_rename_dialog(item.path.clone(), item.name.clone())
                    })
                }
                "bulk_rename" => {
                    let selected = self.selected_items_for(slot);
                    if selected.len() >= 2 {
                        append_menu_button(
                            menu_box,
                            "Bulk Rename\u{2026}",
                            Some("document-edit-symbolic"),
                            false,
                            {
                                let controller = Rc::clone(self);
                                move || controller.show_bulk_rename_dialog(selected.clone())
                            },
                        )
                    }
                }
                "duplicate" => {
                    append_menu_button(menu_box, "Duplicate", Some("edit-copy-symbolic"), false, {
                        let controller = Rc::clone(self);
                        move || controller.duplicate_selected()
                    })
                }
                "copy_path" => {
                    append_menu_button(menu_box, "Copy Path", Some("edit-copy-symbolic"), false, {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.copy_paths_to_clipboard(vec![item.path.clone()])
                    })
                }
                "add_to_holding_tray" => append_menu_button(
                    menu_box,
                    "Add to Holding Tray",
                    Some("mail-attachment-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.add_selection_to_holding_tray(slot)
                    },
                ),
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
                "pin_place" if item.is_dir => append_menu_button(
                    menu_box,
                    "Pin to Places",
                    Some("bookmark-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.pin_place(item.path.clone())
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
                "pin_place" => append_menu_button(
                    menu_box,
                    "Pin to Places",
                    Some("bookmark-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.pin_place(controller.current_dir_for(slot))
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

    fn add_selection_to_holding_tray(self: &Rc<Self>, slot: PaneSlot) {
        let items = self.selected_items_for(slot);
        if items.is_empty() {
            self.status
                .set_message("Select one or more items before adding to the Holding Tray.");
            return;
        }

        let added = add_unique_tray_items(&mut self.holding_tray_items.borrow_mut(), items);
        self.refresh_holding_tray();
        self.toolbar.holding_tray_toggle.set_active(true);
        self.status.set_message(&format!(
            "Added {added} item{} to the Holding Tray.",
            if added == 1 { "" } else { "s" }
        ));
    }

    fn add_paths_to_holding_tray(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let items = paths
            .into_iter()
            .filter_map(|path| self.file_item_for_known_or_local_path(&path))
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.status
                .set_message("No readable local files were available to stage.");
            return;
        }

        let added = add_unique_tray_items(&mut self.holding_tray_items.borrow_mut(), items);
        self.refresh_holding_tray();
        self.toolbar.holding_tray_toggle.set_active(true);
        self.status.set_message(&format!(
            "Added {added} item{} to the Holding Tray.",
            if added == 1 { "" } else { "s" }
        ));
    }

    fn paste_file_clipboard_into_holding_tray(self: &Rc<Self>) {
        let Some(clipboard) = self.file_clipboard.borrow().clone() else {
            self.status
                .set_message("Nothing is waiting to be added to the Holding Tray.");
            return;
        };
        self.add_paths_to_holding_tray(clipboard.paths);
    }

    fn file_item_for_known_or_local_path(&self, path: &Path) -> Option<FileItem> {
        for cell in [&self.items, &self.secondary_items, &self.tertiary_items] {
            if let Some(item) = cell.borrow().iter().find(|item| item.path == path).cloned() {
                return Some(item);
            }
        }

        let file = gio::File::for_path(path);
        let info = file
            .query_info(
                DIRECTORY_ATTRIBUTES,
                gio::FileQueryInfoFlags::NONE,
                None::<&gio::Cancellable>,
            )
            .ok()?;
        FileItem::from_info(
            &path.parent().map(gio::File::for_path)?,
            &info,
            self.show_hidden_cell(self.active_slot()).get(),
        )
    }

    fn refresh_holding_tray(self: &Rc<Self>) {
        let items = self.holding_tray_items.borrow().clone();
        let controller = Rc::clone(self);
        let selected = self.holding_tray_selection.borrow().clone();
        let select_controller = Rc::clone(self);
        let open_controller = Rc::clone(self);
        self.holding_tray.set_items(
            &items,
            &selected,
            move |path| controller.remove_holding_tray_path(&path),
            move |path| select_controller.select_holding_tray_path(path),
            move |path| open_controller.open_holding_tray_path(path),
        );
        self.holding_tray_thumb_loader.cancel();
        self.holding_tray_thumb_loader
            .submit(self.holding_tray.drain_thumb_targets());
    }

    fn remove_holding_tray_path(self: &Rc<Self>, path: &Path) {
        self.holding_tray_items
            .borrow_mut()
            .retain(|item| item.path != path);
        self.holding_tray_selection
            .borrow_mut()
            .retain(|selected| selected != path);
        self.refresh_holding_tray();
        self.status
            .set_message("Removed item from the Holding Tray. The file was not deleted.");
    }

    fn remove_holding_tray_paths(self: &Rc<Self>, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        self.holding_tray_items
            .borrow_mut()
            .retain(|item| !paths.iter().any(|path| path == &item.path));
        self.holding_tray_selection
            .borrow_mut()
            .retain(|item| !paths.iter().any(|path| path == item));
        self.refresh_holding_tray();
    }

    fn clear_holding_tray(self: &Rc<Self>) {
        self.holding_tray_items.borrow_mut().clear();
        self.holding_tray_selection.borrow_mut().clear();
        self.refresh_holding_tray();
        self.status
            .set_message("Holding Tray cleared. No files were deleted.");
    }

    fn holding_tray_paths(&self) -> Vec<PathBuf> {
        self.holding_tray_items
            .borrow()
            .iter()
            .map(|item| item.path.clone())
            .collect()
    }

    fn selected_holding_tray_paths(&self) -> Vec<PathBuf> {
        let selection = self.holding_tray_selection.borrow();
        if selection.is_empty() {
            return self.holding_tray_paths();
        }
        let items = self.holding_tray_items.borrow();
        selection
            .iter()
            .filter(|path| items.iter().any(|item| &item.path == *path))
            .cloned()
            .collect()
    }

    fn select_holding_tray_path(self: &Rc<Self>, path: PathBuf) {
        self.holding_tray_selection.replace(vec![path]);
        self.refresh_holding_tray();
    }

    fn clear_holding_tray_selection(self: &Rc<Self>) {
        if self.holding_tray_selection.borrow().is_empty() {
            return;
        }
        self.holding_tray_selection.borrow_mut().clear();
        self.refresh_holding_tray();
        self.status.set_message("Holding Tray selection cleared.");
    }

    fn remove_selected_holding_tray_items(self: &Rc<Self>) {
        let paths = self.holding_tray_selection.borrow().clone();
        if paths.is_empty() {
            self.status
                .set_message("Select a staged item before removing it from the tray.");
            return;
        }
        self.remove_holding_tray_paths(&paths);
        self.status.set_message(&format!(
            "Removed {} staged item{} from the Holding Tray. No files were deleted.",
            paths.len(),
            if paths.len() == 1 { "" } else { "s" }
        ));
    }

    fn open_selected_holding_tray_item(self: &Rc<Self>) {
        let Some(path) = self.holding_tray_selection.borrow().first().cloned() else {
            self.status
                .set_message("Select a staged item before opening it.");
            return;
        };
        self.open_holding_tray_path(path);
    }

    fn open_holding_tray_path(self: &Rc<Self>, path: PathBuf) {
        if let Some(item) = self
            .holding_tray_items
            .borrow()
            .iter()
            .find(|item| item.path == path)
            .cloned()
        {
            self.open_item(&item);
        }
    }

    fn show_tray_project_dialog(self: &Rc<Self>, action: TrayProjectAction) {
        let paths = self.holding_tray_paths();
        if paths.is_empty() {
            self.status.set_message("The Holding Tray is empty.");
            return;
        }

        let projects = self.projects.borrow().clone();
        if projects.is_empty() {
            self.modal_host.show_error(
                "No Projects Yet",
                "Pin a folder as a project first, then send tray items to it.",
            );
            return;
        }

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.append(&build_modal_prompt(
            "Choose the project that should receive the staged tray items.",
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
                (
                    project.id,
                    project.name.clone(),
                    project.root_path.clone(),
                    button,
                )
            })
            .collect::<Vec<_>>();

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let controller = Rc::clone(self);
        let plan_btn = build_modal_button("Preview", ButtonKind::Primary, move || {
            if let Some((project_id, project_name, project_root, _)) = project_buttons
                .iter()
                .find(|(_, _, _, button)| button.is_active())
            {
                let plan = project_action_plan(action, &paths, project_name, project_root);
                let project_id = *project_id;
                let controller_for_plan = Rc::clone(&controller);
                let paths_for_plan = paths.clone();
                controller.show_action_plan(plan, move || {
                    controller_for_plan.send_paths_to_project(
                        paths_for_plan.clone(),
                        project_id,
                        action.transfer_kind(),
                        Some(TrayCompletion {
                            action: action.title().to_string(),
                            clear_successful_paths: action == TrayProjectAction::Move,
                        }),
                    );
                });
            }
            host.hide();
        });
        actions.append(&plan_btn);

        self.modal_host
            .show_with_custom_ui(action.title(), &content, &actions, false, None);
    }

    fn show_tray_tag_preview(self: &Rc<Self>) {
        let paths = self.holding_tray_paths();
        if paths.is_empty() {
            self.status.set_message("The Holding Tray is empty.");
            return;
        }

        let controller = Rc::clone(self);
        self.modal_host.show_input(
            "Tag Holding Tray",
            "Type a tag name to apply to all staged items. Existing names will be reused.",
            "",
            "Preview",
            move |tag_name| {
                let tag_name = tag_name.trim().to_string();
                if tag_name.is_empty() {
                    controller.status.set_message("Tag name cannot be empty.");
                    return;
                }
                let plan = tag_action_plan(&paths, &tag_name);
                let controller_for_plan = Rc::clone(&controller);
                let paths_for_plan = paths.clone();
                controller.show_action_plan(plan, move || {
                    controller_for_plan
                        .apply_tag_to_tray_paths(paths_for_plan.clone(), tag_name.clone());
                });
            },
        );
    }

    fn show_tray_trash_preview(self: &Rc<Self>) {
        let paths = self.holding_tray_paths();
        if paths.is_empty() {
            self.status.set_message("The Holding Tray is empty.");
            return;
        }
        let plan = trash_action_plan(&paths);
        let controller = Rc::clone(self);
        self.show_action_plan(plan, move || {
            controller.move_tray_paths_to_trash(paths.clone());
        });
    }

    fn copy_holding_tray_paths(self: &Rc<Self>) {
        let paths = self.selected_holding_tray_paths();
        if paths.is_empty() {
            self.status.set_message("The Holding Tray is empty.");
            return;
        }
        let plan = copy_path_action_plan(&paths);
        let controller = Rc::clone(self);
        self.show_action_plan(plan, move || {
            controller.copy_paths_to_clipboard(paths.clone());
            controller.record_tray_receipt("Copy Tray Paths", paths.len(), 0);
        });
    }

    fn show_action_plan<F>(self: &Rc<Self>, plan: ActionPlan, on_accept: F)
    where
        F: Fn() + 'static,
    {
        let content = GtkBox::new(Orientation::Vertical, 8);
        for line in &plan.lines {
            let label = Label::new(Some(line));
            label.add_css_class("dialog-prompt");
            label.set_halign(Align::Start);
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            content.append(&label);
        }

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let kind = if plan.destructive {
            ButtonKind::Danger
        } else {
            ButtonKind::Primary
        };
        let accept_btn = build_modal_button(&plan.confirm_label, kind, move || {
            on_accept();
            host.hide();
        });
        actions.append(&accept_btn);

        self.modal_host
            .show_with_custom_ui(&plan.title, &content, &actions, false, None);
    }

    fn apply_tag_to_tray_paths(self: &Rc<Self>, paths: Vec<PathBuf>, tag_name: String) {
        let result = {
            let mut metadata = self.metadata.borrow_mut();
            let tag = metadata.ensure_tag(&tag_name);
            tag.and_then(|tag| {
                metadata.add_tag_to_paths(tag.id, &paths)?;
                Ok(tag)
            })
        };

        match result {
            Ok(tag) => {
                self.refresh_metadata_sidebar();
                self.status
                    .set_message(&format!("Applied tag #{} to tray items.", tag.name));
                self.record_tray_receipt("Tag Holding Tray", paths.len(), 0);
                self.refresh();
            }
            Err(error) => {
                self.record_tray_receipt("Tag Holding Tray", 0, paths.len());
                self.show_error_dialog("Tag Failed", &error);
            }
        }
    }

    fn move_tray_paths_to_trash(self: &Rc<Self>, paths: Vec<PathBuf>) {
        self.move_paths_to_trash_with_completion(
            paths,
            Some(TrayCompletion {
                action: "Move Tray to Trash".to_string(),
                clear_successful_paths: true,
            }),
        );
    }

    fn record_tray_receipt(&self, action: &str, success_count: usize, failure_count: usize) {
        let detail = format!("{} succeeded · {} failed", success_count, failure_count);
        self.ops_panel
            .add_receipt(action, &detail, failure_count > 0);
        let errors = if failure_count > 0 {
            vec![format!("{failure_count} staged item(s) failed.")]
        } else {
            Vec::new()
        };
        let _ = self.metadata.borrow().log_activity(
            "holding_tray",
            success_count as i32,
            action,
            "Holding Tray",
            None,
            &errors,
        );
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

        let in_trash = matches!(self.current_view_for(slot), PaneView::Trash);
        self.toolbar.empty_trash_host.set_visible(in_trash);
        if in_trash {
            let has_items = !self.items_cell(slot).borrow().is_empty();
            self.toolbar.empty_trash_button.set_sensitive(has_items);
        }
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
        self.set_pane_layout(PaneLayout::Two);
        self.set_active_pane(PaneSlot::Secondary);
        self.navigate_to(PaneSlot::Secondary, path, true);
    }

    fn open_folder_in_other_pane(self: &Rc<Self>, slot: PaneSlot, path: PathBuf) {
        if self.pane_layout.get() == PaneLayout::Single {
            self.open_folder_in_split(path);
            return;
        }

        let target = self.next_visible_slot(slot);
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
        self.wire_tag_filter_for_slot(PaneSlot::Tertiary);
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
        self.set_filter_panel_open_for_slot(slot, open);
    }

    fn set_filter_panel_open_for_slot(self: &Rc<Self>, slot: PaneSlot, open: bool) {
        let revealer = self.pane_widgets(slot).tag_filter_revealer.clone();
        if open {
            revealer.set_visible(true);
            revealer.set_reveal_child(true);
        } else {
            revealer.set_reveal_child(false);
            glib::timeout_add_local_once(std::time::Duration::from_millis(190), move || {
                if !revealer.reveals_child() {
                    revealer.set_visible(false);
                }
            });
        }
        self.sync_filter_button_state(slot);
    }

    fn sync_filter_button_state(&self, slot: PaneSlot) {
        let pane = self.pane_widgets(slot);
        let count = pane.tag_filter.active_count();
        let open = pane.tag_filter_revealer.reveals_child();
        if count > 0 || open {
            pane.filter_toggle_btn.add_css_class("pane-control-active");
        } else {
            pane.filter_toggle_btn
                .remove_css_class("pane-control-active");
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
        if self.plan_mode_active.get() {
            self.queue_plan(FileOpPlan::for_bulk_rename(&renames));
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
            let activity_items: Vec<(PathBuf, Option<PathBuf>)> = renames
                .iter()
                .map(|(old_path, new_name)| {
                    let new_path = old_path
                        .parent()
                        .map(|parent| parent.join(new_name))
                        .unwrap_or_else(|| PathBuf::from(new_name));
                    (old_path.clone(), Some(new_path))
                })
                .collect();
            let source = renames
                .first()
                .and_then(|(path, _)| path.parent())
                .and_then(|path| path.to_str())
                .unwrap_or("");
            let _ = self.metadata.borrow().log_activity_with_items(
                "bulk_rename",
                renames.len() as i32,
                &format!(
                    "Renamed {} file{}",
                    renames.len(),
                    if renames.len() == 1 { "" } else { "s" }
                ),
                source,
                None,
                &failures,
                &activity_items,
            );
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

        if self.plan_mode_active.get() {
            let plan = FileOpPlan::for_rename(&path, &new_name);
            self.queue_plan(plan);
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
                    let source = path
                        .parent()
                        .and_then(|parent| parent.to_str())
                        .unwrap_or("");
                    let _ = controller.metadata.borrow().log_activity_with_items(
                        "rename",
                        1,
                        "Renamed item",
                        source,
                        new_path.parent().and_then(|parent| parent.to_str()),
                        &[],
                        &[(path.clone(), Some(new_path.clone()))],
                    );
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

    // ── Duplicate ─────────────────────────────────────────────────────────────

    fn duplicate_selected(self: &Rc<Self>) {
        let slot = self.active_slot();
        let paths = self.selected_paths_for(slot);
        if paths.is_empty() {
            self.status.set_message("Select files to duplicate.");
            return;
        }
        if self.plan_mode_active.get() {
            self.queue_plan(FileOpPlan::for_duplicate(&paths));
            return;
        }
        self.do_duplicate_files(paths);
    }

    fn do_duplicate_files(self: &Rc<Self>, sources: Vec<PathBuf>) {
        if sources.is_empty() {
            return;
        }
        let items: Vec<(PathBuf, PathBuf, gio::FileCopyFlags)> = sources
            .iter()
            .filter_map(|src| {
                let parent = src.parent()?;
                let dest = duplicate_dest_name(src, parent);
                Some((src.clone(), dest, gio::FileCopyFlags::ALL_METADATA))
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let n = items.len();
        let summary = format!("Duplicate {} item{}", n, if n == 1 { "" } else { "s" });
        self.start_copy_move_op(items, true, &summary, None, Some("duplicate"));
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
        if self.plan_mode_active.get() {
            self.queue_plan(FileOpPlan::for_new_folder(&current_dir, &folder_name));
            return;
        }
        self.exec_create_folder(current_dir, folder_name);
    }

    fn exec_create_folder(self: &Rc<Self>, parent: PathBuf, folder_name: String) {
        let folder_path = parent.join(&folder_name);
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
                    let _ = controller.metadata.borrow().log_activity_with_items(
                        "new_folder",
                        1,
                        "Created folder",
                        parent.to_str().unwrap_or(""),
                        Some(parent.to_str().unwrap_or("")),
                        &[],
                        &[(parent.clone(), Some(folder_path.clone()))],
                    );
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
        if self.plan_mode_active.get() {
            self.queue_plan(FileOpPlan::for_new_file(&current_dir, &document_name));
            return;
        }
        self.exec_create_text_document(slot, current_dir, document_name);
    }

    fn exec_create_text_document(
        self: &Rc<Self>,
        slot: PaneSlot,
        parent: PathBuf,
        document_name: String,
    ) {
        let document_path = parent.join(&document_name);
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
                                let _ = controller.metadata.borrow().log_activity_with_items(
                                    "new_file",
                                    1,
                                    "Created text document",
                                    parent.to_str().unwrap_or(""),
                                    Some(parent.to_str().unwrap_or("")),
                                    &[],
                                    &[(parent.clone(), Some(document_path_for_close.clone()))],
                                );
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

        let result = self.metadata.borrow_mut().create_project(&name, &path);
        match result {
            Ok(project) => {
                self.refresh_metadata_sidebar();
                self.status
                    .set_message(&format!("Pinned project: {}.", project.name));
            }
            Err(error) => self.show_error_dialog("Project Save Failed", &error),
        }
    }

    fn pin_place(self: &Rc<Self>, path: PathBuf) {
        if path == self.places.home {
            self.status.set_message("Home is already fixed in Places.");
            return;
        }
        if !path.is_dir() {
            self.show_error_dialog("Pin Place Failed", "Only folders can be pinned to Places.");
            return;
        }

        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Folder")
            .to_string();
        let result = self.metadata.borrow_mut().create_place(&name, &path);
        match result {
            Ok(place) => {
                self.refresh_metadata_sidebar();
                self.update_sidebar_state();
                self.status
                    .set_message(&format!("Pinned {} to Places.", place.name));
            }
            Err(error) => self.show_error_dialog("Pin Place Failed", &error),
        }
    }

    fn remove_place(self: &Rc<Self>, place_id: i64) {
        let result = self.metadata.borrow_mut().remove_place(place_id);
        match result {
            Ok(()) => {
                self.refresh_metadata_sidebar();
                self.update_sidebar_state();
                self.status.set_message("Removed folder from Places.");
            }
            Err(error) => self.show_error_dialog("Remove Place Failed", &error),
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
                controller.send_paths_to_project(paths.clone(), project_id, kind, None);
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
        completion: Option<TrayCompletion>,
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

        if self.plan_mode_active.get() && completion.is_none() {
            let is_copy = matches!(kind, ProjectTransferKind::Copy);
            self.queue_plan(FileOpPlan::for_send_to_project(
                &paths,
                &project.name,
                &destination_root,
                is_copy,
            ));
            return;
        }

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
            completion,
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
        completion: Option<TrayCompletion>,
    ) {
        if index >= paths.len() {
            let result_snapshot = result.borrow();
            let errs: Vec<String> = result_snapshot.failures.clone();
            self.ops_panel.finish_op(op_id, &errs);
            if let Some(completion) = completion {
                self.record_tray_receipt(
                    &completion.action,
                    result_snapshot.success_count,
                    errs.len(),
                );
                if completion.clear_successful_paths {
                    self.remove_holding_tray_paths(&result_snapshot.successful_paths);
                }
            }
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
            self.run_project_transfer(
                paths,
                index + 1,
                destination_root,
                kind,
                op_id,
                result,
                completion,
            );
            return;
        };

        let destination_path = destination_root.join(file_name);
        if source_path == destination_path {
            result.borrow_mut().failures.push(format!(
                "{}: Source and destination are the same.",
                source_path.display()
            ));
            self.run_project_transfer(
                paths,
                index + 1,
                destination_root,
                kind,
                op_id,
                result,
                completion,
            );
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
                completion,
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
            completion,
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
        completion: Option<TrayCompletion>,
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
        let completion_cancel = completion.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || {
            let errs: Vec<String> = result_cancel.borrow().failures.clone();
            ctrl.ops_panel.finish_op(op_id, &errs);
            if let Some(completion) = completion_cancel.clone() {
                let snapshot = result_cancel.borrow();
                ctrl.record_tray_receipt(&completion.action, snapshot.success_count, errs.len());
                if completion.clear_successful_paths {
                    ctrl.remove_holding_tray_paths(&snapshot.successful_paths);
                }
            }
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
        let completion_rename = completion.clone();
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
                completion_rename.clone(),
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
        let completion_replace = completion.clone();
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
                completion_replace.clone(),
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
        completion: Option<TrayCompletion>,
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
                        Ok(()) => {
                            let mut result = result.borrow_mut();
                            result.success_count += 1;
                            result.successful_paths.push(source_path.clone());
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
                        completion,
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
                                Ok(_) => {
                                    let mut result = result.borrow_mut();
                                    result.success_count += 1;
                                    result.successful_paths.push(source_path.clone());
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
                                completion,
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
                                let mut result = result.borrow_mut();
                                result.success_count += 1;
                                result.successful_paths.push(source_path.clone());
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
                            completion,
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
        if self.plan_mode_active.get() {
            let plan = FileOpPlan::for_paste(&clipboard.paths, &destination, is_copy);
            self.queue_plan(plan);
            return;
        }
        self.handle_dnd_drop(
            clipboard.paths.clone(),
            destination,
            is_copy,
            Some(clipboard),
        );
    }

    fn paste_destination_for_slot(&self, slot: PaneSlot) -> Option<PathBuf> {
        match self.current_view_for(slot) {
            PaneView::Directory(path) => Some(path),
            PaneView::Triage { .. } => Some(self.current_dir_for(slot)),
            PaneView::Tag(_)
            | PaneView::Trash
            | PaneView::SystemDrives
            | PaneView::Recent
            | PaneView::Search(_)
            | PaneView::ActivityLog
            | PaneView::ProjectLanding(_)
            | PaneView::TagManager => None,
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

        if self.plan_mode_active.get() {
            let plan = FileOpPlan::for_trash(&paths);
            self.queue_plan(plan);
            return;
        }
        self.move_paths_to_trash(paths);
    }

    fn move_paths_to_trash(self: &Rc<Self>, paths: Vec<PathBuf>) {
        self.move_paths_to_trash_with_completion(paths, None);
    }

    fn move_paths_to_trash_with_completion(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        completion: Option<TrayCompletion>,
    ) {
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
            Rc::new(RefCell::new(Vec::new())),
            completion,
        );
    }

    fn run_trash_op(
        self: &Rc<Self>,
        op_id: OpId,
        paths: Rc<Vec<PathBuf>>,
        index: usize,
        cancellable: gio::Cancellable,
        errors: Rc<RefCell<Vec<String>>>,
        successes: Rc<RefCell<Vec<PathBuf>>>,
        completion: Option<TrayCompletion>,
    ) {
        let total = paths.len();

        if index >= total {
            let errs = errors.borrow().clone();
            self.ops_panel.finish_op(op_id, &errs);
            // Activity log receipt
            let n = paths.len() as i32;
            let summary = if n == 1 {
                let name = paths[0]
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("item");
                format!("Trashed \"{name}\"")
            } else {
                format!("Trashed {n} files")
            };
            let source = paths[0]
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();
            let _ = self.metadata.borrow().log_activity_with_items(
                "trash",
                n,
                &summary,
                &source,
                None,
                &errs,
                &paths
                    .iter()
                    .map(|path| (path.clone(), None))
                    .collect::<Vec<_>>(),
            );
            if let Some(completion) = completion {
                let successful_paths = successes.borrow().clone();
                self.record_tray_receipt(&completion.action, successful_paths.len(), errs.len());
                if completion.clear_successful_paths {
                    self.remove_holding_tray_paths(&successful_paths);
                }
            }
            self.refresh();
            return;
        }

        if cancellable.is_cancelled() {
            self.ops_panel.finish_op(op_id, &["Cancelled.".to_string()]);
            if let Some(completion) = completion {
                let successful_paths = successes.borrow().clone();
                self.record_tray_receipt(&completion.action, successful_paths.len(), 1);
                if completion.clear_successful_paths {
                    self.remove_holding_tray_paths(&successful_paths);
                }
            }
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
        let successes_clone = Rc::clone(&successes);
        let paths_clone = Rc::clone(&paths);
        let cancellable_clone = cancellable.clone();

        gio::File::for_path(&current_path).trash_async(
            glib::Priority::DEFAULT,
            Some(&cancellable),
            move |result| {
                match result {
                    Ok(_) => {
                        successes_clone.borrow_mut().push(current_path.clone());
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
                    successes_clone,
                    completion,
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
            move || {
                if controller.plan_mode_active.get() {
                    controller.queue_plan(FileOpPlan::for_permanent_delete(&paths));
                } else {
                    controller.delete_items_permanently(paths.clone());
                }
            },
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
            let source = paths
                .first()
                .and_then(|path| path.parent())
                .and_then(|path| path.to_str())
                .unwrap_or("");
            let activity_items: Vec<(PathBuf, Option<PathBuf>)> =
                paths.iter().map(|path| (path.clone(), None)).collect();
            let summary = if total == 1 {
                "Permanently deleted item".to_string()
            } else {
                format!("Permanently deleted {total} items")
            };
            let _ = self.metadata.borrow().log_activity_with_items(
                "permanent_delete",
                total as i32,
                &summary,
                source,
                None,
                &errs,
                &activity_items,
            );
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
                    PaneView::Triage { .. } => vec![self.current_dir_for(slot)],
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
        self.toolbar.forward_button.set_sensitive(
            is_directory
                && !self
                    .forward_history_cell(self.active_slot())
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
            PaneView::Tag(_) => Some(SidebarTarget::Tags),
            PaneView::TagManager => Some(SidebarTarget::Tags),
            PaneView::Triage { .. } => Some(SidebarTarget::Triage),
            PaneView::SystemDrives => Some(SidebarTarget::SystemDrives),
            PaneView::Recent => Some(SidebarTarget::Recent),
            PaneView::Trash => Some(SidebarTarget::Trash),
            PaneView::ActivityLog => Some(SidebarTarget::ActivityLog),
            PaneView::Search(_) => Some(SidebarTarget::Search),
            PaneView::ProjectLanding(id) => Some(SidebarTarget::Project(id)),
            PaneView::Directory(_) => {
                let current = self.current_dir_for(self.active_slot());
                self.user_places
                    .borrow()
                    .iter()
                    .find(|place| current.starts_with(&place.folder_path))
                    .map(|place| SidebarTarget::Place(place.id))
                    .or_else(|| {
                        self.projects
                            .borrow()
                            .iter()
                            .find(|project| current.starts_with(&project.root_path))
                            .map(|project| SidebarTarget::Project(project.id))
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
            PaneSlot::Tertiary => &self.tertiary_thumb_loader,
        }
    }

    fn cancel_active_load(&self, slot: PaneSlot) {
        if let Some(cancellable) = self.load_cancellable_cell(slot).borrow_mut().take() {
            cancellable.cancel();
        }
        if let Some(cancelled) = self.search_cancel_cell(slot).borrow_mut().take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.load_generation_cell(slot)
            .set(self.load_generation_cell(slot).get() + 1);
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
            controller.handle_dnd_drop(paths, dest, is_copy, None);
            true
        });

        pane_root.add_controller(drop_target);
    }

    /// Adds a DropTarget to one sidebar place button so files can be dragged into it.
    fn attach_sidebar_place_dnd(self: &Rc<Self>, button: gtk::Button, dest_path: PathBuf) {
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
        drop_target.connect_drop(move |_, value, _, _| {
            btn_drop.remove_css_class("drop-hover");
            let paths = parse_dropped_uris(value);
            if paths.is_empty() {
                return false;
            }
            let is_copy = last_action.get() == gdk::DragAction::COPY;
            controller.handle_dnd_drop(paths, dest_path.clone(), is_copy, None);
            true
        });

        button.add_controller(drop_target);
    }

    /// Called once from bootstrap. Adds a tray-level DropTarget so grid items
    /// can be staged without using the context menu.
    fn attach_holding_tray_dnd(self: &Rc<Self>) {
        // Tray staging is handled from each grid/list DragSource's end/cancel
        // callbacks. Avoiding tray DropTargets keeps GTK's internal
        // drag-autoscroll path out of this non-file-operation staging flow.
    }

    fn finish_drag_to_holding_tray(self: &Rc<Self>, drag: &gdk::Drag, paths: &[PathBuf]) -> bool {
        if paths.is_empty() || !self.drag_is_over_holding_tray(drag) {
            return false;
        }

        self.add_paths_to_holding_tray(paths.to_vec());
        drag.drop_done(true);
        true
    }

    fn drag_is_over_holding_tray(&self, drag: &gdk::Drag) -> bool {
        let (surface, x, y) = drag.device().surface_at_position();
        let Some(pointer_surface) = surface else {
            return false;
        };
        let Some(window_surface) = self.window.surface() else {
            return false;
        };
        if pointer_surface != window_surface {
            return false;
        }

        self.window
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .as_ref()
            .is_some_and(|widget| widget_or_ancestor_has_css(widget, "holding-tray"))
    }

    fn set_holding_tray_drag_active(&self, active: bool) {
        if active {
            self.holding_tray
                .root
                .add_css_class("holding-tray-drop-active");
        } else {
            self.holding_tray
                .root
                .remove_css_class("holding-tray-drop-active");
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

            // ── Drag source on every card (icon mode) ──────────────────────────
            let drag_source = gtk::DragSource::new();
            drag_source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
            let tray_drag_paths = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
            let tray_drag_staged = Rc::new(Cell::new(false));

            let ctrl = Rc::clone(self);
            let tray_drag_paths_prepare = Rc::clone(&tray_drag_paths);
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
                tray_drag_paths_prepare.replace(paths.clone());

                let uri_list = paths
                    .iter()
                    .map(|p| gio::File::for_path(p).uri().to_string())
                    .collect::<Vec<_>>()
                    .join("\r\n");

                Some(gdk::ContentProvider::for_value(&uri_list.to_value()))
            });

            let ctrl = Rc::clone(self);
            let drag_item = item.clone();
            let drag_shell = flow_child.clone();
            let tray_drag_staged_begin = Rc::clone(&tray_drag_staged);
            drag_source.connect_drag_begin(move |_, drag| {
                tray_drag_staged_begin.set(false);
                ctrl.set_holding_tray_drag_active(true);
                if let Some(shell) = drag_shell.first_child() {
                    shell.add_css_class("dragging");
                }

                let selected = ctrl.pane_widgets(slot).file_grid.selected_indices();
                let count = if selected.contains(&(idx as i32)) {
                    selected.len().max(1)
                } else {
                    1
                };

                let preview = build_drag_preview(&drag_item, count);
                let drag_icon = gtk::DragIcon::for_drag(drag);
                drag_icon.set_child(Some(&preview));
            });

            let drag_shell = flow_child.clone();
            let ctrl = Rc::clone(self);
            let tray_drag_paths_end = Rc::clone(&tray_drag_paths);
            let tray_drag_staged_end = Rc::clone(&tray_drag_staged);
            drag_source.connect_drag_end(move |_, drag, _| {
                if let Some(shell) = drag_shell.first_child() {
                    shell.remove_css_class("dragging");
                }
                ctrl.set_holding_tray_drag_active(false);
                if !tray_drag_staged_end.get()
                    && ctrl.finish_drag_to_holding_tray(drag, &tray_drag_paths_end.borrow())
                {
                    tray_drag_staged_end.set(true);
                }
            });

            let ctrl = Rc::clone(self);
            let tray_drag_paths_cancel = Rc::clone(&tray_drag_paths);
            let tray_drag_staged_cancel = Rc::clone(&tray_drag_staged);
            drag_source.connect_drag_cancel(move |_, drag, _| {
                ctrl.set_holding_tray_drag_active(false);
                if tray_drag_staged_cancel.get() {
                    return true;
                }
                if ctrl.finish_drag_to_holding_tray(drag, &tray_drag_paths_cancel.borrow()) {
                    tray_drag_staged_cancel.set(true);
                    return true;
                }
                false
            });

            flow_child.add_controller(drag_source);

            // ── Drag source on every list row (list mode) ─────────────────────
            if let Some(list_row) = pane.file_grid.list_box.row_at_index(idx as i32) {
                let drag_source_list = gtk::DragSource::new();
                drag_source_list.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
                let tray_drag_paths_list = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
                let tray_drag_staged_list = Rc::new(Cell::new(false));

                let ctrl = Rc::clone(self);
                let tray_drag_paths_prepare = Rc::clone(&tray_drag_paths_list);
                drag_source_list.connect_prepare(move |_, _, _| {
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
                    tray_drag_paths_prepare.replace(paths.clone());
                    let uri_list = paths
                        .iter()
                        .map(|p| gio::File::for_path(p).uri().to_string())
                        .collect::<Vec<_>>()
                        .join("\r\n");
                    Some(gdk::ContentProvider::for_value(&uri_list.to_value()))
                });

                let ctrl = Rc::clone(self);
                let drag_item_list = item.clone();
                let list_row_drag = list_row.clone();
                let tray_drag_staged_begin = Rc::clone(&tray_drag_staged_list);
                drag_source_list.connect_drag_begin(move |_, drag| {
                    tray_drag_staged_begin.set(false);
                    ctrl.set_holding_tray_drag_active(true);
                    list_row_drag.add_css_class("dragging");
                    let selected = ctrl.pane_widgets(slot).file_grid.selected_indices();
                    let count = if selected.contains(&(idx as i32)) {
                        selected.len().max(1)
                    } else {
                        1
                    };
                    let preview = build_drag_preview(&drag_item_list, count);
                    let drag_icon = gtk::DragIcon::for_drag(drag);
                    drag_icon.set_child(Some(&preview));
                });

                let list_row_end = list_row.clone();
                let ctrl = Rc::clone(self);
                let tray_drag_paths_end = Rc::clone(&tray_drag_paths_list);
                let tray_drag_staged_end = Rc::clone(&tray_drag_staged_list);
                drag_source_list.connect_drag_end(move |_, drag, _| {
                    list_row_end.remove_css_class("dragging");
                    ctrl.set_holding_tray_drag_active(false);
                    if !tray_drag_staged_end.get()
                        && ctrl.finish_drag_to_holding_tray(drag, &tray_drag_paths_end.borrow())
                    {
                        tray_drag_staged_end.set(true);
                    }
                });

                let ctrl = Rc::clone(self);
                let tray_drag_paths_cancel = Rc::clone(&tray_drag_paths_list);
                let tray_drag_staged_cancel = Rc::clone(&tray_drag_staged_list);
                drag_source_list.connect_drag_cancel(move |_, drag, _| {
                    ctrl.set_holding_tray_drag_active(false);
                    if tray_drag_staged_cancel.get() {
                        return true;
                    }
                    if ctrl.finish_drag_to_holding_tray(drag, &tray_drag_paths_cancel.borrow()) {
                        tray_drag_staged_cancel.set(true);
                        return true;
                    }
                    false
                });

                list_row.add_controller(drag_source_list);
            }

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
                fc_enter.add_css_class("drop-hover");
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
                fc_leave.remove_css_class("drop-hover");
                if let Some(card) = fc_leave.first_child() {
                    card.remove_css_class("drop-hover");
                }
            });

            let controller = Rc::clone(self);
            let folder_path = item.path.clone();
            let fc_drop = flow_child.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                fc_drop.remove_css_class("drop-hover");
                if let Some(card) = fc_drop.first_child() {
                    card.remove_css_class("drop-hover");
                }
                let paths = parse_dropped_uris(value);
                if paths.is_empty() {
                    return false;
                }
                let is_copy = last_action.get() == gdk::DragAction::COPY;
                let dest = folder_path.clone();
                controller.handle_dnd_drop(paths, dest, is_copy, None);
                true
            });

            flow_child.add_controller(drop_target);

            // ── Drop target on folder list rows ────────────────────────────────
            if let Some(list_row) = pane.file_grid.list_box.row_at_index(idx as i32) {
                let last_action_list = Rc::new(Cell::new(gdk::DragAction::MOVE));
                let la_list = last_action_list.clone();

                let drop_target_list = gtk::DropTarget::new(
                    glib::Type::STRING,
                    gdk::DragAction::COPY | gdk::DragAction::MOVE,
                );

                let lr_enter = list_row.clone();
                drop_target_list.connect_enter(move |_, _, _| {
                    lr_enter.add_css_class("drop-hover");
                    gdk::DragAction::MOVE
                });

                drop_target_list.connect_motion(move |_, _, _| {
                    let a = if ctrl_held() {
                        gdk::DragAction::COPY
                    } else {
                        gdk::DragAction::MOVE
                    };
                    la_list.set(a);
                    a
                });

                let lr_leave = list_row.clone();
                drop_target_list.connect_leave(move |_| {
                    lr_leave.remove_css_class("drop-hover");
                });

                let controller = Rc::clone(self);
                let folder_path_list = item.path.clone();
                let lr_drop = list_row.clone();
                drop_target_list.connect_drop(move |_, value, _, _| {
                    lr_drop.remove_css_class("drop-hover");
                    let paths = parse_dropped_uris(value);
                    if paths.is_empty() {
                        return false;
                    }
                    let is_copy = last_action_list.get() == gdk::DragAction::COPY;
                    let dest = folder_path_list.clone();
                    controller.handle_dnd_drop(paths, dest, is_copy, None);
                    true
                });

                list_row.add_controller(drop_target_list);
            }
        }
    }

    /// Drop handler. Filters trivial paths, then hands off to conflict-aware op starter.
    fn handle_dnd_drop(
        self: &Rc<Self>,
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

        self.start_copy_move_with_conflict_check(
            src_paths,
            dest_dir,
            is_copy,
            String::new(), // label built inside after conflict resolution
            clipboard_state,
        );
    }

    /// Check for conflicts, show the resolver if needed, then start the batch op.
    fn start_copy_move_with_conflict_check(
        self: &Rc<Self>,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        is_copy: bool,
        label_hint: String,
        clipboard_state: Option<FileClipboardState>,
    ) {
        let conflicts = conflict_resolver::collect_conflicts(&sources, &dest_dir);
        let plain = gio::FileCopyFlags::ALL_METADATA;
        let dest_name = dest_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("destination")
            .to_string();
        let verb = if is_copy { "Copy" } else { "Move" };

        // Non-conflicting items — built from sources that are not in the conflict list
        let conflict_names: std::collections::HashSet<&str> =
            conflicts.iter().map(|c| c.name.as_str()).collect();
        let non_conflicting: Vec<(PathBuf, PathBuf, gio::FileCopyFlags)> = sources
            .iter()
            .filter_map(|src| {
                let name = src.file_name()?.to_str()?;
                if conflict_names.contains(name) {
                    return None;
                }
                Some((src.clone(), dest_dir.join(name), plain))
            })
            .collect();

        if conflicts.is_empty() {
            // No conflicts — start immediately
            if non_conflicting.is_empty() {
                return;
            }
            let label = if label_hint.is_empty() {
                format!("{verb} {} item(s) → {dest_name}", non_conflicting.len())
            } else {
                label_hint
            };
            self.start_copy_move_op(non_conflicting, is_copy, &label, clipboard_state, None);
            return;
        }

        // Show conflict resolver
        let ctrl = Rc::clone(self);
        let dest_name_cb = dest_name.clone();
        conflict_resolver::show(&self.modal_host, conflicts, move |result| {
            ctrl.modal_host.hide();
            let Some(decisions) = result else { return };
            let note = conflict_resolver::decisions_note(&decisions);
            let items = conflict_resolver::apply_decisions(&decisions, &non_conflicting);
            if items.is_empty() {
                return;
            }
            let label = format!("{verb} {} item(s) → {dest_name_cb}{note}", items.len());
            ctrl.start_copy_move_op(items, is_copy, &label, clipboard_state.clone(), None);
        });
    }

    fn start_copy_move_op(
        self: &Rc<Self>,
        items: Vec<(PathBuf, PathBuf, gio::FileCopyFlags)>,
        is_copy: bool,
        label: &str,
        clipboard_state: Option<FileClipboardState>,
        activity_operation: Option<&'static str>,
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
            activity_operation,
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
        activity_operation: Option<&'static str>,
    ) {
        let total = items.len();

        if index >= total {
            let errs = errors.borrow().clone();
            self.update_file_clipboard_after_batch(
                clipboard_state.as_ref(),
                &moved_sources.borrow(),
            );
            self.ops_panel.finish_op(op_id, &errs);
            // Activity log receipt
            let n = items.len() as i32;
            let op = activity_operation.unwrap_or(if is_copy { "copy" } else { "move" });
            let verb = match op {
                "duplicate" => "Duplicated",
                _ if is_copy => "Copied",
                _ => "Moved",
            };
            let dest_name = items[0]
                .1
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("destination");
            let summary = format!(
                "{verb} {n} file{} to {dest_name}",
                if n == 1 { "" } else { "s" }
            );
            let source = items[0]
                .0
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();
            let dest = items[0]
                .1
                .parent()
                .and_then(|p| p.to_str())
                .map(|s| s.to_string());
            let activity_items: Vec<(PathBuf, Option<PathBuf>)> = items
                .iter()
                .map(|(source, destination, _)| (source.clone(), Some(destination.clone())))
                .collect();
            let _ = self.metadata.borrow().log_activity_with_items(
                op,
                n,
                &summary,
                &source,
                dest.as_deref(),
                &errs,
                &activity_items,
            );
            self.reload_visible_panes();
            return;
        }

        if cancellable.is_cancelled() {
            self.update_file_clipboard_after_batch(
                clipboard_state.as_ref(),
                &moved_sources.borrow(),
            );
            self.ops_panel.finish_op(op_id, &["Cancelled.".to_string()]);
            self.reload_visible_panes();
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
                activity_operation,
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
                    activity_operation,
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
}

fn add_unique_tray_items(existing: &mut Vec<FileItem>, incoming: Vec<FileItem>) -> usize {
    let mut seen = existing
        .iter()
        .map(|item| item.path.clone())
        .collect::<HashSet<_>>();
    let mut added = 0;
    for item in incoming {
        if seen.insert(item.path.clone()) {
            existing.push(item);
            added += 1;
        }
    }
    added
}

fn project_action_plan(
    action: TrayProjectAction,
    paths: &[PathBuf],
    project_name: &str,
    project_root: &Path,
) -> ActionPlan {
    let verb = match action {
        TrayProjectAction::Copy => "Copy",
        TrayProjectAction::Move => "Move",
    };
    let mut lines = vec![
        format!(
            "{verb} {} staged item(s) to project \"{project_name}\".",
            paths.len()
        ),
        format!("Destination: {}", project_root.display()),
    ];
    lines.extend(plan_path_lines(paths));
    ActionPlan::new(
        action.title(),
        verb,
        action == TrayProjectAction::Move,
        lines,
    )
}

fn tag_action_plan(paths: &[PathBuf], tag_name: &str) -> ActionPlan {
    let mut lines = vec![format!(
        "Apply tag #{tag_name} to {} staged item(s).",
        paths.len()
    )];
    lines.extend(plan_path_lines(paths));
    ActionPlan::new("Tag Holding Tray", "Apply Tag", false, lines)
}

fn trash_action_plan(paths: &[PathBuf]) -> ActionPlan {
    let mut lines = vec![format!("Move {} staged item(s) to Trash.", paths.len())];
    lines.extend(plan_path_lines(paths));
    ActionPlan::new("Move Tray to Trash", "Move to Trash", true, lines)
}

fn copy_path_action_plan(paths: &[PathBuf]) -> ActionPlan {
    let mut lines = vec![format!(
        "Copy {} staged path(s) to the clipboard.",
        paths.len()
    )];
    lines.extend(plan_path_lines(paths));
    ActionPlan::new("Copy Tray Paths", "Copy Paths", false, lines)
}

fn plan_path_lines(paths: &[PathBuf]) -> Vec<String> {
    const MAX_PREVIEW_PATHS: usize = 8;
    let mut lines = paths
        .iter()
        .take(MAX_PREVIEW_PATHS)
        .map(|path| format!("• {}", path.display()))
        .collect::<Vec<_>>();
    if paths.len() > MAX_PREVIEW_PATHS {
        lines.push(format!("… and {} more", paths.len() - MAX_PREVIEW_PATHS));
    }
    lines
}

// ── DnD helpers ─────────────────────────────────────────────────────

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
        "toggle_holding_tray" => Some(WindowCommand::ToggleHoldingTray),
        "new_tab" => Some(WindowCommand::NewTab),
        "close_tab" => Some(WindowCommand::CloseTab),
        "toggle_split" => Some(WindowCommand::ToggleSplit),
        "previous_tab" => Some(WindowCommand::PreviousTab),
        "next_tab" => Some(WindowCommand::NextTab),
        "back" => Some(WindowCommand::GoBack),
        "forward" => Some(WindowCommand::GoForward),
        "up" => Some(WindowCommand::GoUp),
        "cycle_pane" => Some(WindowCommand::CyclePane),
        "escape" => Some(WindowCommand::Escape),
        "view_icons" => Some(WindowCommand::SetViewIcons),
        "view_list" => Some(WindowCommand::SetViewList),
        "toggle_plan_mode" => Some(WindowCommand::TogglePlanMode),
        "empty_trash" => Some(WindowCommand::EmptyTrash),
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
        PaneView::Trash
            | PaneView::SystemDrives
            | PaneView::Recent
            | PaneView::Search(_)
            | PaneView::ActivityLog
    );
    let can_paste_files =
        has_file_clipboard && matches!(view, PaneView::Directory(_) | PaneView::Triage { .. });

    ActionAvailability {
        can_copy_files: selected_count > 0,
        can_cut_files: selected_count > 0 && !read_only_mutation_view,
        can_paste_files,
        can_copy_paths: selected_count > 0 || !matches!(view, PaneView::SystemDrives),
        can_rename: selected_count > 0 && !read_only_mutation_view,
        can_trash: selected_count > 0 && !read_only_mutation_view,
        can_new_folder: matches!(view, PaneView::Directory(_) | PaneView::Triage { .. }),
    }
}

fn widget_or_ancestor_has_css(widget: &gtk::Widget, class_name: &str) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class(class_name) {
            return true;
        }
        current = widget.parent();
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathCompletionMode {
    Absolute,
    Home,
    Relative,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PathCompletionQuery {
    query: String,
    mode: PathCompletionMode,
}

#[allow(deprecated)]
fn update_path_completion_model(
    store: &gtk::ListStore,
    completer: &gio::FilenameCompleter,
    input: &str,
    current_dir: &Path,
    home: &Path,
) {
    store.clear();

    let Some(query) = path_completion_query(input, current_dir, home) else {
        return;
    };

    let mut seen = HashSet::new();
    let mut completions = completer
        .completions(&query.query)
        .into_iter()
        .filter_map(|completion| {
            path_completion_display(&completion, query.mode, current_dir, home)
        })
        .filter(|completion| completion != input)
        .filter(|completion| seen.insert(completion.clone()))
        .collect::<Vec<_>>();

    completions.sort_by_key(|completion| completion.to_ascii_lowercase());
    completions.truncate(24);

    for completion in completions {
        store.insert_with_values(None, &[(0, &completion)]);
    }
}

#[allow(deprecated)]
fn first_path_completion(store: &gtk::ListStore) -> Option<String> {
    let iter = store.iter_first()?;
    Some(store.get::<String>(&iter, 0))
}

fn path_completion_query(
    input: &str,
    current_dir: &Path,
    home: &Path,
) -> Option<PathCompletionQuery> {
    let input = input.trim_start();
    if input.is_empty() || input.starts_with("file://") {
        return None;
    }

    if input == "~" {
        return Some(PathCompletionQuery {
            query: home.display().to_string(),
            mode: PathCompletionMode::Home,
        });
    }

    if let Some(relative_home) = input.strip_prefix("~/") {
        return Some(PathCompletionQuery {
            query: home.join(relative_home).display().to_string(),
            mode: PathCompletionMode::Home,
        });
    }

    let candidate = PathBuf::from(input);
    if candidate.is_absolute() {
        Some(PathCompletionQuery {
            query: input.to_string(),
            mode: PathCompletionMode::Absolute,
        })
    } else {
        Some(PathCompletionQuery {
            query: current_dir.join(input).display().to_string(),
            mode: PathCompletionMode::Relative,
        })
    }
}

fn path_completion_display(
    completion: &str,
    mode: PathCompletionMode,
    current_dir: &Path,
    home: &Path,
) -> Option<String> {
    match mode {
        PathCompletionMode::Absolute => Some(completion.to_string()),
        PathCompletionMode::Home => display_completion_under_root(completion, home, "~"),
        PathCompletionMode::Relative => display_completion_under_root(completion, current_dir, ""),
    }
}

fn display_completion_under_root(completion: &str, root: &Path, prefix: &str) -> Option<String> {
    let root = root.display().to_string();
    if completion == root {
        return Some(prefix.to_string());
    }

    let root_with_slash = format!("{root}/");
    let suffix = completion.strip_prefix(&root_with_slash)?;
    if prefix.is_empty() {
        Some(suffix.to_string())
    } else {
        Some(format!("{prefix}/{suffix}"))
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

fn build_drag_preview(item: &FileItem, count: usize) -> GtkBox {
    let preview = GtkBox::new(Orientation::Horizontal, 8);
    preview.add_css_class("drag-preview");
    preview.add_css_class(item.kind.css_class());
    preview.set_size_request(180, 64);

    let icon = Label::new(Some(item.kind.badge()));
    icon.add_css_class("drag-preview-icon");
    icon.set_halign(Align::Center);
    icon.set_valign(Align::Center);
    preview.append(&icon);

    let copy = GtkBox::new(Orientation::Vertical, 2);
    copy.add_css_class("drag-preview-copy");
    copy.set_valign(Align::Center);

    let title = if count > 1 {
        format!("{count} items")
    } else {
        item.name.clone()
    };
    let title_label = Label::new(Some(&title));
    title_label.add_css_class("drag-preview-title");
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title_label.set_xalign(0.0);
    title_label.set_max_width_chars(18);
    copy.append(&title_label);

    let detail = if count > 1 {
        format!("Including {}", item.name)
    } else {
        item.kind.label().to_string()
    };
    let detail_label = Label::new(Some(&detail));
    detail_label.add_css_class("drag-preview-detail");
    detail_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail_label.set_xalign(0.0);
    detail_label.set_max_width_chars(22);
    copy.append(&detail_label);

    preview.append(&copy);

    if count > 1 {
        let badge = Label::new(Some(&count.to_string()));
        badge.add_css_class("drag-preview-count");
        badge.set_halign(Align::End);
        badge.set_valign(Align::Start);
        preview.append(&badge);
    }

    preview
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
    cancelled: &AtomicBool,
) {
    if cancelled.load(Ordering::Relaxed)
        || depth > MAX_SEARCH_DEPTH
        || results.len() >= MAX_SEARCH_RESULTS
    {
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
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
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
        if cancelled.load(Ordering::Relaxed) || results.len() >= MAX_SEARCH_RESULTS {
            return;
        }
        if let Some(item) = match_entry(e, query, now_secs) {
            results.push(item);
        }
    }

    // Pass 2: subdirectories — add matching ones then recurse
    for e in &subdirs {
        if cancelled.load(Ordering::Relaxed) || results.len() >= MAX_SEARCH_RESULTS {
            return;
        }
        if let Some(item) = match_entry(e, query, now_secs) {
            results.push(item);
        }
        if query.recursive {
            search_directory_blocking(&e.path, query, show_hidden, depth + 1, results, cancelled);
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
fn plan_copy_move_items(
    sources: &[PathBuf],
    dest_dir: &Path,
) -> Vec<(PathBuf, PathBuf, gio::FileCopyFlags)> {
    sources
        .iter()
        .filter_map(|src| {
            let name = src.file_name()?;
            Some((
                src.clone(),
                dest_dir.join(name),
                gio::FileCopyFlags::ALL_METADATA,
            ))
        })
        .collect()
}

fn duplicate_dest_name(src: &Path, parent: &Path) -> PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let base = format!("{stem} copy{ext}");
    let candidate = parent.join(&base);
    if !candidate.exists() {
        return candidate;
    }
    let mut i = 2u32;
    loop {
        let name = format!("{stem} copy {i}{ext}");
        let candidate = parent.join(&name);
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

fn activity_sources(entry: &ActivityLogEntry) -> Vec<PathBuf> {
    entry
        .items
        .iter()
        .map(|item| PathBuf::from(&item.source_path))
        .collect()
}

fn activity_destinations(entry: &ActivityLogEntry) -> Vec<PathBuf> {
    entry
        .items
        .iter()
        .filter_map(|item| item.destination_path.as_ref().map(PathBuf::from))
        .collect()
}

fn activity_renames(entry: &ActivityLogEntry) -> Vec<(PathBuf, String)> {
    entry
        .items
        .iter()
        .filter_map(|item| {
            let destination = item.destination_path.as_ref().map(PathBuf::from)?;
            let name = destination.file_name()?.to_str()?.to_string();
            Some((PathBuf::from(&item.source_path), name))
        })
        .collect()
}

fn activity_created_parent_and_name(entry: &ActivityLogEntry) -> Option<(PathBuf, String)> {
    let path = activity_destinations(entry).into_iter().next()?;
    let parent = path.parent()?.to_path_buf();
    let name = path.file_name()?.to_str()?.to_string();
    Some((parent, name))
}

fn activity_relevant_path(entry: &ActivityLogEntry) -> Option<PathBuf> {
    entry
        .items
        .first()
        .map(|item| {
            item.destination_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&item.source_path))
        })
        .or_else(|| entry.destination_path.as_ref().map(PathBuf::from))
        .or_else(|| (!entry.source_path.is_empty()).then(|| PathBuf::from(&entry.source_path)))
}

fn common_activity_destination_parent(entry: &ActivityLogEntry) -> Option<PathBuf> {
    let mut parents = entry
        .items
        .iter()
        .filter_map(|item| item.destination_path.as_ref())
        .filter_map(|path| Path::new(path).parent().map(Path::to_path_buf));
    let first = parents.next()?;
    parents.all(|path| path == first).then_some(first)
}

struct BodyLayout {
    root: GtkBox,
    sidebar_revealer: Revealer,
    preview_revealer: Revealer,
    split_paned: Paned,
    right_paned: Paned,
}

struct CenterLayout {
    root: GtkBox,
    split_paned: Paned,
    right_paned: Paned,
}

fn build_body(
    sidebar: &Sidebar,
    tab_strip: &TabStrip,
    primary_pane: &PaneWidgets,
    secondary_pane: &PaneWidgets,
    tertiary_pane: &PaneWidgets,
    preview: &PreviewPane,
) -> BodyLayout {
    let outer = GtkBox::new(Orientation::Horizontal, 0);

    let sidebar_revealer = Revealer::new();
    sidebar_revealer.set_transition_type(gtk::RevealerTransitionType::SlideRight);
    sidebar_revealer.set_transition_duration(180);
    sidebar_revealer.set_hexpand(false);
    sidebar_revealer.set_child(Some(&sidebar.root));
    sidebar_revealer.set_reveal_child(true);
    outer.append(&sidebar_revealer);

    let center = build_center(tab_strip, primary_pane, secondary_pane, tertiary_pane);
    center.root.set_hexpand(true);
    center.root.set_vexpand(true);
    outer.append(&center.root);

    let preview_host = GtkBox::new(Orientation::Vertical, 0);
    preview_host.add_css_class("preview-host");
    preview_host.append(&preview.root);

    let preview_revealer = Revealer::new();
    preview_revealer.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
    preview_revealer.set_transition_duration(180);
    preview_revealer.set_hexpand(false);
    preview_revealer.set_child(Some(&preview_host));
    preview_revealer.set_reveal_child(true);
    outer.append(&preview_revealer);

    BodyLayout {
        root: outer,
        sidebar_revealer,
        preview_revealer,
        split_paned: center.split_paned,
        right_paned: center.right_paned,
    }
}

fn build_center(
    tab_strip: &TabStrip,
    primary_pane: &PaneWidgets,
    secondary_pane: &PaneWidgets,
    tertiary_pane: &PaneWidgets,
) -> CenterLayout {
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&tab_strip.root);

    // Inner split: secondary | tertiary
    let right_paned = Paned::new(Orientation::Horizontal);
    right_paned.set_wide_handle(false);
    right_paned.set_resize_start_child(true);
    right_paned.set_shrink_start_child(true);
    right_paned.set_resize_end_child(true);
    right_paned.set_shrink_end_child(true);
    right_paned.set_start_child(Some(&secondary_pane.root));
    right_paned.set_end_child(Some(&tertiary_pane.root));
    right_paned.set_hexpand(true);
    right_paned.set_vexpand(true);
    tertiary_pane.root.set_visible(false);

    // Outer split: primary | right_paned
    let split_paned = Paned::new(Orientation::Horizontal);
    split_paned.set_wide_handle(false);
    split_paned.add_css_class("split-panes");
    split_paned.set_resize_start_child(true);
    split_paned.set_shrink_start_child(true);
    split_paned.set_resize_end_child(true);
    split_paned.set_shrink_end_child(true);
    split_paned.set_start_child(Some(&primary_pane.root));
    split_paned.set_end_child(Some(&right_paned));
    split_paned.set_hexpand(true);
    split_paned.set_vexpand(true);
    // Start in single-pane mode: right section hidden
    right_paned.set_visible(false);

    for pane in [primary_pane, secondary_pane, tertiary_pane] {
        pane.root.set_vexpand(true);
        pane.root.set_hexpand(true);
    }

    vbox.append(&split_paned);
    CenterLayout {
        root: vbox,
        split_paned,
        right_paned,
    }
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
    button.connect_clicked(move |button| {
        if let Some(popover) = button
            .ancestor(Popover::static_type())
            .and_then(|widget| widget.downcast::<Popover>().ok())
        {
            if popover.parent().is_some() {
                popover.unparent();
            }
        }
        action();
    });
    menu_box.append(&button);
}

fn append_menu_sep(menu_box: &GtkBox) {
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("context-menu-sep");
    menu_box.append(&sep);
}

fn sort_items_with(items: &mut [FileItem], field: SortField, direction: SortDirection) {
    items.sort_by(|a, b| {
        let dir_ord = b.is_dir.cmp(&a.is_dir);
        if dir_ord != std::cmp::Ordering::Equal {
            return dir_ord;
        }
        let base = match field {
            SortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortField::Modified => a
                .modified_unix
                .unwrap_or(0)
                .cmp(&b.modified_unix.unwrap_or(0)),
            SortField::Size => a.size_bytes.unwrap_or(0).cmp(&b.size_bytes.unwrap_or(0)),
            SortField::Kind => a
                .kind
                .sort_key()
                .cmp(&b.kind.sort_key())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        };
        match direction {
            SortDirection::Ascending => base,
            SortDirection::Descending => base.reverse(),
        }
    });
}

fn sort_items(items: &mut [FileItem]) {
    sort_items_with(items, SortField::Name, SortDirection::Ascending);
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
        PaneView::Triage { root, filter } => {
            let folder = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            format!("Triage: {} / {}", folder, filter.label())
        }
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
        PaneView::ActivityLog => "Activity Log".to_string(),
        PaneView::ProjectLanding(_) => "Project".to_string(),
        PaneView::TagManager => "Tags".to_string(),
    }
}

fn view_display_label(view: &PaneView, home: &Path) -> String {
    match view {
        PaneView::Directory(path) => format_path(path, home),
        PaneView::Tag(tag) => format!("Tag / #{}", tag.name),
        PaneView::Triage { root, filter } => {
            let folder = format_path(root, home);
            format!("Triage: {} / {}", folder, filter.label())
        }
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
        PaneView::ActivityLog => "Activity Log".to_string(),
        PaneView::ProjectLanding(_) => "Project".to_string(),
        PaneView::TagManager => "Tags".to_string(),
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
    if let Some(split_paths) = &launch.split {
        let left = &split_paths[0];
        let right = &split_paths[1];
        let third = split_paths.get(2);
        let (left_path, left_notice) = validate_launch_directory(left, places, "Left split path");
        let (right_path, right_notice) =
            validate_launch_directory(right, places, "Middle split path");
        let (third_path, third_notice) = third
            .map(|path| validate_launch_directory(path, places, "Right split path"))
            .unwrap_or_else(|| (places.home.clone(), None));
        return LaunchResolution {
            primary_dir: left_path.clone(),
            primary_view: PaneView::Directory(left_path),
            secondary_dir: right_path,
            tertiary_dir: third_path,
            pane_layout: if third.is_some() {
                PaneLayout::Three
            } else {
                PaneLayout::Two
            },
            notice: combine_launch_notices([left_notice, right_notice, third_notice]),
        };
    }

    if let Some(path) = &launch.path {
        let (resolved_path, notice) = validate_launch_directory(path, places, "Launch path");
        return LaunchResolution {
            primary_dir: resolved_path.clone(),
            primary_view: PaneView::Directory(resolved_path),
            secondary_dir: places.home.clone(),
            tertiary_dir: places.home.clone(),
            pane_layout: PaneLayout::Single,
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
            PaneView::Triage {
                root: primary_dir.clone(),
                filter: TriageFilter::All,
            }
        };
        return LaunchResolution {
            primary_dir,
            primary_view,
            secondary_dir: places.home.clone(),
            tertiary_dir: places.home.clone(),
            pane_layout: PaneLayout::Single,
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
                tertiary_dir: places.home.clone(),
                pane_layout: PaneLayout::Single,
                notice: None,
            },
            Some(project) => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::Directory(places.home.clone()),
                secondary_dir: places.home.clone(),
                tertiary_dir: places.home.clone(),
                pane_layout: PaneLayout::Single,
                notice: Some(format!(
                    "Project '{}' points to a missing folder. Opened Home instead.",
                    project.name
                )),
            },
            None => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::Directory(places.home.clone()),
                secondary_dir: places.home.clone(),
                tertiary_dir: places.home.clone(),
                pane_layout: PaneLayout::Single,
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
        tertiary_dir: places.home.clone(),
        pane_layout: PaneLayout::Single,
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

fn combine_launch_notices(notices: [Option<String>; 3]) -> Option<String> {
    let notices = notices.into_iter().flatten().collect::<Vec<_>>();
    (!notices.is_empty()).then(|| notices.join(" "))
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

fn filter_triage_items(
    items: Vec<FileItem>,
    filter: TriageFilter,
    duplicate_set: Option<&std::collections::HashSet<PathBuf>>,
) -> Vec<FileItem> {
    items
        .into_iter()
        .filter(|item| matches_triage_filter(item, filter, duplicate_set))
        .collect()
}

fn matches_triage_filter(
    item: &FileItem,
    filter: TriageFilter,
    duplicate_set: Option<&std::collections::HashSet<PathBuf>>,
) -> bool {
    match filter {
        TriageFilter::All => true,
        TriageFilter::Today => item
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
        TriageFilter::ThisWeek => item
            .modified_unix
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| Some(value.to_unix())),
            )
            .map(|(modified, now)| now.saturating_sub(modified) <= 7 * 24 * 60 * 60)
            .unwrap_or(false),
        TriageFilter::ThisMonth => item
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
        TriageFilter::OlderThanOneMonth => item
            .modified_unix
            .zip(
                glib::DateTime::now_local()
                    .ok()
                    .and_then(|value| Some(value.to_unix())),
            )
            .map(|(modified, now)| now.saturating_sub(modified) > 30 * 24 * 60 * 60)
            .unwrap_or(false),
        TriageFilter::Images => item.kind == crate::ui::file_grid::FileKind::Image,
        TriageFilter::Videos => item.kind == crate::ui::file_grid::FileKind::Video,
        TriageFilter::Archives => item.kind == crate::ui::file_grid::FileKind::Archive,
        TriageFilter::Documents => {
            matches!(
                item.kind,
                crate::ui::file_grid::FileKind::Document
                    | crate::ui::file_grid::FileKind::Text
                    | crate::ui::file_grid::FileKind::ConfigCode
            )
        }
        TriageFilter::LargeFiles => item.size_bytes.unwrap_or(0) >= TRIAGE_LARGE_FILE_BYTES,
        TriageFilter::Audio => item.kind == crate::ui::file_grid::FileKind::Audio,
        TriageFilter::Executables => !item.is_dir && is_executable(&item.path),
        TriageFilter::Empty => !item.is_dir && item.size_bytes == Some(0),
        TriageFilter::Duplicates => duplicate_set
            .map(|set| set.contains(&item.path))
            .unwrap_or(false),
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

fn fnv1a_64(data: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn compute_duplicate_set_from_dir(dir: &Path) -> std::collections::HashSet<PathBuf> {
    use std::collections::HashMap;
    use std::io::Read;

    let Ok(entries) = std::fs::read_dir(dir) else {
        return std::collections::HashSet::new();
    };

    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() || meta.len() == 0 {
            continue;
        }
        by_size.entry(meta.len()).or_default().push(entry.path());
    }

    let mut duplicates = std::collections::HashSet::new();
    for (_size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        let mut by_hash: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        for path in &paths {
            let Ok(mut file) = std::fs::File::open(path) else {
                continue;
            };
            let mut buf = vec![0u8; 65536];
            let n = file.read(&mut buf).unwrap_or(0);
            let hash = fnv1a_64(&buf[..n]);
            by_hash.entry(hash).or_default().push(path.clone());
        }
        for (_hash, group) in by_hash {
            if group.len() >= 2 {
                duplicates.extend(group);
            }
        }
    }
    duplicates
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
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&downloads).unwrap();
        Places { home, downloads }
    }

    fn test_tag() -> TagRecord {
        TagRecord {
            id: 7,
            name: "Focus".to_string(),
            color: None,
        }
    }

    fn test_item(path: &str) -> FileItem {
        let path = PathBuf::from(path);
        FileItem {
            name: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("item")
                .to_string(),
            path,
            kind: FileKind::Text,
            is_dir: false,
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            original_path: None,
        }
    }

    #[test]
    fn holding_tray_deduplicates_items_by_path() {
        let mut existing = vec![test_item("/tmp/a.txt")];
        let added = add_unique_tray_items(
            &mut existing,
            vec![test_item("/tmp/a.txt"), test_item("/tmp/b.txt")],
        );

        assert_eq!(added, 1);
        assert_eq!(
            existing
                .iter()
                .map(|item| item.path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")]
        );
    }

    #[test]
    fn tray_action_plan_summarizes_long_path_lists() {
        let paths = (0..10)
            .map(|index| PathBuf::from(format!("/tmp/item-{index}.txt")))
            .collect::<Vec<_>>();

        let plan = trash_action_plan(&paths);

        assert_eq!(plan.title, "Move Tray to Trash");
        assert!(plan.destructive);
        assert!(plan.lines[0].contains("10 staged item"));
        assert!(plan.lines.iter().any(|line| line.contains("and 2 more")));
    }

    #[test]
    fn path_completion_query_supports_home_absolute_and_relative_inputs() {
        let current_dir = PathBuf::from("/home/tester/Downloads");
        let home = PathBuf::from("/home/tester");

        assert_eq!(
            path_completion_query("~/Doc", &current_dir, &home),
            Some(PathCompletionQuery {
                query: "/home/tester/Doc".to_string(),
                mode: PathCompletionMode::Home,
            })
        );
        assert_eq!(
            path_completion_query("/var/lo", &current_dir, &home),
            Some(PathCompletionQuery {
                query: "/var/lo".to_string(),
                mode: PathCompletionMode::Absolute,
            })
        );
        assert_eq!(
            path_completion_query("Pho", &current_dir, &home),
            Some(PathCompletionQuery {
                query: "/home/tester/Downloads/Pho".to_string(),
                mode: PathCompletionMode::Relative,
            })
        );
        assert_eq!(path_completion_query("", &current_dir, &home), None);
        assert_eq!(
            path_completion_query("file:///home/tester", &current_dir, &home),
            None
        );
    }

    #[test]
    fn path_completion_display_preserves_user_facing_prefix_style() {
        let current_dir = PathBuf::from("/home/tester/Downloads");
        let home = PathBuf::from("/home/tester");

        assert_eq!(
            path_completion_display(
                "/home/tester/Documents",
                PathCompletionMode::Home,
                &current_dir,
                &home
            ),
            Some("~/Documents".to_string())
        );
        assert_eq!(
            path_completion_display(
                "/home/tester/Downloads/Photos",
                PathCompletionMode::Relative,
                &current_dir,
                &home
            ),
            Some("Photos".to_string())
        );
        assert_eq!(
            path_completion_display(
                "/var/log",
                PathCompletionMode::Absolute,
                &current_dir,
                &home
            ),
            Some("/var/log".to_string())
        );
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
            &PaneView::Triage {
                root: PathBuf::from("/tmp/triage"),
                filter: TriageFilter::All,
            },
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
    fn pane_layout_cycles_and_reports_visible_slots() {
        assert_eq!(PaneLayout::Single.next(), PaneLayout::Two);
        assert_eq!(PaneLayout::Two.next(), PaneLayout::Three);
        assert_eq!(PaneLayout::Three.next(), PaneLayout::Single);
        assert!(PaneLayout::Single.includes(PaneSlot::Primary));
        assert!(!PaneLayout::Single.includes(PaneSlot::Secondary));
        assert!(PaneLayout::Two.includes(PaneSlot::Secondary));
        assert!(!PaneLayout::Two.includes(PaneSlot::Tertiary));
        assert!(PaneLayout::Three.includes(PaneSlot::Tertiary));
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
            window_command_from_key(gdk::Key::h, ctrl | gdk::ModifierType::ALT_MASK),
            Some(WindowCommand::ToggleHoldingTray)
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
            window_command_from_key(gdk::Key::backslash, ctrl),
            Some(WindowCommand::ToggleSplit)
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
            window_command_from_key(gdk::Key::Delete, ctrl_shift),
            Some(WindowCommand::EmptyTrash)
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
        let project_root = places.home.join("documents").join("workspace");
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
            split: Some(vec![valid_left.clone(), invalid_right]),
            ..LaunchConfig::default()
        };

        let resolution = resolve_launch(&launch, &places, &metadata);
        assert_eq!(resolution.pane_layout, PaneLayout::Two);
        assert_eq!(resolution.primary_dir, valid_left);
        assert_eq!(resolution.secondary_dir, places.home);
        assert!(resolution.notice.unwrap().contains("Middle split path"));
    }

    #[test]
    fn resolve_launch_three_pane_split_preserves_valid_sides_and_falls_back_invalid_side() {
        let places = test_places();
        let metadata = MetadataStore::open_in_memory().unwrap();
        let valid_left = places.downloads.clone();
        let valid_middle = places.home.join("documents");
        fs::create_dir_all(&valid_middle).unwrap();
        let invalid_right = places.home.join("missing-right");
        let launch = LaunchConfig {
            split: Some(vec![
                valid_left.clone(),
                valid_middle.clone(),
                invalid_right,
            ]),
            ..LaunchConfig::default()
        };

        let resolution = resolve_launch(&launch, &places, &metadata);
        assert_eq!(resolution.pane_layout, PaneLayout::Three);
        assert_eq!(resolution.primary_dir, valid_left);
        assert_eq!(resolution.secondary_dir, valid_middle);
        assert_eq!(resolution.tertiary_dir, places.home);
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
