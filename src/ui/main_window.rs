use crate::action_plan::{ActionPlan as FileOpPlan, OpKind as FileOpKind, RestoreSpec};
use crate::config::{shortcut_tooltip, AppConfig, CustomActionConfig};
use crate::converter::{
    cleanup_orphaned_temps_in, ConversionQueue, ConvertItem, ConvertSettings, MediaKind,
};
use crate::metadata::{
    ActivityLogEntry, CloudRecord, FolderViewState, MetadataStore, PlaceRecord, ProjectRecord,
    Shape, TagRecord, TintRecord,
};
use crate::terroir_client;
use crate::ui::{
    activity_log_panel::{ActivityLogAction, ActivityLogPanel},
    bulk_naming_panel::BulkNamingPanel,
    cloud_landing_panel::CloudLandingPanel,
    conflict_resolver,
    convert_progress_panel::ConvertProgressPanel,
    file_grid::{FileGrid, FileItem, FileKind, ViewMode},
    holding_tray::HoldingTray,
    media_convert_panel::{ConvertSourceMode, MediaConvertPanel},
    modal_host::{
        build_modal_actions, build_modal_button, build_modal_prompt, ButtonKind, ModalHost,
    },
    ops_panel::{OpId, OpsPanel},
    painting_toolbar::{PaintTool, PaintType, PaintingToolbar},
    palette_board_panel::PaletteBoardPanel,
    picker::{show_picker_modal, PickerConfig, PickerResult},
    plan_queue_panel::{PlanQueuePanel, QueueAction},
    preview_pane::PreviewPane,
    project_landing_panel::ProjectLandingPanel,
    project_manager_panel::ProjectManagerPanel,
    search_panel::{SearchAgeFilter, SearchKindFilter, SearchPanel, SearchQuery, SearchSizeFilter},
    sidebar::{DriveEntry, Sidebar, SidebarTarget},
    space_viewer_panel::SpaceViewerPanel,
    status_bar::StatusBar,
    tab_strip::TabStrip,
    tag_filter::{TagFilterPanel, TagFilterSpec},
    tints_tags_panel::TintsTagsPanel,
    toolbar::Toolbar,
    watercolor_panel::{WatercolorPanel, WatercolorPanelData, WatercolorPanelView},
};
use gdk_pixbuf::Pixbuf;
use gio::prelude::*;
use glib::UserDirectory;
use gtk::gdk;
use gtk::prelude::*;
use gtk::{
    Adjustment, Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider,
    DrawingArea, Entry, FlowBox, HeaderBar, Image, Label, ListBox, ListBoxRow, Orientation, Paned,
    Popover, Revealer, Scale, Separator,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

const DIRECTORY_ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::is-hidden,standard::size,time::modified";
const TRASH_ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::is-hidden,standard::size,time::modified,trash::orig-path,standard::target-uri";
const PREVIEW_ATTRIBUTES: &str =
    "standard::display-name,standard::type,standard::content-type,standard::size,time::modified";
const TERMINAL_ENV_VAR: &str = "LATTICE_TERMINAL";
const TEXT_PREVIEW_LIMIT_BYTES: usize = 64 * 1024;
const TEXT_PREVIEW_DISPLAY_CHARS: usize = 4_000;
const TRIAGE_LARGE_FILE_BYTES: u64 = 50 * 1024 * 1024;
const TRASH_GVFS_DIAGNOSTIC: &str = "Trash support may require GVfs. On Arch/CachyOS, install gvfs, udisks2, and polkit, then log out/in or reboot.\n\nTroubleshooting:\nsudo pacman -Syu --needed gvfs udisks2 polkit\ngio list trash:///\ngio trash --list\ngio mount -l";
const DRIVES_GVFS_DIAGNOSTIC: &str = "No system drives found through GIO/GVfs.\n\nInstall gvfs, udisks2, and polkit, then log out/in or reboot.\n\nTroubleshooting:\nsudo pacman -Syu --needed gvfs udisks2 polkit\ngio mount -l\nudisksctl status\nlsblk -f";
const GVFS_REMOTE_DIAGNOSTIC: &str = "GVfs remote is unavailable. Possible causes:\n• GVfs daemon not running or backend not installed\n• Remote host unreachable or credentials expired\n• SMB shares need gvfs-smb; SFTP/FTP need gvfs-fuse\n\nUbuntu/Debian: sudo apt install gvfs gvfs-backends\nArch/CachyOS:  sudo pacman -S gvfs gvfs-smb gvfs-mtp\n\nDiagnostics:\ngio mount <uri>\ngio mount -l";

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
            Self::Copy => "Copy Tray to Palette",
            Self::Move => "Move Tray to Palette",
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
    ToggleShapeBadges,
    SortOrder,
    ToggleSidebar,
    TogglePreview,
    ToggleHoldingTray,
    TrayAddSelection,
    TrayMoveToProject,
    TrayCopyToProject,
    TrayTag,
    TrayTrash,
    TrayCopyPaths,
    TrayClear,
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
    OpenHome,
    OpenSystemDrives,
    OpenRecent,
    OpenTrash,
    OpenPalettes,
    OpenTintsTags,
    OpenSpaceViewer,
    OpenTriage,
    OpenBulkNaming,
    OpenConvert,
    OpenActivityLog,
    SetViewIcons,
    SetViewList,
    TogglePlanMode,
    TogglePaintMode,
    PaintCursor,
    PaintBrush,
    PaintEraser,
    PaintEyedropper,
    PaintFill,
    PaintUndo,
    PaintRedo,
    PaintToggleContents,
    EmptyTrash,
    TrayAddByTint,
    TrayAddByShape,
    TrayApplyMark,
    TrayResetMark,
    PlanExecute,
    PlanClear,
    ConvertStart,
    ConvertCancel,
    ConvertRetryFailed,
    ConvertOpenOutput,
    ConvertDismiss,
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
    BulkNaming { root: PathBuf },
    SpaceViewer { root: PathBuf },
    MediaConvert { from_dir: PathBuf },
    ActivityLog,
    ProjectLanding(i64),
    CloudLanding(i64),
    ProjectManager,
    TagManager,
    Watercolor(WatercolorView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WatercolorView {
    Status,
    Workspaces,
    Palettes,
    BrokenRefs,
}

/// One file/folder change captured for undo/redo.
enum PaintOp {
    Mark {
        prev: Option<(i64, Shape)>,
        next: Option<(i64, Shape)>,
    },
    /// `added = true` → brush applied tag; `false` → eraser removed tag.
    Tag { tag_id: i64, added: bool },
}

struct PaintHistoryEntry {
    path: PathBuf,
    op: PaintOp,
}

/// A group of changes that constitute one undoable paint operation.
struct PaintHistoryStep {
    entries: Vec<PaintHistoryEntry>,
}

const PAINT_HISTORY_LIMIT: usize = 64;

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
    primary_show_shape_badges: bool,
    secondary_show_shape_badges: bool,
    tertiary_show_shape_badges: bool,
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
        let vs = crate::view_state::ViewState::load();
        let vm = match vs.view_mode.as_str() {
            "list" => ViewMode::List,
            _ => ViewMode::Icons,
        };
        let sf = match vs.sort_field.as_str() {
            "modified" => SortField::Modified,
            "size" => SortField::Size,
            "kind" => SortField::Kind,
            _ => SortField::Name,
        };
        let sd = match vs.sort_direction.as_str() {
            "descending" => SortDirection::Descending,
            _ => SortDirection::Ascending,
        };
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
            primary_view_mode: vm,
            secondary_view_mode: vm,
            tertiary_view_mode: vm,
            primary_show_hidden: vs.show_hidden,
            secondary_show_hidden: vs.show_hidden,
            tertiary_show_hidden: vs.show_hidden,
            primary_show_shape_badges: vs.show_shape_badges,
            secondary_show_shape_badges: vs.show_shape_badges,
            tertiary_show_shape_badges: vs.show_shape_badges,
            primary_sort_field: sf,
            primary_sort_direction: sd,
            secondary_sort_field: sf,
            secondary_sort_direction: sd,
            tertiary_sort_field: sf,
            tertiary_sort_direction: sd,
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
        let vs = crate::view_state::ViewState::load();
        let dflt_view_mode = match vs.view_mode.as_str() {
            "list" => ViewMode::List,
            _ => ViewMode::Icons,
        };
        let dflt_sort_field = match vs.sort_field.as_str() {
            "modified" => SortField::Modified,
            "size" => SortField::Size,
            "kind" => SortField::Kind,
            _ => SortField::Name,
        };
        let dflt_sort_dir = match vs.sort_direction.as_str() {
            "descending" => SortDirection::Descending,
            _ => SortDirection::Ascending,
        };
        let dflt_show_hidden = vs.show_hidden;
        let dflt_show_shape_badges = vs.show_shape_badges;
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
                primary_view_mode: dflt_view_mode,
                secondary_view_mode: dflt_view_mode,
                tertiary_view_mode: dflt_view_mode,
                primary_show_hidden: dflt_show_hidden,
                secondary_show_hidden: dflt_show_hidden,
                tertiary_show_hidden: dflt_show_hidden,
                primary_show_shape_badges: dflt_show_shape_badges,
                secondary_show_shape_badges: dflt_show_shape_badges,
                tertiary_show_shape_badges: dflt_show_shape_badges,
                primary_sort_field: dflt_sort_field,
                primary_sort_direction: dflt_sort_dir,
                secondary_sort_field: dflt_sort_field,
                secondary_sort_direction: dflt_sort_dir,
                tertiary_sort_field: dflt_sort_field,
                tertiary_sort_direction: dflt_sort_dir,
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
    shape_badge_toggle_btn: Button,
    shape_badge_toggle_icon: gtk::Image,
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
    cloud_landing_panel: CloudLandingPanel,
    palette_board_panel: PaletteBoardPanel,
    project_manager_panel: ProjectManagerPanel,
    tag_manager_panel: TintsTagsPanel,
    bulk_naming_panel: BulkNamingPanel,
    space_viewer_panel: SpaceViewerPanel,
    media_convert_panel: MediaConvertPanel,
    watercolor_panel: WatercolorPanel,
}

impl PaneWidgets {
    fn build(slot: PaneSlot, config: &AppConfig) -> Self {
        let tt = |label: &str, action: &str| shortcut_tooltip(config, label, action);
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
        crate::ui::attach_tooltip(&filter_toggle_btn, tt("Tag filter", "filter_tags"));
        header.append(&filter_toggle_btn);

        let hidden_toggle_icon = gtk::Image::from_icon_name("view-reveal-symbolic");
        let hidden_toggle_btn = Button::new();
        hidden_toggle_btn.set_child(Some(&hidden_toggle_icon));
        hidden_toggle_btn.add_css_class("pane-view-btn");
        hidden_toggle_btn.add_css_class("pane-hidden-btn");
        hidden_toggle_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(&hidden_toggle_btn, tt("Hidden files", "show_hidden"));
        header.append(&hidden_toggle_btn);

        let shape_badge_toggle_icon = gtk::Image::from_icon_name("emblem-default-symbolic");
        let shape_badge_toggle_btn = Button::new();
        shape_badge_toggle_btn.set_child(Some(&shape_badge_toggle_icon));
        shape_badge_toggle_btn.add_css_class("pane-view-btn");
        shape_badge_toggle_btn.add_css_class("pane-shape-badge-btn");
        shape_badge_toggle_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(
            &shape_badge_toggle_btn,
            tt("Shape badges", "toggle_shape_badges"),
        );
        header.append(&shape_badge_toggle_btn);

        let sort_icon = gtk::Image::from_icon_name("view-sort-ascending-symbolic");
        let sort_btn = Button::new();
        sort_btn.set_child(Some(&sort_icon));
        sort_btn.add_css_class("pane-view-btn");
        sort_btn.set_valign(Align::Center);
        crate::ui::attach_tooltip(&sort_btn, tt("Sort order", "sort_order"));
        header.append(&sort_btn);

        let view_mode_icon = gtk::Image::from_icon_name("view-grid-symbolic");
        let view_mode_btn = Button::new();
        view_mode_btn.set_child(Some(&view_mode_icon));
        view_mode_btn.add_css_class("pane-view-btn");
        view_mode_btn.set_valign(Align::Center);
        let view_tt = match (
            crate::config::configured_shortcut(config, "view_icons"),
            crate::config::configured_shortcut(config, "view_list"),
        ) {
            (Some(icons), Some(list)) => format!("Icon/list view ({icons}/{list})"),
            (Some(icons), None) => format!("Icon/list view ({icons})"),
            (None, Some(list)) => format!("Icon/list view ({list})"),
            (None, None) => "Icon/list view".to_string(),
        };
        crate::ui::attach_tooltip(&view_mode_btn, view_tt);
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
        let cloud_landing_panel = CloudLandingPanel::build();
        let palette_board_panel = PaletteBoardPanel::build();
        let project_manager_panel = ProjectManagerPanel::build();

        let tag_manager_panel = TintsTagsPanel::build();
        let bulk_naming_panel = BulkNamingPanel::build();
        let space_viewer_panel = SpaceViewerPanel::build();
        let media_convert_panel = MediaConvertPanel::build(config);
        let watercolor_panel = WatercolorPanel::build();

        root.append(&header);
        root.append(&tag_filter_revealer);
        root.append(&view_strip);
        root.append(&search_revealer);
        root.append(&file_grid.root);
        root.append(&activity_log_panel.root);
        root.append(&project_landing_panel.root);
        root.append(&cloud_landing_panel.root);
        root.append(&palette_board_panel.root);
        root.append(&project_manager_panel.root);
        root.append(&tag_manager_panel.root);
        root.append(&bulk_naming_panel.root);
        root.append(&space_viewer_panel.root);
        root.append(&media_convert_panel.root);
        root.append(&watercolor_panel.root);

        Self {
            root,
            path_label,
            filter_toggle_btn,
            hidden_toggle_btn,
            hidden_toggle_icon,
            shape_badge_toggle_btn,
            shape_badge_toggle_icon,
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
            cloud_landing_panel,
            palette_board_panel,
            project_manager_panel,
            tag_manager_panel,
            bulk_naming_panel,
            space_viewer_panel,
            media_convert_panel,
            watercolor_panel,
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
        window.set_titlebar(Some(&build_titlebar(&window)));

        let places = Places::discover();
        let toolbar = Toolbar::build(&config);
        let sidebar = Sidebar::build(&config);
        let tab_strip = TabStrip::build(&config);
        let primary_pane = PaneWidgets::build(PaneSlot::Primary, &config);
        let secondary_pane = PaneWidgets::build(PaneSlot::Secondary, &config);
        let tertiary_pane = PaneWidgets::build(PaneSlot::Tertiary, &config);
        let preview = PreviewPane::build();
        let holding_tray = HoldingTray::build(&config);
        let plan_queue_panel = PlanQueuePanel::build(&config);
        let ops_panel = OpsPanel::build();
        let convert_progress = ConvertProgressPanel::build(&config);
        let status = StatusBar::build();
        let painting_toolbar = PaintingToolbar::build(&config);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&toolbar.root);
        root.append(&painting_toolbar.revealer);

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
        root.append(&convert_progress.root);
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
            convert_progress,
            body.sidebar_revealer.clone(),
            body.preview_revealer.clone(),
            body.split_paned.clone(),
            body.right_paned.clone(),
            modal_host,
            config,
            painting_toolbar,
        );
        controller.bootstrap();

        window
    }
}

fn build_titlebar(window: &ApplicationWindow) -> HeaderBar {
    let titlebar = HeaderBar::new();
    titlebar.set_show_title_buttons(false);
    titlebar.add_css_class("lattice-titlebar");

    let title = Label::new(Some("Lattice"));
    title.add_css_class("title");
    title.set_single_line_mode(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::None);
    title.set_width_chars("Lattice".chars().count() as i32);
    titlebar.set_title_widget(Some(&title));

    let controls = build_window_controls(window);
    titlebar.pack_end(&controls);

    titlebar
}

fn build_window_controls(window: &ApplicationWindow) -> GtkBox {
    let controls = GtkBox::new(Orientation::Horizontal, 2);
    controls.add_css_class("lattice-window-controls");

    let minimize_button = window_control_button("window-minimize-symbolic", "Minimize");
    {
        let window = window.clone();
        minimize_button.connect_clicked(move |_| window.minimize());
    }
    controls.append(&minimize_button);

    let maximize_button = window_control_button("window-maximize-symbolic", "Maximize / Restore");
    sync_maximize_button(&maximize_button, window.is_maximized());
    {
        let window = window.clone();
        maximize_button.connect_clicked(move |_| {
            if window.is_maximized() {
                window.unmaximize();
            } else {
                window.maximize();
            }
        });
    }
    {
        let maximize_button = maximize_button.clone();
        window.connect_maximized_notify(move |window| {
            sync_maximize_button(&maximize_button, window.is_maximized());
        });
    }
    controls.append(&maximize_button);

    let close_button = window_control_button("window-close-symbolic", "Close");
    close_button.add_css_class("lattice-window-close-button");
    {
        let window = window.clone();
        close_button.connect_clicked(move |_| window.close());
    }
    controls.append(&close_button);

    controls
}

fn window_control_button(icon_name: &str, tooltip: &str) -> Button {
    let button = Button::builder().icon_name(icon_name).build();
    button.add_css_class("lattice-window-control-button");
    button.set_tooltip_text(Some(tooltip));
    button
}

fn sync_maximize_button(button: &Button, is_maximized: bool) {
    let icon_name = if is_maximized {
        "view-restore-symbolic"
    } else {
        "window-maximize-symbolic"
    };
    button.set_icon_name(icon_name);
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
struct ConfirmationPreview {
    title: String,
    confirm_label: String,
    destructive: bool,
    lines: Vec<String>,
}

impl ConfirmationPreview {
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
    cloud_locations: RefCell<Vec<CloudRecord>>,
    removable_drives: RefCell<Vec<DriveEntry>>,
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
    primary_show_shape_badges: Cell<bool>,
    secondary_show_shape_badges: Cell<bool>,
    tertiary_show_shape_badges: Cell<bool>,
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
    path_box_debounce: RefCell<Option<glib::SourceId>>,
    path_box_scope_dir: RefCell<Option<PathBuf>>,
    primary_search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    secondary_search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    tertiary_search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    ops_panel: OpsPanel,
    convert_progress: ConvertProgressPanel,
    plan_queue_panel: PlanQueuePanel,
    conversion_queue: ConversionQueue,
    plan_mode_active: Cell<bool>,
    action_queue: RefCell<Vec<crate::action_plan::ActionPlan>>,
    executing_plan_queue: Cell<bool>,
    paint_mode_active: Cell<bool>,
    active_paint_type: Cell<PaintType>,
    active_paint_tint_id: Cell<i64>,
    active_paint_tint_color: RefCell<String>,
    active_paint_tint_name: RefCell<String>,
    active_paint_shape: Cell<Shape>,
    active_paint_tool: Cell<PaintTool>,
    active_paint_tag_id: Cell<i64>,
    active_paint_tag_name: RefCell<String>,
    paint_contents: Cell<bool>,
    current_drag_painted: RefCell<std::collections::HashSet<PathBuf>>,
    drag_history_accumulator: RefCell<Option<Vec<PaintHistoryEntry>>>,
    paint_undo_stack: RefCell<Vec<PaintHistoryStep>>,
    paint_redo_stack: RefCell<Vec<PaintHistoryStep>>,
    painting_toolbar: PaintingToolbar,
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
    // Tracks whether paint mode auto-showed badges for a pane (so we can hide them on exit)
    primary_badges_hidden_by_paint: Cell<bool>,
    secondary_badges_hidden_by_paint: Cell<bool>,
    tertiary_badges_hidden_by_paint: Cell<bool>,
    tint_css_provider: CssProvider,
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
        convert_progress: ConvertProgressPanel,
        sidebar_revealer: Revealer,
        preview_revealer: Revealer,
        split_paned: Paned,
        right_paned: Paned,
        modal_host: ModalHost,
        config: AppConfig,
        painting_toolbar: PaintingToolbar,
    ) -> Rc<Self> {
        let metadata = MetadataStore::open().or_else(|error| {
            eprintln!("Lattice metadata fallback: {error}");
            MetadataStore::open_in_memory()
        });
        let metadata = metadata.expect("Lattice could not initialize metadata storage.");
        let (initial_tab, launch_notice) = TabState::for_launch(launch, &places, &metadata);
        let vs = crate::view_state::ViewState::load();
        let dflt_view_mode = match vs.view_mode.as_str() {
            "list" => ViewMode::List,
            _ => ViewMode::Icons,
        };
        let dflt_sort_field = match vs.sort_field.as_str() {
            "modified" => SortField::Modified,
            "size" => SortField::Size,
            "kind" => SortField::Kind,
            _ => SortField::Name,
        };
        let dflt_sort_dir = match vs.sort_direction.as_str() {
            "descending" => SortDirection::Descending,
            _ => SortDirection::Ascending,
        };
        let default_tint = metadata
            .list_tints()
            .unwrap_or_default()
            .into_iter()
            .find(|t| t.is_default)
            .unwrap_or_else(|| crate::metadata::TintRecord {
                id: 1,
                name: "Beige".to_string(),
                color: Some("#806040".to_string()),
                position: 0,
                is_default: true,
            });
        Rc::new(Self {
            window,
            metadata: RefCell::new(metadata),
            user_places: RefCell::new(Vec::new()),
            cloud_locations: RefCell::new(Vec::new()),
            removable_drives: RefCell::new(Vec::new()),
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
            primary_show_hidden: Cell::new(vs.show_hidden),
            secondary_show_hidden: Cell::new(vs.show_hidden),
            tertiary_show_hidden: Cell::new(vs.show_hidden),
            primary_show_shape_badges: Cell::new(vs.show_shape_badges),
            secondary_show_shape_badges: Cell::new(vs.show_shape_badges),
            tertiary_show_shape_badges: Cell::new(vs.show_shape_badges),
            sidebar_visible: Cell::new(true),
            preview_visible: Cell::new(true),
            suppress_panel_toggle_handlers: Cell::new(false),
            pane_layout: Cell::new(PaneLayout::Single),
            primary_view_mode: Cell::new(dflt_view_mode),
            secondary_view_mode: Cell::new(dflt_view_mode),
            tertiary_view_mode: Cell::new(dflt_view_mode),
            primary_sort_field: Cell::new(dflt_sort_field),
            primary_sort_direction: Cell::new(dflt_sort_dir),
            secondary_sort_field: Cell::new(dflt_sort_field),
            secondary_sort_direction: Cell::new(dflt_sort_dir),
            tertiary_sort_field: Cell::new(dflt_sort_field),
            tertiary_sort_direction: Cell::new(dflt_sort_dir),
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
            path_box_debounce: RefCell::new(None),
            path_box_scope_dir: RefCell::new(None),
            primary_search_cancel: RefCell::new(None),
            secondary_search_cancel: RefCell::new(None),
            tertiary_search_cancel: RefCell::new(None),
            ops_panel,
            convert_progress,
            plan_queue_panel,
            conversion_queue: ConversionQueue::new(),
            plan_mode_active: Cell::new(false),
            action_queue: RefCell::new(Vec::new()),
            executing_plan_queue: Cell::new(false),
            paint_mode_active: Cell::new(false),
            active_paint_type: Cell::new(PaintType::Mark),
            active_paint_tint_id: Cell::new(default_tint.id),
            active_paint_tint_color: RefCell::new(
                default_tint
                    .color
                    .clone()
                    .unwrap_or_else(|| "#806040".to_string()),
            ),
            active_paint_tint_name: RefCell::new(default_tint.name.clone()),
            active_paint_shape: Cell::new(Shape::DEFAULT),
            active_paint_tool: Cell::new(PaintTool::Brush),
            active_paint_tag_id: Cell::new(0),
            active_paint_tag_name: RefCell::new(String::new()),
            paint_contents: Cell::new(false),
            current_drag_painted: RefCell::new(std::collections::HashSet::new()),
            drag_history_accumulator: RefCell::new(None),
            paint_undo_stack: RefCell::new(Vec::new()),
            paint_redo_stack: RefCell::new(Vec::new()),
            painting_toolbar,
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
            primary_badges_hidden_by_paint: Cell::new(false),
            secondary_badges_hidden_by_paint: Cell::new(false),
            tertiary_badges_hidden_by_paint: Cell::new(false),
            tint_css_provider: CssProvider::new(),
        })
    }

    fn bootstrap(self: &Rc<Self>) {
        self.init_tint_css();
        self.apply_tint_css();
        self.connect_navigation();
        {
            let vs = crate::view_state::ViewState::load();
            self.apply_sidebar_visibility(vs.sidebar_visible);
            self.apply_preview_visibility(vs.preview_visible);
        }
        self.cleanup_convert_temps();
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
        self.connect_media_convert_actions();
        self.refresh_metadata_sidebar();
        self.refresh_watercolor_sidebar_visibility();
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
        self.toolbar
            .paint_mode_toggle
            .connect_toggled(move |toggle| controller.set_paint_mode(toggle.is_active()));

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
        self.attach_path_live_search();

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
            .add_by_tint_button
            .connect_clicked(move |btn| {
                controller.show_add_to_tray_by_tint_popover(btn);
            });

        let controller = Rc::clone(self);
        self.holding_tray
            .add_by_shape_button
            .connect_clicked(move |btn| {
                controller.show_add_to_tray_by_shape_popover(btn);
            });

        let controller = Rc::clone(self);
        self.holding_tray
            .apply_mark_button
            .connect_clicked(move |_| {
                controller.show_tray_apply_mark_preview();
            });

        let controller = Rc::clone(self);
        self.holding_tray
            .reset_mark_button
            .connect_clicked(move |_| {
                controller.show_tray_reset_mark_preview();
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

    fn attach_path_live_search(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.toolbar.path_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();

            if let Some(id) = controller.path_box_debounce.borrow_mut().take() {
                id.remove();
            }

            if text.is_empty() {
                if let Some(dir) = controller.path_box_scope_dir.borrow_mut().take() {
                    controller.navigate_to(controller.active_slot(), dir, false);
                }
                return;
            }

            if looks_like_explicit_path(&text) {
                return;
            }

            let slot = controller.active_slot();
            if let PaneView::Search(ref q) = controller.current_view_for(slot) {
                if q.name == text && controller.path_box_scope_dir.borrow().is_some() {
                    return;
                }
            }

            let c = Rc::clone(&controller);
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(280),
                move || {
                    c.path_box_debounce.borrow_mut().take();
                    c.run_path_box_search();
                },
            );
            *controller.path_box_debounce.borrow_mut() = Some(id);
        });
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
            self.sidebar.projects_button.clone(),
            self.sidebar.tags_button.clone(),
            self.sidebar.search_button.clone(),
            self.sidebar.space_viewer_button.clone(),
            self.sidebar.triage_button.clone(),
            self.sidebar.bulk_naming_button.clone(),
            self.sidebar.convert_button.clone(),
            self.sidebar.activity_log_button.clone(),
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
            .bulk_naming_button
            .connect_clicked(move |_| controller.open_bulk_naming_tool());
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
        let controller = Rc::clone(self);
        self.sidebar
            .projects_button
            .connect_clicked(move |_| controller.open_project_manager());
        let controller = Rc::clone(self);
        self.sidebar
            .space_viewer_button
            .connect_clicked(move |_| controller.open_space_viewer());
        let controller = Rc::clone(self);
        self.sidebar
            .convert_button
            .connect_clicked(move |_| controller.open_convert_from_sidebar());
        let controller = Rc::clone(self);
        self.sidebar
            .watercolor_status_button
            .connect_clicked(move |_| controller.open_watercolor(WatercolorView::Status));
        let controller = Rc::clone(self);
        self.sidebar
            .watercolor_workspaces_button
            .connect_clicked(move |_| controller.open_watercolor(WatercolorView::Workspaces));
        let controller = Rc::clone(self);
        self.sidebar
            .watercolor_palettes_button
            .connect_clicked(move |_| controller.open_watercolor(WatercolorView::Palettes));
        let controller = Rc::clone(self);
        self.sidebar
            .watercolor_broken_refs_button
            .connect_clicked(move |_| controller.open_watercolor(WatercolorView::BrokenRefs));
        // cloud_add_button and rclone_setup_button are persistent, so connect once here
        let controller = Rc::clone(self);
        self.sidebar
            .cloud_add_button
            .connect_clicked(move |_| controller.show_add_cloud_dialog(None));
        let controller = Rc::clone(self);
        self.sidebar
            .rclone_setup_button
            .connect_clicked(move |_| controller.show_rclone_setup_dialog());

        let volume_monitor = gio::VolumeMonitor::get();
        let ctrl = Rc::clone(self);
        volume_monitor.connect_mount_added(move |_, _| ctrl.refresh_drive_sidebar());
        let ctrl = Rc::clone(self);
        volume_monitor.connect_mount_removed(move |_, _| ctrl.refresh_drive_sidebar());
    }

    fn open_convert_from_sidebar(self: &Rc<Self>) {
        let slot = self.active_slot();
        let items = self.resolve_convert_source_items(slot);
        self.open_media_convert_with_items(slot, items);
    }

    fn resolve_convert_source_items(self: &Rc<Self>, slot: PaneSlot) -> Vec<FileItem> {
        let mode = self.pane_widgets(slot).media_convert_panel.source_mode();
        let tray = self.holding_tray_items.borrow().clone();
        let sel = self.selected_items_for(slot);
        match mode {
            ConvertSourceMode::Auto => {
                if !tray.is_empty() {
                    tray
                } else {
                    sel
                }
            }
            ConvertSourceMode::Tray => tray,
            ConvertSourceMode::Selection => sel,
        }
    }

    fn reload_convert_items(self: &Rc<Self>, slot: PaneSlot) {
        let items = self.resolve_convert_source_items(slot);
        let convert_items: Vec<crate::converter::ConvertItem> = items
            .into_iter()
            .filter(|i| matches!(i.kind, FileKind::Image | FileKind::Video | FileKind::Audio))
            .map(|i| crate::converter::ConvertItem {
                path: i.path.clone(),
                kind: match i.kind {
                    FileKind::Image => crate::converter::MediaKind::Image,
                    FileKind::Audio => crate::converter::MediaKind::Audio,
                    _ => crate::converter::MediaKind::Video,
                },
            })
            .collect();
        self.pane_widgets(slot).media_convert_panel.set_items(
            convert_items,
            &self.conversion_queue.tools,
            None,
        );
    }

    fn open_space_viewer(self: &Rc<Self>) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::SpaceViewer { .. }) {
            return;
        }
        self.save_dir_to_history_if_in_directory(slot);
        let dir = self.tool_scope_dir_for(slot);
        self.current_view_cell(slot)
            .replace(PaneView::SpaceViewer { root: dir });
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_space_viewer_view(slot);
    }

    fn load_space_viewer_view(self: &Rc<Self>, slot: PaneSlot) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let pane = self.pane_widgets(slot);
        let dir = match self.current_view_for(slot) {
            PaneView::SpaceViewer { root } => root,
            _ => self.tool_scope_dir_for(slot),
        };
        let display_label = self.display_label_for(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        let controller = Rc::clone(self);
        pane.space_viewer_panel.connect_open_file(move |path| {
            controller.open_file(&path);
        });
        let controller = Rc::clone(self);
        pane.space_viewer_panel.connect_reveal_file(move |path| {
            let folder = if path.is_dir() {
                path
            } else {
                path.parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| controller.current_dir_for(controller.active_slot()))
            };
            controller.navigate_to(controller.active_slot(), folder, true);
        });
        let controller = Rc::clone(self);
        pane.space_viewer_panel.connect_add_to_tray(move |paths| {
            controller.add_paths_to_holding_tray(paths);
        });
        let controller = Rc::clone(self);
        pane.space_viewer_panel.connect_copy_path(move |path| {
            controller.copy_paths_to_clipboard(vec![path]);
        });
        let controller = Rc::clone(self);
        pane.space_viewer_panel.connect_trash_file(move |path| {
            controller.move_paths_to_trash(vec![path]);
        });
        let controller = Rc::clone(self);
        pane.space_viewer_panel
            .connect_scan_complete(move |scan_root| {
                controller.load_space_viewer_mark_stats(&scan_root, slot);
            });

        pane.space_viewer_panel.set_folder(&dir);
        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            let cloud_ctx = self
                .cloud_name_for_path(&dir)
                .map(|(name, kind)| format!("☁ {name} ({kind})"));
            self.status.set_cloud_context(cloud_ctx.as_deref());
            if let Some((name, kind)) = self.cloud_name_for_path(&dir) {
                self.status.set_message(&format!(
                    "☁ Cloud drive: {name} ({kind}) — recursive scans may be slow; use cancel if needed"
                ));
            }
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }
    }

    fn load_space_viewer_mark_stats(self: &Rc<Self>, root: &std::path::Path, slot: PaneSlot) {
        use crate::ui::space_viewer_panel::{ShapeStat, TintStat};
        let marks = self
            .metadata
            .borrow()
            .list_marks_under_prefix(root)
            .unwrap_or_default();
        if marks.is_empty() {
            self.pane_widgets(slot)
                .space_viewer_panel
                .set_mark_stats(Vec::new(), Vec::new());
            return;
        }
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        let tint_map: std::collections::HashMap<i64, &crate::metadata::TintRecord> =
            tints.iter().map(|t| (t.id, t)).collect();

        let mut tint_stats: std::collections::HashMap<i64, TintStat> =
            std::collections::HashMap::new();
        let mut shape_stats: std::collections::HashMap<String, ShapeStat> =
            std::collections::HashMap::new();

        for (path, tint_id, shape) in &marks {
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            let te = tint_stats.entry(*tint_id).or_insert_with(|| {
                let t = tint_map.get(tint_id);
                TintStat {
                    tint_id: *tint_id,
                    name: t
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| format!("Tint {tint_id}")),
                    color: t
                        .and_then(|t| t.color.clone())
                        .unwrap_or_else(|| "#806040".to_string()),
                    count: 0,
                    bytes: 0,
                }
            });
            te.count += 1;
            te.bytes += size;

            let se = shape_stats
                .entry(shape.as_str().to_string())
                .or_insert_with(|| ShapeStat {
                    shape: *shape,
                    count: 0,
                    bytes: 0,
                });
            se.count += 1;
            se.bytes += size;
        }

        let mut by_tint: Vec<TintStat> = tint_stats.into_values().collect();
        by_tint.sort_by(|a, b| b.count.cmp(&a.count));
        let mut by_shape: Vec<ShapeStat> = shape_stats.into_values().collect();
        by_shape.sort_by(|a, b| b.count.cmp(&a.count));

        self.pane_widgets(slot)
            .space_viewer_panel
            .set_mark_stats(by_tint, by_shape);
    }

    fn open_activity_log(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.save_dir_to_history_if_in_directory(slot);
        self.current_view_cell(slot).replace(PaneView::ActivityLog);
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_activity_log_view(slot);
    }

    fn open_watercolor(self: &Rc<Self>, view: WatercolorView) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::Watercolor(current) if current == view) {
            return;
        }
        self.save_dir_to_history_if_in_directory(slot);
        self.current_view_cell(slot)
            .replace(PaneView::Watercolor(view.clone()));
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_watercolor_view(slot, view);
        self.update_navigation_state();
    }

    fn load_watercolor_view(self: &Rc<Self>, slot: PaneSlot, view: WatercolorView) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let pane = self.pane_widgets(slot);
        let display_label = watercolor_display_label(&view);
        pane.path_label.set_label(display_label);
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();
        pane.watercolor_panel
            .set_loading(&watercolor_panel_view(&view));
        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(display_label);
            self.status
                .set_message("Loading Watercolor context from Terroir.");
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
            self.preview.set_action_state(false, false, false);
        }

        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(fetch_watercolor_panel_data());
        });

        let controller = Rc::clone(self);
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || match receiver
            .try_recv()
        {
            Ok(data) => {
                if !matches!(controller.current_view_for(slot), PaneView::Watercolor(current) if current == view)
                {
                    return glib::ControlFlow::Break;
                }

                let open_controller = Rc::clone(&controller);
                let on_open_path = move |path: PathBuf| {
                    open_controller.open_watercolor_path(path);
                };
                let refresh_controller = Rc::clone(&controller);
                let on_refresh = move || {
                    refresh_controller.refresh_watercolor_context();
                };

                controller.pane_widgets(slot).watercolor_panel.populate(
                    watercolor_panel_view(&view),
                    &data,
                    on_open_path,
                    on_refresh,
                );
                if slot == controller.active_slot() {
                    match &data.status {
                        Ok(_) => controller.status.set_message("Watercolor context loaded."),
                        Err(error) => controller
                            .status
                            .set_message(&format!("Watercolor context unavailable: {error}")),
                    }
                    controller.status.set_counts(data.workspaces.len(), 0);
                    controller.update_action_state();
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                controller
                    .status
                    .set_message("Watercolor context request failed.");
                glib::ControlFlow::Break
            }
        });
    }

    fn open_watercolor_path(self: &Rc<Self>, path: PathBuf) {
        if !path.exists() {
            self.status.set_message(&format!(
                "Watercolor reference is missing: {}",
                path.display()
            ));
            return;
        }

        if path.is_dir() {
            self.navigate_to(self.active_slot(), path, true);
            return;
        }

        self.open_file(&path);
    }

    fn refresh_watercolor_context(self: &Rc<Self>) {
        self.status
            .set_message("Refreshing Watercolor context through Terroir.");
        let current_view = match self.current_view_for(self.active_slot()) {
            PaneView::Watercolor(view) => Some(view),
            _ => None,
        };
        let slot = self.active_slot();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(terroir_client::reindex());
        });

        let controller = Rc::clone(self);
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || match receiver
            .try_recv()
        {
            Ok(Ok(result)) => {
                controller.status.set_message(&format!(
                    "Watercolor context refreshed: indexed {} workspace(s), {} error(s).",
                    result.indexed_workspaces, result.errors
                ));
                controller.refresh_watercolor_sidebar_visibility();
                if let Some(view) = current_view.clone() {
                    if matches!(controller.current_view_for(slot), PaneView::Watercolor(current) if current == view)
                    {
                        controller.load_watercolor_view(slot, view);
                    }
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                controller.status.set_message(&format!(
                    "Watercolor refresh unavailable: {}",
                    terroir_error_message(&error)
                ));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                controller.status.set_message("Watercolor refresh failed.");
                glib::ControlFlow::Break
            }
        });
    }

    fn refresh_watercolor_sidebar_visibility(self: &Rc<Self>) {
        let config_enabled = self.config.enable_terroir_context;
        self.sidebar.set_watercolor_visible(config_enabled);

        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let running = terroir_client::status().is_ok();
            let has_workspaces = terroir_client::list_workspaces()
                .map(|workspaces| !workspaces.is_empty())
                .unwrap_or(false);
            let _ = sender.send(running || has_workspaces);
        });

        let sidebar = self.sidebar.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || match receiver
            .try_recv()
        {
            Ok(available) => {
                sidebar.set_watercolor_visible(config_enabled || available);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
    }

    fn open_project_manager(self: &Rc<Self>) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::ProjectManager) {
            return;
        }
        self.save_dir_to_history_if_in_directory(slot);
        self.current_view_cell(slot)
            .replace(PaneView::ProjectManager);
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_project_manager_view(slot);
        self.update_navigation_state();
    }

    fn load_project_manager_view(self: &Rc<Self>, slot: PaneSlot) {
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

        let projects = self.projects.borrow().clone();
        let project_count = projects.len();
        pane.project_manager_panel.set_projects(&projects);

        let controller = Rc::clone(self);
        pane.project_manager_panel
            .connect_project_clicked(move |project_id| {
                controller.open_project(project_id);
            });
        let controller = Rc::clone(self);
        pane.project_manager_panel
            .connect_project_created(move |name| {
                controller.handle_project_created(name);
            });
        let controller = Rc::clone(self);
        pane.project_manager_panel
            .connect_project_renamed(move |id, name| {
                controller.handle_project_renamed(id, name);
            });
        let controller = Rc::clone(self);
        pane.project_manager_panel
            .connect_project_deleted(move |id| {
                controller.handle_project_deleted(id);
            });

        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(project_count, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }
    }

    fn handle_project_created(self: &Rc<Self>, name: String) {
        let result = self.metadata.borrow_mut().create_project(&name, None);
        match result {
            Ok(_) => {
                self.refresh_metadata_sidebar();
                self.reload_project_manager_if_visible();
            }
            Err(error) => {
                self.modal_host.show_error("Create Palette Failed", &error);
            }
        }
    }

    fn handle_project_renamed(self: &Rc<Self>, id: i64, name: String) {
        let result = self.metadata.borrow_mut().rename_project(id, &name);
        match result {
            Ok(_) => {
                self.refresh_metadata_sidebar();
                self.reload_project_manager_if_visible();
            }
            Err(error) => {
                self.modal_host.show_error("Rename Palette Failed", &error);
            }
        }
    }

    fn handle_project_deleted(self: &Rc<Self>, project_id: i64) {
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
            "Delete Palette",
            &format!(
                "Delete \u{201c}{}\u{201d}? Pinned folders will not be affected.",
                project.name
            ),
            "Delete",
            true,
            false,
            move || {
                let _ = controller.metadata.borrow_mut().delete_project(project_id);
                for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
                    if matches!(
                        controller.current_view_for(slot),
                        PaneView::ProjectLanding(id) if id == project_id
                    ) {
                        controller
                            .current_view_cell(slot)
                            .replace(PaneView::ProjectManager);
                        controller.load_project_manager_view(slot);
                    }
                }
                controller.refresh_metadata_sidebar();
                controller.reload_project_manager_if_visible();
            },
        );
    }

    fn reload_project_manager_if_visible(self: &Rc<Self>) {
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::ProjectManager) {
                self.load_project_manager_view(slot);
            }
        }
    }

    fn open_tag_manager(self: &Rc<Self>) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::TagManager) {
            return;
        }
        self.save_dir_to_history_if_in_directory(slot);
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

        let (tints, tags, counts) = {
            let meta = self.metadata.borrow();
            let tints = meta.list_tints().unwrap_or_default();
            let tags = meta.list_tags().unwrap_or_default();
            let counts = meta.count_files_per_tag().unwrap_or_default();
            (tints, tags, counts)
        };
        let item_count = tints.len() + tags.len();
        pane.tag_manager_panel.set_tints(&tints);
        pane.tag_manager_panel.set_tags(&tags, &counts, &tints);

        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_clicked(move |tag_id| {
            controller.open_tag(tag_id);
        });
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_created(move |name| {
            controller.handle_tag_created(name);
        });
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_renamed(move |id, name| {
            controller.handle_tag_renamed(id, name);
        });
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tag_deleted(move |id| {
            controller.handle_tag_deleted(id);
        });
        let controller = Rc::clone(self);
        pane.tag_manager_panel
            .connect_tint_created(move |name, color| {
                controller.handle_tint_created(name, color);
            });
        let controller = Rc::clone(self);
        pane.tag_manager_panel
            .connect_tint_renamed(move |id, name| {
                controller.handle_tint_renamed(id, name);
            });
        let controller = Rc::clone(self);
        pane.tag_manager_panel
            .connect_tint_color_changed(move |id, color| {
                controller.handle_tint_color_changed(id, color);
            });
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tint_color_pick_requested(
            move |title, initial, on_selected| {
                controller.show_tint_color_picker(&title, &initial, on_selected);
            },
        );
        let controller = Rc::clone(self);
        pane.tag_manager_panel.connect_tint_deleted(move |id| {
            controller.handle_tint_deleted(id);
        });

        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(item_count, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }
    }

    fn handle_tag_created(self: &Rc<Self>, name: String) {
        let result = self.metadata.borrow_mut().ensure_tag(&name);
        match result {
            Ok(_) => {
                self.refresh_metadata_sidebar();
                self.reload_tag_manager_if_visible();
                self.status
                    .set_message(&format!("Tag \u{2018}{name}\u{2019} created."));
            }
            Err(e) => {
                self.modal_host.show_error("Create Tag Failed", &e);
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
                self.modal_host.show_error("Rename Tag Failed", &e);
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
            format!("Delete tag \u{2018}{tag_name}\u{2019}? This cannot be undone.")
        } else {
            format!(
                "Delete tag \u{2018}{tag_name}\u{2019}? It will be removed from {count} file(s). This cannot be undone."
            )
        };
        let controller = Rc::clone(self);
        self.modal_host
            .show_confirm("Delete Tag", &prompt, "Delete", true, false, move || {
                let result = controller.metadata.borrow_mut().delete_tag(id);
                match result {
                    Ok(()) => {
                        controller.refresh_metadata_sidebar();
                        controller.reload_tag_manager_if_visible();
                        controller.status.set_message("Tag deleted.");
                    }
                    Err(e) => {
                        controller.modal_host.show_error("Delete Tag Failed", &e);
                    }
                }
            });
    }

    fn reload_tag_manager_if_visible(self: &Rc<Self>) {
        self.apply_tint_css();
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::TagManager) {
                self.load_tag_manager_view(slot);
            }
        }
    }

    fn handle_tint_created(self: &Rc<Self>, name: String, color: String) {
        let result = self.metadata.borrow_mut().create_tint(&name, &color);
        match result {
            Ok(_) => {
                self.reload_tag_manager_if_visible();
                self.status
                    .set_message(&format!("Tint \u{2018}{name}\u{2019} created."));
            }
            Err(e) => {
                self.modal_host.show_error("Create Tint Failed", &e);
            }
        }
    }

    fn handle_tint_renamed(self: &Rc<Self>, id: i64, new_name: String) {
        let result = self.metadata.borrow_mut().rename_tint(id, &new_name);
        match result {
            Ok(()) => self.reload_tag_manager_if_visible(),
            Err(e) => self.modal_host.show_error("Rename Tint Failed", &e),
        }
    }

    fn handle_tint_color_changed(self: &Rc<Self>, id: i64, color: String) {
        let result = self.metadata.borrow_mut().update_tint_color(id, &color);
        match result {
            Ok(()) => self.reload_tag_manager_if_visible(),
            Err(e) => self.modal_host.show_error("Update Tint Color Failed", &e),
        }
    }

    fn show_tint_color_picker(
        self: &Rc<Self>,
        title: &str,
        initial: &str,
        on_selected: Box<dyn Fn(String)>,
    ) {
        let state = Rc::new(RefCell::new(parse_hex_rgb(initial)));

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.add_css_class("tint-picker-content");
        content.set_size_request(520, 380);
        content.set_hexpand(false);
        content.set_vexpand(false);

        let preview_row = GtkBox::new(Orientation::Horizontal, 12);
        preview_row.add_css_class("tint-picker-preview-row");

        let preview = DrawingArea::new();
        preview.add_css_class("tint-picker-preview");
        preview.set_content_width(132);
        preview.set_content_height(82);
        preview.set_size_request(132, 82);
        {
            let state = Rc::clone(&state);
            preview.set_draw_func(move |_, cr, w, h| {
                let (r, g, b) = *state.borrow();
                cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                cr.rectangle(0.0, 0.0, w as f64, h as f64);
                let _ = cr.fill();
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
                cr.set_line_width(1.0);
                cr.rectangle(0.5, 0.5, (w - 1) as f64, (h - 1) as f64);
                let _ = cr.stroke();
            });
        }
        preview_row.append(&preview);

        let preview_meta = GtkBox::new(Orientation::Vertical, 6);
        preview_meta.set_hexpand(true);
        let picker_title = Label::new(Some("Tint color"));
        picker_title.add_css_class("tint-picker-label");
        picker_title.set_halign(Align::Start);
        let hex_label = Label::new(Some(&rgb_to_hex(*state.borrow())));
        hex_label.add_css_class("tint-picker-hex");
        hex_label.set_halign(Align::Start);
        let note = Label::new(Some("Choose a preset or tune red, green, and blue."));
        note.add_css_class("tint-picker-note");
        note.set_halign(Align::Start);
        note.set_wrap(true);
        preview_meta.append(&picker_title);
        preview_meta.append(&hex_label);
        preview_meta.append(&note);
        preview_row.append(&preview_meta);
        content.append(&preview_row);

        let update_ui: Rc<dyn Fn()> = {
            let preview = preview.clone();
            let hex_label = hex_label.clone();
            let state = Rc::clone(&state);
            Rc::new(move || {
                hex_label.set_label(&rgb_to_hex(*state.borrow()));
                preview.queue_draw();
            })
        };

        let red = build_tint_channel_row("Red", state.borrow().0);
        let green = build_tint_channel_row("Green", state.borrow().1);
        let blue = build_tint_channel_row("Blue", state.borrow().2);
        content.append(&red.0);
        content.append(&green.0);
        content.append(&blue.0);

        wire_tint_channel(&red.1, Rc::clone(&state), 0, Rc::clone(&update_ui));
        wire_tint_channel(&green.1, Rc::clone(&state), 1, Rc::clone(&update_ui));
        wire_tint_channel(&blue.1, Rc::clone(&state), 2, Rc::clone(&update_ui));

        let presets_label = Label::new(Some("Presets"));
        presets_label.add_css_class("tint-picker-label");
        presets_label.set_halign(Align::Start);
        content.append(&presets_label);

        let presets = GtkBox::new(Orientation::Horizontal, 8);
        presets.add_css_class("tint-picker-presets");
        for hex in [
            "#806040", "#c9962e", "#c84070", "#8040a0", "#5080b8", "#3d8060", "#d09458", "#e8dcc8",
        ] {
            let btn = Button::new();
            btn.add_css_class("tint-picker-preset");
            btn.set_size_request(42, 30);
            let swatch = DrawingArea::new();
            swatch.set_content_width(34);
            swatch.set_content_height(22);
            swatch.set_size_request(34, 22);
            let (sr, sg, sb) = parse_hex_rgb(hex);
            swatch.set_draw_func(move |_, cr, w, h| {
                cr.set_source_rgb(sr as f64 / 255.0, sg as f64 / 255.0, sb as f64 / 255.0);
                cr.rectangle(0.0, 0.0, w as f64, h as f64);
                let _ = cr.fill();
            });
            btn.set_child(Some(&swatch));
            {
                let state = Rc::clone(&state);
                let update_ui = Rc::clone(&update_ui);
                let red_scale = red.1.clone();
                let green_scale = green.1.clone();
                let blue_scale = blue.1.clone();
                btn.connect_clicked(move |_| {
                    *state.borrow_mut() = (sr, sg, sb);
                    red_scale.set_value(sr as f64);
                    green_scale.set_value(sg as f64);
                    blue_scale.set_value(sb as f64);
                    update_ui();
                });
            }
            presets.append(&btn);
        }
        content.append(&presets);

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let state_for_apply = Rc::clone(&state);
        let on_selected = Rc::new(RefCell::new(Some(on_selected)));
        let apply_btn = build_modal_button("Apply", ButtonKind::Primary, move || {
            if let Some(callback) = on_selected.borrow_mut().take() {
                callback(rgb_to_hex(*state_for_apply.borrow()));
            }
            host.hide();
        });
        actions.append(&apply_btn);

        let host = self.modal_host.clone();
        self.modal_host.show_with_custom_ui(
            title,
            &content,
            &actions,
            true,
            Some(Box::new(move || host.hide())),
        );
        apply_btn.grab_focus();
    }

    fn handle_tint_deleted(self: &Rc<Self>, id: i64) {
        let tint_name = {
            self.metadata
                .borrow()
                .list_tints()
                .unwrap_or_default()
                .into_iter()
                .find(|t| t.id == id)
                .map(|t| t.name)
                .unwrap_or_default()
        };
        let prompt = format!("Delete tint \u{2018}{tint_name}\u{2019}? This cannot be undone.");
        let controller = Rc::clone(self);
        self.modal_host
            .show_confirm("Delete Tint", &prompt, "Delete", true, false, move || {
                let result = controller.metadata.borrow_mut().delete_tint(id);
                match result {
                    Ok(()) => {
                        controller.reload_tag_manager_if_visible();
                        controller.status.set_message("Tint deleted.");
                    }
                    Err(e) => {
                        controller.modal_host.show_error("Delete Tint Failed", &e);
                    }
                }
            });
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

        pane.activity_log_panel.connect_cleanup({
            let controller = Rc::clone(self);
            move |cutoff_ms| {
                controller
                    .metadata
                    .borrow()
                    .delete_activity_before(cutoff_ms);
                controller.load_activity_log_view(slot);
            }
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
            self.status.set_message("Palette not found.");
            return;
        };

        let destinations = self
            .metadata
            .borrow()
            .list_project_destinations(project_id)
            .unwrap_or_default();

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
        let on_remove_pin = move |dest_id: i64| {
            controller.remove_project_destination(dest_id, project_id, slot);
        };

        let controller = Rc::clone(self);
        let on_pin_folder = move || {
            controller.show_pin_folder_dialog(project_id, slot, None, None);
        };

        pane.project_landing_panel.populate(
            &project,
            &destinations,
            on_navigate,
            on_remove_pin,
            on_pin_folder,
        );

        // Populate the Palette Board and wire back navigation into its toolbar
        self.populate_palette_board(slot, project_id);
        {
            let ctrl = Rc::clone(self);
            self.pane_widgets(slot)
                .palette_board_panel
                .set_palette_info(&project.name, move || ctrl.open_project_manager());
        }

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

    fn populate_palette_board(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
        let items = self
            .metadata
            .borrow()
            .list_palette_items(palette_id)
            .unwrap_or_default();
        let links = self
            .metadata
            .borrow()
            .list_palette_links(palette_id)
            .unwrap_or_default();
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();

        let pane = self.pane_widgets(slot);
        let board = pane.palette_board_panel.clone();

        // Wire up board callbacks
        {
            let controller = Rc::clone(self);
            let board2 = board.clone();
            board.set_callbacks(
                // on_item_moved
                {
                    let ctrl = Rc::clone(&controller);
                    let brd = board2.clone();
                    move |id, x, y| {
                        let item = brd.items.borrow().iter().find(|i| i.id == id).cloned();
                        if let Some(item) = item {
                            let _ = ctrl.metadata.borrow_mut().update_palette_item_geometry(
                                id,
                                x,
                                y,
                                item.width,
                                item.height,
                            );
                        }
                    }
                },
                // on_item_resized
                {
                    let ctrl = Rc::clone(&controller);
                    let brd = board2.clone();
                    move |id, w, h| {
                        let item = brd.items.borrow().iter().find(|i| i.id == id).cloned();
                        if let Some(item) = item {
                            let _ = ctrl
                                .metadata
                                .borrow_mut()
                                .update_palette_item_geometry(id, item.x, item.y, w, h);
                        }
                    }
                },
                // on_item_deleted
                {
                    let ctrl = Rc::clone(&controller);
                    move |id| {
                        let _ = ctrl.metadata.borrow_mut().delete_palette_item(id);
                    }
                },
                // on_note_edited
                {
                    let ctrl = Rc::clone(&controller);
                    move |id, title: Option<String>, body: Option<String>| {
                        let _ = ctrl.metadata.borrow_mut().update_palette_item_content(
                            id,
                            title.as_deref(),
                            body.as_deref(),
                        );
                    }
                },
                // on_link_created
                {
                    let ctrl = Rc::clone(&controller);
                    let brd = board2.clone();
                    move |src_id, dst_id, strength: String| {
                        let result = ctrl
                            .metadata
                            .borrow_mut()
                            .create_palette_link(palette_id, src_id, dst_id, &strength);
                        if let Ok(_link) = result {
                            // Refresh links on the board
                            let links = ctrl
                                .metadata
                                .borrow()
                                .list_palette_links(palette_id)
                                .unwrap_or_default();
                            brd.set_links(links);
                        }
                    }
                },
                // on_link_deleted
                {
                    let ctrl = Rc::clone(&controller);
                    move |id| {
                        let _ = ctrl.metadata.borrow_mut().delete_palette_link(id);
                    }
                },
                // on_add_file_card
                {
                    let ctrl = Rc::clone(&controller);
                    move || {
                        ctrl.show_add_file_card_dialog(slot, palette_id);
                    }
                },
                // on_add_folder_card
                {
                    let ctrl = Rc::clone(&controller);
                    move || {
                        ctrl.show_add_folder_card_dialog(slot, palette_id);
                    }
                },
                // on_add_note_card
                {
                    let ctrl = Rc::clone(&controller);
                    move || {
                        ctrl.add_note_card_to_board(slot, palette_id);
                    }
                },
            );
        }

        board.populate(palette_id, items, links, tints);
    }

    fn show_add_file_card_dialog(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
        let initial_dir = self.current_dir_for(slot);
        let places = self.user_places.borrow().clone();
        let cloud_locs = self.cloud_locations.borrow().clone();
        let recent = self
            .metadata
            .borrow()
            .list_recent_locations(8)
            .unwrap_or_default();
        let controller = Rc::clone(self);
        show_picker_modal(
            &self.modal_host,
            PickerConfig::open_file(initial_dir),
            &places,
            &cloud_locs,
            &recent,
            move |result| {
                let PickerResult::Single(path) = result else {
                    return;
                };
                let path_str = path.to_string_lossy().to_string();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path_str.clone());
                let offset = controller
                    .metadata
                    .borrow()
                    .list_palette_items(palette_id)
                    .map(|items| items.len() as i64 * 20)
                    .unwrap_or(0);
                let result = controller.metadata.borrow_mut().create_palette_item(
                    palette_id,
                    "file",
                    Some(&path_str),
                    Some(&name),
                    None,
                    None,
                    None,
                    60 + offset,
                    60 + offset,
                    220,
                    160,
                );
                if let Ok(item) = result {
                    controller
                        .pane_widgets(slot)
                        .palette_board_panel
                        .add_card(item);
                }
            },
            || {},
        );
    }

    fn show_add_folder_card_dialog(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
        let initial_dir = self.current_dir_for(slot);
        let places = self.user_places.borrow().clone();
        let cloud_locs = self.cloud_locations.borrow().clone();
        let recent = self
            .metadata
            .borrow()
            .list_recent_locations(8)
            .unwrap_or_default();
        let controller = Rc::clone(self);
        show_picker_modal(
            &self.modal_host,
            PickerConfig::open_folder(initial_dir),
            &places,
            &cloud_locs,
            &recent,
            move |result| {
                let PickerResult::Single(path) = result else {
                    return;
                };
                let path_str = path.to_string_lossy().to_string();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path_str.clone());
                let offset = controller
                    .metadata
                    .borrow()
                    .list_palette_items(palette_id)
                    .map(|items| items.len() as i64 * 20)
                    .unwrap_or(0);
                let result = controller.metadata.borrow_mut().create_palette_item(
                    palette_id,
                    "folder",
                    Some(&path_str),
                    Some(&name),
                    None,
                    None,
                    None,
                    60 + offset,
                    60 + offset,
                    220,
                    160,
                );
                if let Ok(item) = result {
                    controller
                        .pane_widgets(slot)
                        .palette_board_panel
                        .add_card(item);
                }
            },
            || {},
        );
    }

    fn add_note_card_to_board(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
        let offset = {
            self.metadata
                .borrow()
                .list_palette_items(palette_id)
                .map(|items| items.len() as i64 * 20)
                .unwrap_or(0)
        };
        let result = self.metadata.borrow_mut().create_palette_item(
            palette_id,
            "note",
            None,
            Some(""),
            Some(""),
            None,
            None,
            60 + offset,
            60 + offset,
            220,
            160,
        );
        if let Ok(item) = result {
            self.pane_widgets(slot).palette_board_panel.add_card(item);
        }
    }

    fn show_pin_folder_dialog(
        self: &Rc<Self>,
        project_id: i64,
        slot: PaneSlot,
        prefill_name: Option<String>,
        prefill_path: Option<PathBuf>,
    ) {
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.append(&build_modal_prompt(
            "Enter a name and choose the folder to pin.",
        ));

        let name_entry = Entry::new();
        name_entry.set_placeholder_text(Some("Display name (e.g. Inbox)"));
        if let Some(ref n) = prefill_name {
            name_entry.set_text(n);
        }
        content.append(&name_entry);

        // Path row: read-only display + Browse button
        let path_display = Entry::new();
        path_display.set_editable(false);
        path_display.add_css_class("picker-chosen-path-display");
        path_display.set_hexpand(true);
        if let Some(ref p) = prefill_path {
            path_display.set_text(&p.to_string_lossy());
        } else {
            path_display.set_placeholder_text(Some("No folder chosen"));
        }

        let chosen_path: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(prefill_path.clone()));

        let browse_btn = build_modal_button("Browse…", ButtonKind::Secondary, || {});
        browse_btn.connect_clicked({
            let controller = Rc::clone(self);
            let name_entry = name_entry.clone();
            let chosen_path = Rc::clone(&chosen_path);
            move |_| {
                let current_name = name_entry.text().to_string();
                let prev_path = chosen_path.borrow().clone();
                let places = controller.user_places.borrow().clone();
                let cloud_locs = controller.cloud_locations.borrow().clone();
                let recent = controller
                    .metadata
                    .borrow()
                    .list_recent_locations(8)
                    .unwrap_or_default();
                let ctrl = Rc::clone(&controller);
                let ctrl2 = Rc::clone(&controller);
                let nm = current_name.clone();
                show_picker_modal(
                    &controller.modal_host,
                    PickerConfig::open_folder(glib::home_dir()),
                    &places,
                    &cloud_locs,
                    &recent,
                    move |result| {
                        let PickerResult::Single(path) = result else {
                            return;
                        };
                        ctrl.show_pin_folder_dialog(
                            project_id,
                            slot,
                            Some(current_name.clone()),
                            Some(path),
                        );
                    },
                    move || {
                        ctrl2.show_pin_folder_dialog(
                            project_id,
                            slot,
                            Some(nm.clone()),
                            prev_path.clone(),
                        );
                    },
                );
            }
        });

        let path_row = GtkBox::new(Orientation::Horizontal, 6);
        path_row.set_hexpand(true);
        path_row.append(&path_display);
        path_row.append(&browse_btn);
        content.append(&path_row);

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let pin_btn = build_modal_button("Pin", ButtonKind::Primary, || {});
        let has_name = prefill_name
            .as_ref()
            .map(|n| !n.trim().is_empty())
            .unwrap_or(false);
        pin_btn.set_sensitive(has_name && prefill_path.is_some());

        // Update sensitivity as name changes
        {
            let pin_btn = pin_btn.clone();
            let chosen = Rc::clone(&chosen_path);
            name_entry.connect_changed(move |e| {
                pin_btn.set_sensitive(!e.text().trim().is_empty() && chosen.borrow().is_some());
            });
        }

        pin_btn.connect_clicked({
            let host = self.modal_host.clone();
            let controller = Rc::clone(self);
            let name_entry = name_entry.clone();
            let chosen = Rc::clone(&chosen_path);
            move |_| {
                let name = name_entry.text().trim().to_string();
                let path = chosen.borrow().clone();
                if let (false, Some(p)) = (name.is_empty(), path) {
                    let path_str = p.to_string_lossy().to_string();
                    let _ = controller
                        .metadata
                        .borrow_mut()
                        .add_project_destination(project_id, &name, &path_str);
                    controller.load_project_landing_view(slot, project_id);
                }
                host.hide();
            }
        });
        actions.append(&pin_btn);

        self.modal_host
            .show_with_custom_ui("Pin Folder", &content, &actions, false, None);
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
                let mut vs = crate::view_state::ViewState::load();
                vs.sort_field = match new_field {
                    SortField::Modified => "modified",
                    SortField::Size => "size",
                    SortField::Kind => "kind",
                    _ => "name",
                }
                .to_string();
                vs.sort_direction = match new_dir {
                    SortDirection::Descending => "descending",
                    _ => "ascending",
                }
                .to_string();
                vs.save();
                controller.save_folder_view_state_for(slot);
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
                let field = controller.sort_field_cell(slot).get();
                let mut vs = crate::view_state::ViewState::load();
                vs.sort_field = match field {
                    SortField::Modified => "modified",
                    SortField::Size => "size",
                    SortField::Kind => "kind",
                    _ => "name",
                }
                .to_string();
                vs.sort_direction = match dir {
                    SortDirection::Descending => "descending",
                    _ => "ascending",
                }
                .to_string();
                vs.save();
                controller.save_folder_view_state_for(slot);
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
                is_openable: true,
                detail: None,
                size_bytes: (info.size() >= 0).then_some(info.size() as u64),
                modified_unix: info.modification_date_time().map(|value| value.to_unix()),
                tags: Vec::new(),
                mark_tint_id: 0,
                mark_tint_color: None,
                mark_shape: Shape::DEFAULT,
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
        self.apply_tint_css();
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

        self.refresh_cloud_sidebar();
        self.refresh_drive_sidebar();

        self.refresh_search_tag_buttons(PaneSlot::Primary);
        self.refresh_search_tag_buttons(PaneSlot::Secondary);
        self.refresh_search_tag_buttons(PaneSlot::Tertiary);
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        self.primary_pane.tag_filter.set_tags(&tags);
        self.secondary_pane.tag_filter.set_tags(&tags);
        self.tertiary_pane.tag_filter.set_tags(&tags);
        self.primary_pane.tag_filter.set_tints(&tints);
        self.secondary_pane.tag_filter.set_tints(&tints);
        self.tertiary_pane.tag_filter.set_tints(&tints);
        self.update_sidebar_state();
    }

    fn refresh_cloud_sidebar(self: &Rc<Self>) {
        let locations = self
            .metadata
            .borrow()
            .list_cloud_locations()
            .unwrap_or_default();
        self.cloud_locations.replace(locations.clone());
        self.sidebar.set_cloud_locations(&locations);

        for (loc, button) in self.sidebar.cloud_buttons() {
            let controller = Rc::clone(self);
            let cloud_id = loc.id;
            button.connect_clicked(move |_| controller.open_cloud(cloud_id));

            let controller = Rc::clone(self);
            let loc_for_menu = loc.clone();
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            gesture.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(widget) = gesture.widget() else {
                    return;
                };
                controller.show_cloud_context_menu(loc_for_menu.clone(), widget, x, y);
            });
            button.add_controller(gesture);
        }
    }

    fn refresh_drive_sidebar(self: &Rc<Self>) {
        let drives = collect_removable_drives();
        self.removable_drives.replace(drives.clone());
        self.sidebar.set_removable_drives(&drives);
        for (entry, button) in self.sidebar.drive_buttons.borrow().clone() {
            let controller = Rc::clone(self);
            let path = entry.path.clone();
            button.connect_clicked(move |_| controller.navigate_to_active(path.clone()));
        }
    }

    fn open_cloud(self: &Rc<Self>, cloud_id: i64) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::CloudLanding(id) if id == cloud_id) {
            return;
        }
        self.save_dir_to_history_if_in_directory(slot);
        self.current_view_cell(slot)
            .replace(PaneView::CloudLanding(cloud_id));
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_cloud_landing_view(slot, cloud_id);
        self.update_navigation_state();
    }

    fn load_cloud_landing_view(self: &Rc<Self>, slot: PaneSlot, cloud_id: i64) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let Some(record) = self
            .cloud_locations
            .borrow()
            .iter()
            .find(|r| r.id == cloud_id)
            .cloned()
        else {
            self.status.set_message("Cloud location not found.");
            return;
        };

        let pane = self.pane_widgets(slot);
        let display_label = record.name.clone();
        pane.path_label.set_label(&display_label);
        pane.file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();

        let record_path = record.path.clone();

        let controller = Rc::clone(self);
        let path_for_drive = record_path.clone();
        let on_open_drive = move || {
            let path = &path_for_drive;
            if is_gio_uri(path) {
                // Try to resolve to a local GVfs FUSE path first; fall back to URI navigation
                let file = gio::File::for_uri(path);
                let nav_path = file.path().unwrap_or_else(|| PathBuf::from(path));
                controller.navigate_to(slot, nav_path, true);
            } else {
                controller.navigate_to(slot, PathBuf::from(path), true);
            }
        };

        let controller = Rc::clone(self);
        let path_for_sv = record_path.clone();
        let on_space_viewer = move || {
            controller
                .current_view_cell(slot)
                .replace(PaneView::SpaceViewer {
                    root: PathBuf::from(&path_for_sv),
                });
            controller
                .current_dir_cell(slot)
                .replace(PathBuf::from(&path_for_sv));
            controller.sync_active_tab_state();
            controller.update_view_strip(slot);
            if slot == PaneSlot::Primary {
                controller.rebuild_tab_strip();
            }
            controller.load_space_viewer_view(slot);
        };

        let controller = Rc::clone(self);
        let path_for_triage = record_path.clone();
        let on_triage = move || {
            controller.open_triage(PathBuf::from(&path_for_triage), TriageFilter::All);
        };

        let controller = Rc::clone(self);
        let on_edit = move || {
            controller.show_edit_cloud_dialog(cloud_id, None);
        };

        let controller = Rc::clone(self);
        let on_remove = move || {
            controller.show_remove_cloud_confirm(cloud_id);
        };

        let is_rclone_mountable = record.kind == "rclone" && record.remote_name.is_some();

        let on_mount: Option<Box<dyn Fn()>> = if is_rclone_mountable {
            let controller = Rc::clone(self);
            Some(Box::new(move || controller.mount_cloud_profile(cloud_id)))
        } else {
            None
        };

        let on_unmount: Option<Box<dyn Fn()>> = if is_rclone_mountable {
            let controller = Rc::clone(self);
            Some(Box::new(move || controller.unmount_cloud_profile(cloud_id)))
        } else {
            None
        };

        pane.cloud_landing_panel.populate(
            &record,
            on_open_drive,
            on_space_viewer,
            on_triage,
            on_edit,
            on_remove,
            on_mount,
            on_unmount,
        );

        let controller = Rc::clone(self);
        let path_for_check = record_path.clone();
        glib::idle_add_local_once(move || {
            let available =
                if path_for_check.contains("://") && !path_for_check.starts_with("file://") {
                    let file = gio::File::for_uri(&path_for_check);
                    file.query_exists(gio::Cancellable::NONE)
                } else {
                    std::path::Path::new(&path_for_check).exists()
                };
            controller
                .pane_widgets(slot)
                .cloud_landing_panel
                .set_availability(Some(available));
        });

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

    fn show_add_cloud_dialog(self: &Rc<Self>, prefill_path: Option<String>) {
        let path_str = prefill_path.unwrap_or_default();
        let controller = Rc::clone(self);
        let on_browse: Rc<dyn Fn()> = {
            let ctrl = Rc::clone(self);
            Rc::new(move || {
                let places = ctrl.user_places.borrow().clone();
                let cloud_locs = ctrl.cloud_locations.borrow().clone();
                let recent = ctrl
                    .metadata
                    .borrow()
                    .list_recent_locations(8)
                    .unwrap_or_default();
                let ctrl2 = Rc::clone(&ctrl);
                show_picker_modal(
                    &ctrl.modal_host,
                    PickerConfig::open_folder(glib::home_dir()),
                    &places,
                    &cloud_locs,
                    &recent,
                    move |result| {
                        let PickerResult::Single(path) = result else {
                            return;
                        };
                        ctrl2.show_add_cloud_dialog(Some(path.to_string_lossy().to_string()));
                    },
                    || {},
                );
            })
        };
        let prefill = if path_str.is_empty() {
            None
        } else {
            Some(("", path_str.as_str(), "manual", None::<&str>))
        };
        self.show_cloud_form_dialog(
            None,
            prefill,
            move |name, path, kind, notes, remote_name| {
                let result = controller.metadata.borrow_mut().create_cloud_location(
                    &name,
                    &path,
                    &kind,
                    remote_name.as_deref(),
                    notes.as_deref(),
                );
                match result {
                    Ok(record) => {
                        controller.refresh_metadata_sidebar();
                        controller.open_cloud(record.id);
                    }
                    Err(err) => {
                        controller
                            .status
                            .set_message(&format!("Failed to add cloud location: {err}"));
                    }
                }
            },
            Some(on_browse),
        );
    }

    fn show_edit_cloud_dialog(self: &Rc<Self>, cloud_id: i64, prefill_path: Option<String>) {
        let Some(record) = self
            .cloud_locations
            .borrow()
            .iter()
            .find(|r| r.id == cloud_id)
            .cloned()
        else {
            return;
        };
        let on_browse: Rc<dyn Fn()> = {
            let ctrl = Rc::clone(self);
            Rc::new(move || {
                let places = ctrl.user_places.borrow().clone();
                let cloud_locs = ctrl.cloud_locations.borrow().clone();
                let recent = ctrl
                    .metadata
                    .borrow()
                    .list_recent_locations(8)
                    .unwrap_or_default();
                let ctrl2 = Rc::clone(&ctrl);
                show_picker_modal(
                    &ctrl.modal_host,
                    PickerConfig::open_folder(glib::home_dir()),
                    &places,
                    &cloud_locs,
                    &recent,
                    move |result| {
                        let PickerResult::Single(path) = result else {
                            return;
                        };
                        ctrl2.show_edit_cloud_dialog(
                            cloud_id,
                            Some(path.to_string_lossy().to_string()),
                        );
                    },
                    || {},
                );
            })
        };
        // prefill overrides just the path field when Browse was used
        let prefill_path_str = prefill_path.unwrap_or_else(|| record.path.clone());
        let controller = Rc::clone(self);
        self.show_cloud_form_dialog(
            Some(&record),
            Some(("", prefill_path_str.as_str(), "", None::<&str>)),
            move |name, path, kind, notes, remote_name| {
                let result = controller.metadata.borrow_mut().update_cloud_location(
                    cloud_id,
                    &name,
                    &path,
                    &kind,
                    remote_name.as_deref(),
                    notes.as_deref(),
                );
                match result {
                    Ok(()) => {
                        controller.refresh_metadata_sidebar();
                        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
                            if matches!(controller.current_view_for(slot), PaneView::CloudLanding(id) if id == cloud_id)
                            {
                                controller.load_cloud_landing_view(slot, cloud_id);
                            }
                        }
                    }
                    Err(err) => {
                        controller
                            .status
                            .set_message(&format!("Failed to update cloud location: {err}"));
                    }
                }
            },
            Some(on_browse),
        );
    }

    fn show_cloud_form_dialog<F>(
        self: &Rc<Self>,
        existing: Option<&CloudRecord>,
        prefill: Option<(&str, &str, &str, Option<&str>)>,
        on_save: F,
        on_browse_path: Option<Rc<dyn Fn()>>,
    ) where
        F: Fn(String, String, String, Option<String>, Option<String>) + 'static,
    {
        let title = if existing.is_some() {
            "Edit Cloud Drive"
        } else {
            "Add Cloud Drive"
        };

        let kind_options = [
            "rclone", "pcloud", "gvfs", "sftp", "ftp", "webdav", "manual",
        ];

        let name_entry = gtk::Entry::new();
        name_entry.set_placeholder_text(Some("Display name"));
        name_entry.set_width_chars(28);
        name_entry.set_max_width_chars(28);
        if let Some(r) = existing {
            name_entry.set_text(&r.name);
        }

        let path_entry = gtk::Entry::new();
        path_entry.set_placeholder_text(Some("/mnt/gdrive  or  sftp://host/path"));
        path_entry.set_width_chars(28);
        path_entry.set_max_width_chars(40);
        if let Some(r) = existing {
            path_entry.set_text(&r.path);
        }

        let kind_dropdown = gtk::DropDown::from_strings(&kind_options);
        if let Some(r) = existing {
            if let Some(pos) = kind_options.iter().position(|k| *k == r.kind) {
                kind_dropdown.set_selected(pos as u32);
            }
        }

        let remote_entry = gtk::Entry::new();
        remote_entry.set_placeholder_text(Some("rclone remote name, e.g. gdrive (optional)"));
        remote_entry.set_width_chars(28);
        remote_entry.set_max_width_chars(40);
        if let Some(r) = existing {
            if let Some(rn) = &r.remote_name {
                remote_entry.set_text(rn);
            }
        }

        let notes_entry = gtk::Entry::new();
        notes_entry.set_placeholder_text(Some("Optional notes"));
        notes_entry.set_width_chars(28);
        notes_entry.set_max_width_chars(40);
        if let Some(r) = existing {
            if let Some(notes) = &r.notes {
                notes_entry.set_text(notes);
            }
        }

        // Pre-fill: full prefill for new entries; path-only override for edits (Browse button)
        if let Some((pf_name, pf_path, pf_kind, pf_remote)) = prefill {
            if existing.is_none() {
                if !pf_name.is_empty() {
                    name_entry.set_text(pf_name);
                }
                if !pf_path.is_empty() {
                    path_entry.set_text(pf_path);
                }
                if !pf_kind.is_empty() {
                    if let Some(pos) = kind_options.iter().position(|k| *k == pf_kind) {
                        kind_dropdown.set_selected(pos as u32);
                    }
                }
                if let Some(rn) = pf_remote {
                    remote_entry.set_text(rn);
                }
            } else if !pf_path.is_empty() {
                // Partial override for edits: only path (used when Browse button picks a new folder)
                path_entry.set_text(pf_path);
            }
        }

        let form = GtkBox::new(Orientation::Vertical, 8);
        form.set_margin_top(4);
        form.set_margin_bottom(4);

        let make_row = |lbl: &str, widget: &gtk::Widget| -> GtkBox {
            let row = GtkBox::new(Orientation::Horizontal, 8);
            let label = Label::new(Some(lbl));
            label.set_halign(gtk::Align::End);
            label.set_width_chars(8);
            row.append(&label);
            row.append(widget);
            row
        };

        // Path row: entry + optional Browse button
        let path_row = GtkBox::new(Orientation::Horizontal, 4);
        path_row.set_hexpand(true);
        path_entry.set_hexpand(true);
        path_row.append(&path_entry);
        if let Some(ref browse_fn) = on_browse_path {
            let browse_fn = Rc::clone(browse_fn);
            let browse_btn = Button::with_label("…");
            browse_btn.add_css_class("picker-browse-btn");
            browse_btn.connect_clicked(move |_| browse_fn());
            path_row.append(&browse_btn);
        }

        form.append(&make_row("Name", name_entry.upcast_ref()));
        form.append(&make_row("Path", path_row.upcast_ref()));
        form.append(&make_row("Kind", kind_dropdown.upcast_ref()));
        form.append(&make_row("Remote", remote_entry.upcast_ref()));
        form.append(&make_row("Notes", notes_entry.upcast_ref()));

        let name_entry_c = name_entry.clone();
        let path_entry_c = path_entry.clone();
        let notes_entry_c = notes_entry.clone();
        let remote_entry_c = remote_entry.clone();
        let kind_dropdown_c = kind_dropdown.clone();

        let actions = build_modal_actions();

        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let on_save_rc = Rc::new(on_save);
        let save_btn = build_modal_button(title, ButtonKind::Primary, move || {
            let name = name_entry_c.text().to_string();
            let path = path_entry_c.text().to_string();
            let kind_idx = kind_dropdown_c.selected() as usize;
            let kind = kind_options
                .get(kind_idx)
                .copied()
                .unwrap_or("manual")
                .to_string();
            let remote_text = remote_entry_c.text().to_string();
            let remote_name = if remote_text.trim().is_empty() {
                None
            } else {
                Some(remote_text)
            };
            let notes_text = notes_entry_c.text().to_string();
            let notes = if notes_text.trim().is_empty() {
                None
            } else {
                Some(notes_text)
            };
            on_save_rc(name, path, kind, notes, remote_name);
            host.hide();
        });
        let initial_ok =
            !name_entry.text().trim().is_empty() && !path_entry.text().trim().is_empty();
        save_btn.set_sensitive(initial_ok);
        actions.append(&save_btn);

        // Keep save button sensitive only while both Name and Path are non-empty
        {
            let save_btn = save_btn.clone();
            let name_entry_v = name_entry.clone();
            let path_entry_v = path_entry.clone();
            let update = move || {
                save_btn.set_sensitive(
                    !name_entry_v.text().trim().is_empty()
                        && !path_entry_v.text().trim().is_empty(),
                );
            };
            let update2 = update.clone();
            name_entry.connect_changed(move |_| update());
            path_entry.connect_changed(move |_| update2());
        }

        // Auto-detect kind from URI scheme when editing is not for an existing entry
        if existing.is_none() {
            let kind_dropdown_uri = kind_dropdown.clone();
            path_entry.connect_changed(move |entry| {
                let text = entry.text();
                let t = text.as_str();
                let detected = if t.starts_with("sftp://") || t.starts_with("ssh://") {
                    kind_options.iter().position(|k| *k == "sftp")
                } else if t.starts_with("ftp://") {
                    kind_options.iter().position(|k| *k == "ftp")
                } else if t.starts_with("smb://") {
                    kind_options.iter().position(|k| *k == "gvfs")
                } else if t.starts_with("dav://") || t.starts_with("davs://") {
                    kind_options.iter().position(|k| *k == "webdav")
                } else {
                    None
                };
                if let Some(idx) = detected {
                    kind_dropdown_uri.set_selected(idx as u32);
                }
            });
        }

        let host = self.modal_host.clone();
        self.modal_host.show_with_custom_ui(
            title,
            &form,
            &actions,
            true,
            Some(Box::new(move || host.hide())),
        );
        name_entry.grab_focus();
    }

    fn show_rclone_setup_dialog(self: &Rc<Self>) {
        let rclone = crate::rclone::detect();
        let home = glib::home_dir();
        let home_str = home.to_string_lossy().to_string();

        let outer = GtkBox::new(Orientation::Vertical, 0);
        outer.set_margin_top(4);
        outer.set_margin_bottom(4);
        outer.set_hexpand(true);

        // ── Status section ──────────────────────────────────────────────────
        let status_row = GtkBox::new(Orientation::Horizontal, 8);
        status_row.set_margin_bottom(8);
        match &rclone.version {
            Some(ver) => {
                let lbl = Label::new(Some(&format!("✓ {ver} detected")));
                lbl.add_css_class("rclone-status-ok");
                lbl.set_halign(gtk::Align::Start);
                status_row.append(&lbl);
            }
            None => {
                let lbl = Label::new(Some("✗ rclone not found"));
                lbl.add_css_class("rclone-status-missing");
                lbl.set_halign(gtk::Align::Start);
                status_row.append(&lbl);
                let note = Label::new(Some("  Install from rclone.org/install"));
                note.add_css_class("rclone-status-install-note");
                note.set_halign(gtk::Align::Start);
                status_row.append(&note);
            }
        }
        outer.append(&status_row);

        // ── Remotes section (only when rclone is available) ─────────────────
        if rclone.version.is_some() {
            let remotes_heading = Label::new(Some("CONFIGURED REMOTES"));
            remotes_heading.add_css_class("landing-section-heading");
            remotes_heading.set_halign(gtk::Align::Start);
            remotes_heading.set_margin_top(4);
            remotes_heading.set_margin_bottom(4);
            outer.append(&remotes_heading);

            if rclone.remotes.is_empty() {
                let empty = Label::new(Some("No remotes configured yet. Run:  rclone config"));
                empty.add_css_class("rclone-status-install-note");
                empty.set_halign(gtk::Align::Start);
                empty.set_margin_bottom(8);
                outer.append(&empty);
            } else {
                for remote in &rclone.remotes {
                    let mount_path = format!("{home_str}/Cloud/Lattice/{remote}");
                    let mount_cmd =
                        format!("rclone mount {remote}: {mount_path} --vfs-cache-mode writes");

                    let row = GtkBox::new(Orientation::Horizontal, 6);
                    row.add_css_class("rclone-remote-row");
                    row.set_margin_bottom(2);

                    let name_lbl = Label::new(Some(remote));
                    name_lbl.add_css_class("rclone-remote-name");
                    name_lbl.set_halign(gtk::Align::Start);
                    name_lbl.set_hexpand(true);
                    row.append(&name_lbl);

                    // Copy mount command button
                    let copy_btn = Button::with_label("Copy mount cmd");
                    copy_btn.add_css_class("landing-add-btn");
                    {
                        let cmd = mount_cmd.clone();
                        let window = self.window.clone();
                        let status = self.status.clone();
                        copy_btn.connect_clicked(move |_| {
                            window.clipboard().set_text(&cmd);
                            status.set_message("Mount command copied to clipboard.");
                        });
                    }
                    crate::ui::attach_tooltip(&copy_btn, &format!("Copy: {mount_cmd}"));
                    row.append(&copy_btn);

                    // Add to Cloud button
                    let add_btn = Button::with_label("Add to Cloud");
                    add_btn.add_css_class("landing-add-btn");
                    {
                        let controller = Rc::clone(self);
                        let r = remote.clone();
                        let mp = mount_path.clone();
                        add_btn.connect_clicked(move |_| {
                            controller.modal_host.hide();
                            let r2 = r.clone();
                            let mp2 = mp.clone();
                            controller.show_cloud_form_dialog(
                                None,
                                Some((&r2, &mp2, "rclone", Some(&r2))),
                                {
                                    let ctrl = Rc::clone(&controller);
                                    move |name, path, kind, notes, remote_name| {
                                        let result =
                                            ctrl.metadata.borrow_mut().create_cloud_location(
                                                &name,
                                                &path,
                                                &kind,
                                                remote_name.as_deref(),
                                                notes.as_deref(),
                                            );
                                        match result {
                                            Ok(record) => {
                                                ctrl.refresh_metadata_sidebar();
                                                ctrl.open_cloud(record.id);
                                            }
                                            Err(err) => {
                                                ctrl.status.set_message(&format!(
                                                    "Failed to add cloud location: {err}"
                                                ));
                                            }
                                        }
                                    }
                                },
                                None,
                            );
                        });
                    }
                    crate::ui::attach_tooltip(
                        &add_btn,
                        &format!("Pre-fill Add Cloud Drive with path: {mount_path}"),
                    );
                    row.append(&add_btn);

                    outer.append(&row);
                }
            }
        }

        // ── Mount guide section ─────────────────────────────────────────────
        let guide_heading = Label::new(Some("HOW TO MOUNT"));
        guide_heading.add_css_class("landing-section-heading");
        guide_heading.set_halign(gtk::Align::Start);
        guide_heading.set_margin_top(12);
        guide_heading.set_margin_bottom(6);
        outer.append(&guide_heading);

        let guide_text = Label::new(Some(
            "Mount a remote externally, then register the folder as a Cloud entry in Lattice.\n\
             Suggested mount location:\n\
             \n\
             \u{00a0}\u{00a0}~/Cloud/Lattice/<remote-name>\n\
             \n\
             Example command (run in a terminal, then keep it running):"
                .trim_start(),
        ));
        guide_text.add_css_class("rclone-status-install-note");
        guide_text.set_halign(gtk::Align::Start);
        guide_text.set_wrap(true);
        guide_text.set_max_width_chars(52);
        outer.append(&guide_text);

        let code_block = Label::new(Some(
            "rclone mount <remote>: ~/Cloud/Lattice/<remote> \\\n  --vfs-cache-mode writes",
        ));
        code_block.add_css_class("rclone-guide-code");
        code_block.set_halign(gtk::Align::Start);
        code_block.set_selectable(true);
        outer.append(&code_block);

        let footer = Label::new(Some(
            "Then use \"Add Cloud Drive\" to register the mounted folder.\n\
             Credentials are managed by rclone config — not by Lattice.",
        ));
        footer.add_css_class("rclone-status-install-note");
        footer.set_halign(gtk::Align::Start);
        footer.set_wrap(true);
        footer.set_max_width_chars(52);
        footer.set_margin_top(6);
        outer.append(&footer);

        // ── Wrap in a scroll so long remote lists don't overflow ────────────
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(420)
            .propagate_natural_height(true)
            .hexpand(true)
            .build();
        scroll.set_child(Some(&outer));

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let close_btn = build_modal_button("Close", ButtonKind::Secondary, move || host.hide());
        actions.append(&close_btn);

        let host = self.modal_host.clone();
        self.modal_host.show_with_custom_ui(
            "rclone Remotes",
            &scroll,
            &actions,
            true,
            Some(Box::new(move || host.hide())),
        );
    }

    fn show_remove_cloud_confirm(self: &Rc<Self>, cloud_id: i64) {
        let Some(record) = self
            .cloud_locations
            .borrow()
            .iter()
            .find(|r| r.id == cloud_id)
            .cloned()
        else {
            return;
        };

        let prompt = format!(
            "Remove \"{}\" from Cloud?\n\nThis removes the Lattice entry only — no files are deleted.",
            record.name
        );
        let controller = Rc::clone(self);
        self.modal_host.show_confirm(
            "Remove Cloud Drive",
            &prompt,
            "Remove",
            true,
            false,
            move || {
                let _ = controller
                    .metadata
                    .borrow_mut()
                    .delete_cloud_location(cloud_id);
                controller.refresh_metadata_sidebar();
                for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
                    if matches!(controller.current_view_for(slot), PaneView::CloudLanding(id) if id == cloud_id)
                    {
                        controller.navigate_to(slot, controller.places.home.clone(), true);
                    }
                }
            },
        );
    }

    fn refresh_cloud_landing_availability(self: &Rc<Self>, cloud_id: i64, available: bool) {
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::CloudLanding(id) if id == cloud_id) {
                self.pane_widgets(slot)
                    .cloud_landing_panel
                    .set_availability(Some(available));
            }
        }
    }

    fn set_cloud_landing_mount_busy(self: &Rc<Self>, cloud_id: i64, busy: bool) {
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::CloudLanding(id) if id == cloud_id) {
                self.pane_widgets(slot)
                    .cloud_landing_panel
                    .set_mount_busy(busy);
            }
        }
    }

    fn mount_cloud_profile(self: &Rc<Self>, cloud_id: i64) {
        let Some(record) = self
            .cloud_locations
            .borrow()
            .iter()
            .find(|r| r.id == cloud_id)
            .cloned()
        else {
            return;
        };

        let Some(remote_name) = record.remote_name.clone() else {
            self.status.set_message(
                "No rclone remote name set — edit this entry and fill in the Remote field.",
            );
            return;
        };

        let mount_path = std::path::PathBuf::from(&record.path);
        let op_id = self
            .ops_panel
            .add_op(&format!("Mounting {remote_name}…"), None);
        self.set_cloud_landing_mount_busy(cloud_id, true);

        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let result =
                gio::spawn_blocking(move || crate::rclone::mount(&remote_name, &mount_path))
                    .await
                    .unwrap_or_else(|_| Err("Mount task panicked".to_string()));

            match result {
                Ok(()) => {
                    controller.ops_panel.finish_op(op_id, &[]);
                    controller.refresh_cloud_landing_availability(cloud_id, true);
                    controller.status.set_message("Mounted successfully.");
                }
                Err(err) => {
                    controller.ops_panel.finish_op(op_id, &[err.clone()]);
                    controller.refresh_cloud_landing_availability(cloud_id, false);
                    controller
                        .status
                        .set_message(&format!("Mount failed: {err}"));
                }
            }
        });
    }

    fn unmount_cloud_profile(self: &Rc<Self>, cloud_id: i64) {
        let Some(record) = self
            .cloud_locations
            .borrow()
            .iter()
            .find(|r| r.id == cloud_id)
            .cloned()
        else {
            return;
        };

        let mount_path = std::path::PathBuf::from(&record.path);
        let record_path = record.path.clone();
        let op_id = self
            .ops_panel
            .add_op(&format!("Unmounting {}…", record.name), None);
        self.set_cloud_landing_mount_busy(cloud_id, true);

        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || crate::rclone::unmount(&mount_path))
                .await
                .unwrap_or_else(|_| Err("Unmount task panicked".to_string()));

            match result {
                Ok(()) => {
                    controller.ops_panel.finish_op(op_id, &[]);
                    controller.refresh_cloud_landing_availability(cloud_id, false);
                    controller.status.set_message("Unmounted.");
                }
                Err(err) => {
                    controller.ops_panel.finish_op(op_id, &[err.clone()]);
                    let still_mounted =
                        crate::rclone::is_mounted(std::path::Path::new(&record_path));
                    controller.refresh_cloud_landing_availability(cloud_id, still_mounted);
                    controller
                        .status
                        .set_message(&format!("Unmount failed: {err}"));
                }
            }
        });
    }

    fn show_cloud_context_menu(
        self: &Rc<Self>,
        record: CloudRecord,
        widget: impl IsA<gtk::Widget>,
        x: f64,
        y: f64,
    ) {
        let menu = GtkBox::new(Orientation::Vertical, 0);
        menu.add_css_class("context-menu");

        let make_item = |label: &str| -> Button {
            let btn = Button::with_label(label);
            btn.add_css_class("context-menu-item");
            btn.set_halign(gtk::Align::Fill);
            btn
        };

        let open_btn = make_item("Open");
        let sv_btn = make_item("Space Viewer");
        let triage_btn = make_item("Triage");
        let edit_btn = make_item("Edit");
        let remove_btn = make_item("Remove");

        menu.append(&open_btn);
        menu.append(&sv_btn);
        menu.append(&triage_btn);
        menu.append(&edit_btn);
        menu.append(&remove_btn);

        let popover = Popover::new();
        popover.add_css_class("context-popover");
        popover.set_child(Some(&menu));
        popover.set_has_arrow(false);
        popover.set_parent(widget.upcast_ref::<gtk::Widget>());
        let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));

        *self.context_popover.borrow_mut() = Some(popover.clone());
        popover.popup();

        let cloud_id = record.id;
        let path = record.path.clone();

        let controller = Rc::clone(self);
        let popover_c = popover.clone();
        open_btn.connect_clicked(move |_| {
            popover_c.popdown();
            controller.navigate_to_active(PathBuf::from(&path));
        });

        let controller = Rc::clone(self);
        let path2 = record.path.clone();
        let popover_c = popover.clone();
        sv_btn.connect_clicked(move |_| {
            popover_c.popdown();
            let slot = controller.active_slot();
            controller
                .current_view_cell(slot)
                .replace(PaneView::SpaceViewer {
                    root: PathBuf::from(&path2),
                });
            controller
                .current_dir_cell(slot)
                .replace(PathBuf::from(&path2));
            controller.sync_active_tab_state();
            controller.update_view_strip(slot);
            if slot == PaneSlot::Primary {
                controller.rebuild_tab_strip();
            }
            controller.load_space_viewer_view(slot);
        });

        let controller = Rc::clone(self);
        let path3 = record.path.clone();
        let popover_c = popover.clone();
        triage_btn.connect_clicked(move |_| {
            popover_c.popdown();
            controller.open_triage(PathBuf::from(&path3), TriageFilter::All);
        });

        let controller = Rc::clone(self);
        let popover_c = popover.clone();
        edit_btn.connect_clicked(move |_| {
            popover_c.popdown();
            controller.show_edit_cloud_dialog(cloud_id, None);
        });

        let controller = Rc::clone(self);
        let popover_c = popover.clone();
        remove_btn.connect_clicked(move |_| {
            popover_c.popdown();
            controller.show_remove_cloud_confirm(cloud_id);
        });
    }

    fn is_in_cloud_location(&self, path: &Path) -> bool {
        self.cloud_locations.borrow().iter().any(|loc| {
            let loc_path = std::path::Path::new(&loc.path);
            path.starts_with(loc_path)
        })
    }

    fn cloud_name_for_path(&self, path: &Path) -> Option<(String, String)> {
        let path_str = path.to_string_lossy();
        self.cloud_locations.borrow().iter().find_map(|loc| {
            let matches = if is_gio_uri(&loc.path) {
                // URI prefix match: "sftp://host/path/sub" starts with "sftp://host/path"
                path_str.starts_with(&loc.path)
            } else {
                path.starts_with(std::path::Path::new(&loc.path))
            };
            matches.then(|| (loc.name.clone(), loc.kind.clone()))
        })
    }

    fn cloud_summary(&self, summary: &str, path: &Path) -> String {
        if self.cloud_name_for_path(path).is_some() {
            format!("☁ {summary}")
        } else {
            summary.to_string()
        }
    }

    fn open_project(self: &Rc<Self>, project_id: i64) {
        let slot = self.active_slot();
        if matches!(self.current_view_for(slot), PaneView::ProjectLanding(id) if id == project_id) {
            return;
        }
        self.save_dir_to_history_if_in_directory(slot);
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
        self.save_dir_to_history_if_in_directory(slot);
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
        self.save_dir_to_history_if_in_directory(slot);
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
        if let Some((name, _)) = self.cloud_name_for_path(&root) {
            self.status.set_message(&format!(
                "☁ Triage on '{name}' — loading metadata may be slow on cloud drives"
            ));
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
        let root = self.tool_scope_dir_for(slot);
        self.open_triage(root, TriageFilter::All);
    }

    fn open_bulk_naming_tool(self: &Rc<Self>) {
        let slot = self.active_slot();
        let root = self.tool_scope_dir_for(slot);
        if matches!(self.current_view_for(slot), PaneView::BulkNaming { root: ref current } if current == &root)
        {
            return;
        }
        self.save_dir_to_history_if_in_directory(slot);
        self.current_dir_cell(slot).replace(root.clone());
        self.current_view_cell(slot)
            .replace(PaneView::BulkNaming { root: root.clone() });
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_bulk_naming_view(slot, root);
        self.update_navigation_state();
    }

    fn open_bulk_naming_with_items(self: &Rc<Self>, items: Vec<FileItem>) {
        if items.is_empty() {
            return;
        }
        let slot = self.active_slot();
        let root = common_parent_for_items(&items).unwrap_or_else(|| self.tool_scope_dir_for(slot));
        let selected_paths = items
            .iter()
            .map(|item| item.path.clone())
            .collect::<HashSet<_>>();
        let sibling_names = self.sibling_names_outside_selection(slot, &selected_paths);

        self.save_dir_to_history_if_in_directory(slot);
        self.current_dir_cell(slot).replace(root.clone());
        self.current_view_cell(slot)
            .replace(PaneView::BulkNaming { root: root.clone() });
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_bulk_naming_items(slot, items, sibling_names, false);
        self.update_navigation_state();
    }

    fn open_media_convert_with_items(self: &Rc<Self>, slot: PaneSlot, items: Vec<FileItem>) {
        let convert_items: Vec<ConvertItem> = items
            .into_iter()
            .filter(|i| matches!(i.kind, FileKind::Image | FileKind::Video | FileKind::Audio))
            .map(|i| ConvertItem {
                path: i.path.clone(),
                kind: match i.kind {
                    FileKind::Image => MediaKind::Image,
                    FileKind::Audio => MediaKind::Audio,
                    _ => MediaKind::Video,
                },
            })
            .collect();

        let from_dir = convert_items
            .first()
            .and_then(|i| i.path.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.tool_scope_dir_for(slot));

        self.save_dir_to_history_if_in_directory(slot);
        self.current_dir_cell(slot).replace(from_dir.clone());
        self.current_view_cell(slot)
            .replace(PaneView::MediaConvert { from_dir });
        self.sync_active_tab_state();
        self.update_view_strip(slot);
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.pane_widgets(slot).media_convert_panel.set_items(
            convert_items,
            &self.conversion_queue.tools,
            None,
        );
        if slot == self.active_slot() {
            let display = self.display_label_for(slot);
            self.toolbar.set_breadcrumb_path(&display);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
        }
    }

    /// Scan common user directories for orphaned `.lattice_converting_*` temp files
    /// left by a previous crash and delete them. Runs off the main thread.
    fn cleanup_convert_temps(self: &Rc<Self>) {
        let dirs: Vec<PathBuf> = std::iter::once(Some(glib::home_dir()))
            .chain([
                glib::user_special_dir(UserDirectory::Desktop),
                glib::user_special_dir(UserDirectory::Downloads),
                glib::user_special_dir(UserDirectory::Pictures),
                glib::user_special_dir(UserDirectory::Videos),
                glib::user_special_dir(UserDirectory::Music),
                glib::user_special_dir(UserDirectory::Documents),
            ])
            .flatten()
            .collect();

        gio::spawn_blocking(move || {
            for dir in dirs {
                cleanup_orphaned_temps_in(&dir);
            }
        });
    }

    fn connect_media_convert_actions(self: &Rc<Self>) {
        // Load and apply saved conversion settings to all panels
        let saved = ConvertSettings::load();
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            self.pane_widgets(slot)
                .media_convert_panel
                .apply_settings(&saved);
        }

        // OpsPanel: overall fraction + done receipt
        let ops_for_progress = self.ops_panel.clone();
        self.conversion_queue
            .connect_progress(move |op_id, fraction, detail| {
                ops_for_progress.update_progress(op_id, fraction, detail);
            });

        let ops_for_done = self.ops_panel.clone();
        self.conversion_queue.connect_done(move |op_id, errors| {
            ops_for_done.finish_op(op_id, &errors);
        });

        // ConvertProgressPanel: per-job status
        let cp = self.convert_progress.clone();
        self.conversion_queue
            .connect_job_status(move |job_id, status| {
                cp.update_job_status(job_id, status);
            });

        let cp = self.convert_progress.clone();
        self.conversion_queue
            .connect_batch_progress(move |progress| {
                cp.update_batch_progress(progress);
            });

        let cp = self.convert_progress.clone();
        self.conversion_queue
            .connect_job_progress(move |job_id, fraction| {
                cp.update_job_progress(job_id, fraction);
            });

        // Wire copy-error to clipboard
        let window = self.window.clone();
        self.convert_progress.set_copy_error_fn(move |text| {
            window.clipboard().set_text(&text);
        });

        // Cancel
        let queue = self.conversion_queue.clone();
        self.convert_progress.connect_cancel(move || {
            queue.cancel();
        });

        // Retry failed
        let queue = self.conversion_queue.clone();
        let cp = self.convert_progress.clone();
        self.convert_progress
            .connect_retry_failed(move |failed_jobs| {
                for job in failed_jobs {
                    queue.retry_job(job);
                }
                let _ = cp.wire_copy_buttons();
            });

        // Open output folder in active pane
        let controller = Rc::clone(self);
        self.convert_progress.connect_open_output(move |path| {
            let slot = controller.active_slot();
            controller.navigate_to(slot, path, true);
        });

        // Per-pane: wire start callback + folder picker + settings persistence
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            let controller = Rc::clone(self);
            let panel = self.pane_widgets(slot).media_convert_panel.clone();

            // start callback
            panel.connect_start({
                let controller = Rc::clone(&controller);
                move |batch| {
                    if batch.active_count() == 0 {
                        return;
                    }
                    let output_dir = batch.representative_output_dir();
                    let active_jobs = batch.into_active_jobs();
                    let n = active_jobs.len();
                    controller
                        .convert_progress
                        .start_batch(&active_jobs, output_dir);
                    let op_id = controller.ops_panel.add_op(
                        &format!("Converting {} file{}", n, if n == 1 { "" } else { "s" }),
                        None,
                    );
                    controller.conversion_queue.enqueue_jobs(active_jobs, op_id);
                }
            });

            // folder picker callback
            panel.connect_folder_pick({
                let controller = Rc::clone(&controller);
                let panel = self.pane_widgets(slot).media_convert_panel.clone();
                move || {
                    let places = controller.user_places.borrow().clone();
                    let cloud_locs = controller.cloud_locations.borrow().clone();
                    let recent = controller
                        .metadata
                        .borrow()
                        .list_recent_locations(8)
                        .unwrap_or_default();
                    let panel_confirm = panel.clone();
                    let panel_cancel = panel.clone();
                    show_picker_modal(
                        &controller.modal_host,
                        PickerConfig::open_folder(glib::home_dir()),
                        &places,
                        &cloud_locs,
                        &recent,
                        move |result| {
                            let PickerResult::Single(path) = result else {
                                return;
                            };
                            panel_confirm.set_chosen_folder(path);
                        },
                        move || panel_cancel.folder_pick_cancelled(),
                    );
                }
            });

            // settings persistence
            panel.connect_settings_changed({
                move |preset_id, output_mode, conflict_policy| {
                    let mut s = ConvertSettings::load();
                    // Update the appropriate per-kind preset slot
                    use crate::converter::{all_presets, OutputConflictPolicy, OutputLocationMode};
                    if let Some(preset) = all_presets().iter().find(|p| p.id == preset_id) {
                        match preset.kind {
                            crate::converter::MediaKind::Image => {
                                s.last_preset_image = preset_id.to_string();
                            }
                            crate::converter::MediaKind::Audio => {
                                s.last_preset_audio = preset_id.to_string();
                            }
                            crate::converter::MediaKind::Video => {
                                s.last_preset_video = preset_id.to_string();
                            }
                            crate::converter::MediaKind::Unknown => {}
                        }
                    }
                    s.output_mode = match output_mode {
                        OutputLocationMode::NextToSource => "next_to_source".to_string(),
                        OutputLocationMode::Subfolder(_) => "converted_subfolder".to_string(),
                        OutputLocationMode::ChosenFolder(_) => "chosen_folder".to_string(),
                    };
                    s.conflict_policy = match conflict_policy {
                        OutputConflictPolicy::AutoRename => "auto_rename".to_string(),
                        OutputConflictPolicy::Skip => "skip".to_string(),
                        OutputConflictPolicy::Overwrite => "overwrite".to_string(),
                    };
                    s.save();
                }
            });

            // source mode toggle — reload items without nav side effects
            panel.connect_source_mode_changed({
                let controller = Rc::clone(&controller);
                move |_mode| {
                    controller.reload_convert_items(slot);
                }
            });
        }
    }

    fn load_bulk_naming_view(self: &Rc<Self>, slot: PaneSlot, root: PathBuf) {
        let recursive = self.pane_widgets(slot).bulk_naming_panel.recursive_active();
        self.load_bulk_naming_folder(slot, root, recursive);
    }

    fn load_bulk_naming_folder(self: &Rc<Self>, slot: PaneSlot, root: PathBuf, recursive: bool) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_label = self.display_label_for(slot);
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_label);
        pane.file_grid.clear_selection();
        pane.bulk_naming_panel.set_scope(recursive);
        pane.bulk_naming_panel
            .set_loading("Loading files for Bulk Naming...");
        self.reset_keyboard_state(slot);
        self.items_cell(slot).borrow_mut().clear();
        self.all_items_cell(slot).borrow_mut().clear();

        let (tints, tags) = {
            let metadata = self.metadata.borrow();
            (
                metadata.list_tints().unwrap_or_default(),
                metadata.list_tags().unwrap_or_default(),
            )
        };
        pane.bulk_naming_panel.set_reference_data(&tints, &tags);
        self.connect_bulk_naming_panel(slot);
        self.update_view_strip(slot);

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
            self.preview
                .show_folder("Bulk Naming", &display_label, None, None, "Bulk Naming");
            self.preview.set_action_state(false, false, false);
        }

        let generation = self.load_generation_cell(slot).get() + 1;
        self.load_generation_cell(slot).set(generation);
        let root_for_worker = root.clone();
        let show_hidden = self.show_hidden_cell(slot).get();
        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let raw = gio::spawn_blocking(move || {
                collect_bulk_naming_items_blocking(&root_for_worker, recursive, show_hidden)
            })
            .await
            .unwrap_or_default();
            if !controller.is_current_load(slot, generation) {
                return;
            }
            let items = controller.enrich_items(raw);
            controller.finish_bulk_naming_load(slot, generation, items);
        });
    }

    fn finish_bulk_naming_load(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        mut items: Vec<FileItem>,
    ) {
        if !self.is_current_load(slot, generation) {
            return;
        }
        sort_items_with(
            &mut items,
            self.sort_field_cell(slot).get(),
            self.sort_direction_cell(slot).get(),
        );
        self.load_bulk_naming_items(slot, items, HashMap::new(), true);
    }

    fn load_bulk_naming_items(
        self: &Rc<Self>,
        slot: PaneSlot,
        items: Vec<FileItem>,
        sibling_names: HashMap<PathBuf, HashSet<String>>,
        recursive: bool,
    ) {
        let display_label = self.display_label_for(slot);
        let pane = self.pane_widgets(slot);
        let (tints, tags) = {
            let metadata = self.metadata.borrow();
            (
                metadata.list_tints().unwrap_or_default(),
                metadata.list_tags().unwrap_or_default(),
            )
        };
        pane.bulk_naming_panel.set_scope(recursive);
        pane.bulk_naming_panel.set_reference_data(&tints, &tags);
        pane.bulk_naming_panel
            .set_items(items.clone(), sibling_names);
        self.connect_bulk_naming_panel(slot);
        self.all_items_cell(slot).replace(items.clone());
        self.items_cell(slot).replace(items.clone());

        if slot == self.active_slot() {
            self.status.set_path(&display_label);
            self.status.set_counts(items.len(), 0);
            self.status.set_message(&format!(
                "Bulk Naming loaded {} item{}.",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            ));
            self.update_sidebar_state();
            self.update_navigation_state();
            self.update_action_state();
            self.refresh_preview();
        }
    }

    fn connect_bulk_naming_panel(self: &Rc<Self>, slot: PaneSlot) {
        let controller = Rc::clone(self);
        self.pane_widgets(slot)
            .bulk_naming_panel
            .connect_refresh(move |recursive| {
                let PaneView::BulkNaming { root } = controller.current_view_for(slot) else {
                    return;
                };
                controller.load_bulk_naming_folder(slot, root, recursive);
            });
        let controller = Rc::clone(self);
        self.pane_widgets(slot)
            .bulk_naming_panel
            .connect_apply(move |renames| controller.apply_bulk_rename(renames));
    }

    fn sibling_names_outside_selection(
        &self,
        slot: PaneSlot,
        selected_paths: &HashSet<PathBuf>,
    ) -> HashMap<PathBuf, HashSet<String>> {
        let mut map: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        for item in self.all_items_cell(slot).borrow().iter() {
            if selected_paths.contains(&item.path) {
                continue;
            }
            let Some(parent) = item.path.parent().map(Path::to_path_buf) else {
                continue;
            };
            map.entry(parent).or_default().insert(item.name.clone());
        }
        map
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
        let show_file_controls = pane_view_uses_file_grid_controls(&self.current_view_for(slot));
        pane.filter_toggle_btn.set_visible(show_file_controls);
        pane.hidden_toggle_btn.set_visible(show_file_controls);
        pane.shape_badge_toggle_btn.set_visible(show_file_controls);
        pane.sort_btn.set_visible(show_file_controls);
        pane.view_mode_btn.set_visible(show_file_controls);

        let is_search = matches!(self.current_view_for(slot), PaneView::Search(_));
        pane.search_panel.root.set_visible(is_search);
        pane.search_revealer.set_reveal_child(is_search);

        let is_activity_log = matches!(self.current_view_for(slot), PaneView::ActivityLog);
        let is_project_landing = matches!(self.current_view_for(slot), PaneView::ProjectLanding(_));
        let is_cloud_landing = matches!(self.current_view_for(slot), PaneView::CloudLanding(_));
        let is_project_manager = matches!(self.current_view_for(slot), PaneView::ProjectManager);
        let is_tag_manager = matches!(self.current_view_for(slot), PaneView::TagManager);
        let is_bulk_naming = matches!(self.current_view_for(slot), PaneView::BulkNaming { .. });
        let is_space_viewer = matches!(self.current_view_for(slot), PaneView::SpaceViewer { .. });
        let is_media_convert = matches!(self.current_view_for(slot), PaneView::MediaConvert { .. });
        let is_watercolor = matches!(self.current_view_for(slot), PaneView::Watercolor(_));
        pane.file_grid.root.set_visible(
            !is_activity_log
                && !is_project_landing
                && !is_cloud_landing
                && !is_project_manager
                && !is_tag_manager
                && !is_bulk_naming
                && !is_space_viewer
                && !is_media_convert
                && !is_watercolor,
        );
        pane.activity_log_panel.root.set_visible(is_activity_log);
        pane.project_landing_panel
            .root
            .set_visible(is_project_landing);
        pane.cloud_landing_panel.root.set_visible(is_cloud_landing);
        pane.palette_board_panel
            .root
            .set_visible(is_project_landing);
        pane.project_manager_panel
            .root
            .set_visible(is_project_manager);
        pane.tag_manager_panel.root.set_visible(is_tag_manager);
        pane.bulk_naming_panel.root.set_visible(is_bulk_naming);
        pane.space_viewer_panel.root.set_visible(is_space_viewer);
        pane.media_convert_panel.root.set_visible(is_media_convert);
        pane.watercolor_panel.root.set_visible(is_watercolor);
        if !is_space_viewer {
            pane.space_viewer_panel.cancel_scan();
        }

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
            PaneView::CloudLanding(_) => {
                pane.view_strip.set_visible(false);
                pane.tag_filter_revealer.set_reveal_child(false);
                pane.tag_filter_revealer.set_visible(false);
                self.sync_filter_button_state(slot);
            }
            PaneView::ProjectManager => {
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
            PaneView::BulkNaming { .. } => {
                pane.view_strip.set_visible(false);
                pane.tag_filter_revealer.set_reveal_child(false);
                pane.tag_filter_revealer.set_visible(false);
                self.sync_filter_button_state(slot);
            }
            PaneView::SpaceViewer { .. } => {
                pane.view_strip.set_visible(false);
                pane.tag_filter_revealer.set_reveal_child(false);
                pane.tag_filter_revealer.set_visible(false);
                self.sync_filter_button_state(slot);
            }
            PaneView::MediaConvert { .. } => {
                pane.view_strip.set_visible(false);
                pane.tag_filter_revealer.set_reveal_child(false);
                pane.tag_filter_revealer.set_visible(false);
                self.sync_filter_button_state(slot);
            }
            PaneView::Watercolor(_) => {
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
            PaneView::BulkNaming { root } => self.load_bulk_naming_view(slot, root),
            PaneView::ActivityLog => self.load_activity_log_view(slot),
            PaneView::ProjectLanding(project_id) => {
                self.load_project_landing_view(slot, project_id)
            }
            PaneView::CloudLanding(cloud_id) => self.load_cloud_landing_view(slot, cloud_id),
            PaneView::ProjectManager => self.load_project_manager_view(slot),
            PaneView::TagManager => self.load_tag_manager_view(slot),
            PaneView::SpaceViewer { .. } => self.load_space_viewer_view(slot),
            PaneView::MediaConvert { .. } => {
                // Items are pre-loaded by open_media_convert_with_items; nothing to reload.
            }
            PaneView::Watercolor(view) => self.load_watercolor_view(slot, view),
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
                let anchor: gtk::Widget = flow.clone().upcast();
                controller.show_current_folder_menu(slot, anchor, x, y);
            }
        });
        pane.file_grid.flow.add_controller(gesture);

        let controller = Rc::clone(self);
        let icon_scroll = pane.file_grid.icon_scroll.clone();
        let icon_scroll_rclick = gtk::GestureClick::new();
        icon_scroll_rclick.set_button(3);
        icon_scroll_rclick.set_propagation_phase(gtk::PropagationPhase::Capture);
        icon_scroll_rclick.connect_pressed(move |gesture, _, x, y| {
            controller.set_active_pane(slot);
            if controller
                .icon_item_at_scroll_point(slot, &icon_scroll, x, y)
                .is_some()
            {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let anchor: gtk::Widget = icon_scroll.clone().upcast();
            controller.show_current_folder_menu(slot, anchor, x, y);
        });
        pane.file_grid
            .icon_scroll
            .add_controller(icon_scroll_rclick);

        let controller = Rc::clone(self);
        let flow_click = gtk::GestureClick::new();
        flow_click.set_button(0);
        flow_click.connect_pressed(move |_, _, _, _| controller.set_active_pane(slot));
        pane.file_grid.flow.add_controller(flow_click);

        // Paint mode: drag-to-paint on icon grid
        // (GestureDrag.drag_begin fires on initial press too, so no separate GestureClick needed)
        let controller = Rc::clone(self);
        let flow = pane.file_grid.flow.clone();
        let paint_drag = gtk::GestureDrag::new();
        paint_drag.set_button(1);
        paint_drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let controller = Rc::clone(&controller);
            let flow = flow.clone();
            paint_drag.connect_drag_begin(move |gesture, x, y| {
                if !controller.paint_mode_active.get()
                    || controller.active_paint_tool.get() == PaintTool::Cursor
                {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                // Claim immediately — prevents FlowBox rubber-band selection and click-selection
                gesture.set_state(gtk::EventSequenceState::Claimed);
                controller.current_drag_painted.borrow_mut().clear();
                // Open a drag history accumulator — all strokes become one undo step
                *controller.drag_history_accumulator.borrow_mut() = Some(Vec::new());
                if let Some(child) = flow.child_at_pos(x as i32, y as i32) {
                    controller.set_active_pane(slot);
                    controller.dispatch_paint_tool(slot, child.index());
                }
            });
        }
        {
            let controller = Rc::clone(&controller);
            let flow = flow.clone();
            paint_drag.connect_drag_update(move |gesture, offset_x, offset_y| {
                if !controller.paint_mode_active.get() {
                    return;
                }
                let Some((start_x, start_y)) = gesture.start_point() else {
                    return;
                };
                let abs_x = (start_x + offset_x) as i32;
                let abs_y = (start_y + offset_y) as i32;
                if let Some(child) = flow.child_at_pos(abs_x, abs_y) {
                    if let Some(item) = controller.item_for_index(slot, child.index()) {
                        if controller
                            .current_drag_painted
                            .borrow()
                            .contains(&item.path)
                        {
                            return;
                        }
                    }
                    controller.dispatch_paint_tool(slot, child.index());
                }
            });
        }
        {
            let controller = Rc::clone(&controller);
            paint_drag.connect_drag_end(move |_, _, _| {
                controller.current_drag_painted.borrow_mut().clear();
                // Commit accumulated drag entries as a single undo step
                if let Some(entries) = controller.drag_history_accumulator.borrow_mut().take() {
                    if !entries.is_empty() {
                        controller.commit_paint_history(PaintHistoryStep { entries });
                    }
                }
            });
        }
        pane.file_grid.flow.add_controller(paint_drag);

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
                let anchor: gtk::Widget = list_box.clone().upcast();
                controller.show_current_folder_menu(slot, anchor, x, y);
            }
        });
        pane.file_grid.list_box.add_controller(list_rclick);

        let controller = Rc::clone(self);
        let list_scroll = pane.file_grid.list_scroll.clone();
        let list_scroll_rclick = gtk::GestureClick::new();
        list_scroll_rclick.set_button(3);
        list_scroll_rclick.set_propagation_phase(gtk::PropagationPhase::Capture);
        list_scroll_rclick.connect_pressed(move |gesture, _, x, y| {
            controller.set_active_pane(slot);
            if controller
                .list_item_at_scroll_point(slot, &list_scroll, x, y)
                .is_some()
            {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let anchor: gtk::Widget = list_scroll.clone().upcast();
            controller.show_current_folder_menu(slot, anchor, x, y);
        });
        pane.file_grid
            .list_scroll
            .add_controller(list_scroll_rclick);

        let controller = Rc::clone(self);
        let list_click = gtk::GestureClick::new();
        list_click.set_button(0);
        list_click.connect_pressed(move |_, _, _, _| controller.set_active_pane(slot));
        pane.file_grid.list_box.add_controller(list_click);

        // Paint mode: left-click intercept on list
        let controller = Rc::clone(self);
        let list_box = pane.file_grid.list_box.clone();
        let paint_list_click = gtk::GestureClick::new();
        paint_list_click.set_button(1);
        paint_list_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        paint_list_click.connect_pressed(move |gesture, _, _x, y| {
            if !controller.paint_mode_active.get()
                || controller.active_paint_tool.get() == PaintTool::Cursor
            {
                return;
            }
            // Claim immediately — prevents ListBox row selection
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(row) = list_box.row_at_y(y as i32) {
                controller.set_active_pane(slot);
                controller.dispatch_paint_tool(slot, row.index());
            }
        });
        pane.file_grid.list_box.add_controller(paint_list_click);

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
        pane.shape_badge_toggle_btn.connect_clicked(move |_| {
            controller.set_active_pane(slot);
            controller.set_show_shape_badges_for_slot(
                slot,
                !controller.show_shape_badges_cell(slot).get(),
            );
        });

        let controller = Rc::clone(self);
        pane.view_mode_btn.connect_clicked(move |_| {
            controller.set_active_pane(slot);
            let next = match controller.view_mode_cell(slot).get() {
                ViewMode::Icons => ViewMode::List,
                ViewMode::List => ViewMode::Icons,
            };
            controller.set_view_mode(slot, next);
            controller.save_folder_view_state_for(slot);
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
            WindowCommand::ToggleShapeBadges => {
                let slot = self.active_slot();
                self.set_show_shape_badges_for_slot(slot, !self.show_shape_badges_cell(slot).get());
                true
            }
            WindowCommand::SortOrder => {
                let slot = self.active_slot();
                let pane = self.pane_widgets(slot);
                let widget: gtk::Widget = pane.sort_btn.clone().upcast();
                self.show_sort_popover(slot, widget);
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
            WindowCommand::TrayAddSelection => {
                self.add_selection_to_holding_tray(self.active_slot());
                true
            }
            WindowCommand::TrayMoveToProject => {
                self.show_tray_project_dialog(TrayProjectAction::Move);
                true
            }
            WindowCommand::TrayCopyToProject => {
                self.show_tray_project_dialog(TrayProjectAction::Copy);
                true
            }
            WindowCommand::TrayTag => {
                self.show_tray_tag_preview();
                true
            }
            WindowCommand::TrayTrash => {
                self.show_tray_trash_preview();
                true
            }
            WindowCommand::TrayCopyPaths => {
                self.copy_holding_tray_paths();
                true
            }
            WindowCommand::TrayClear => {
                self.clear_holding_tray();
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
            WindowCommand::OpenHome => {
                self.navigate_to(self.active_slot(), self.places.home.clone(), true);
                true
            }
            WindowCommand::OpenSystemDrives => {
                self.open_system_drives();
                true
            }
            WindowCommand::OpenRecent => {
                self.open_recent();
                true
            }
            WindowCommand::OpenTrash => {
                self.open_trash();
                true
            }
            WindowCommand::OpenPalettes => {
                self.open_project_manager();
                true
            }
            WindowCommand::OpenTintsTags => {
                self.open_tag_manager();
                true
            }
            WindowCommand::OpenSpaceViewer => {
                self.open_space_viewer();
                true
            }
            WindowCommand::OpenTriage => {
                self.triage_active_folder();
                true
            }
            WindowCommand::OpenBulkNaming => {
                self.open_bulk_naming_tool();
                true
            }
            WindowCommand::OpenConvert => {
                self.open_convert_from_sidebar();
                true
            }
            WindowCommand::OpenActivityLog => {
                self.open_activity_log();
                true
            }
            WindowCommand::SetViewIcons => {
                let slot = self.active_slot();
                self.set_view_mode(slot, ViewMode::Icons);
                self.save_folder_view_state_for(slot);
                true
            }
            WindowCommand::SetViewList => {
                let slot = self.active_slot();
                self.set_view_mode(slot, ViewMode::List);
                self.save_folder_view_state_for(slot);
                true
            }
            WindowCommand::TogglePlanMode => {
                self.set_plan_mode(!self.plan_mode_active.get());
                true
            }
            WindowCommand::TogglePaintMode => {
                self.set_paint_mode(!self.paint_mode_active.get());
                true
            }
            WindowCommand::PaintCursor => {
                if self.paint_mode_active.get() {
                    self.active_paint_tool.set(PaintTool::Cursor);
                    self.painting_toolbar.set_active_tool(PaintTool::Cursor);
                }
                self.paint_mode_active.get()
            }
            WindowCommand::PaintBrush => {
                if self.paint_mode_active.get() {
                    self.active_paint_tool.set(PaintTool::Brush);
                    self.painting_toolbar.set_active_tool(PaintTool::Brush);
                }
                self.paint_mode_active.get()
            }
            WindowCommand::PaintEraser => {
                if self.paint_mode_active.get() {
                    self.active_paint_tool.set(PaintTool::Eraser);
                    self.painting_toolbar.set_active_tool(PaintTool::Eraser);
                }
                self.paint_mode_active.get()
            }
            WindowCommand::PaintEyedropper => {
                if self.paint_mode_active.get() {
                    self.active_paint_tool.set(PaintTool::Eyedropper);
                    self.painting_toolbar.set_active_tool(PaintTool::Eyedropper);
                }
                self.paint_mode_active.get()
            }
            WindowCommand::PaintFill => {
                if self.paint_mode_active.get() {
                    let slot = self.active_slot();
                    self.paint_fill_selection(slot);
                    true
                } else {
                    false
                }
            }
            WindowCommand::PaintUndo => {
                if self.paint_mode_active.get() {
                    self.paint_undo();
                    true
                } else {
                    false
                }
            }
            WindowCommand::PaintRedo => {
                if self.paint_mode_active.get() {
                    self.paint_redo();
                    true
                } else {
                    false
                }
            }
            WindowCommand::PaintToggleContents => {
                if self.paint_mode_active.get() {
                    let next = !self.paint_contents.get();
                    self.paint_contents.set(next);
                    self.painting_toolbar.set_paint_contents(next);
                    true
                } else {
                    false
                }
            }
            WindowCommand::EmptyTrash => {
                self.empty_trash();
                true
            }
            WindowCommand::TrayAddByTint => {
                let widget: gtk::Widget = self.holding_tray.add_by_tint_button.clone().upcast();
                self.show_add_to_tray_by_tint_popover(&widget);
                true
            }
            WindowCommand::TrayAddByShape => {
                let widget: gtk::Widget = self.holding_tray.add_by_shape_button.clone().upcast();
                self.show_add_to_tray_by_shape_popover(&widget);
                true
            }
            WindowCommand::TrayApplyMark => {
                self.show_tray_apply_mark_preview();
                true
            }
            WindowCommand::TrayResetMark => {
                self.show_tray_reset_mark_preview();
                true
            }
            WindowCommand::PlanExecute => {
                if self.plan_mode_active.get() && !self.action_queue.borrow().is_empty() {
                    self.execute_plan_queue();
                    true
                } else {
                    false
                }
            }
            WindowCommand::PlanClear => {
                if self.plan_mode_active.get() && !self.action_queue.borrow().is_empty() {
                    self.action_queue.borrow_mut().clear();
                    self.refresh_plan_queue_panel();
                    true
                } else {
                    false
                }
            }
            WindowCommand::ConvertStart => {
                let slot = self.active_slot();
                if matches!(self.current_view_for(slot), PaneView::MediaConvert { .. }) {
                    self.pane_widgets(slot)
                        .media_convert_panel
                        .start_current_batch()
                } else {
                    false
                }
            }
            WindowCommand::ConvertCancel => self.convert_progress.trigger_cancel(),
            WindowCommand::ConvertRetryFailed => self.convert_progress.trigger_retry_failed(),
            WindowCommand::ConvertOpenOutput => self.convert_progress.trigger_open_output(),
            WindowCommand::ConvertDismiss => self.convert_progress.trigger_dismiss(),
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
        if modifiers == gdk::ModifierType::CONTROL_MASK && key_char(key) == Some('c') {
            self.copy_holding_tray_paths();
            return true;
        }

        if modifiers == gdk::ModifierType::CONTROL_MASK && key_char(key) == Some('v') {
            self.paste_file_clipboard_into_holding_tray();
            return true;
        }

        if !modifiers.is_empty() {
            return false;
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

    // Save the current directory to back history when entering a tool view from a directory.
    // Tool-to-tool transitions are intentionally ignored so back always returns to the last
    // real folder, not a phantom home path set by a previous tool.
    fn save_dir_to_history_if_in_directory(&self, slot: PaneSlot) {
        if let PaneView::Directory(path) = self.current_view_for(slot) {
            self.back_history_cell(slot).borrow_mut().push(path);
            self.forward_history_cell(slot).borrow_mut().clear();
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

    fn tool_scope_dir_for(&self, slot: PaneSlot) -> PathBuf {
        resolve_tool_scope_dir(
            &self.current_view_for(slot),
            &self.current_dir_for(slot),
            &self.places.home,
        )
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

    fn show_shape_badges_cell(&self, slot: PaneSlot) -> &Cell<bool> {
        match slot {
            PaneSlot::Primary => &self.primary_show_shape_badges,
            PaneSlot::Secondary => &self.secondary_show_shape_badges,
            PaneSlot::Tertiary => &self.tertiary_show_shape_badges,
        }
    }

    fn badges_hidden_by_paint_cell(&self, slot: PaneSlot) -> &Cell<bool> {
        match slot {
            PaneSlot::Primary => &self.primary_badges_hidden_by_paint,
            PaneSlot::Secondary => &self.secondary_badges_hidden_by_paint,
            PaneSlot::Tertiary => &self.tertiary_badges_hidden_by_paint,
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
        let mut vs = crate::view_state::ViewState::load();
        vs.view_mode = match mode {
            ViewMode::Icons => "icons",
            ViewMode::List => "list",
        }
        .to_string();
        vs.save();
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
        let mut vs = crate::view_state::ViewState::load();
        vs.show_hidden = show_hidden;
        vs.save();
        self.save_folder_view_state_for(slot);
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

    fn set_show_shape_badges_for_slot(self: &Rc<Self>, slot: PaneSlot, show_badges: bool) {
        // User explicitly changed state; cancel any paint-mode auto-show tracking
        self.badges_hidden_by_paint_cell(slot).set(false);
        if self.show_shape_badges_cell(slot).get() == show_badges {
            self.sync_show_shape_badges_button_state(slot);
            return;
        }
        self.show_shape_badges_cell(slot).set(show_badges);
        self.pane_widgets(slot)
            .file_grid
            .set_shape_badges_visible(show_badges);
        self.sync_show_shape_badges_button_state(slot);
        self.sync_active_tab_state();
        let mut vs = crate::view_state::ViewState::load();
        vs.show_shape_badges = show_badges;
        vs.save();
        self.save_folder_view_state_for(slot);
    }

    fn sync_show_shape_badges_button_state(&self, slot: PaneSlot) {
        let pane = self.pane_widgets(slot);
        let show_badges = self.show_shape_badges_cell(slot).get();
        pane.file_grid.set_shape_badges_visible(show_badges);
        if show_badges {
            pane.shape_badge_toggle_btn
                .add_css_class("pane-control-active");
            pane.shape_badge_toggle_icon
                .set_icon_name(Some("emblem-default-symbolic"));
        } else {
            pane.shape_badge_toggle_btn
                .remove_css_class("pane-control-active");
            pane.shape_badge_toggle_icon
                .set_icon_name(Some("emblem-default-symbolic"));
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
            tab.primary_show_shape_badges = self.primary_show_shape_badges.get();
            tab.secondary_show_shape_badges = self.secondary_show_shape_badges.get();
            tab.tertiary_show_shape_badges = self.tertiary_show_shape_badges.get();
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
            let close_host = crate::ui::tooltip_host(
                &close_button,
                shortcut_tooltip(&self.config, "Close tab", "close_tab"),
            );
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
        self.primary_show_shape_badges
            .set(tab.primary_show_shape_badges);
        self.secondary_show_shape_badges
            .set(tab.secondary_show_shape_badges);
        self.tertiary_show_shape_badges
            .set(tab.tertiary_show_shape_badges);
        self.primary_sort_field.set(tab.primary_sort_field);
        self.primary_sort_direction.set(tab.primary_sort_direction);
        self.secondary_sort_field.set(tab.secondary_sort_field);
        self.secondary_sort_direction
            .set(tab.secondary_sort_direction);
        self.tertiary_sort_field.set(tab.tertiary_sort_field);
        self.tertiary_sort_direction
            .set(tab.tertiary_sort_direction);
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            self.set_view_mode(slot, self.view_mode_cell(slot).get());
            self.sync_show_hidden_button_state(slot);
            self.sync_show_shape_badges_button_state(slot);
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
        if let PaneView::Directory(ref path) = self.current_view_for(PaneSlot::Primary).clone() {
            self.apply_folder_view_state(PaneSlot::Primary, path);
        }
        self.load_current_view(PaneSlot::Primary);
        for slot in [PaneSlot::Secondary, PaneSlot::Tertiary] {
            if self.pane_layout.get().includes(slot) {
                self.update_view_strip(slot);
                if let PaneView::Directory(ref path) = self.current_view_for(slot).clone() {
                    self.apply_folder_view_state(slot, path);
                }
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
        let (icon, label) = match self.pane_layout.get() {
            PaneLayout::Single => ("view-list-symbolic", "Switch to 2 panels"),
            PaneLayout::Two => ("view-dual-symbolic", "Switch to 3 panels"),
            PaneLayout::Three => ("view-grid-symbolic", "Switch to 1 panel"),
        };
        self.toolbar.set_split_icon_state(icon);
        self.toolbar
            .split_tooltip_label
            .set_label(&shortcut_tooltip(&self.config, label, "toggle_split"));
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
        self.apply_folder_view_state(slot, &path);
        self.load_directory(slot, path);
    }

    fn navigate_to_active(self: &Rc<Self>, path: PathBuf) {
        self.navigate_to(self.active_slot(), path, true);
    }

    fn go_back(self: &Rc<Self>) {
        let slot = self.active_slot();
        let previous = self.back_history_cell(slot).borrow_mut().pop();
        if let Some(path) = previous {
            // Only push to forward history when leaving a real directory so that
            // going forward after returning from a tool doesn't land on a phantom path.
            if self.is_directory_view(slot) {
                let current = self.current_dir_for(slot);
                self.forward_history_cell(slot).borrow_mut().push(current);
            }
            self.current_dir_cell(slot).replace(path.clone());
            self.current_view_cell(slot)
                .replace(PaneView::Directory(path.clone()));
            self.sync_active_tab_state();
            self.update_view_strip(slot);
            if slot == PaneSlot::Primary {
                self.rebuild_tab_strip();
            }
            self.apply_folder_view_state(slot, &path);
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
            self.apply_folder_view_state(slot, &path);
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
        let current_str = current.to_string_lossy();
        if is_gio_uri(&current_str) {
            let file = gio::File::for_uri(current_str.as_ref());
            if let Some(parent) = file.parent() {
                self.navigate_to(slot, PathBuf::from(parent.uri().as_str()), true);
            }
        } else if let Some(parent) = current.parent() {
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

        let target_path = match target_file.path() {
            Some(p) => p,
            None => {
                // No local path — allow navigation for GIO remote URIs
                let raw = raw_input.trim();
                if is_gio_uri(raw) {
                    self.navigate_to(self.active_slot(), PathBuf::from(raw), true);
                } else {
                    self.show_error_dialog(
                        "Unsupported Path",
                        "That location could not be resolved to a local filesystem path.",
                    );
                    self.sync_path_entry_to_display();
                }
                return;
            }
        };

        let file_type =
            target_file.query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>);
        if file_type == gio::FileType::Unknown
            && !target_file.query_exists(None::<&gio::Cancellable>)
        {
            if !looks_like_explicit_path(&raw_input) {
                let slot = self.active_slot();
                let mut query = SearchQuery::new(self.current_dir_for(slot));
                query.name = raw_input;
                query.recursive = false;
                self.open_search(slot, query);
                return;
            }
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
        let slot = self.active_slot();
        let text = if self.path_box_scope_dir.borrow().is_some() {
            if let PaneView::Search(ref q) = self.current_view_for(slot) {
                q.name.clone()
            } else {
                self.current_dir_for(slot).display().to_string()
            }
        } else {
            self.current_dir_for(slot).display().to_string()
        };
        self.toolbar.show_entry_mode();
        self.toolbar.path_entry.set_text(&text);
        let entry = self.toolbar.path_entry.clone();
        glib::idle_add_local_once(move || {
            entry.grab_focus();
            entry.select_region(0, -1);
        });
    }

    fn finish_path_entry_editing(&self) {
        if let Some(id) = self.path_box_debounce.borrow_mut().take() {
            id.remove();
        }
        self.sync_path_entry_to_display();
    }

    fn cancel_path_entry_editing(self: &Rc<Self>) {
        if let Some(id) = self.path_box_debounce.borrow_mut().take() {
            id.remove();
        }
        if let Some(dir) = self.path_box_scope_dir.borrow_mut().take() {
            self.navigate_to(self.active_slot(), dir, false);
        } else {
            self.sync_path_entry_to_display();
        }
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
        let mut vs = crate::view_state::ViewState::load();
        vs.sidebar_visible = visible;
        vs.save();
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
        let mut vs = crate::view_state::ViewState::load();
        vs.preview_visible = visible;
        vs.save();
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

    // ── Painting Mode ────────────────────────────────────────────────────────

    fn set_paint_mode(self: &Rc<Self>, active: bool) {
        self.paint_mode_active.set(active);
        if self.toolbar.paint_mode_toggle.is_active() != active {
            self.toolbar.paint_mode_toggle.set_active(active);
        }
        self.painting_toolbar.set_reveal(active);
        if active {
            // Auto-show mark badges in all visible panes so painted files are immediately visible
            for slot in self.visible_slots() {
                if !self.show_shape_badges_cell(slot).get() {
                    self.badges_hidden_by_paint_cell(slot).set(true);
                    self.show_shape_badges_cell(slot).set(true);
                    self.pane_widgets(slot)
                        .file_grid
                        .set_shape_badges_visible(true);
                    self.sync_show_shape_badges_button_state(slot);
                }
            }
            // Always start in cursor/select mode so paint mode is opt-in for each stroke
            self.active_paint_tool.set(PaintTool::Cursor);
            let tints = self.metadata.borrow().list_tints().unwrap_or_default();
            self.painting_toolbar
                .set_tints(&tints, self.active_paint_tint_id.get());
            self.painting_toolbar
                .set_active_shape(self.active_paint_shape.get());
            self.painting_toolbar.set_active_tool(PaintTool::Cursor);
            self.painting_toolbar
                .set_paint_contents(self.paint_contents.get());
            self.painting_toolbar
                .set_paint_type(self.active_paint_type.get());
            let tags = self.metadata.borrow().list_tags().unwrap_or_default();
            self.painting_toolbar
                .set_tags(&tags, self.active_paint_tag_id.get());
            {
                let ctrl = Rc::clone(self);
                self.painting_toolbar
                    .connect_tint_changed(move |id| ctrl.on_paint_tint_changed(id));
            }
            {
                let ctrl = Rc::clone(self);
                let pt = self.painting_toolbar.clone();
                self.painting_toolbar.connect_shape_changed(move |s| {
                    ctrl.active_paint_shape.set(s);
                    pt.set_active_shape(s);
                });
            }
            {
                let ctrl = Rc::clone(self);
                self.painting_toolbar.connect_tool_changed(move |t| {
                    ctrl.active_paint_tool.set(t);
                    // Cursor and FillSelection keep normal selection; stroke tools disable it
                    // so dragging paints rather than rubber-band selects
                    let sel_enabled = matches!(t, PaintTool::Cursor | PaintTool::FillSelection);
                    for slot in ctrl.visible_slots() {
                        ctrl.pane_widgets(slot)
                            .file_grid
                            .set_selection_enabled(sel_enabled);
                    }
                });
            }
            {
                let ctrl = Rc::clone(self);
                self.painting_toolbar
                    .connect_paint_contents_changed(move |on| ctrl.paint_contents.set(on));
            }
            {
                let ctrl = Rc::clone(self);
                self.painting_toolbar
                    .connect_paint_type_changed(move |pt| ctrl.active_paint_type.set(pt));
            }
            {
                let ctrl = Rc::clone(self);
                self.painting_toolbar
                    .connect_tag_changed(move |id| ctrl.on_paint_tag_changed(id));
            }
            {
                let ctrl = Rc::clone(self);
                self.painting_toolbar
                    .connect_undo(move || ctrl.paint_undo());
            }
            {
                let ctrl = Rc::clone(self);
                self.painting_toolbar
                    .connect_redo(move || ctrl.paint_redo());
            }
            self.refresh_paint_undo_redo_state();
            self.status
                .set_message("🎨 Painting Mode — click files to apply marks or tags.");
        } else {
            // Restore selection and badge state for all visible panes
            for slot in self.visible_slots() {
                self.pane_widgets(slot)
                    .file_grid
                    .set_selection_enabled(true);
                if self.badges_hidden_by_paint_cell(slot).get() {
                    self.badges_hidden_by_paint_cell(slot).set(false);
                    self.show_shape_badges_cell(slot).set(false);
                    self.pane_widgets(slot)
                        .file_grid
                        .set_shape_badges_visible(false);
                    self.sync_show_shape_badges_button_state(slot);
                }
            }
            self.status.set_message("Painting Mode OFF.");
        }
    }

    fn on_paint_tint_changed(self: &Rc<Self>, tint_id: i64) {
        let meta = self.metadata.borrow();
        let tints = meta.list_tints().unwrap_or_default();
        if let Some(t) = tints.iter().find(|t| t.id == tint_id) {
            self.active_paint_tint_id.set(tint_id);
            let color = t.color.as_deref().unwrap_or("#806040");
            *self.active_paint_tint_color.borrow_mut() = color.to_string();
            *self.active_paint_tint_name.borrow_mut() = t.name.clone();
            drop(meta);
            let tints2 = self.metadata.borrow().list_tints().unwrap_or_default();
            self.painting_toolbar.set_tints(&tints2, tint_id);
        }
    }

    fn on_paint_tag_changed(self: &Rc<Self>, tag_id: i64) {
        let tags = self.metadata.borrow().list_tags().unwrap_or_default();
        if let Some(t) = tags.iter().find(|t| t.id == tag_id) {
            self.active_paint_tag_id.set(tag_id);
            *self.active_paint_tag_name.borrow_mut() = t.name.clone();
            self.painting_toolbar.set_active_tag_display(&t.name);
            self.painting_toolbar.set_tags(&tags, tag_id);
        }
    }

    fn dispatch_paint_tool(self: &Rc<Self>, slot: PaneSlot, index: i32) {
        let Some(item) = self.item_for_index(slot, index) else {
            return;
        };
        match self.active_paint_type.get() {
            PaintType::Mark => match self.active_paint_tool.get() {
                PaintTool::Cursor => {}
                PaintTool::Brush => self.paint_brush_item(slot, &item),
                PaintTool::Eraser => self.paint_eraser_item(slot, &item),
                PaintTool::Eyedropper => self.paint_eyedropper_item(&item),
                PaintTool::FillSelection => self.paint_fill_selection(slot),
            },
            PaintType::Tag => match self.active_paint_tool.get() {
                PaintTool::Cursor => {}
                PaintTool::Brush => self.paint_tag_brush_item(slot, &item),
                PaintTool::Eraser => self.paint_tag_eraser_item(slot, &item),
                PaintTool::Eyedropper => self.paint_tag_eyedropper_item(&item),
                PaintTool::FillSelection => self.paint_tag_fill_selection(slot),
            },
        }
    }

    fn paint_brush_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
        if item.is_dir && self.paint_contents.get() {
            self.paint_folder_with_preview(slot, item.path.clone());
            return;
        }
        let tint_id = self.active_paint_tint_id.get();
        let shape = self.active_paint_shape.get();
        let tint_name = self.active_paint_tint_name.borrow().clone();
        if self.should_queue_actions() {
            self.queue_plan(FileOpPlan::for_paint_mark(
                std::slice::from_ref(&item.path),
                tint_id,
                &tint_name,
                shape,
                false,
            ));
            return;
        }
        // Capture previous mark before overwriting
        let prev = self.read_explicit_mark(&item.path);
        let result = self
            .metadata
            .borrow_mut()
            .set_file_mark(&item.path, tint_id, shape);
        if result.is_ok() {
            self.current_drag_painted
                .borrow_mut()
                .insert(item.path.clone());
            self.update_item_mark_in_grid(slot, &item.path, tint_id, shape);
            let entry = PaintHistoryEntry {
                path: item.path.clone(),
                op: PaintOp::Mark {
                    prev,
                    next: Some((tint_id, shape)),
                },
            };
            if !self.append_or_commit_history(entry) {
                self.commit_paint_history(PaintHistoryStep {
                    entries: vec![PaintHistoryEntry {
                        path: item.path.clone(),
                        op: PaintOp::Mark {
                            prev,
                            next: Some((tint_id, shape)),
                        },
                    }],
                });
            }
            self.log_paint_mark(slot, &[item.path.clone()], tint_id, shape);
        }
    }

    fn paint_eraser_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
        if self.should_queue_actions() {
            self.queue_plan(FileOpPlan::for_reset_mark(
                std::slice::from_ref(&item.path),
                false,
            ));
            return;
        }
        let prev = self.read_explicit_mark(&item.path);
        let _ = self.metadata.borrow_mut().clear_file_mark(&item.path);
        self.current_drag_painted
            .borrow_mut()
            .insert(item.path.clone());
        let default = {
            let meta = self.metadata.borrow();
            meta.list_tints()
                .unwrap_or_default()
                .into_iter()
                .find(|t| t.is_default)
        };
        let (default_tint_id, default_shape) = default
            .map(|t| (t.id, Shape::DEFAULT))
            .unwrap_or((self.active_paint_tint_id.get(), Shape::DEFAULT));
        self.update_item_mark_in_grid(slot, &item.path, default_tint_id, default_shape);
        let entry = PaintHistoryEntry {
            path: item.path.clone(),
            op: PaintOp::Mark { prev, next: None },
        };
        if !self.append_or_commit_history(entry) {
            self.commit_paint_history(PaintHistoryStep {
                entries: vec![PaintHistoryEntry {
                    path: item.path.clone(),
                    op: PaintOp::Mark { prev, next: None },
                }],
            });
        }
        self.log_erase_mark(slot, &[item.path.clone()]);
    }

    fn paint_eyedropper_item(self: &Rc<Self>, item: &FileItem) {
        let tint_id = item.mark_tint_id;
        let shape = item.mark_shape;
        self.active_paint_tint_id.set(tint_id);
        self.active_paint_shape.set(shape);
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        if let Some(t) = tints.iter().find(|t| t.id == tint_id) {
            let color = t.color.as_deref().unwrap_or("#806040");
            *self.active_paint_tint_color.borrow_mut() = color.to_string();
            *self.active_paint_tint_name.borrow_mut() = t.name.clone();
            self.painting_toolbar.set_tints(&tints, tint_id);
        }
        self.painting_toolbar.set_active_shape(shape);
        self.status.set_message(&format!(
            "Eyedropper: picked {} {}",
            self.active_paint_tint_name.borrow(),
            shape.display_name()
        ));
    }

    fn paint_fill_selection(self: &Rc<Self>, slot: PaneSlot) {
        let items = self.selected_items_for(slot);
        if items.is_empty() {
            return;
        }
        let tint_id = self.active_paint_tint_id.get();
        let shape = self.active_paint_shape.get();
        let tint_name = self.active_paint_tint_name.borrow().clone();
        if self.should_queue_actions() {
            let paths = items
                .iter()
                .map(|item| item.path.clone())
                .collect::<Vec<_>>();
            self.queue_plan(FileOpPlan::for_paint_mark(
                &paths, tint_id, &tint_name, shape, false,
            ));
            return;
        }
        let mut history_entries = Vec::new();
        let mut paths = Vec::new();
        {
            let mut meta = self.metadata.borrow_mut();
            for item in &items {
                let prev = read_explicit_mark_from_item(item);
                if meta.set_file_mark(&item.path, tint_id, shape).is_ok() {
                    paths.push(item.path.clone());
                    history_entries.push(PaintHistoryEntry {
                        path: item.path.clone(),
                        op: PaintOp::Mark {
                            prev,
                            next: Some((tint_id, shape)),
                        },
                    });
                }
            }
        }
        for item in &items {
            if paths.contains(&item.path) {
                self.update_item_mark_in_grid(slot, &item.path, tint_id, shape);
            }
        }
        if !history_entries.is_empty() {
            self.commit_paint_history(PaintHistoryStep {
                entries: history_entries,
            });
        }
        if !paths.is_empty() {
            self.log_paint_mark(slot, &paths, tint_id, shape);
        }
    }

    fn paint_tag_brush_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
        let tag_id = self.active_paint_tag_id.get();
        if tag_id == 0 {
            self.status
                .set_message("No tag selected — choose a tag in the toolbar.");
            return;
        }
        let tag_name = self.active_paint_tag_name.borrow().clone();
        let already_has = item.tags.iter().any(|t| t.id == tag_id);
        if already_has {
            return;
        }
        let paths = std::slice::from_ref(&item.path);
        if self
            .metadata
            .borrow_mut()
            .add_tag_to_paths(tag_id, paths)
            .is_ok()
        {
            self.current_drag_painted
                .borrow_mut()
                .insert(item.path.clone());
            let tags = self
                .metadata
                .borrow()
                .tags_for_paths(paths)
                .unwrap_or_default()
                .remove(&item.path)
                .unwrap_or_default();
            self.update_item_tags_in_grid(slot, &item.path, tags);
            let entry = PaintHistoryEntry {
                path: item.path.clone(),
                op: PaintOp::Tag {
                    tag_id,
                    added: true,
                },
            };
            if !self.append_or_commit_history(entry) {
                self.commit_paint_history(PaintHistoryStep {
                    entries: vec![PaintHistoryEntry {
                        path: item.path.clone(),
                        op: PaintOp::Tag {
                            tag_id,
                            added: true,
                        },
                    }],
                });
            }
            self.status.set_message(&format!("Tagged: {tag_name}"));
        }
    }

    fn paint_tag_eraser_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
        let tag_id = self.active_paint_tag_id.get();
        if tag_id == 0 {
            return;
        }
        let tag_name = self.active_paint_tag_name.borrow().clone();
        let had_tag = item.tags.iter().any(|t| t.id == tag_id);
        if !had_tag {
            return;
        }
        let paths = std::slice::from_ref(&item.path);
        if self
            .metadata
            .borrow_mut()
            .remove_tag_from_paths(tag_id, paths)
            .is_ok()
        {
            self.current_drag_painted
                .borrow_mut()
                .insert(item.path.clone());
            let tags = self
                .metadata
                .borrow()
                .tags_for_paths(paths)
                .unwrap_or_default()
                .remove(&item.path)
                .unwrap_or_default();
            self.update_item_tags_in_grid(slot, &item.path, tags);
            let entry = PaintHistoryEntry {
                path: item.path.clone(),
                op: PaintOp::Tag {
                    tag_id,
                    added: false,
                },
            };
            if !self.append_or_commit_history(entry) {
                self.commit_paint_history(PaintHistoryStep {
                    entries: vec![PaintHistoryEntry {
                        path: item.path.clone(),
                        op: PaintOp::Tag {
                            tag_id,
                            added: false,
                        },
                    }],
                });
            }
            self.status.set_message(&format!("Removed tag: {tag_name}"));
        }
    }

    fn paint_tag_eyedropper_item(self: &Rc<Self>, item: &FileItem) {
        let Some(tag) = item.tags.first() else {
            self.status.set_message("Eyedropper: no tag on this item.");
            return;
        };
        let tag_id = tag.id;
        let tag_name = tag.name.clone();
        self.active_paint_tag_id.set(tag_id);
        *self.active_paint_tag_name.borrow_mut() = tag_name.clone();
        let tags = self.metadata.borrow().list_tags().unwrap_or_default();
        self.painting_toolbar.set_tags(&tags, tag_id);
        self.status
            .set_message(&format!("Eyedropper: picked tag '{tag_name}'"));
    }

    fn paint_tag_fill_selection(self: &Rc<Self>, slot: PaneSlot) {
        let tag_id = self.active_paint_tag_id.get();
        if tag_id == 0 {
            self.status
                .set_message("No tag selected — choose a tag in the toolbar.");
            return;
        }
        let tag_name = self.active_paint_tag_name.borrow().clone();
        let items = self.selected_items_for(slot);
        if items.is_empty() {
            return;
        }
        let paths_to_tag: Vec<PathBuf> = items
            .iter()
            .filter(|i| !i.tags.iter().any(|t| t.id == tag_id))
            .map(|i| i.path.clone())
            .collect();
        if paths_to_tag.is_empty() {
            return;
        }
        let mut history_entries = Vec::new();
        {
            let mut meta = self.metadata.borrow_mut();
            for path in &paths_to_tag {
                if meta
                    .add_tag_to_paths(tag_id, std::slice::from_ref(path))
                    .is_ok()
                {
                    history_entries.push(PaintHistoryEntry {
                        path: path.clone(),
                        op: PaintOp::Tag {
                            tag_id,
                            added: true,
                        },
                    });
                }
            }
        }
        let all_paths: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
        let mut tags_by_path = self
            .metadata
            .borrow()
            .tags_for_paths(&all_paths)
            .unwrap_or_default();
        for path in &paths_to_tag {
            let tags = tags_by_path.remove(path).unwrap_or_default();
            self.update_item_tags_in_grid(slot, path, tags);
        }
        if !history_entries.is_empty() {
            self.commit_paint_history(PaintHistoryStep {
                entries: history_entries,
            });
        }
        let count = paths_to_tag.len();
        self.status.set_message(&format!(
            "Tagged {} item{} as '{tag_name}'",
            count,
            if count == 1 { "" } else { "s" }
        ));
    }

    fn update_item_mark_in_grid(
        self: &Rc<Self>,
        slot: PaneSlot,
        path: &PathBuf,
        tint_id: i64,
        shape: Shape,
    ) {
        let items = self.items_cell(slot).borrow();
        if let Some(index) = items.iter().position(|i| &i.path == path) {
            drop(items);
            let tint_color = self.tint_color_for_id(tint_id);
            self.pane_widgets(slot).file_grid.update_item_mark(
                index,
                tint_id,
                tint_color.as_deref(),
                shape,
            );
            // Also update the in-memory record
            let mut items = self.items_cell(slot).borrow_mut();
            if let Some(item) = items.get_mut(index) {
                item.mark_tint_id = tint_id;
                item.mark_tint_color = tint_color;
                item.mark_shape = shape;
            }
        }
    }

    fn update_item_tags_in_grid(
        self: &Rc<Self>,
        slot: PaneSlot,
        path: &PathBuf,
        tags: Vec<crate::metadata::TagRecord>,
    ) {
        let mut items = self.items_cell(slot).borrow_mut();
        if let Some(idx) = items.iter().position(|i| &i.path == path) {
            items[idx].tags = tags.clone();
            drop(items);
            self.pane_widgets(slot)
                .file_grid
                .update_item_tags(idx, &tags);
        }
    }

    fn tint_color_for_id(&self, tint_id: i64) -> Option<String> {
        self.metadata
            .borrow()
            .list_tints()
            .unwrap_or_default()
            .into_iter()
            .find(|tint| tint.id == tint_id)
            .and_then(|tint| tint.color)
    }

    fn log_paint_mark(
        self: &Rc<Self>,
        slot: PaneSlot,
        paths: &[PathBuf],
        _tint_id: i64,
        shape: Shape,
    ) {
        let tint_name = self.active_paint_tint_name.borrow().clone();
        let count = paths.len();
        let summary = format!(
            "Marked {} item{} {} {}",
            count,
            if count == 1 { "" } else { "s" },
            tint_name,
            shape.display_name()
        );
        let source = self.current_dir_for(slot).display().to_string();
        let items: Vec<(PathBuf, Option<PathBuf>)> =
            paths.iter().map(|p| (p.clone(), None)).collect();
        self.metadata
            .borrow()
            .log_activity_with_items(
                "paint_mark",
                count as i32,
                &summary,
                &source,
                None,
                &[],
                &items,
            )
            .ok();
        self.status.set_message(&summary);
    }

    fn log_erase_mark(self: &Rc<Self>, slot: PaneSlot, paths: &[PathBuf]) {
        let count = paths.len();
        let summary = format!(
            "Reset {} item{} to Beige Square",
            count,
            if count == 1 { "" } else { "s" }
        );
        let source = self.current_dir_for(slot).display().to_string();
        let items: Vec<(PathBuf, Option<PathBuf>)> =
            paths.iter().map(|p| (p.clone(), None)).collect();
        self.metadata
            .borrow()
            .log_activity_with_items(
                "erase_mark",
                count as i32,
                &summary,
                &source,
                None,
                &[],
                &items,
            )
            .ok();
        self.status.set_message(&summary);
    }

    // ── Paint history helpers ────────────────────────────────────────────────

    /// Read the explicit mark for a path (None = path uses default, no explicit row).
    fn read_explicit_mark(&self, path: &PathBuf) -> Option<(i64, Shape)> {
        let meta = self.metadata.borrow();
        let paths = std::slice::from_ref(path);
        let marks = meta.marks_for_paths(paths).unwrap_or_default();
        if let Some(m) = marks.get(path) {
            // created_at == 0 is the synthetic default sentinel
            if m.created_at != 0 {
                return Some((m.tint_id, m.shape));
            }
        }
        None
    }

    /// Append a history entry to the ongoing drag accumulator, or return false
    /// if there is no open accumulator (single-click case, caller should commit).
    fn append_or_commit_history(&self, entry: PaintHistoryEntry) -> bool {
        let mut acc = self.drag_history_accumulator.borrow_mut();
        if let Some(ref mut entries) = *acc {
            entries.push(entry);
            true
        } else {
            false
        }
    }

    /// Push a completed step onto the undo stack and clear the redo stack.
    fn commit_paint_history(&self, step: PaintHistoryStep) {
        let mut undo = self.paint_undo_stack.borrow_mut();
        undo.push(step);
        if undo.len() > PAINT_HISTORY_LIMIT {
            undo.remove(0);
        }
        drop(undo);
        self.paint_redo_stack.borrow_mut().clear();
        self.refresh_paint_undo_redo_state();
    }

    fn refresh_paint_undo_redo_state(&self) {
        let can_undo = !self.paint_undo_stack.borrow().is_empty();
        let can_redo = !self.paint_redo_stack.borrow().is_empty();
        self.painting_toolbar.set_undo_enabled(can_undo);
        self.painting_toolbar.set_redo_enabled(can_redo);
    }

    fn paint_undo(self: &Rc<Self>) {
        let step = self.paint_undo_stack.borrow_mut().pop();
        let Some(step) = step else { return };
        let slot = self.active_slot();
        self.apply_paint_history_step(slot, &step, true);
        self.paint_redo_stack.borrow_mut().push(step);
        self.refresh_paint_undo_redo_state();
        self.status.set_message("Paint undo.");
    }

    fn paint_redo(self: &Rc<Self>) {
        let step = self.paint_redo_stack.borrow_mut().pop();
        let Some(step) = step else { return };
        let slot = self.active_slot();
        self.apply_paint_history_step(slot, &step, false);
        self.paint_undo_stack.borrow_mut().push(step);
        self.refresh_paint_undo_redo_state();
        self.status.set_message("Paint redo.");
    }

    /// Apply a history step (undo = restore `prev`; redo = restore `next`).
    fn apply_paint_history_step(
        self: &Rc<Self>,
        slot: PaneSlot,
        step: &PaintHistoryStep,
        is_undo: bool,
    ) {
        // When restoring to "no explicit mark", show the system default tint, not the active
        // paint tint. Same approach as paint_eraser_item.
        let system_default_tint_id = self
            .metadata
            .borrow()
            .list_tints()
            .unwrap_or_default()
            .into_iter()
            .find(|t| t.is_default)
            .map(|t| t.id)
            .unwrap_or_else(|| self.active_paint_tint_id.get());
        for entry in &step.entries {
            match &entry.op {
                PaintOp::Mark { prev, next } => {
                    let target = if is_undo { *prev } else { *next };
                    match target {
                        Some((tint_id, shape)) => {
                            self.metadata
                                .borrow_mut()
                                .set_file_mark(&entry.path, tint_id, shape)
                                .ok();
                            self.update_item_mark_in_grid(slot, &entry.path, tint_id, shape);
                        }
                        None => {
                            self.metadata.borrow_mut().clear_file_mark(&entry.path).ok();
                            self.update_item_mark_in_grid(
                                slot,
                                &entry.path,
                                system_default_tint_id,
                                Shape::DEFAULT,
                            );
                        }
                    }
                }
                PaintOp::Tag { tag_id, added } => {
                    // undo of add → remove; undo of remove → add; redo is opposite
                    let should_add = if is_undo { !added } else { *added };
                    let paths = std::slice::from_ref(&entry.path);
                    if should_add {
                        self.metadata
                            .borrow_mut()
                            .add_tag_to_paths(*tag_id, paths)
                            .ok();
                    } else {
                        self.metadata
                            .borrow_mut()
                            .remove_tag_from_paths(*tag_id, paths)
                            .ok();
                    }
                    let tags = self
                        .metadata
                        .borrow()
                        .tags_for_paths(paths)
                        .unwrap_or_default()
                        .remove(&entry.path)
                        .unwrap_or_default();
                    self.update_item_tags_in_grid(slot, &entry.path, tags);
                }
            }
        }
    }

    fn paint_folder_with_preview(self: &Rc<Self>, slot: PaneSlot, folder_path: PathBuf) {
        let tint_id = self.active_paint_tint_id.get();
        let shape = self.active_paint_shape.get();
        let tint_name = self.active_paint_tint_name.borrow().clone();
        let folder_name = folder_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("folder")
            .to_string();
        let cloud_suffix = self
            .cloud_name_for_path(&folder_path)
            .map(|(name, _)| {
                format!("\n\n☁ '{name}' is a cloud drive — marking many files may be slow.")
            })
            .unwrap_or_default();
        let prompt = format!(
            "Paint all contents of \"{}\" as {} {} recursively?\n\nAll files and subfolders will receive this mark.{}",
            folder_name,
            tint_name,
            shape.display_name(),
            cloud_suffix
        );
        let title = format!("Paint Contents of {folder_name}");
        let controller = Rc::clone(self);
        self.modal_host
            .show_confirm(&title, &prompt, "Paint", false, true, move || {
                if controller.should_queue_actions() {
                    controller.queue_plan(FileOpPlan::for_paint_mark(
                        std::slice::from_ref(&folder_path),
                        tint_id,
                        &tint_name,
                        shape,
                        true,
                    ));
                    return;
                }
                controller.do_paint_folder_recursive(
                    slot,
                    folder_path.clone(),
                    tint_id,
                    shape,
                    tint_name.clone(),
                );
            });
    }

    fn do_paint_folder_recursive(
        self: &Rc<Self>,
        slot: PaneSlot,
        folder_path: PathBuf,
        tint_id: i64,
        shape: Shape,
        tint_name: String,
    ) {
        self.status.set_message("Painting folder contents…");
        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let paths = gio::spawn_blocking(move || collect_paths_recursively(&folder_path))
                .await
                .unwrap_or_default();
            let mut ok_count = 0i32;
            {
                let mut meta = controller.metadata.borrow_mut();
                for path in &paths {
                    if meta.set_file_mark(path, tint_id, shape).is_ok() {
                        ok_count += 1;
                    }
                }
            }
            let summary = format!(
                "Marked {} item{} {} {}",
                ok_count,
                if ok_count == 1 { "" } else { "s" },
                tint_name,
                shape.display_name()
            );
            let source = folder_path_str_from_slot(&controller, slot);
            let item_pairs: Vec<(PathBuf, Option<PathBuf>)> =
                paths.iter().map(|p| (p.clone(), None)).collect();
            controller
                .metadata
                .borrow()
                .log_activity_with_items(
                    "paint_mark",
                    ok_count,
                    &summary,
                    &source,
                    None,
                    &[],
                    &item_pairs,
                )
                .ok();
            controller.load_current_view(slot);
            controller.status.set_message(&summary);
        });
    }

    fn queue_plan(self: &Rc<Self>, mut plan: crate::action_plan::ActionPlan) {
        if let Some(path) = plan.cloud_probe_path() {
            if let Some((name, kind)) = self.cloud_name_for_path(path) {
                plan = plan.with_cloud_note(format!(
                    "☁ Cloud drive: {name} ({kind}) — may be slower or sync remotely"
                ));
            }
        }
        self.action_queue.borrow_mut().push(plan);
        self.refresh_plan_queue_panel();
        let n = self.action_queue.borrow().len();
        self.status.set_message(&format!(
            "{n} action{} queued — execute or clear in the plan queue panel.",
            if n == 1 { "" } else { "s" }
        ));
    }

    fn should_queue_actions(&self) -> bool {
        should_queue_actions_state(self.plan_mode_active.get(), self.executing_plan_queue.get())
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
        self.executing_plan_queue.set(true);
        for plan in queue {
            let tray_completion = plan
                .tray_completion
                .clone()
                .map(|completion| TrayCompletion {
                    action: completion.action,
                    clear_successful_paths: completion.clear_successful_paths,
                });
            match plan.kind {
                FileOpKind::Trash { paths } => {
                    if let Some(completion) = tray_completion {
                        self.move_paths_to_trash_with_completion(paths, Some(completion));
                    } else {
                        self.move_paths_to_trash(paths);
                    }
                }
                FileOpKind::CopyMove {
                    sources,
                    destination,
                    is_copy,
                } => {
                    self.start_copy_move_with_conflict_check(
                        sources,
                        destination,
                        is_copy,
                        plan.summary,
                        None,
                    );
                }
                FileOpKind::Rename(spec) => {
                    self.rename_path(spec.path, spec.new_name);
                }
                FileOpKind::BulkRename { renames } => {
                    let renames: Vec<(PathBuf, String)> = renames
                        .into_iter()
                        .map(|spec| (spec.path, spec.new_name))
                        .collect();
                    if !renames.is_empty() {
                        self.apply_bulk_rename(renames);
                    }
                }
                FileOpKind::Duplicate { paths } => {
                    self.do_duplicate_files(paths);
                }
                FileOpKind::PermanentDelete { paths } => {
                    self.delete_items_permanently(paths);
                }
                FileOpKind::NewFolder { parent, name } => {
                    self.exec_create_folder(parent, name);
                }
                FileOpKind::NewFile { parent, name } => {
                    self.exec_create_text_document(self.active_slot(), parent, name);
                }
                FileOpKind::SendToProject {
                    sources,
                    destination,
                    is_copy,
                } => {
                    if let Some(completion) = tray_completion {
                        let kind = if is_copy {
                            ProjectTransferKind::Copy
                        } else {
                            ProjectTransferKind::Move
                        };
                        let op_id = self.ops_panel.add_op(&plan.summary, None);
                        self.run_project_transfer(
                            sources,
                            0,
                            destination,
                            kind,
                            op_id,
                            Rc::new(RefCell::new(BatchResult::default())),
                            Some(completion),
                        );
                    } else {
                        let items = plan_copy_move_items(&sources, &destination);
                        if !items.is_empty() {
                            self.start_copy_move_op(items, is_copy, &plan.summary, None, None);
                        }
                    }
                }
                FileOpKind::PaintMark {
                    paths,
                    tint_id,
                    tint_name,
                    shape,
                    recursive,
                } => {
                    if recursive {
                        let count = paths.len();
                        for src in paths {
                            self.do_paint_folder_recursive(
                                self.active_slot(),
                                src,
                                tint_id,
                                shape,
                                tint_name.clone(),
                            );
                        }
                        if let Some(completion) = tray_completion {
                            self.record_tray_receipt(&completion.action, count, 0);
                        }
                    } else {
                        let count = paths.len();
                        self.apply_mark_to_paths_direct(paths, tint_id, shape, &tint_name);
                        if let Some(completion) = tray_completion {
                            self.record_tray_receipt(&completion.action, count, 0);
                        }
                    }
                }
                FileOpKind::ResetMark { paths, recursive } => {
                    if recursive {
                        let count = paths.len();
                        self.reset_mark_recursive(paths);
                        if let Some(completion) = tray_completion {
                            self.record_tray_receipt(&completion.action, count, 0);
                        }
                    } else {
                        let count = paths.len();
                        self.reset_mark_for_paths_direct(paths);
                        if let Some(completion) = tray_completion {
                            self.record_tray_receipt(&completion.action, count, 0);
                        }
                    }
                }
                FileOpKind::ApplyTag { paths, tag_name } => {
                    let count = paths.len();
                    self.apply_tag_to_paths(paths, tag_name);
                    if let Some(completion) = tray_completion {
                        self.record_tray_receipt(&completion.action, count, 0);
                    }
                }
                FileOpKind::RemoveTags { paths, tag_ids, .. } => {
                    self.remove_tags_from_paths(paths, tag_ids);
                }
                FileOpKind::CopyPaths { paths } => {
                    self.copy_paths_to_clipboard(paths.clone());
                    if let Some(completion) = tray_completion {
                        self.record_tray_receipt(&completion.action, paths.len(), 0);
                    }
                }
                FileOpKind::RestoreTrash { items } => {
                    let items = items
                        .into_iter()
                        .map(|item| FileItem {
                            name: item.display_name,
                            path: item.trash_path,
                            kind: FileKind::Unknown,
                            is_dir: false,
                            is_openable: true,
                            detail: None,
                            size_bytes: None,
                            modified_unix: None,
                            tags: Vec::new(),
                            original_path: item.original_path,
                            mark_tint_id: 0,
                            mark_tint_color: None,
                            mark_shape: Shape::DEFAULT,
                        })
                        .collect();
                    self.restore_items_from_trash(items);
                }
                FileOpKind::EmptyTrash => self.do_empty_trash(),
            }
        }
        self.executing_plan_queue.set(false);
    }

    fn apply_mark_to_paths_direct(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        tint_id: i64,
        shape: Shape,
        tint_name: &str,
    ) {
        let count = paths.len() as i32;
        {
            let mut meta = self.metadata.borrow_mut();
            for path in &paths {
                let _ = meta.set_file_mark(path, tint_id, shape);
            }
            let summary = format!(
                "Marked {} item{} {} {}",
                count,
                if count == 1 { "" } else { "s" },
                tint_name,
                shape.display_name(),
            );
            let _ = meta.log_activity(
                "paint_mark",
                count,
                &summary,
                paths
                    .first()
                    .map(|p| p.to_string_lossy())
                    .unwrap_or_default()
                    .as_ref(),
                None,
                &[],
            );
        }
        self.reload_active_tab();
    }

    fn reset_mark_for_paths_direct(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let count = paths.len() as i32;
        {
            let mut meta = self.metadata.borrow_mut();
            for path in &paths {
                let _ = meta.clear_file_mark(path);
            }
            let summary = format!(
                "Reset {} item{} to Beige Square",
                count,
                if count == 1 { "" } else { "s" },
            );
            let _ = meta.log_activity(
                "erase_mark",
                count,
                &summary,
                paths
                    .first()
                    .map(|p| p.to_string_lossy())
                    .unwrap_or_default()
                    .as_ref(),
                None,
                &[],
            );
        }
        self.reload_active_tab();
    }

    fn reset_mark_recursive(self: &Rc<Self>, folder_paths: Vec<PathBuf>) {
        self.status.set_message("Resetting marks…");
        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let all_paths = gio::spawn_blocking(move || {
                let mut acc = Vec::new();
                for root in &folder_paths {
                    acc.extend(collect_paths_recursively(root));
                }
                acc
            })
            .await
            .unwrap_or_default();
            controller.reset_mark_for_paths_direct(all_paths);
        });
    }

    fn apply_folder_view_state(self: &Rc<Self>, slot: PaneSlot, path: &PathBuf) {
        let path_str = path.to_string_lossy();
        let fvs = self
            .metadata
            .borrow()
            .get_folder_view_state(&path_str)
            .unwrap_or_else(|| {
                let g = crate::view_state::ViewState::load();
                FolderViewState {
                    view_mode: g.view_mode,
                    show_hidden: g.show_hidden,
                    show_shape_badges: g.show_shape_badges,
                    sort_field: g.sort_field,
                    sort_direction: g.sort_direction,
                }
            });

        let vm = match fvs.view_mode.as_str() {
            "list" => ViewMode::List,
            _ => ViewMode::Icons,
        };
        let sf = match fvs.sort_field.as_str() {
            "modified" => SortField::Modified,
            "size" => SortField::Size,
            "kind" => SortField::Kind,
            _ => SortField::Name,
        };
        let sd = match fvs.sort_direction.as_str() {
            "descending" => SortDirection::Descending,
            _ => SortDirection::Ascending,
        };

        self.view_mode_cell(slot).set(vm);
        self.show_hidden_cell(slot).set(fvs.show_hidden);
        self.show_shape_badges_cell(slot).set(fvs.show_shape_badges);
        self.sort_field_cell(slot).set(sf);
        self.sort_direction_cell(slot).set(sd);

        let pane = self.pane_widgets(slot);
        pane.file_grid.set_view_mode(vm);
        let vm_icon = match vm {
            ViewMode::Icons => "view-grid-symbolic",
            ViewMode::List => "view-list-compact-symbolic",
        };
        pane.view_mode_icon.set_icon_name(Some(vm_icon));
        self.sync_show_hidden_button_state(slot);
        self.sync_show_shape_badges_button_state(slot);

        // When navigating while paint mode is active, keep badges in sync with paint-mode tracking
        if self.paint_mode_active.get() {
            if fvs.show_shape_badges {
                // New folder already has badges on; clear auto-show tracking
                self.badges_hidden_by_paint_cell(slot).set(false);
            } else {
                // New folder has badges off; auto-show for paint mode
                self.badges_hidden_by_paint_cell(slot).set(true);
                self.show_shape_badges_cell(slot).set(true);
                self.sync_show_shape_badges_button_state(slot);
            }
        }
        let sort_icon = match sd {
            SortDirection::Descending => "view-sort-descending-symbolic",
            _ => "view-sort-ascending-symbolic",
        };
        pane.sort_icon.set_icon_name(Some(sort_icon));
    }

    fn save_folder_view_state_for(&self, slot: PaneSlot) {
        if !matches!(self.current_view_for(slot), PaneView::Directory(_)) {
            return;
        }
        let path = self.current_dir_for(slot);
        let state = FolderViewState {
            view_mode: match self.view_mode_cell(slot).get() {
                ViewMode::Icons => "icons",
                ViewMode::List => "list",
            }
            .to_string(),
            show_hidden: self.show_hidden_cell(slot).get(),
            show_shape_badges: self.show_shape_badges_cell(slot).get(),
            sort_field: match self.sort_field_cell(slot).get() {
                SortField::Modified => "modified",
                SortField::Size => "size",
                SortField::Kind => "kind",
                _ => "name",
            }
            .to_string(),
            sort_direction: match self.sort_direction_cell(slot).get() {
                SortDirection::Descending => "descending",
                _ => "ascending",
            }
            .to_string(),
        };
        self.metadata
            .borrow()
            .set_folder_view_state(&path.to_string_lossy(), &state);
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

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_path);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_path);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            let cloud_ctx = self
                .cloud_name_for_path(&path)
                .map(|(name, kind)| format!("☁ {name} ({kind})"));
            self.status.set_cloud_context(cloud_ctx.as_deref());
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

        let path_str = path.to_string_lossy();
        let directory = if is_gio_uri(&path_str) {
            gio::File::for_uri(path_str.as_ref())
        } else {
            gio::File::for_path(&path)
        };
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
        if matches!(self.current_view_for(slot), PaneView::Directory(_))
            && !is_gio_uri(&path.to_string_lossy())
        {
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
        let dir_error = if is_gio_uri(&path.to_string_lossy()) {
            GVFS_REMOTE_DIAGNOSTIC
        } else {
            "Unable to read this folder."
        };
        self.pane_widgets(slot)
            .file_grid
            .set_empty_message(match self.current_view_for(slot) {
                PaneView::Directory(_) => dir_error,
                PaneView::Tag(_) => "Unable to read tagged files.",
                PaneView::Triage { .. } => "Unable to read this folder.",
                PaneView::SystemDrives => "Unable to read mounted volumes.",
                PaneView::Recent => "Unable to read recent folders.",
                PaneView::Trash => "Unable to read Trash.",
                PaneView::Search(_) => "Search failed.",
                PaneView::BulkNaming { .. } => "Unable to load files for Bulk Naming.",
                PaneView::ActivityLog => "Unable to load activity log.",
                PaneView::ProjectLanding(_)
                | PaneView::CloudLanding(_)
                | PaneView::ProjectManager
                | PaneView::TagManager
                | PaneView::SpaceViewer { .. }
                | PaneView::MediaConvert { .. }
                | PaneView::Watercolor(_) => "",
            });

        let display_path = match self.current_view_for(slot) {
            PaneView::Directory(_) => self.display_path(path),
            _ => self.display_label_for(slot),
        };
        self.pane_widgets(slot).path_label.set_label(&display_path);
        if slot == self.active_slot() {
            self.status.set_cloud_context(None);
            // For cloud paths, provide a more specific error message
            if let Some((name, _)) = self.cloud_name_for_path(path) {
                let cloud_msg =
                    format!("☁ '{name}' is not accessible — check that the drive is mounted");
                self.pane_widgets(slot).file_grid.set_empty_message(&format!(
                    "☁ '{name}' is not accessible.\nCheck that the cloud drive is mounted and try again."
                ));
                self.status.set_message(&cloud_msg);
                self.status.set_counts(0, 0);
                self.status.set_path(&display_path);
                self.toolbar.set_breadcrumb_path(&display_path);
                self.toolbar.show_breadcrumb_mode();
                self.update_navigation_state();
                self.update_action_state();
                self.preview.set_action_state(false, false, false);
                return;
            }
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
        self.save_dir_to_history_if_in_directory(slot);
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
        let diagnostics = drive_listing_status_message(&listing);
        let items = listing.items;
        self.items_cell(slot).replace(items.clone());

        if items.is_empty() {
            self.pane_widgets(slot)
                .file_grid
                .set_empty_message(DRIVES_GVFS_DIAGNOSTIC);
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
            if items.is_empty() && self.preview_visible.get() {
                self.preview
                    .show_error("System Drives", DRIVES_GVFS_DIAGNOSTIC);
                self.preview.set_action_state(false, false, false);
            } else {
                self.show_empty_selection_preview(slot, &display_label, items.len());
            }
            self.status.set_counts(items.len(), 0);
            if let Some(message) = diagnostics {
                self.status.set_message(&message);
            } else if items.is_empty() {
                self.status
                    .set_message("No system drives found through GIO/GVfs.");
            }
            if !items.is_empty() {
                self.refresh_preview();
            }
        }
    }

    fn open_recent(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.save_dir_to_history_if_in_directory(slot);
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
        self.save_dir_to_history_if_in_directory(slot);
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
                                    is_openable: true,
                                    detail: None,
                                    kind,
                                    size_bytes: (info.size() >= 0).then_some(info.size() as u64),
                                    modified_unix: info
                                        .modification_date_time()
                                        .map(|dt| dt.to_unix()),
                                    tags: Vec::new(),
                                    mark_tint_id: 0,
                                    mark_tint_color: None,
                                    mark_shape: Shape::DEFAULT,
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

    fn finish_trash_load(
        self: &Rc<Self>,
        slot: PaneSlot,
        generation: u64,
        mut items: Vec<FileItem>,
    ) {
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
                .set_empty_message("Trash is empty.");
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
            .set_empty_message(TRASH_GVFS_DIAGNOSTIC);

        let display_label = self.display_label_for(slot);
        self.pane_widgets(slot).path_label.set_label(&display_label);
        if slot == self.active_slot() {
            self.preview.show_error(
                "Trash",
                &format!(
                    "{}\n\n{}",
                    friendly_error_detail(error),
                    TRASH_GVFS_DIAGNOSTIC
                ),
            );
            self.status.set_counts(0, 0);
            self.status
                .set_message("Trash is unavailable; GVfs may be missing.");
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
        if self.should_queue_actions() {
            let specs = items
                .into_iter()
                .map(|item| RestoreSpec {
                    trash_path: item.path,
                    original_path: item.original_path,
                    display_name: item.name,
                })
                .collect();
            self.queue_plan(FileOpPlan::for_restore_trash(specs));
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
            move || {
                if controller.should_queue_actions() {
                    controller.queue_plan(FileOpPlan::for_empty_trash(item_count));
                } else {
                    controller.do_empty_trash();
                }
            },
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

    fn run_path_box_search(self: &Rc<Self>) {
        let text = self.toolbar.path_entry.text().to_string();
        if text.is_empty() || looks_like_explicit_path(&text) {
            return;
        }
        let slot = self.active_slot();
        let scope = {
            let saved = self.path_box_scope_dir.borrow().clone();
            match saved {
                Some(dir) => dir,
                None => {
                    let dir = self.current_dir_for(slot);
                    *self.path_box_scope_dir.borrow_mut() = Some(dir.clone());
                    dir
                }
            }
        };
        let mut query = SearchQuery::new(scope);
        query.name = text;
        query.recursive = false;
        self.open_search(slot, query);
        let toolbar = self.toolbar.clone();
        let entry = self.toolbar.path_entry.clone();
        glib::idle_add_local_once(move || {
            toolbar.show_entry_mode();
            entry.grab_focus();
        });
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
        let scope = self.tool_scope_dir_for(slot);
        let query = SearchQuery::new(scope);
        self.open_search(slot, query);
    }

    fn open_search(self: &Rc<Self>, slot: PaneSlot, query: SearchQuery) {
        self.save_dir_to_history_if_in_directory(slot);
        self.current_dir_cell(slot).replace(query.scope_dir.clone());
        self.current_view_cell(slot)
            .replace(PaneView::Search(query.clone()));
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        // Build tag and mark rows first so sync_from_query can reflect chip state correctly
        let tags = self.tags.borrow().clone();
        self.pane_widgets(slot).search_panel.set_tags(&tags);
        self.wire_search_tag_buttons(slot);
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        self.pane_widgets(slot).search_panel.set_tints(&tints);
        self.wire_search_mark_buttons(slot);
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
        query.tint_id = None;
        query.shape = None;
        query.default_mark_only = false;
        for (chip, btn) in panel.mark_buttons() {
            if btn.has_css_class("active") {
                use crate::ui::search_panel::MarkChip;
                match chip {
                    MarkChip::AnyMark => {}
                    MarkChip::DefaultMark => query.default_mark_only = true,
                    MarkChip::Tint(id) => query.tint_id = Some(id),
                    MarkChip::Shape(s) => query.shape = Some(s),
                }
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

        if slot == self.active_slot() {
            self.toolbar.set_breadcrumb_path(&display_label);
            self.toolbar.show_breadcrumb_mode();
            self.status.set_path(&display_label);
            self.status.clear_message();
            self.status.set_counts(0, 0);
            let cloud_ctx = self
                .cloud_name_for_path(&query.scope_dir)
                .map(|(name, kind)| format!("☁ {name} ({kind})"));
            self.status.set_cloud_context(cloud_ctx.as_deref());
            if let Some((name, _)) = self.cloud_name_for_path(&query.scope_dir) {
                self.status.set_message(&format!(
                    "☁ Searching in '{name}' — cloud searches may be slow"
                ));
            }
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
            let query_name_lower = query_clone.name.to_lowercase();
            let raw = gio::spawn_blocking(move || {
                let mut results = Vec::new();
                search_directory_blocking(
                    &query_clone.scope_dir,
                    &query_clone,
                    &query_name_lower,
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

            // Enrich with tags+marks, then apply optional filters
            let enriched = controller.enrich_items_with_tags(raw);
            let items: Vec<FileItem> = if let Some(tag_id) = query.tag_id {
                enriched
                    .into_iter()
                    .filter(|item| item.tags.iter().any(|t| t.id == tag_id))
                    .collect()
            } else if query.default_mark_only {
                // created_at == 0 is the default mark sentinel (no explicit DB row)
                enriched
                    .into_iter()
                    .filter(|item| item.mark_tint_id == 0)
                    .collect()
            } else if let Some(tint_id) = query.tint_id {
                enriched
                    .into_iter()
                    .filter(|item| item.mark_tint_id == tint_id)
                    .collect()
            } else if let Some(shape) = query.shape {
                enriched
                    .into_iter()
                    .filter(|item| item.mark_shape == shape)
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
            // Re-apply cloud context after results arrive (load_search_view cleared message)
            if let PaneView::Search(ref q) = self.current_view_for(slot) {
                let cloud_ctx = self
                    .cloud_name_for_path(&q.scope_dir)
                    .map(|(name, kind)| format!("☁ {name} ({kind})"));
                self.status.set_cloud_context(cloud_ctx.as_deref());
            }
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

    fn wire_search_mark_buttons(self: &Rc<Self>, slot: PaneSlot) {
        let panel = self.pane_widgets(slot).search_panel.clone();
        for (chip, btn) in panel.mark_buttons() {
            let controller = Rc::clone(self);
            let all_mark = panel.mark_buttons();
            btn.connect_clicked(move |clicked| {
                for (_, b) in &all_mark {
                    b.remove_css_class("active");
                }
                clicked.add_css_class("active");
                if matches!(controller.current_view_for(slot), PaneView::Search(_)) {
                    controller.rerun_search(slot);
                }
                let _ = &chip;
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
                                is_openable: true,
                                detail: None,
                                size_bytes: (info.size() >= 0).then_some(info.size() as u64),
                                modified_unix: info
                                    .modification_date_time()
                                    .and_then(|value| Some(value.to_unix())),
                                tags: Vec::new(),
                                mark_tint_id: 0,
                                mark_tint_color: None,
                                mark_shape: Shape::DEFAULT,
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

    fn enrich_items_with_tags(&self, items: Vec<FileItem>) -> Vec<FileItem> {
        self.enrich_items(items)
    }

    fn enrich_items(&self, mut items: Vec<FileItem>) -> Vec<FileItem> {
        let paths: Vec<_> = items.iter().map(|item| item.path.clone()).collect();
        let (tags_by_path, marks_by_path) = {
            let meta = self.metadata.borrow();
            let tags = meta.tags_for_paths(&paths).unwrap_or_default();
            let marks = meta.marks_for_paths(&paths).unwrap_or_default();
            (tags, marks)
        };
        let tint_colors: HashMap<i64, String> = self
            .metadata
            .borrow()
            .list_tints()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tint| tint.color.map(|color| (tint.id, color)))
            .collect();
        for item in &mut items {
            item.tags = tags_by_path.get(&item.path).cloned().unwrap_or_default();
            if let Some(mark) = marks_by_path.get(&item.path) {
                item.mark_tint_id = mark.tint_id;
                item.mark_tint_color = tint_colors.get(&mark.tint_id).cloned();
                item.mark_shape = mark.shape;
            }
        }
        items
    }

    fn preview_identity_for_item(
        &self,
        item: &FileItem,
    ) -> (String, Shape, Option<String>, Vec<TagRecord>) {
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        let (tint_name, tint_color) =
            tint_name_and_color(&tints, item.mark_tint_id, item.mark_tint_color.clone());
        (tint_name, item.mark_shape, tint_color, item.tags.clone())
    }

    fn preview_identity_for_path(
        &self,
        path: &Path,
    ) -> Option<(String, Shape, Option<String>, Vec<TagRecord>)> {
        let path_buf = path.to_path_buf();
        let (mark, tags, tints) = {
            let meta = self.metadata.borrow();
            let mark = meta.mark_for_path(path).ok()?;
            let tags = meta.tags_for_selection(&[path_buf]).unwrap_or_default();
            let tints = meta.list_tints().unwrap_or_default();
            (mark, tags, tints)
        };
        let (tint_name, tint_color) = tint_name_and_color(&tints, mark.tint_id, None);
        Some((tint_name, mark.shape, tint_color, tags))
    }

    fn init_tint_css(self: &Rc<Self>) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        gtk::style_context_add_provider_for_display(
            &display,
            &self.tint_css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    }

    fn apply_tint_css(self: &Rc<Self>) {
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        let css = generate_tint_css(&tints);
        self.tint_css_provider.load_from_string(&css);
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
            PaneView::BulkNaming { root } => {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Bulk Naming".to_string());
                self.preview.show_folder(
                    &format!("Bulk Naming: {name}"),
                    display_label,
                    None,
                    Some(item_count),
                    "Bulk Naming",
                );
                self.preview.set_action_state(false, false, false);
            }
            PaneView::ActivityLog => {
                self.preview
                    .show_folder("Activity Log", display_label, None, None, "File History");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::ProjectLanding(_) => {
                self.preview
                    .show_folder("Palette", display_label, None, None, "Palette");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::CloudLanding(_) => {
                self.preview
                    .show_folder("Cloud Drive", display_label, None, None, "Cloud");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::ProjectManager => {
                self.preview
                    .show_folder("Palettes", display_label, None, None, "Palette Manager");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::TagManager => {
                self.preview
                    .show_folder("Tints & Tags", display_label, None, None, "Tints & Tags");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::SpaceViewer { root } => {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Space Viewer".to_string());
                self.preview
                    .show_folder(&name, display_label, None, None, "Space Viewer");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::MediaConvert { .. } => {
                self.preview
                    .show_folder("Convert", display_label, None, None, "Media Conversion");
                self.preview.set_action_state(false, false, false);
            }
            PaneView::Watercolor(view) => {
                self.preview.show_folder(
                    watercolor_tab_title(&view),
                    display_label,
                    None,
                    None,
                    "Watercolor",
                );
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
            if item.is_openable {
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
                // File / palette group
                append_menu_sep(&menu_box);
                append_menu_button(
                    &menu_box,
                    "Pin as Palette",
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
                let note = Label::new(Some(
                    item.detail
                        .as_deref()
                        .unwrap_or("This item is not mounted or directly openable."),
                ));
                note.set_margin_start(8);
                note.set_margin_end(8);
                note.set_halign(gtk::Align::Start);
                note.add_css_class("context-note");
                menu_box.append(&note);
            }
        } else {
            // Normal directory view — show cloud badge header when item is on a cloud drive
            if let Some((cloud_name, cloud_kind)) = self.cloud_name_for_path(&item.path) {
                let note = Label::new(Some(&format!("☁ {cloud_name}  ({cloud_kind})")));
                note.add_css_class("context-cloud-note");
                note.set_halign(gtk::Align::Start);
                note.set_margin_start(8);
                note.set_margin_end(8);
                menu_box.append(&note);
                append_menu_sep(&menu_box);
            }
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
        self.pane_widgets(slot).file_grid.select_only_index(index);
        self.set_keyboard_focus(slot, index, true);
    }

    fn icon_item_at_scroll_point(
        &self,
        slot: PaneSlot,
        scroll: &gtk::ScrolledWindow,
        x: f64,
        y: f64,
    ) -> Option<i32> {
        let grid = &self.pane_widgets(slot).file_grid;
        let scroll_point = gtk::graphene::Point::new(x as f32, y as f32);
        let point = scroll.compute_point(&grid.flow, &scroll_point)?;
        grid.flow
            .child_at_pos(point.x() as i32, point.y() as i32)
            .map(|child| child.index())
    }

    fn list_item_at_scroll_point(
        &self,
        slot: PaneSlot,
        scroll: &gtk::ScrolledWindow,
        x: f64,
        y: f64,
    ) -> Option<i32> {
        let grid = &self.pane_widgets(slot).file_grid;
        let scroll_point = gtk::graphene::Point::new(x as f32, y as f32);
        let point = scroll.compute_point(&grid.list_box, &scroll_point)?;
        grid.list_box
            .row_at_y(point.y() as i32)
            .map(|row| row.index())
    }

    fn show_current_folder_menu(
        self: &Rc<Self>,
        slot: PaneSlot,
        anchor: gtk::Widget,
        x: f64,
        y: f64,
    ) {
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
        popover.set_parent(&anchor);

        let menu_box = GtkBox::new(Orientation::Vertical, 2);
        menu_box.set_margin_top(6);
        menu_box.set_margin_bottom(6);
        menu_box.set_margin_start(6);
        menu_box.set_margin_end(6);
        menu_box.set_size_request(190, -1);

        // Cloud badge header when right-clicking inside a cloud folder
        let cur_dir = self.current_dir_for(slot);
        if let Some((cloud_name, cloud_kind)) = self.cloud_name_for_path(&cur_dir) {
            let note = Label::new(Some(&format!("☁ {cloud_name}  ({cloud_kind})")));
            note.add_css_class("context-cloud-note");
            note.set_halign(gtk::Align::Start);
            note.set_margin_start(8);
            note.set_margin_end(8);
            menu_box.append(&note);
            append_menu_sep(&menu_box);
        }
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
                    "compress",
                    "separator",
                    "rename",
                    "duplicate",
                    "copy_path",
                    "terminal_here",
                    "separator",
                    "pin_place",
                    "separator",
                    "move_to_trash",
                    "delete_permanently",
                ]
            } else {
                vec![
                    "open",
                    "open_with",
                    "convert",
                    "extract",
                    "compress",
                    "separator",
                    "rename",
                    "duplicate",
                    "copy_path",
                    "terminal_here",
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
                "convert" if !item.is_dir => {
                    let selected = self.selected_items_for(slot);
                    let has_media = selected.iter().any(|i| {
                        matches!(i.kind, FileKind::Image | FileKind::Video | FileKind::Audio)
                    });
                    if has_media {
                        append_menu_button(
                            menu_box,
                            "Convert\u{2026}",
                            Some("media-playback-start-symbolic"),
                            false,
                            {
                                let controller = Rc::clone(self);
                                move || {
                                    controller.open_media_convert_with_items(slot, selected.clone())
                                }
                            },
                        );
                    }
                }
                "extract" if !item.is_dir && is_supported_archive_path(&item.path) => {
                    append_menu_button(
                        menu_box,
                        "Extract Archive",
                        Some("archive-extract-symbolic"),
                        false,
                        {
                            let controller = Rc::clone(self);
                            let item = item.clone();
                            move || controller.extract_archive_from_menu(slot, item.path.clone())
                        },
                    );
                }
                "compress" => append_menu_button(
                    menu_box,
                    "Compress to ZIP",
                    Some("package-x-generic-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || controller.compress_selection_from_menu(slot, item.path.clone())
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
                            "Bulk Naming\u{2026}",
                            Some("document-edit-symbolic"),
                            false,
                            {
                                let controller = Rc::clone(self);
                                move || controller.open_bulk_naming_with_items(selected.clone())
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
                "terminal_here" => {
                    if is_gio_uri(&item.path.to_string_lossy()) {
                        let note = Label::new(Some("Terminal unavailable for remote URI paths"));
                        note.add_css_class("context-note");
                        note.set_margin_start(8);
                        note.set_margin_end(8);
                        note.set_halign(gtk::Align::Start);
                        menu_box.append(&note);
                    } else {
                        append_menu_button(
                            menu_box,
                            "Terminal Here",
                            Some("utilities-terminal-symbolic"),
                            false,
                            {
                                let controller = Rc::clone(self);
                                let item = item.clone();
                                move || {
                                    controller
                                        .open_terminal_for_path(item.path.clone(), item.is_dir)
                                }
                            },
                        )
                    }
                }
                "pin_project" if item.is_dir => append_menu_button(
                    menu_box,
                    "Pin as Palette",
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
                    "Add to Places",
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
                    "Send to Palette",
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
                "mark_as_active" => {
                    let tint_name = self.active_paint_tint_name.borrow().clone();
                    let shape = self.active_paint_shape.get();
                    let label = format!("Mark as {} {}", tint_name, shape.display_name());
                    append_menu_button(
                        menu_box,
                        &label,
                        Some("preferences-color-symbolic"),
                        false,
                        {
                            let controller = Rc::clone(self);
                            let item = item.clone();
                            move || {
                                let tint_id = controller.active_paint_tint_id.get();
                                let shape = controller.active_paint_shape.get();
                                if controller
                                    .metadata
                                    .borrow_mut()
                                    .set_file_mark(&item.path, tint_id, shape)
                                    .is_ok()
                                {
                                    controller
                                        .update_item_mark_in_grid(slot, &item.path, tint_id, shape);
                                    controller.log_paint_mark(
                                        slot,
                                        &[item.path.clone()],
                                        tint_id,
                                        shape,
                                    );
                                }
                            }
                        },
                    );
                }
                "reset_mark" => append_menu_button(
                    menu_box,
                    "Reset Mark",
                    Some("edit-clear-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || {
                            controller.paint_eraser_item(slot, &item);
                        }
                    },
                ),
                "select_same_tint" => {
                    append_menu_button(menu_box, "Select Same Tint", None, false, {
                        let controller = Rc::clone(self);
                        let tint_id = item.mark_tint_id;
                        move || {
                            let items = controller.items_cell(slot).borrow();
                            let grid = &controller.pane_widgets(slot).file_grid;
                            grid.clear_selection();
                            for (i, it) in items.iter().enumerate() {
                                if it.mark_tint_id == tint_id {
                                    grid.select_range(i as i32, i as i32, false);
                                }
                            }
                        }
                    })
                }
                "select_same_shape" => {
                    append_menu_button(menu_box, "Select Same Shape", None, false, {
                        let controller = Rc::clone(self);
                        let shape = item.mark_shape;
                        move || {
                            let items = controller.items_cell(slot).borrow();
                            let grid = &controller.pane_widgets(slot).file_grid;
                            grid.clear_selection();
                            for (i, it) in items.iter().enumerate() {
                                if it.mark_shape == shape {
                                    grid.select_range(i as i32, i as i32, false);
                                }
                            }
                        }
                    })
                }
                "select_same_mark" => {
                    append_menu_button(menu_box, "Select Same Mark", None, false, {
                        let controller = Rc::clone(self);
                        let tint_id = item.mark_tint_id;
                        let shape = item.mark_shape;
                        move || {
                            let items = controller.items_cell(slot).borrow();
                            let grid = &controller.pane_widgets(slot).file_grid;
                            grid.clear_selection();
                            for (i, it) in items.iter().enumerate() {
                                if it.mark_tint_id == tint_id && it.mark_shape == shape {
                                    grid.select_range(i as i32, i as i32, false);
                                }
                            }
                        }
                    })
                }
                "add_same_mark_to_tray" => {
                    append_menu_button(menu_box, "Add Same Mark to Tray", None, false, {
                        let controller = Rc::clone(self);
                        let tint_id = item.mark_tint_id;
                        let shape = item.mark_shape;
                        move || {
                            let items = controller.items_cell(slot).borrow();
                            let paths: Vec<_> = items
                                .iter()
                                .filter(|it| it.mark_tint_id == tint_id && it.mark_shape == shape)
                                .map(|it| it.path.clone())
                                .collect();
                            drop(items);
                            if !paths.is_empty() {
                                controller.add_paths_to_holding_tray(paths);
                            }
                        }
                    })
                }
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
                    "Pin as Palette",
                    Some("starred-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.show_pin_project_dialog(controller.current_dir_for(slot))
                    },
                ),
                "pin_place" => append_menu_button(
                    menu_box,
                    "Add to Places",
                    Some("bookmark-new-symbolic"),
                    false,
                    {
                        let controller = Rc::clone(self);
                        move || controller.pin_place(controller.current_dir_for(slot))
                    },
                ),
                "terminal_here" => {
                    let cur_dir = self.current_dir_for(slot);
                    if is_gio_uri(&cur_dir.to_string_lossy()) {
                        let note = Label::new(Some("Terminal unavailable for remote URI paths"));
                        note.add_css_class("context-note");
                        note.set_margin_start(8);
                        note.set_margin_end(8);
                        note.set_halign(gtk::Align::Start);
                        menu_box.append(&note);
                    } else {
                        append_menu_button(
                            menu_box,
                            "Terminal Here",
                            Some("utilities-terminal-symbolic"),
                            false,
                            {
                                let controller = Rc::clone(self);
                                move || controller.open_current_folder_terminal()
                            },
                        )
                    }
                }
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
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        let tint_colors = HoldingTray::tint_color_map(&tints);
        let cloud_flags: Vec<bool> = items
            .iter()
            .map(|item| self.is_in_cloud_location(&item.path))
            .collect();
        self.holding_tray.set_items(
            &items,
            &selected,
            &tint_colors,
            &cloud_flags,
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
                "No Palettes Yet",
                "Create a palette and pin a folder first, then send tray items to it.",
            );
            return;
        }

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.append(&build_modal_prompt(
            "Choose the palette that should receive the staged tray items.",
        ));

        let project_box = GtkBox::new(Orientation::Vertical, 8);
        project_box.set_halign(Align::Fill);
        project_box.set_hexpand(true);
        content.append(&project_box);

        let mut first_project_button: Option<gtk::CheckButton> = None;
        let project_buttons = projects
            .iter()
            .map(|project| {
                let button = gtk::CheckButton::with_label(&project.name);
                button.set_halign(Align::Start);
                if let Some(first) = &first_project_button {
                    button.set_group(Some(first));
                } else {
                    button.set_active(true);
                    first_project_button = Some(button.clone());
                }
                project_box.append(&button);
                (project.id, project.name.clone(), button)
            })
            .collect::<Vec<_>>();

        let actions = build_modal_actions();
        let host = self.modal_host.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.modal_host.clone();
        let controller = Rc::clone(self);
        let plan_btn = build_modal_button("Preview", ButtonKind::Primary, move || {
            if let Some((project_id, project_name, _)) = project_buttons
                .iter()
                .find(|(_, _, button)| button.is_active())
            {
                let project_id = *project_id;
                let paths_for_plan = paths.clone();
                let project_name = project_name.clone();
                if controller.should_queue_actions() {
                    let destination_root = controller
                        .metadata
                        .borrow()
                        .list_project_destinations(project_id)
                        .ok()
                        .and_then(|mut dests| {
                            dests.retain(|d| !d.path.is_empty());
                            dests.into_iter().next()
                        })
                        .map(|pin| PathBuf::from(pin.path));
                    if let Some(destination_root) = destination_root {
                        let is_copy = action == TrayProjectAction::Copy;
                        controller.queue_plan(
                            FileOpPlan::for_send_to_project(
                                &paths_for_plan,
                                &project_name,
                                &destination_root,
                                is_copy,
                            )
                            .with_tray_completion(
                                action.title(),
                                action == TrayProjectAction::Move,
                            ),
                        );
                    } else {
                        controller.modal_host.show_error(
                            "No Pinned Folders",
                            "Pin a folder to this palette before sending files to it.",
                        );
                    }
                    host.hide();
                    return;
                }
                controller.send_paths_to_project(
                    paths_for_plan.clone(),
                    project_id,
                    action.transfer_kind(),
                    Some(TrayCompletion {
                        action: action.title().to_string(),
                        clear_successful_paths: action == TrayProjectAction::Move,
                    }),
                );
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
                if controller.should_queue_actions() {
                    controller.queue_plan(
                        FileOpPlan::for_apply_tag(&paths, &tag_name)
                            .with_tray_completion("Tag Holding Tray", false),
                    );
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
        if self.should_queue_actions() {
            self.queue_plan(
                FileOpPlan::for_trash(&paths).with_tray_completion("Move Tray to Trash", true),
            );
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
        if self.should_queue_actions() {
            self.queue_plan(
                FileOpPlan::for_copy_paths(&paths).with_tray_completion("Copy Tray Paths", false),
            );
            return;
        }
        let plan = copy_path_action_plan(&paths);
        let controller = Rc::clone(self);
        self.show_action_plan(plan, move || {
            controller.copy_paths_to_clipboard(paths.clone());
            controller.record_tray_receipt("Copy Tray Paths", paths.len(), 0);
        });
    }

    fn show_tray_apply_mark_preview(self: &Rc<Self>) {
        let paths = self.selected_holding_tray_paths();
        if paths.is_empty() {
            self.status.set_message("The Holding Tray is empty.");
            return;
        }
        let tint_id = self.active_paint_tint_id.get();
        let tint_name = self.active_paint_tint_name.borrow().clone();
        let shape = self.active_paint_shape.get();
        if self.should_queue_actions() {
            self.queue_plan(
                FileOpPlan::for_paint_mark(&paths, tint_id, &tint_name, shape, false)
                    .with_tray_completion("Apply Mark to Tray", false),
            );
            return;
        }
        let plan = apply_mark_action_plan(&paths, &tint_name, shape);
        let controller = Rc::clone(self);
        self.show_action_plan(plan, move || {
            controller.apply_mark_to_paths_direct(paths.clone(), tint_id, shape, &tint_name);
        });
    }

    fn show_tray_reset_mark_preview(self: &Rc<Self>) {
        let paths = self.selected_holding_tray_paths();
        if paths.is_empty() {
            self.status.set_message("The Holding Tray is empty.");
            return;
        }
        if self.should_queue_actions() {
            self.queue_plan(
                FileOpPlan::for_reset_mark(&paths, false).with_tray_completion("Reset Mark", false),
            );
            return;
        }
        let plan = reset_mark_action_plan(&paths);
        let controller = Rc::clone(self);
        self.show_action_plan(plan, move || {
            controller.reset_mark_for_paths_direct(paths.clone());
        });
    }

    fn show_add_to_tray_by_tint_popover(
        self: &Rc<Self>,
        anchor: &impl gtk::prelude::IsA<gtk::Widget>,
    ) {
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        if tints.is_empty() {
            self.status.set_message("No tints defined.");
            return;
        }
        let popover = gtk::Popover::new();
        popover.add_css_class("context-menu");
        popover.set_parent(anchor);
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for tint in tints {
            let btn = gtk::Button::with_label(&tint.name);
            btn.add_css_class("context-menu-button");
            let controller = Rc::clone(self);
            let popover_ref = popover.clone();
            btn.connect_clicked(move |_| {
                popover_ref.popdown();
                let slot = controller.active_slot();
                let items = controller.items_cell(slot).borrow().clone();
                let tint_id = tint.id;
                let paths: Vec<_> = items
                    .iter()
                    .filter(|i| i.mark_tint_id == tint_id)
                    .map(|i| i.path.clone())
                    .collect();
                if paths.is_empty() {
                    controller
                        .status
                        .set_message(&format!("No {} items in current folder.", tint.name));
                    return;
                }
                controller.add_paths_to_holding_tray(paths);
            });
            vbox.append(&btn);
        }
        popover.set_child(Some(&vbox));
        popover.popup();
    }

    fn show_add_to_tray_by_shape_popover(
        self: &Rc<Self>,
        anchor: &impl gtk::prelude::IsA<gtk::Widget>,
    ) {
        use crate::metadata::Shape;
        let shapes = [
            Shape::Circle,
            Shape::Square,
            Shape::Triangle,
            Shape::Pentagon,
            Shape::Hexagon,
            Shape::Octagon,
            Shape::Trapezoid,
        ];
        let popover = gtk::Popover::new();
        popover.add_css_class("context-menu");
        popover.set_parent(anchor);
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for shape in shapes {
            let label = format!("{} {}", shape.glyph(), shape.display_name());
            let btn = gtk::Button::with_label(&label);
            btn.add_css_class("context-menu-button");
            let controller = Rc::clone(self);
            let popover_ref = popover.clone();
            btn.connect_clicked(move |_| {
                popover_ref.popdown();
                let slot = controller.active_slot();
                let items = controller.items_cell(slot).borrow().clone();
                let paths: Vec<_> = items
                    .iter()
                    .filter(|i| i.mark_shape == shape)
                    .map(|i| i.path.clone())
                    .collect();
                if paths.is_empty() {
                    controller.status.set_message(&format!(
                        "No {} items in current folder.",
                        shape.display_name()
                    ));
                    return;
                }
                controller.add_paths_to_holding_tray(paths);
            });
            vbox.append(&btn);
        }
        popover.set_child(Some(&vbox));
        popover.popup();
    }

    fn show_action_plan<F>(self: &Rc<Self>, plan: ConfirmationPreview, on_accept: F)
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
                        if let Some((tint_name, shape, tint_color, tags)) =
                            controller.preview_identity_for_path(&path)
                        {
                            controller.preview.set_identity(
                                &tint_name,
                                shape,
                                tint_color.as_deref(),
                                &tags,
                            );
                        }
                        controller
                            .preview
                            .set_action_state(false, true, path.parent().is_some());
                        controller.load_terroir_context_for_preview(generation, path.clone());
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
            let (tint_name, shape, tint_color, tags) = self.preview_identity_for_item(&item);
            self.preview
                .set_identity(&tint_name, shape, tint_color.as_deref(), &tags);
            self.preview.set_action_state(true, true, true);
            self.load_terroir_context_for_preview(generation, item.path.clone());
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
            let (tint_name, shape, tint_color, tags) = self.preview_identity_for_item(&item);
            self.preview
                .set_identity(&tint_name, shape, tint_color.as_deref(), &tags);
            self.preview.set_mime_type(Some(mime));
            self.preview.set_action_state(true, true, true);
            self.load_terroir_context_for_preview(generation, item.path.clone());
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
        let (tint_name, shape, tint_color, tags) = self.preview_identity_for_item(&item);
        self.preview
            .set_identity(&tint_name, shape, tint_color.as_deref(), &tags);
        self.preview.set_mime_type(Some(mime));
        self.preview.set_action_state(true, true, true);
        self.load_terroir_context_for_preview(generation, item.path.clone());
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
                        let (tint_name, shape, tint_color, tags) =
                            controller.preview_identity_for_item(&item);
                        controller.preview.set_identity(
                            &tint_name,
                            shape,
                            tint_color.as_deref(),
                            &tags,
                        );
                        controller.preview.set_mime_type(Some(&mime));
                        controller.preview.set_action_state(true, true, true);
                        controller.load_terroir_context_for_preview(generation, item.path.clone());
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
                        let (tint_name, shape, tint_color, tags) =
                            controller.preview_identity_for_item(&item);
                        controller.preview.set_identity(
                            &tint_name,
                            shape,
                            tint_color.as_deref(),
                            &tags,
                        );
                        controller.preview.set_mime_type(Some(&mime));
                        controller.preview.set_action_state(true, true, true);
                        controller.load_terroir_context_for_preview(generation, item.path.clone());
                    }
                }
            },
        );
    }

    fn load_terroir_context_for_preview(self: &Rc<Self>, generation: u64, path: PathBuf) {
        if !self.config.enable_terroir_context {
            self.preview.clear_watercolor_context();
            return;
        }

        let (sender, receiver) = mpsc::channel::<
            Result<crate::terroir_client::TerroirContext, terroir_client::TerroirError>,
        >();
        std::thread::spawn(move || {
            let result =
                terroir_client::status().and_then(|_| terroir_client::context_for_path(&path));
            let _ = sender.send(result);
        });

        let controller = Rc::clone(self);
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
            if !controller.is_current_preview(generation) {
                return glib::ControlFlow::Break;
            }

            match receiver.try_recv() {
                Ok(Ok(context)) => {
                    controller.preview.set_watercolor_context(&context);
                    glib::ControlFlow::Break
                }
                Ok(Err(_)) => {
                    controller.preview.set_watercolor_unavailable();
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
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
        if !item.is_openable {
            let detail = item
                .detail
                .as_deref()
                .unwrap_or("This item is not directly openable.");
            self.status.set_message(&format!("{}: {detail}", item.name));
            return;
        }

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
        let path_str = path.to_string_lossy();
        let file = if is_gio_uri(&path_str) {
            gio::File::for_uri(path_str.as_ref())
        } else {
            gio::File::for_path(path)
        };
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
            _ => self.open_bulk_naming_with_items(items),
        }
    }

    fn apply_bulk_rename(self: &Rc<Self>, renames: Vec<(PathBuf, String)>) {
        if renames.is_empty() {
            return;
        }
        if self.should_queue_actions() {
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

        if self.should_queue_actions() {
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
        if self.should_queue_actions() {
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

    fn compress_selection_from_menu(self: &Rc<Self>, slot: PaneSlot, fallback_path: PathBuf) {
        if is_gio_uri(&fallback_path.to_string_lossy()) {
            self.show_error_dialog(
                "Compress Unavailable",
                "Archive creation is only available for local filesystem paths.",
            );
            return;
        }

        let mut paths = self.selected_paths_for(slot);
        if paths.is_empty() {
            paths.push(fallback_path);
        }
        paths.retain(|path| !is_gio_uri(&path.to_string_lossy()));
        if paths.is_empty() {
            self.status
                .set_message("Select one or more local items to compress.");
            return;
        }

        let Some(parent) =
            common_parent(&paths).or_else(|| paths[0].parent().map(Path::to_path_buf))
        else {
            self.show_error_dialog(
                "Compress Failed",
                "Could not resolve an archive destination.",
            );
            return;
        };
        let dest = next_available_path(&parent.join(suggested_archive_name(&paths)));
        let label = format!(
            "Compress: {} item{}",
            paths.len(),
            if paths.len() == 1 { "" } else { "s" }
        );
        let op_id = self.ops_panel.add_op(&label, None);
        self.ops_panel
            .update_progress(op_id, 0.2, "Creating ZIP archive…");
        self.status.set_message("Creating ZIP archive…");

        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || compress_paths_to_zip(paths, parent, dest))
                .await
                .unwrap_or_else(|_| ArchiveOpResult::failure("Archive worker stopped."));
            controller.finish_archive_op(op_id, result, "archive_compress");
        });
    }

    fn extract_archive_from_menu(self: &Rc<Self>, _slot: PaneSlot, archive: PathBuf) {
        if is_gio_uri(&archive.to_string_lossy()) {
            self.show_error_dialog(
                "Extract Unavailable",
                "Archive extraction is only available for local filesystem paths.",
            );
            return;
        }
        if !is_supported_archive_path(&archive) {
            self.show_error_dialog(
                "Extract Unavailable",
                "Lattice can extract ZIP, TAR, compressed TAR, 7-Zip, and RAR archives when the matching command-line tool is installed.",
            );
            return;
        }

        let Some(parent) = archive.parent().map(Path::to_path_buf) else {
            self.show_error_dialog("Extract Failed", "Could not resolve an extraction folder.");
            return;
        };
        let dest = next_available_path(&parent.join(archive_output_folder_name(&archive)));
        let label = archive
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("Extract: {name}"))
            .unwrap_or_else(|| "Extract archive".to_string());
        let op_id = self.ops_panel.add_op(&label, None);
        self.ops_panel
            .update_progress(op_id, 0.2, "Extracting archive…");
        self.status.set_message("Extracting archive…");

        let controller = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || extract_archive_to_folder(archive, dest))
                .await
                .unwrap_or_else(|_| ArchiveOpResult::failure("Archive worker stopped."));
            controller.finish_archive_op(op_id, result, "archive_extract");
        });
    }

    fn finish_archive_op(self: &Rc<Self>, op_id: OpId, result: ArchiveOpResult, op_kind: &str) {
        let errors = result.error.clone().into_iter().collect::<Vec<_>>();
        self.ops_panel.finish_op(op_id, &errors);
        if errors.is_empty() {
            if let Some(output) = result.output_path.clone() {
                self.pending_reveal_cell(self.active_slot())
                    .replace(Some(output.clone()));
                let summary = if op_kind == "archive_extract" {
                    "Extracted archive"
                } else {
                    "Created ZIP archive"
                };
                self.pending_status_message
                    .replace(Some(format!("{summary}: {}.", output.display())));
                let source = result
                    .source_paths
                    .first()
                    .and_then(|path| path.parent())
                    .and_then(|path| path.to_str())
                    .unwrap_or("");
                let _ = self.metadata.borrow().log_activity_with_items(
                    op_kind,
                    result.source_paths.len() as i32,
                    summary,
                    source,
                    output.parent().and_then(|parent| parent.to_str()),
                    &[],
                    &result
                        .source_paths
                        .iter()
                        .map(|path| (path.clone(), Some(output.clone())))
                        .collect::<Vec<_>>(),
                );
            }
            self.refresh();
        } else {
            self.show_error_dialog("Archive Operation Failed", &errors.join("\n"));
        }
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
        if self.should_queue_actions() {
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
        if self.should_queue_actions() {
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
            "Pin as Palette",
            "Choose a palette name for this folder.",
            &initial_name,
            "Pin",
            move |name| controller.pin_project(path.clone(), name),
        );
    }

    fn pin_project(self: &Rc<Self>, path: PathBuf, name: String) {
        if name.trim().is_empty() {
            self.show_error_dialog("Invalid Name", "Palette names cannot be empty.");
            return;
        }

        let result = self.metadata.borrow_mut().create_project(&name, None);
        match result {
            Ok(project) => {
                // Auto-pin the chosen folder as the first destination
                let path_str = path.to_string_lossy().to_string();
                let dest_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Root")
                    .to_string();
                let _ = self
                    .metadata
                    .borrow_mut()
                    .add_project_destination(project.id, &dest_name, &path_str);
                self.refresh_metadata_sidebar();
                self.status
                    .set_message(&format!("Created palette: {}.", project.name));
            }
            Err(error) => self.show_error_dialog("Palette Save Failed", &error),
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
        if self.should_queue_actions() {
            self.queue_plan(FileOpPlan::for_apply_tag(&paths, tag_name.trim()));
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
                (tag.id, tag.name, check)
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
                .filter(|(_, _, check)| check.is_active())
                .map(|(tag_id, _, _)| *tag_id)
                .collect::<Vec<_>>();
            let selected_names = checks
                .iter()
                .filter(|(_, _, check)| check.is_active())
                .map(|(_, name, _)| name.clone())
                .collect::<Vec<_>>();
            if controller.should_queue_actions() {
                controller.queue_plan(FileOpPlan::for_remove_tags(
                    &paths,
                    &selected,
                    &selected_names,
                ));
            } else {
                controller.remove_tags_from_paths(paths.clone(), selected);
            }
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
                .set_message("Select an item before sending it to a palette.");
            return;
        }

        let projects = self.projects.borrow().clone();
        if projects.is_empty() {
            self.modal_host.show_error(
                "No Palettes Yet",
                "Create a palette and pin a folder first, then send files to it.",
            );
            return;
        }

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.append(&build_modal_prompt(
            "Choose a destination palette and whether to copy or move.",
        ));

        let project_box = GtkBox::new(Orientation::Vertical, 8);
        project_box.set_halign(Align::Fill);
        project_box.set_hexpand(true);
        content.append(&project_box);

        let mut first_project_button: Option<gtk::CheckButton> = None;
        let project_buttons = projects
            .iter()
            .map(|project| {
                let button = gtk::CheckButton::with_label(&project.name);
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
        let copy_button = gtk::CheckButton::with_label("Copy to palette");
        copy_button.set_active(true);
        copy_button.set_halign(Align::Start);
        action_row.append(&copy_button);

        let move_button = gtk::CheckButton::with_label("Move to palette");
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
            .show_with_custom_ui("Send to Palette", &content, &actions, false, None);
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
            self.show_error_dialog("Palette Missing", "That palette no longer exists.");
            return;
        };

        let first_pin = self
            .metadata
            .borrow()
            .list_project_destinations(project.id)
            .ok()
            .and_then(|mut dests| {
                dests.retain(|d| !d.path.is_empty());
                dests.into_iter().next()
            });

        let Some(pin) = first_pin else {
            self.modal_host.show_error(
                "No Pinned Folders",
                "Pin a folder to this palette before sending files to it.",
            );
            return;
        };
        let destination_root = PathBuf::from(&pin.path);

        if self.should_queue_actions() && completion.is_none() {
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
            "A palette destination already contains:\n{}\n\nChoose how to continue.",
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
                    let cancellable = gio::Cancellable::new();
                    match copy_path_recursively(&source, &destination, &cancellable, overwrite) {
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
        if self.should_queue_actions() {
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
            | PaneView::BulkNaming { .. }
            | PaneView::ActivityLog
            | PaneView::ProjectLanding(_)
            | PaneView::CloudLanding(_)
            | PaneView::ProjectManager
            | PaneView::TagManager
            | PaneView::SpaceViewer { .. }
            | PaneView::MediaConvert { .. }
            | PaneView::Watercolor(_) => None,
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

        if self.should_queue_actions() {
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

            // Cloud-specific: if trash failed with NotSupported/NotMounted on a cloud path,
            // show a modal so the user knows to use Permanent Delete explicitly.
            if !errs.is_empty() {
                if let Some((name, _)) = self.cloud_name_for_path(&paths[0]) {
                    let has_mount_error = errs.iter().any(|e| {
                        e.contains("not support")
                            || e.contains("Not Supported")
                            || e.contains("not mounted")
                            || e.contains("NotMounted")
                    });
                    if has_mount_error {
                        self.modal_host.show_error(
                            "Trash Unavailable on Cloud Drive",
                            &format!(
                                "'{name}' does not support Trash.\n\n\
                                 Files on this cloud drive cannot be moved to Trash. \
                                 To delete, use Permanent Delete (Shift+Delete or the \
                                 context menu), which requires an explicit confirmation \
                                 and cannot be undone."
                            ),
                        );
                    }
                }
            }

            // Activity log receipt
            let n = paths.len() as i32;
            let raw_summary = if n == 1 {
                let name = paths[0]
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("item");
                format!("Trashed \"{name}\"")
            } else {
                format!("Trashed {n} files")
            };
            let summary = self.cloud_summary(&raw_summary, &paths[0]);
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
                            trash_operation_error_detail(e)
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
                if controller.should_queue_actions() {
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
            let raw_summary = if total == 1 {
                "Permanently deleted item".to_string()
            } else {
                format!("Permanently deleted {total} items")
            };
            let summary = paths
                .first()
                .map_or(raw_summary.clone(), |p| self.cloud_summary(&raw_summary, p));
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
        if self.should_queue_actions() {
            self.queue_plan(FileOpPlan::for_copy_paths(&paths));
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
        // Terminal cannot open remote URI paths — the CWD must be a local filesystem path
        if is_gio_uri(&path.to_string_lossy()) {
            self.status
                .set_message("Terminal unavailable: path is a remote URI, not a local folder.");
            return;
        }
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
            .filter(|item| !item.path.as_os_str().is_empty())
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
        let slot = self.active_slot();
        let is_directory = self.is_directory_view(slot);
        // Back works from any view — tool views save to history so the user can always
        // return to the last real folder regardless of which tool they switched to.
        self.toolbar
            .back_button
            .set_sensitive(!self.back_history_cell(slot).borrow().is_empty());
        // Forward stays directory-only: after returning from a tool the forward slot is
        // intentionally empty so pressing forward doesn't land on a phantom path.
        self.toolbar
            .forward_button
            .set_sensitive(is_directory && !self.forward_history_cell(slot).borrow().is_empty());
        self.toolbar.up_button.set_sensitive(
            is_directory
                && self
                    .current_dir_for(slot)
                    .parent()
                    .map(Path::exists)
                    .unwrap_or(false),
        );
        self.toolbar.refresh_button.set_sensitive(true);
    }

    fn update_sidebar_state(&self) {
        // Clear the cloud context badge for any view that is not a browseable directory/tool.
        // Directory, Triage, and SpaceViewer views manage their own cloud context inline.
        match self.current_view_for(self.active_slot()) {
            PaneView::Directory(_) | PaneView::Triage { .. } | PaneView::SpaceViewer { .. } => {}
            _ => {
                self.status.set_cloud_context(None);
            }
        }

        let active = match self.current_view_for(self.active_slot()) {
            PaneView::Tag(_) => Some(SidebarTarget::Tags),
            PaneView::TagManager => Some(SidebarTarget::Tags),
            PaneView::ProjectLanding(_) | PaneView::ProjectManager => Some(SidebarTarget::Projects),
            PaneView::CloudLanding(cloud_id) => Some(SidebarTarget::Cloud(cloud_id)),
            PaneView::Triage { .. } => Some(SidebarTarget::Triage),
            PaneView::SystemDrives => Some(SidebarTarget::SystemDrives),
            PaneView::Recent => Some(SidebarTarget::Recent),
            PaneView::Trash => Some(SidebarTarget::Trash),
            PaneView::ActivityLog => Some(SidebarTarget::ActivityLog),
            PaneView::Search(_) => Some(SidebarTarget::Search),
            PaneView::BulkNaming { .. } => Some(SidebarTarget::BulkNaming),
            PaneView::SpaceViewer { .. } => Some(SidebarTarget::SpaceViewer),
            PaneView::MediaConvert { .. } => Some(SidebarTarget::Convert),
            PaneView::Watercolor(WatercolorView::Status) => Some(SidebarTarget::WatercolorStatus),
            PaneView::Watercolor(WatercolorView::Workspaces) => {
                Some(SidebarTarget::WatercolorWorkspaces)
            }
            PaneView::Watercolor(WatercolorView::Palettes) => {
                Some(SidebarTarget::WatercolorPalettes)
            }
            PaneView::Watercolor(WatercolorView::BrokenRefs) => {
                Some(SidebarTarget::WatercolorBrokenRefs)
            }
            PaneView::Directory(_) => {
                let current = self.current_dir_for(self.active_slot());
                self.user_places
                    .borrow()
                    .iter()
                    .find(|place| current.starts_with(&place.folder_path))
                    .map(|place| SidebarTarget::Place(place.id))
                    .or_else(|| {
                        self.removable_drives
                            .borrow()
                            .iter()
                            .find(|d| current.starts_with(&d.path))
                            .map(|d| SidebarTarget::Drive(d.path.clone()))
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
        if input.starts_with("file://") || is_gio_uri(input) {
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
            gdk::FileList::static_type(),
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
            gdk::FileList::static_type(),
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

        // surface_transform gives the offset from surface coords to widget coords.
        let (sx, sy) = self.window.surface_transform();
        self.window
            .pick(x - sx, y - sy, gtk::PickFlags::DEFAULT)
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

                let files: Vec<gio::File> = paths.iter().map(|p| gio::File::for_path(p)).collect();
                let file_list = gdk::FileList::from_array(&files);
                Some(gdk::ContentProvider::for_value(&file_list.to_value()))
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
                    let files: Vec<gio::File> =
                        paths.iter().map(|p| gio::File::for_path(p)).collect();
                    let file_list = gdk::FileList::from_array(&files);
                    Some(gdk::ContentProvider::for_value(&file_list.to_value()))
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
                gdk::FileList::static_type(),
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
                    gdk::FileList::static_type(),
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

        if self.should_queue_actions() {
            self.queue_plan(FileOpPlan::for_paste(&src_paths, &dest_dir, is_copy));
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
            let raw_summary = format!(
                "{verb} {n} file{} to {dest_name}",
                if n == 1 { "" } else { "s" }
            );
            let source = items[0]
                .0
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();
            let summary = self.cloud_summary(&raw_summary, &items[0].0);
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
            let cancellable_for_copy = cancellable_clone.clone();
            glib::MainContext::default().spawn_local(async move {
                let src_file = gio::File::for_path(&src_path);
                let dst_file = gio::File::for_path(&dst_path);
                let result = match gio::spawn_blocking(move || {
                    copy_path_recursively(
                        &src_file,
                        &dst_file,
                        &cancellable_for_copy,
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

fn should_queue_actions_state(plan_mode_active: bool, executing_plan_queue: bool) -> bool {
    plan_mode_active && !executing_plan_queue
}

fn tag_action_plan(paths: &[PathBuf], tag_name: &str) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Apply tag #{tag_name} to {} staged item(s).",
        paths.len()
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Tag Holding Tray", "Apply Tag", false, lines)
}

fn trash_action_plan(paths: &[PathBuf]) -> ConfirmationPreview {
    let mut lines = vec![format!("Move {} staged item(s) to Trash.", paths.len())];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Move Tray to Trash", "Move to Trash", true, lines)
}

fn copy_path_action_plan(paths: &[PathBuf]) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Copy {} staged path(s) to the clipboard.",
        paths.len()
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Copy Tray Paths", "Copy Paths", false, lines)
}

fn apply_mark_action_plan(paths: &[PathBuf], tint_name: &str, shape: Shape) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Mark {} staged item(s) as {} {}.",
        paths.len(),
        tint_name,
        shape.display_name(),
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Apply Mark to Tray", "Apply Mark", false, lines)
}

fn reset_mark_action_plan(paths: &[PathBuf]) -> ConfirmationPreview {
    let mut lines = vec![format!(
        "Reset {} staged item(s) to Beige Square.",
        paths.len(),
    )];
    lines.extend(plan_path_lines(paths));
    ConfirmationPreview::new("Reset Mark", "Reset Mark", false, lines)
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
        "toggle_shape_badges" => Some(WindowCommand::ToggleShapeBadges),
        "sort_order" => Some(WindowCommand::SortOrder),
        "toggle_sidebar" => Some(WindowCommand::ToggleSidebar),
        "toggle_preview" => Some(WindowCommand::TogglePreview),
        "toggle_holding_tray" => Some(WindowCommand::ToggleHoldingTray),
        "tray_add_selection" => Some(WindowCommand::TrayAddSelection),
        "tray_move_to_project" => Some(WindowCommand::TrayMoveToProject),
        "tray_copy_to_project" => Some(WindowCommand::TrayCopyToProject),
        "tray_tag" => Some(WindowCommand::TrayTag),
        "tray_trash" => Some(WindowCommand::TrayTrash),
        "tray_copy_paths" => Some(WindowCommand::TrayCopyPaths),
        "tray_clear" => Some(WindowCommand::TrayClear),
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
        "open_home" => Some(WindowCommand::OpenHome),
        "open_system_drives" => Some(WindowCommand::OpenSystemDrives),
        "open_recent" => Some(WindowCommand::OpenRecent),
        "open_trash" => Some(WindowCommand::OpenTrash),
        "open_palettes" => Some(WindowCommand::OpenPalettes),
        "open_tints_tags" => Some(WindowCommand::OpenTintsTags),
        "open_space_viewer" => Some(WindowCommand::OpenSpaceViewer),
        "open_triage" => Some(WindowCommand::OpenTriage),
        "open_bulk_naming" => Some(WindowCommand::OpenBulkNaming),
        "open_convert" => Some(WindowCommand::OpenConvert),
        "open_activity_log" => Some(WindowCommand::OpenActivityLog),
        "view_icons" => Some(WindowCommand::SetViewIcons),
        "view_list" => Some(WindowCommand::SetViewList),
        "toggle_plan_mode" => Some(WindowCommand::TogglePlanMode),
        "toggle_paint_mode" => Some(WindowCommand::TogglePaintMode),
        "paint_cursor" => Some(WindowCommand::PaintCursor),
        "paint_brush" => Some(WindowCommand::PaintBrush),
        "paint_eraser" => Some(WindowCommand::PaintEraser),
        "paint_eyedropper" => Some(WindowCommand::PaintEyedropper),
        "paint_fill" => Some(WindowCommand::PaintFill),
        "paint_undo" => Some(WindowCommand::PaintUndo),
        "paint_redo" => Some(WindowCommand::PaintRedo),
        "paint_toggle_contents" => Some(WindowCommand::PaintToggleContents),
        "empty_trash" => Some(WindowCommand::EmptyTrash),
        "tray_add_by_tint" => Some(WindowCommand::TrayAddByTint),
        "tray_add_by_shape" => Some(WindowCommand::TrayAddByShape),
        "tray_apply_mark" => Some(WindowCommand::TrayApplyMark),
        "tray_reset_mark" => Some(WindowCommand::TrayResetMark),
        "plan_execute" => Some(WindowCommand::PlanExecute),
        "plan_clear" => Some(WindowCommand::PlanClear),
        "convert_start" => Some(WindowCommand::ConvertStart),
        "convert_cancel" => Some(WindowCommand::ConvertCancel),
        "convert_retry_failed" => Some(WindowCommand::ConvertRetryFailed),
        "convert_open_output" => Some(WindowCommand::ConvertOpenOutput),
        "convert_dismiss" => Some(WindowCommand::ConvertDismiss),
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
            "backspace" => key = Some(BindingKey::Named("backspace")),
            "enter" | "return" => key = Some(BindingKey::Named("enter")),
            "home" => key = Some(BindingKey::Named("home")),
            "end" => key = Some(BindingKey::Named("end")),
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
                    | ("backspace", gdk::Key::BackSpace)
                    | ("enter", gdk::Key::Return)
                    | ("enter", gdk::Key::KP_Enter)
                    | ("home", gdk::Key::Home)
                    | ("end", gdk::Key::End)
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
            | PaneView::BulkNaming { .. }
            | PaneView::ActivityLog
            | PaneView::CloudLanding(_)
            | PaneView::SpaceViewer { .. }
            | PaneView::MediaConvert { .. }
            | PaneView::Watercolor(_)
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

fn watercolor_panel_view(view: &WatercolorView) -> WatercolorPanelView {
    match view {
        WatercolorView::Status => WatercolorPanelView::Status,
        WatercolorView::Workspaces => WatercolorPanelView::Workspaces,
        WatercolorView::Palettes => WatercolorPanelView::Palettes,
        WatercolorView::BrokenRefs => WatercolorPanelView::BrokenRefs,
    }
}

fn watercolor_tab_title(view: &WatercolorView) -> &'static str {
    match view {
        WatercolorView::Status => "Watercolor",
        WatercolorView::Workspaces => "Watercolor Workspaces",
        WatercolorView::Palettes => "Watercolor Palettes",
        WatercolorView::BrokenRefs => "Broken References",
    }
}

fn watercolor_display_label(view: &WatercolorView) -> &'static str {
    match view {
        WatercolorView::Status => "Watercolor Context",
        WatercolorView::Workspaces => "Watercolor Workspaces",
        WatercolorView::Palettes => "Watercolor Palettes",
        WatercolorView::BrokenRefs => "Broken Watercolor References",
    }
}

fn terroir_error_message(error: &terroir_client::TerroirError) -> String {
    match error {
        terroir_client::TerroirError::Unavailable(message)
        | terroir_client::TerroirError::Protocol(message)
        | terroir_client::TerroirError::Api(message) => message.clone(),
    }
}

fn fetch_watercolor_panel_data() -> WatercolorPanelData {
    let status = terroir_client::status().map_err(|error| terroir_error_message(&error));
    let workspaces = terroir_client::list_workspaces().unwrap_or_default();
    let palettes = terroir_client::list_palettes().unwrap_or_default();
    let broken_refs = terroir_client::broken_refs().unwrap_or_default();
    let doctor = terroir_client::doctor_summary().ok();

    WatercolorPanelData {
        status,
        workspaces,
        palettes,
        broken_refs,
        doctor,
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
        .get::<gdk::FileList>()
        .map(|fl| fl.files().iter().filter_map(|f| f.path()).collect())
        .unwrap_or_default()
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
    query_name_lower: &str,
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
        if let Some(item) = match_entry(e, query, query_name_lower, now_secs) {
            results.push(item);
        }
    }

    // Pass 2: subdirectories — add matching ones then recurse
    for e in &subdirs {
        if cancelled.load(Ordering::Relaxed) || results.len() >= MAX_SEARCH_RESULTS {
            return;
        }
        if let Some(item) = match_entry(e, query, query_name_lower, now_secs) {
            results.push(item);
        }
        if query.recursive {
            search_directory_blocking(
                &e.path,
                query,
                query_name_lower,
                show_hidden,
                depth + 1,
                results,
                cancelled,
            );
        }
    }
}

fn match_entry(
    e: &SearchEntry,
    query: &SearchQuery,
    query_name_lower: &str,
    now_secs: i64,
) -> Option<FileItem> {
    // Name filter
    if !query_name_lower.is_empty() && !e.fname.to_lowercase().contains(query_name_lower) {
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
        is_openable: true,
        detail: None,
        kind,
        size_bytes: if e.is_dir { None } else { Some(e.size) },
        modified_unix: Some(e.modified_secs),
        tags: Vec::new(),
        mark_tint_id: 0,
        mark_tint_color: None,
        mark_shape: Shape::DEFAULT,
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
    gio_mounts: usize,
    unmounted_volumes: usize,
    detected_drives: usize,
    fallback_mounts: usize,
    skipped_inaccessible: usize,
    skipped_non_local: usize,
}

struct RecentFolderListing {
    items: Vec<FileItem>,
    skipped_missing: usize,
}

fn collect_removable_drives() -> Vec<DriveEntry> {
    let monitor = gio::VolumeMonitor::get();
    let mut drives = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();
    for mount in monitor.mounts() {
        let root = mount.root();
        let Some(path) = root.path() else { continue };
        if !seen_paths.insert(path.clone()) {
            continue;
        }
        let name = mount.name().to_string();
        let is_removable = mount
            .volume()
            .and_then(|v| v.drive())
            .map_or(false, |d| d.is_removable());
        drives.push(DriveEntry {
            name,
            path,
            is_removable,
        });
    }
    drives
}

fn collect_mounted_volume_items() -> MountedVolumeListing {
    let monitor = gio::VolumeMonitor::get();
    let mut items = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut seen_volume_names = HashSet::new();
    let mut gio_mounts = 0usize;
    let mut unmounted_volumes = 0usize;
    let mut detected_drives = 0usize;
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

        gio_mounts += 1;
        seen_volume_names.insert(mount.name().to_string());
        let detail = format!("Mounted: {}", path.display());
        items.push(FileItem {
            name: mount.name().to_string(),
            path,
            kind: FileKind::Folder,
            is_dir: true,
            is_openable: true,
            detail: Some(detail),
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
            original_path: None,
        });
    }

    for volume in monitor.volumes() {
        if let Some(mount) = volume.get_mount() {
            seen_volume_names.insert(mount.name().to_string());
            continue;
        }

        let name = volume.name().to_string();
        if !seen_volume_names.insert(name.clone()) {
            continue;
        }

        unmounted_volumes += 1;
        items.push(FileItem {
            name,
            path: PathBuf::new(),
            kind: FileKind::Folder,
            is_dir: false,
            is_openable: false,
            detail: Some("Unmounted volume (mounting not implemented)".to_string()),
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
            original_path: None,
        });
    }

    for drive in monitor.connected_drives() {
        let name = drive.name().to_string();
        if drive.volumes().is_empty() && seen_volume_names.insert(name.clone()) {
            detected_drives += 1;
            let state = if drive.has_media() {
                "Drive detected (no mounted volume)"
            } else {
                "Drive detected (no media mounted)"
            };
            items.push(FileItem {
                name,
                path: PathBuf::new(),
                kind: FileKind::Folder,
                is_dir: false,
                is_openable: false,
                detail: Some(state.to_string()),
                size_bytes: None,
                modified_unix: None,
                tags: Vec::new(),
                mark_tint_id: 0,
                mark_tint_color: None,
                mark_shape: Shape::DEFAULT,
                original_path: None,
            });
        }
    }

    let fallback = collect_fallback_mounted_locations(&mut seen_paths);
    let fallback_mounts = fallback.items.len();
    let skipped_inaccessible = fallback.skipped_inaccessible;
    items.extend(fallback.items);

    sort_items(&mut items);

    MountedVolumeListing {
        items,
        gio_mounts,
        unmounted_volumes,
        detected_drives,
        fallback_mounts,
        skipped_inaccessible,
        skipped_non_local,
    }
}

struct FallbackMountListing {
    items: Vec<FileItem>,
    skipped_inaccessible: usize,
}

fn collect_fallback_mounted_locations(seen_paths: &mut HashSet<PathBuf>) -> FallbackMountListing {
    let user_name = glib::user_name();
    let candidates = [
        PathBuf::from("/run/media").join(user_name),
        PathBuf::from("/media"),
        PathBuf::from("/mnt"),
    ];

    let mut items = Vec::new();
    let mut skipped_inaccessible = 0usize;

    for base in candidates {
        let base_file = gio::File::for_path(&base);
        if !base_file.query_exists(None::<&gio::Cancellable>) {
            continue;
        }

        let enumerator = match base_file.enumerate_children(
            "standard::name,standard::display-name,standard::type,standard::is-hidden",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            None::<&gio::Cancellable>,
        ) {
            Ok(enumerator) => enumerator,
            Err(_) => {
                skipped_inaccessible += 1;
                continue;
            }
        };

        loop {
            let next = match enumerator.next_file(None::<&gio::Cancellable>) {
                Ok(next) => next,
                Err(_) => {
                    skipped_inaccessible += 1;
                    break;
                }
            };

            let Some(info) = next else { break };
            if info.is_hidden() || info.file_type() != gio::FileType::Directory {
                continue;
            }

            let path = base.join(info.name());
            if !seen_paths.insert(path.clone()) {
                continue;
            }

            items.push(FileItem {
                name: info.display_name().to_string(),
                path: path.clone(),
                kind: FileKind::Folder,
                is_dir: true,
                is_openable: true,
                detail: Some(format!("Mounted Locations: {}", path.display())),
                size_bytes: None,
                modified_unix: None,
                tags: Vec::new(),
                mark_tint_id: 0,
                mark_tint_color: None,
                mark_shape: Shape::DEFAULT,
                original_path: None,
            });
        }
    }

    FallbackMountListing {
        items,
        skipped_inaccessible,
    }
}

fn drive_listing_status_message(listing: &MountedVolumeListing) -> Option<String> {
    let mut parts = Vec::new();

    if listing.gio_mounts > 0 {
        parts.push(format!("{} GIO mount(s)", listing.gio_mounts));
    }
    if listing.unmounted_volumes > 0 {
        parts.push(format!(
            "{} unmounted volume(s); mounting is not implemented",
            listing.unmounted_volumes
        ));
    }
    if listing.fallback_mounts > 0 {
        parts.push(format!(
            "{} fallback mounted location(s)",
            listing.fallback_mounts
        ));
    }
    if listing.detected_drives > 0 {
        parts.push(format!(
            "{} drive(s) detected without mounted volumes",
            listing.detected_drives
        ));
    }
    if listing.skipped_non_local > 0 {
        parts.push(format!(
            "{} non-local mount(s) skipped",
            listing.skipped_non_local
        ));
    }
    if listing.skipped_inaccessible > 0 {
        parts.push(format!(
            "{} mounted-location folder(s) inaccessible",
            listing.skipped_inaccessible
        ));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
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
            is_openable: true,
            detail: None,
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
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
        PaneView::BulkNaming { .. } => "Bulk Naming".to_string(),
        PaneView::ActivityLog => "Activity Log".to_string(),
        PaneView::ProjectLanding(_) => "Palette".to_string(),
        PaneView::CloudLanding(_) => "Cloud Drive".to_string(),
        PaneView::ProjectManager => "Palettes".to_string(),
        PaneView::TagManager => "Tints & Tags".to_string(),
        PaneView::SpaceViewer { root } => {
            let folder = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Space Viewer".to_string());
            format!("Space: {folder}")
        }
        PaneView::MediaConvert { .. } => "Convert".to_string(),
        PaneView::Watercolor(view) => watercolor_tab_title(view).to_string(),
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
        PaneView::BulkNaming { root } => format!("Bulk Naming in {}", format_path(root, home)),
        PaneView::ActivityLog => "Activity Log".to_string(),
        PaneView::ProjectLanding(_) => "Palette".to_string(),
        PaneView::CloudLanding(_) => "Cloud Drive".to_string(),
        PaneView::ProjectManager => "Palettes".to_string(),
        PaneView::TagManager => "Tints & Tags".to_string(),
        PaneView::SpaceViewer { root } => format!("Space: {}", format_path(root, home)),
        PaneView::MediaConvert { from_dir } => {
            format!("Convert · {}", format_path(from_dir, home))
        }
        PaneView::Watercolor(view) => watercolor_display_label(view).to_string(),
    }
}

fn pane_view_uses_file_grid_controls(view: &PaneView) -> bool {
    matches!(
        view,
        PaneView::Directory(_)
            | PaneView::Tag(_)
            | PaneView::Triage { .. }
            | PaneView::SystemDrives
            | PaneView::Recent
            | PaneView::Trash
            | PaneView::Search(_)
    )
}

// ── Tint CSS generation ────────────────────────────────────────────────────────

fn parse_hex_rgb(hex: &str) -> (u8, u8, u8) {
    let s = hex.trim().trim_start_matches('#');
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return (128, 96, 64);
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(96);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(64);
    (r, g, b)
}

fn rgb_to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn build_tint_channel_row(label_text: &str, value: u8) -> (GtkBox, Scale) {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    row.add_css_class("tint-picker-channel-row");

    let label = Label::new(Some(label_text));
    label.add_css_class("tint-picker-channel-label");
    label.set_width_chars(6);
    label.set_halign(Align::End);
    row.append(&label);

    let adjustment = Adjustment::new(value as f64, 0.0, 255.0, 1.0, 16.0, 0.0);
    let scale = Scale::new(Orientation::Horizontal, Some(&adjustment));
    scale.add_css_class("tint-picker-scale");
    scale.set_draw_value(true);
    scale.set_digits(0);
    scale.set_hexpand(true);
    row.append(&scale);

    (row, scale)
}

fn wire_tint_channel(
    scale: &Scale,
    state: Rc<RefCell<(u8, u8, u8)>>,
    channel: usize,
    update_ui: Rc<dyn Fn()>,
) {
    scale.connect_value_changed(move |scale| {
        let value = scale.value().round().clamp(0.0, 255.0) as u8;
        {
            let mut rgb = state.borrow_mut();
            match channel {
                0 => rgb.0 = value,
                1 => rgb.1 = value,
                _ => rgb.2 = value,
            }
        }
        update_ui();
    });
}

fn generate_tint_css(tints: &[TintRecord]) -> String {
    let mut css = String::new();
    for tint in tints {
        let color = tint.color.as_deref().unwrap_or("#806040");
        // Icon card: tint glow is interactive only. Resting cards should stay
        // visually quiet; hover/selection reveal the Mark's tint.
        let (hover_ring_a, hover_glow_a) = if tint.is_default {
            ("28", "14")
        } else {
            ("58", "44")
        };
        css.push_str(&format!(
            ".file-card-shell:hover > .file-card.mark-tint-{id} {{ box-shadow: 0 0 0 1.5px {c}{hover_ring}, 0 4px 18px 0 {c}{hover_glow}; }}\n",
            id = tint.id, c = color, hover_ring = hover_ring_a, hover_glow = hover_glow_a,
        ));
        css.push_str(&format!(
            "flowboxchild:selected > .file-card-shell > .file-card.mark-tint-{id} {{ box-shadow: 0 0 0 1.5px {c}{hover_ring}, 0 4px 18px 0 {c}{hover_glow}; }}\n",
            id = tint.id, c = color, hover_ring = hover_ring_a, hover_glow = hover_glow_a,
        ));
        // List row: inset left accent via box-shadow on the inner row
        let list_a = if tint.is_default { "22" } else { "58" };
        css.push_str(&format!(
            ".file-list > row.mark-tint-{id} > .file-list-row-inner {{ border-left-color: {c}{list_a}; }}\n",
            id = tint.id, c = color, list_a = list_a,
        ));
    }
    css
}

/// Returns true for GIO/GVfs remote URI schemes (sftp, ftp, smb, dav, davs, nfs, ssh, afp).
/// Does NOT match file://, trash://, or other virtual GIO backends.
fn looks_like_explicit_path(input: &str) -> bool {
    input.starts_with('/') || input.starts_with('~') || input.contains('/') || is_gio_uri(input)
}

fn is_gio_uri(s: &str) -> bool {
    let scheme = s.split_once("://").map(|(s, _)| s).unwrap_or("");
    matches!(
        scheme,
        "sftp" | "ftp" | "smb" | "dav" | "davs" | "nfs" | "ssh" | "afp"
    )
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
            Some(project) => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::ProjectLanding(project.id),
                secondary_dir: places.home.clone(),
                tertiary_dir: places.home.clone(),
                pane_layout: PaneLayout::Single,
                notice: None,
            },
            None => LaunchResolution {
                primary_dir: places.home.clone(),
                primary_view: PaneView::Directory(places.home.clone()),
                secondary_dir: places.home.clone(),
                tertiary_dir: places.home.clone(),
                pane_layout: PaneLayout::Single,
                notice: Some(format!(
                    "Palette '{}' was not found. Opened Home instead.",
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

fn pane_view_scope_dir(view: &PaneView) -> Option<PathBuf> {
    match view {
        PaneView::Directory(path) => Some(path.clone()),
        PaneView::Triage { root, .. } => Some(root.clone()),
        PaneView::Search(query) => Some(query.scope_dir.clone()),
        PaneView::BulkNaming { root } => Some(root.clone()),
        PaneView::SpaceViewer { root } => Some(root.clone()),
        PaneView::MediaConvert { from_dir } => Some(from_dir.clone()),
        PaneView::Tag(_)
        | PaneView::SystemDrives
        | PaneView::Recent
        | PaneView::Trash
        | PaneView::ActivityLog
        | PaneView::ProjectLanding(_)
        | PaneView::CloudLanding(_)
        | PaneView::ProjectManager
        | PaneView::TagManager
        | PaneView::Watercolor(_) => None,
    }
}

fn resolve_tool_scope_dir(view: &PaneView, current_dir: &Path, home: &Path) -> PathBuf {
    pane_view_scope_dir(view)
        .or_else(|| is_launchable_directory(current_dir).then(|| current_dir.to_path_buf()))
        .unwrap_or_else(|| home.to_path_buf())
}

#[derive(Debug)]
struct ArchiveOpResult {
    source_paths: Vec<PathBuf>,
    output_path: Option<PathBuf>,
    error: Option<String>,
}

impl ArchiveOpResult {
    fn success(source_paths: Vec<PathBuf>, output_path: PathBuf) -> Self {
        Self {
            source_paths,
            output_path: Some(output_path),
            error: None,
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            source_paths: Vec::new(),
            output_path: None,
            error: Some(error.into()),
        }
    }
}

fn compress_paths_to_zip(paths: Vec<PathBuf>, parent: PathBuf, dest: PathBuf) -> ArchiveOpResult {
    if paths.is_empty() {
        return ArchiveOpResult::failure("No files were selected.");
    }

    let mut args = vec![OsString::from("-r"), dest.as_os_str().to_os_string()];
    args.push(OsString::from("--"));
    for path in &paths {
        match path.strip_prefix(&parent) {
            Ok(relative) if !relative.as_os_str().is_empty() => {
                args.push(relative.as_os_str().to_os_string());
            }
            _ => args.push(path.as_os_str().to_os_string()),
        }
    }

    match run_archive_command("zip", &args, Some(&parent)) {
        Ok(()) => ArchiveOpResult::success(paths, dest),
        Err(error) => ArchiveOpResult::failure(error),
    }
}

fn extract_archive_to_folder(archive: PathBuf, dest: PathBuf) -> ArchiveOpResult {
    if let Err(error) = std::fs::create_dir_all(&dest) {
        return ArchiveOpResult::failure(format!(
            "Could not create extraction folder '{}': {error}",
            dest.display()
        ));
    }

    let Some(kind) = archive_kind(&archive) else {
        return ArchiveOpResult::failure("Unsupported archive type.");
    };

    let result = match kind {
        ArchiveKind::Zip => run_archive_command(
            "unzip",
            &[
                OsString::from("-n"),
                archive.as_os_str().to_os_string(),
                OsString::from("-d"),
                dest.as_os_str().to_os_string(),
            ],
            None,
        ),
        ArchiveKind::Tar => run_archive_command(
            "tar",
            &[
                OsString::from("-xf"),
                archive.as_os_str().to_os_string(),
                OsString::from("-C"),
                dest.as_os_str().to_os_string(),
            ],
            None,
        ),
        ArchiveKind::SevenZip => run_archive_command(
            "7z",
            &[
                OsString::from("x"),
                archive.as_os_str().to_os_string(),
                OsString::from(format!("-o{}", dest.display())),
                OsString::from("-aos"),
            ],
            None,
        ),
    };

    match result {
        Ok(()) => ArchiveOpResult::success(vec![archive], dest),
        Err(error) => {
            let _ = std::fs::remove_dir(&dest);
            ArchiveOpResult::failure(error)
        }
    }
}

fn run_archive_command(
    program: &str,
    args: &[OsString],
    current_dir: Option<&Path>,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "Required archive tool '{program}' is not installed or is not on PATH."
            ));
        }
        Err(error) => return Err(error.to_string()),
    };

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "Archive command failed."
    };
    Err(format!(
        "{program} exited with status {}: {}",
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        truncate_archive_detail(detail, 600)
    ))
}

fn truncate_archive_detail(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    Tar,
    SevenZip,
}

fn archive_kind(path: &Path) -> Option<ArchiveKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".zip") || name.ends_with(".jar") || name.ends_with(".epub") {
        Some(ArchiveKind::Zip)
    } else if name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".tar.xz")
        || name.ends_with(".txz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tbz")
        || name.ends_with(".tbz2")
    {
        Some(ArchiveKind::Tar)
    } else if name.ends_with(".7z") || name.ends_with(".rar") {
        Some(ArchiveKind::SevenZip)
    } else {
        None
    }
}

fn is_supported_archive_path(path: &Path) -> bool {
    archive_kind(path).is_some()
}

fn archive_output_folder_name(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Archive");
    for suffix in [
        ".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".tbz", ".zip", ".7z", ".rar",
        ".jar", ".epub", ".tar",
    ] {
        if name.to_ascii_lowercase().ends_with(suffix) && name.len() > suffix.len() {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Archive")
        .to_string()
}

fn suggested_archive_name(paths: &[PathBuf]) -> String {
    let base = if paths.len() == 1 {
        paths[0]
            .file_stem()
            .or_else(|| paths[0].file_name())
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Archive")
            .to_string()
    } else {
        "Archive".to_string()
    };
    format!("{base}.zip")
}

fn common_parent(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?.parent()?.to_path_buf();
    paths
        .iter()
        .all(|path| path.parent() == Some(first.as_path()))
        .then_some(first)
}

fn next_available_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Archive");
    let extension = path.extension().and_then(|value| value.to_str());

    for attempt in 2.. {
        let name = match extension {
            Some(ext) if !ext.is_empty() => format!("{stem} ({attempt}).{ext}"),
            _ => format!("{stem} ({attempt})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
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
    cancellable: &gio::Cancellable,
    overwrite: bool,
) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(glib::Error::new(gio::IOErrorEnum::Cancelled, "Cancelled."));
    }

    let source_type = source.query_file_type(gio::FileQueryInfoFlags::NONE, Some(cancellable));
    if source_type != gio::FileType::Directory {
        return source.copy(
            destination,
            if overwrite {
                gio::FileCopyFlags::OVERWRITE
            } else {
                gio::FileCopyFlags::NONE
            },
            Some(cancellable),
            None::<&mut dyn FnMut(i64, i64)>,
        );
    }

    if destination.query_exists(Some(cancellable)) {
        let destination_type =
            destination.query_file_type(gio::FileQueryInfoFlags::NONE, Some(cancellable));
        if destination_type != gio::FileType::Directory {
            if overwrite {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "Cannot safely replace a file with a folder.",
                ));
            } else {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "Destination already exists.",
                ));
            }
        }
    } else {
        destination.make_directory_with_parents(Some(cancellable))?;
    }

    let enumerator = source.enumerate_children(
        DIRECTORY_ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        Some(cancellable),
    )?;

    while let Some(info) = enumerator.next_file(Some(cancellable))? {
        if cancellable.is_cancelled() {
            return Err(glib::Error::new(gio::IOErrorEnum::Cancelled, "Cancelled."));
        }
        let child_source = source.child(&info.name());
        let child_destination = destination.child(&info.name());
        copy_path_recursively(&child_source, &child_destination, cancellable, overwrite)?;
    }

    Ok(())
}

fn collect_bulk_naming_items_blocking(
    root: &Path,
    recursive: bool,
    show_hidden: bool,
) -> Vec<FileItem> {
    let mut items = Vec::new();
    collect_bulk_naming_items_from_dir(root, recursive, show_hidden, &mut items, 0);
    items
}

fn collect_bulk_naming_items_from_dir(
    root: &Path,
    recursive: bool,
    show_hidden: bool,
    items: &mut Vec<FileItem>,
    depth: usize,
) {
    const MAX_BULK_NAMING_RECURSION_DEPTH: usize = 32;
    if depth > MAX_BULK_NAMING_RECURSION_DEPTH {
        return;
    }
    let directory = gio::File::for_path(root);
    let Ok(enumerator) = directory.enumerate_children(
        DIRECTORY_ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    ) else {
        return;
    };

    loop {
        let info = match enumerator.next_file(None::<&gio::Cancellable>) {
            Ok(Some(info)) => info,
            Ok(None) | Err(_) => break,
        };
        let Some(item) = FileItem::from_info(&directory, &info, show_hidden) else {
            continue;
        };
        let should_recurse = recursive && item.is_dir;
        let child_path = item.path.clone();
        items.push(item);
        if should_recurse {
            collect_bulk_naming_items_from_dir(
                &child_path,
                recursive,
                show_hidden,
                items,
                depth + 1,
            );
        }
    }
}

fn common_parent_for_items(items: &[FileItem]) -> Option<PathBuf> {
    let mut parent = items.first()?.path.parent()?.to_path_buf();
    for item in items.iter().skip(1) {
        let item_parent = item.path.parent()?;
        while !item_parent.starts_with(&parent) {
            if !parent.pop() {
                return None;
            }
        }
    }
    Some(parent)
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

fn compute_duplicate_set_from_dir(dir: &Path) -> std::collections::HashSet<PathBuf> {
    use std::collections::HashMap;

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
            if let Some(hash) = hash_file_contents(path) {
                by_hash.entry(hash).or_default().push(path.clone());
            }
        }
        for (_hash, group) in by_hash {
            if group.len() >= 2 {
                duplicates.extend(group);
            }
        }
    }
    duplicates
}

fn hash_file_contents(path: &Path) -> Option<u64> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut hash = 14695981039346656037u64;
    let mut buf = [0u8; 65536];

    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            return Some(hash);
        }
        hash = fnv1a_continue(hash, &buf[..n]);
    }
}

fn fnv1a_continue(mut hash: u64, data: &[u8]) -> u64 {
    for b in data {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
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
            associated_tint_id: None,
            associated_shape: None,
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
            is_openable: true,
            detail: None,
            size_bytes: None,
            modified_unix: None,
            tags: Vec::new(),
            mark_tint_id: 0,
            mark_tint_color: None,
            mark_shape: Shape::DEFAULT,
            original_path: None,
        }
    }

    #[test]
    fn queue_guard_disables_queueing_while_executing_plan_queue() {
        assert!(!should_queue_actions_state(false, false));
        assert!(should_queue_actions_state(true, false));
        assert!(!should_queue_actions_state(true, true));
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
    fn archive_helpers_detect_supported_formats_and_output_names() {
        assert_eq!(
            archive_kind(Path::new("photos.zip")),
            Some(ArchiveKind::Zip)
        );
        assert_eq!(
            archive_kind(Path::new("backup.tar.gz")),
            Some(ArchiveKind::Tar)
        );
        assert_eq!(
            archive_kind(Path::new("bundle.7z")),
            Some(ArchiveKind::SevenZip)
        );
        assert_eq!(archive_kind(Path::new("plain.gz")), None);

        assert_eq!(
            archive_output_folder_name(Path::new("backup.tar.gz")),
            "backup"
        );
        assert_eq!(
            archive_output_folder_name(Path::new("photos.zip")),
            "photos"
        );
    }

    #[test]
    fn archive_destination_names_are_safe_and_predictable() {
        assert_eq!(
            suggested_archive_name(&[PathBuf::from("/tmp/Project Folder")]),
            "Project Folder.zip"
        );
        assert_eq!(
            suggested_archive_name(&[PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")]),
            "Archive.zip"
        );
        assert_eq!(
            common_parent(&[PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")]),
            Some(PathBuf::from("/tmp"))
        );
        assert_eq!(
            common_parent(&[PathBuf::from("/tmp/a.txt"), PathBuf::from("/var/b.txt")]),
            None
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
    fn duplicate_detection_requires_full_file_match() {
        let root = temp_test_dir("duplicates");
        fs::create_dir_all(&root).unwrap();
        let same_prefix_a = root.join("same-prefix-a.bin");
        let same_prefix_b = root.join("same-prefix-b.bin");
        let duplicate_a = root.join("duplicate-a.bin");
        let duplicate_b = root.join("duplicate-b.bin");

        let mut first = vec![b'a'; 65_536];
        first.extend_from_slice(b"tail-a");
        let mut second = vec![b'a'; 65_536];
        second.extend_from_slice(b"tail-b");
        fs::write(&same_prefix_a, &first).unwrap();
        fs::write(&same_prefix_b, &second).unwrap();
        fs::write(&duplicate_a, b"identical full contents").unwrap();
        fs::write(&duplicate_b, b"identical full contents").unwrap();

        let duplicates = compute_duplicate_set_from_dir(&root);
        assert!(!duplicates.contains(&same_prefix_a));
        assert!(!duplicates.contains(&same_prefix_b));
        assert!(duplicates.contains(&duplicate_a));
        assert!(duplicates.contains(&duplicate_b));
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
    fn file_grid_controls_are_hidden_for_full_panel_tools() {
        let tag = TagRecord {
            id: 1,
            name: "Work".to_string(),
            color: None,
            associated_tint_id: None,
            associated_shape: None,
        };

        assert!(pane_view_uses_file_grid_controls(&PaneView::Directory(
            PathBuf::from("/tmp")
        )));
        assert!(pane_view_uses_file_grid_controls(&PaneView::Tag(tag)));
        assert!(pane_view_uses_file_grid_controls(&PaneView::Triage {
            root: PathBuf::from("/tmp"),
            filter: TriageFilter::Images,
        }));
        assert!(pane_view_uses_file_grid_controls(&PaneView::Search(
            SearchQuery::new(PathBuf::from("/tmp"))
        )));
        assert!(pane_view_uses_file_grid_controls(&PaneView::Trash));

        assert!(!pane_view_uses_file_grid_controls(&PaneView::ActivityLog));
        assert!(!pane_view_uses_file_grid_controls(&PaneView::BulkNaming {
            root: PathBuf::from("/tmp"),
        }));
        assert!(!pane_view_uses_file_grid_controls(&PaneView::SpaceViewer {
            root: PathBuf::from("/tmp"),
        }));
        assert!(!pane_view_uses_file_grid_controls(
            &PaneView::MediaConvert {
                from_dir: PathBuf::from("/tmp"),
            }
        ));
        assert!(!pane_view_uses_file_grid_controls(
            &PaneView::ProjectManager
        ));
        assert!(!pane_view_uses_file_grid_controls(&PaneView::TagManager));
        assert!(!pane_view_uses_file_grid_controls(
            &PaneView::ProjectLanding(1)
        ));
        assert!(!pane_view_uses_file_grid_controls(&PaneView::CloudLanding(
            1
        )));
    }

    #[test]
    fn tool_scope_uses_folder_backed_view_scope() {
        let dir = PathBuf::from("/tmp/folder");
        let triage = PathBuf::from("/tmp/triage");
        let search = PathBuf::from("/tmp/search");
        let bulk = PathBuf::from("/tmp/bulk");
        let space = PathBuf::from("/tmp/space");
        let convert = PathBuf::from("/tmp/convert");

        assert_eq!(
            pane_view_scope_dir(&PaneView::Directory(dir.clone())),
            Some(dir)
        );
        assert_eq!(
            pane_view_scope_dir(&PaneView::Triage {
                root: triage.clone(),
                filter: TriageFilter::All,
            }),
            Some(triage)
        );
        assert_eq!(
            pane_view_scope_dir(&PaneView::Search(SearchQuery::new(search.clone()))),
            Some(search)
        );
        assert_eq!(
            pane_view_scope_dir(&PaneView::BulkNaming { root: bulk.clone() }),
            Some(bulk)
        );
        assert_eq!(
            pane_view_scope_dir(&PaneView::SpaceViewer {
                root: space.clone()
            }),
            Some(space)
        );
        assert_eq!(
            pane_view_scope_dir(&PaneView::MediaConvert {
                from_dir: convert.clone()
            }),
            Some(convert)
        );
    }

    #[test]
    fn tool_scope_falls_back_to_last_usable_folder_for_special_views() {
        let places = test_places();
        let current = places.downloads.clone();
        let tag = test_tag();
        let special_views = vec![
            PaneView::Tag(tag),
            PaneView::SystemDrives,
            PaneView::Recent,
            PaneView::Trash,
            PaneView::ActivityLog,
            PaneView::ProjectLanding(1),
            PaneView::CloudLanding(1),
            PaneView::ProjectManager,
            PaneView::TagManager,
        ];

        for view in special_views {
            assert_eq!(
                resolve_tool_scope_dir(&view, &current, &places.home),
                current
            );
        }
    }

    #[test]
    fn tool_scope_falls_back_home_when_last_folder_is_unusable() {
        let places = test_places();
        let missing = places.downloads.join("missing");

        assert_eq!(
            resolve_tool_scope_dir(&PaneView::Recent, &missing, &places.home),
            places.home
        );
    }

    #[test]
    fn window_shortcuts_dispatch_standard_commands() {
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let ctrl_shift = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
        let ctrl_alt = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK;

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
            window_command_from_key(gdk::Key::h, ctrl_alt),
            Some(WindowCommand::ToggleHoldingTray)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::a, ctrl_alt),
            Some(WindowCommand::TrayAddSelection)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::m, ctrl_alt),
            Some(WindowCommand::TrayMoveToProject)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::c, ctrl_alt),
            Some(WindowCommand::TrayCopyToProject)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::t, ctrl_alt),
            Some(WindowCommand::TrayTag)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::Delete, ctrl_alt),
            Some(WindowCommand::TrayTrash)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::p, ctrl_alt),
            Some(WindowCommand::TrayCopyPaths)
        );
        assert_eq!(
            window_command_from_key(gdk::Key::k, ctrl_alt),
            Some(WindowCommand::TrayClear)
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
    fn configured_shortcuts_dispatch_extended_commands() {
        let mut config = AppConfig::default();
        config
            .shortcuts
            .insert("open_convert".to_string(), "Ctrl+Alt+F".to_string());
        config
            .shortcuts
            .insert("open_activity_log".to_string(), "Ctrl+Alt+J".to_string());
        config
            .shortcuts
            .insert("plan_execute".to_string(), "Ctrl+Alt+Enter".to_string());
        config
            .shortcuts
            .insert("toggle_shape_badges".to_string(), "Ctrl+Alt+B".to_string());

        let ctrl_alt = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK;

        assert_eq!(
            configured_window_command_from_key(&config, gdk::Key::f, ctrl_alt),
            Some(WindowCommand::OpenConvert)
        );
        assert_eq!(
            configured_window_command_from_key(&config, gdk::Key::j, ctrl_alt),
            Some(WindowCommand::OpenActivityLog)
        );
        assert_eq!(
            configured_window_command_from_key(&config, gdk::Key::Return, ctrl_alt),
            Some(WindowCommand::PlanExecute)
        );
        assert_eq!(
            configured_window_command_from_key(&config, gdk::Key::b, ctrl_alt),
            Some(WindowCommand::ToggleShapeBadges)
        );
    }

    #[test]
    fn configured_shortcuts_can_disable_defaults() {
        let mut config = AppConfig::default();
        config.shortcuts.remove("new_folder");

        assert_eq!(
            configured_window_command_from_key(
                &config,
                gdk::Key::n,
                gdk::ModifierType::CONTROL_MASK
            ),
            None
        );
    }

    #[test]
    fn all_documented_builtin_shortcut_ids_dispatch() {
        for (action_id, _) in crate::config::BUILTIN_SHORTCUT_ACTIONS {
            assert!(
                builtin_command(action_id).is_some(),
                "documented shortcut action does not dispatch: {action_id}"
            );
        }
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
        let project = metadata.create_project("Alpha", Some("#00e5ff")).unwrap();

        let launch = LaunchConfig {
            project: Some("alpha".to_string()),
            ..LaunchConfig::default()
        };

        let resolution = resolve_launch(&launch, &places, &metadata);
        assert_eq!(resolution.primary_dir, places.home);
        assert_eq!(
            resolution.primary_view,
            PaneView::ProjectLanding(project.id)
        );
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

fn trash_operation_error_detail(error: &glib::Error) -> String {
    let base = friendly_error_detail(error);
    match error.kind::<gio::IOErrorEnum>() {
        Some(gio::IOErrorEnum::NotSupported) => format!(
            "{base}. This filesystem or mount may not support Trash. Permanent Delete is available only through the explicit confirmation flow."
        ),
        Some(gio::IOErrorEnum::NotMounted) => {
            format!("{base}. The mount may have disappeared or is not available.")
        }
        Some(gio::IOErrorEnum::PermissionDenied) => {
            format!("{base}. Check mount permissions or GVfs/portal access.")
        }
        _ => format!("{base}. {TRASH_GVFS_DIAGNOSTIC}"),
    }
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

fn tint_name_and_color(
    tints: &[TintRecord],
    tint_id: i64,
    fallback_color: Option<String>,
) -> (String, Option<String>) {
    if let Some(tint) = tints.iter().find(|tint| tint.id == tint_id) {
        return (
            tint.name.clone(),
            fallback_color.or_else(|| tint.color.clone()),
        );
    }

    if let Some(default) = tints.iter().find(|tint| tint.is_default) {
        return (
            default.name.clone(),
            fallback_color.or_else(|| default.color.clone()),
        );
    }

    ("Beige".to_string(), fallback_color)
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

fn read_explicit_mark_from_item(item: &FileItem) -> Option<(i64, Shape)> {
    if item.mark_tint_id != 0 {
        Some((item.mark_tint_id, item.mark_shape))
    } else {
        None
    }
}

fn collect_paths_recursively(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_paths_inner(root, &mut out);
    out
}

fn collect_paths_inner(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        out.push(path.clone());
        if path.is_dir() {
            collect_paths_inner(&path, out);
        }
    }
}

fn folder_path_str_from_slot(controller: &BrowserController, slot: PaneSlot) -> String {
    controller.current_dir_for(slot).display().to_string()
}
