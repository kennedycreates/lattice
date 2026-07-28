use crate::config::{shortcut_tooltip, AppConfig, CustomActionConfig};
use crate::converter::ConversionQueue;
use crate::metadata::{CloudRecord, MetadataStore, PlaceRecord, ProjectRecord, Shape, TagRecord};
use crate::terroir_client;
use crate::ui::{
    activity_log_panel::ActivityLogPanel,
    bulk_naming_panel::BulkNamingPanel,
    cloud_landing_panel::CloudLandingPanel,
    convert_progress_panel::ConvertProgressPanel,
    file_grid::{FileGrid, FileItem, ViewMode},
    holding_tray::HoldingTray,
    media_convert_panel::MediaConvertPanel,
    modal_host::ModalHost,
    ops_panel::OpsPanel,
    painting_toolbar::{PaintTool, PaintType, PaintingToolbar},
    palette_board_panel::PaletteBoardPanel,
    plan_queue_panel::PlanQueuePanel,
    preview_pane::PreviewPane,
    project_landing_panel::ProjectLandingPanel,
    project_manager_panel::ProjectManagerPanel,
    search_panel::{SearchPanel, SearchQuery},
    sidebar::{DriveEntry, Sidebar},
    space_viewer_panel::SpaceViewerPanel,
    status_bar::StatusBar,
    tab_strip::TabStrip,
    tag_filter::TagFilterPanel,
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
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, FlowBox, HeaderBar,
    Image, Label, Orientation, Paned, Popover, Revealer, Separator,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{atomic::AtomicBool, Arc};

mod action_preview;
mod activity;
mod archive;
mod controller_core;
mod controller_core2;
mod controller_ext;
mod controller_ext2;
mod copy_util;
mod dedup;
mod drive;
mod format;
mod keys;
mod path_complete;
mod paths;
#[cfg(test)]
use path_complete::{
    path_completion_display, path_completion_query, PathCompletionMode, PathCompletionQuery,
};
#[cfg(test)]
use paths::pane_view_scope_dir;
use paths::resolve_launch;
mod search;
mod sort;
mod tint_css;
mod triage;
mod view_label;
#[cfg(test)]
use crate::ui::file_grid::FileKind;
#[cfg(test)]
use action_preview::trash_action_plan;
#[cfg(test)]
use archive::{archive_kind, archive_output_folder_name, suggested_archive_name, ArchiveKind};
#[cfg(test)]
use copy_util::common_parent;
#[cfg(test)]
use dedup::compute_duplicate_set_from_dir;
#[cfg(test)]
use keys::{builtin_command, configured_window_command_from_key, window_command_from_key};
#[cfg(test)]
use paths::next_new_text_document_path;
#[cfg(test)]
use paths::resolve_tool_scope_dir;
#[cfg(test)]
use view_label::pane_view_uses_file_grid_controls;
use view_label::tab_title_for_view;

pub(super) const DIRECTORY_ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::is-hidden,standard::size,time::modified";
const TRASH_ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::is-hidden,standard::size,time::modified,trash::orig-path,standard::target-uri";
const PREVIEW_ATTRIBUTES: &str =
    "standard::display-name,standard::type,standard::content-type,standard::size,time::modified";
pub(super) const TERMINAL_ENV_VAR: &str = "LATTICE_TERMINAL";
const TEXT_PREVIEW_LIMIT_BYTES: usize = 64 * 1024;
const TEXT_PREVIEW_DISPLAY_CHARS: usize = 4_000;
pub(super) const TRIAGE_LARGE_FILE_BYTES: u64 = 50 * 1024 * 1024;
const TRASH_GVFS_DIAGNOSTIC: &str = "Trash support may require GVfs. Install gvfs, udisks2, and polkit, then log out/in or reboot.\n\nUbuntu/Pop!_OS: sudo apt install gvfs udisks2 polkitd\nArch/CachyOS:   sudo pacman -Syu --needed gvfs udisks2 polkit\nFedora:         sudo dnf install gvfs udisks2 polkit\nVoid:           sudo xbps-install gvfs udisks2 polkit dbus\n\nTroubleshooting:\ngio list trash:///\ngio trash --list\ngio mount -l";
const DRIVES_GVFS_DIAGNOSTIC: &str = "No system drives found through GIO/GVfs.\n\nInstall gvfs, udisks2, and polkit, then log out/in or reboot.\n\nUbuntu/Pop!_OS: sudo apt install gvfs udisks2 polkitd\nArch/CachyOS:   sudo pacman -Syu --needed gvfs udisks2 polkit\nFedora:         sudo dnf install gvfs udisks2 polkit\nVoid:           sudo xbps-install gvfs udisks2 polkit dbus\n\nTroubleshooting:\ngio mount -l\nudisksctl status\nlsblk -f";
const GVFS_REMOTE_DIAGNOSTIC: &str = "GVfs remote is unavailable. Possible causes:\n• GVfs daemon not running or backend not installed\n• Remote host unreachable or credentials expired\n• SMB shares need gvfs-smb; SFTP/FTP/DAV are in the core gvfs package\n\nUbuntu/Debian: sudo apt install gvfs gvfs-backends\nArch/CachyOS:  sudo pacman -S gvfs gvfs-smb gvfs-mtp\nFedora:        sudo dnf install gvfs gvfs-smb gvfs-fuse\nVoid:          sudo xbps-install gvfs gvfs-smb\n\nDiagnostics:\ngio mount <uri>\ngio mount -l";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneSlot {
    Primary,
    Secondary,
    Tertiary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneLayout {
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
pub(super) enum TriageFilter {
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
pub(super) enum PaneView {
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
pub(super) enum WatercolorView {
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

pub(super) struct LaunchResolution {
    pub(super) primary_dir: PathBuf,
    pub(super) primary_view: PaneView,
    pub(super) secondary_dir: PathBuf,
    pub(super) tertiary_dir: PathBuf,
    pub(super) pane_layout: PaneLayout,
    pub(super) notice: Option<String>,
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
    // Factory that builds and wires the concrete window; returns the GTK widget
    // rather than `Self` (a zero-sized marker), so `new` is the natural name.
    #[allow(clippy::new_ret_no_self)]
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
pub(super) struct Places {
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
pub(super) struct ConfirmationPreview {
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

/// State for an in-progress rubber-band (marquee) selection drag.
struct MarqueeSession {
    /// Drag start point in the active container's coordinate space.
    start: (f64, f64),
    /// Selection that existed before the drag (for Ctrl/Shift additive drags).
    base: Vec<i32>,
    /// Ctrl or Shift held — union with the base selection instead of replacing it.
    additive: bool,
    /// Whether the pointer moved far enough to count as a drag (vs. a bare click).
    moved: bool,
}

pub(super) struct DriveState {
    entry: DriveEntry,
    mount: Option<gio::Mount>,
    volume: Option<gio::Volume>,
}

struct BrowserController {
    window: ApplicationWindow,
    places: Places,
    metadata: RefCell<MetadataStore>,
    user_places: RefCell<Vec<PlaceRecord>>,
    cloud_locations: RefCell<Vec<CloudRecord>>,
    removable_drives: RefCell<Vec<DriveState>>,
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
    /// Active rubber-band (marquee) selection session, if a drag is in progress.
    marquee: RefCell<Option<MarqueeSession>>,
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

pub(super) fn watercolor_tab_title(view: &WatercolorView) -> &'static str {
    match view {
        WatercolorView::Status => "Watercolor",
        WatercolorView::Workspaces => "Watercolor Workspaces",
        WatercolorView::Palettes => "Watercolor Palettes",
        WatercolorView::BrokenRefs => "Broken References",
    }
}

pub(super) fn watercolor_display_label(view: &WatercolorView) -> &'static str {
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

// A handful of free helper functions still follow this module; the Phase-4
// module split relocates tests to the end of each submodule and drops this.
#[allow(clippy::items_after_test_module)]
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

/// Log (but never surface) a failure from a best-effort activity-log write.
/// The activity log is an audit convenience, so a write failure should be
/// visible in stderr without interrupting the user's action.
fn log_activity_result(result: Result<i64, String>) {
    if let Err(e) = result {
        log_warn!("activity log write failed: {e}");
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
