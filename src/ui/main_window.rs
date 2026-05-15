use crate::ui::{
    file_grid::{FileGrid, FileItem},
    preview_pane::PreviewPane,
    sidebar::{Sidebar, SidebarLocation},
    status_bar::StatusBar,
    tab_strip::TabStrip,
    toolbar::Toolbar,
};
use gio::prelude::*;
use glib::UserDirectory;
use gtk::gdk;
use gtk::prelude::*;
use gtk::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, Entry, Label, Orientation, Paned,
    Popover, ResponseType, Separator,
};
use std::cell::{Cell, RefCell};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const DIRECTORY_ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::is-hidden";
const PREVIEW_ATTRIBUTES: &str =
    "standard::display-name,standard::type,standard::content-type,standard::size,time::modified";
const TERMINAL_ENV_VAR: &str = "LATTICE_TERMINAL";
const TEXT_PREVIEW_LIMIT_BYTES: usize = 64 * 1024;
const TEXT_PREVIEW_DISPLAY_CHARS: usize = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneSlot {
    Primary,
    Secondary,
}

#[derive(Clone, Debug)]
struct TabState {
    title: String,
    primary_dir: PathBuf,
    primary_back_history: Vec<PathBuf>,
    secondary_dir: PathBuf,
    secondary_back_history: Vec<PathBuf>,
    split_enabled: bool,
    active_pane: PaneSlot,
}

impl TabState {
    fn new(path: PathBuf) -> Self {
        let title = tab_title_for_path(&path);
        Self {
            title,
            primary_dir: path.clone(),
            primary_back_history: Vec::new(),
            secondary_dir: path,
            secondary_back_history: Vec::new(),
            split_enabled: false,
            active_pane: PaneSlot::Primary,
        }
    }

    fn current_dir(&self, slot: PaneSlot) -> &PathBuf {
        match slot {
            PaneSlot::Primary => &self.primary_dir,
            PaneSlot::Secondary => &self.secondary_dir,
        }
    }
}

#[derive(Clone)]
struct PaneWidgets {
    root: GtkBox,
    path_label: Label,
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
        path_label.set_margin_start(12);
        path_label.set_margin_end(12);
        path_label.set_margin_top(8);
        path_label.set_margin_bottom(8);
        header.append(&path_label);

        let file_grid = FileGrid::build();
        file_grid.root.set_vexpand(true);
        file_grid.root.set_hexpand(true);

        root.append(&header);
        root.append(&file_grid.root);

        Self {
            root,
            path_label,
            file_grid,
        }
    }
}

pub struct MainWindow;

impl MainWindow {
    pub fn new(app: &Application) -> ApplicationWindow {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Lattice")
            .default_width(1280)
            .default_height(800)
            .build();

        window.add_css_class("app-window");

        let places = Places::discover();
        let toolbar = Toolbar::build();
        let sidebar = Sidebar::build(places.projects.is_some());
        let tab_strip = TabStrip::build();
        let primary_pane = PaneWidgets::build(PaneSlot::Primary);
        let secondary_pane = PaneWidgets::build(PaneSlot::Secondary);
        let preview = PreviewPane::build();
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

        root.append(&status.root);
        window.set_child(Some(&root));

        let controller = BrowserController::new(
            window.clone(),
            places,
            toolbar.clone(),
            sidebar.clone(),
            tab_strip.clone(),
            primary_pane.clone(),
            secondary_pane.clone(),
            preview.clone(),
            body.preview_host.clone(),
            status.clone(),
        );
        controller.bootstrap();

        window
    }
}

#[derive(Clone)]
struct Places {
    home: PathBuf,
    downloads: PathBuf,
    documents: PathBuf,
    projects: Option<PathBuf>,
}

impl Places {
    fn discover() -> Self {
        let home = glib::home_dir();
        let downloads = glib::user_special_dir(UserDirectory::Downloads)
            .unwrap_or_else(|| home.join("Downloads"));
        let documents = glib::user_special_dir(UserDirectory::Documents)
            .unwrap_or_else(|| home.join("Documents"));
        let projects_path = home.join("Projects");
        let projects = projects_path.exists().then_some(projects_path);

        Self {
            home,
            downloads,
            documents,
            projects,
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
    tabs: RefCell<Vec<TabState>>,
    active_tab: Cell<usize>,
    active_pane: Cell<PaneSlot>,
    current_dir: RefCell<PathBuf>,
    back_history: RefCell<Vec<PathBuf>>,
    items: RefCell<Vec<FileItem>>,
    secondary_current_dir: RefCell<PathBuf>,
    secondary_back_history: RefCell<Vec<PathBuf>>,
    secondary_items: RefCell<Vec<FileItem>>,
    pending_reveal_path: RefCell<Option<PathBuf>>,
    secondary_pending_reveal_path: RefCell<Option<PathBuf>>,
    pending_status_message: RefCell<Option<String>>,
    show_hidden: Cell<bool>,
    preview_visible: Cell<bool>,
    split_enabled: Cell<bool>,
    load_generation: Cell<u64>,
    load_cancellable: RefCell<Option<gio::Cancellable>>,
    secondary_load_generation: Cell<u64>,
    secondary_load_cancellable: RefCell<Option<gio::Cancellable>>,
    preview_generation: Cell<u64>,
    preview_cancellable: RefCell<Option<gio::Cancellable>>,
}

impl BrowserController {
    fn new(
        window: ApplicationWindow,
        places: Places,
        toolbar: Toolbar,
        sidebar: Sidebar,
        tab_strip: TabStrip,
        primary_pane: PaneWidgets,
        secondary_pane: PaneWidgets,
        preview: PreviewPane,
        preview_host: GtkBox,
        status: StatusBar,
    ) -> Rc<Self> {
        let initial_tab = TabState::new(places.home.clone());
        Rc::new(Self {
            window,
            tabs: RefCell::new(vec![initial_tab]),
            active_tab: Cell::new(0),
            active_pane: Cell::new(PaneSlot::Primary),
            current_dir: RefCell::new(places.home.clone()),
            secondary_current_dir: RefCell::new(places.home.clone()),
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
            back_history: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            secondary_back_history: RefCell::new(Vec::new()),
            secondary_items: RefCell::new(Vec::new()),
            pending_reveal_path: RefCell::new(None),
            secondary_pending_reveal_path: RefCell::new(None),
            pending_status_message: RefCell::new(None),
            show_hidden: Cell::new(false),
            preview_visible: Cell::new(true),
            split_enabled: Cell::new(false),
            load_generation: Cell::new(0),
            load_cancellable: RefCell::new(None),
            secondary_load_generation: Cell::new(0),
            secondary_load_cancellable: RefCell::new(None),
            preview_generation: Cell::new(0),
            preview_cancellable: RefCell::new(None),
        })
    }

    fn bootstrap(self: &Rc<Self>) {
        self.connect_navigation();
        self.connect_sidebar();
        self.connect_tab_strip();
        self.connect_panes();
        self.connect_preview_actions();
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
        self.toolbar
            .split_toggle
            .connect_toggled(move |toggle| controller.set_split_enabled(toggle.is_active()));

        let controller = Rc::clone(self);
        self.toolbar
            .show_hidden_toggle
            .connect_toggled(move |toggle| controller.set_show_hidden(toggle.is_active()));

        let controller = Rc::clone(self);
        self.toolbar
            .preview_toggle
            .connect_toggled(move |toggle| controller.set_preview_visible(toggle.is_active()));

        let controller = Rc::clone(self);
        self.toolbar
            .new_folder_button
            .connect_clicked(move |_| controller.create_new_folder());

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

        if let Some(projects) = self.places.projects.clone() {
            connect_directory_button(self, &self.sidebar.projects_button, projects);
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

    fn current_dir_for(&self, slot: PaneSlot) -> PathBuf {
        self.current_dir_cell(slot).borrow().clone()
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
            tab.secondary_dir = self.secondary_current_dir.borrow().clone();
            tab.secondary_back_history = self.secondary_back_history.borrow().clone();
            tab.split_enabled = self.split_enabled.get();
            tab.active_pane = self.active_pane.get();
            tab.title = tab_title_for_path(&tab.primary_dir);
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
            tab_button.set_tooltip_text(Some(
                &tab.current_dir(PaneSlot::Primary).display().to_string(),
            ));
            let controller = Rc::clone(self);
            tab_button.connect_clicked(move |_| controller.switch_to_tab(index));

            let close_button = Button::with_label("×");
            close_button.add_css_class("tab-close-button");
            close_button.set_sensitive(can_close);
            close_button.set_tooltip_text(Some("Close tab"));
            let controller = Rc::clone(self);
            close_button.connect_clicked(move |_| controller.close_tab(index));

            tab_chip.append(&tab_button);
            tab_chip.append(&close_button);
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
        self.secondary_current_dir
            .replace(tab.secondary_dir.clone());
        self.secondary_back_history
            .replace(tab.secondary_back_history.clone());
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
        self.load_directory(PaneSlot::Primary, tab.primary_dir);
        if self.split_enabled.get() {
            self.load_directory(PaneSlot::Secondary, tab.secondary_dir);
        } else {
            self.secondary_pane.file_grid.clear_selection();
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
            self.load_directory(
                PaneSlot::Secondary,
                self.current_dir_for(PaneSlot::Secondary),
            );
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
        let display_path = self.display_path(&self.current_dir_for(target));
        self.status.set_path(&display_path);
        self.update_navigation_state();
        self.update_sidebar_state();
        self.sync_path_entry_to_display();
        self.update_selection();
    }

    fn update_selection_for(self: &Rc<Self>, slot: PaneSlot) {
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
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_directory(slot, path);
    }

    fn go_back(self: &Rc<Self>) {
        let slot = self.active_slot();
        let previous = self.back_history_cell(slot).borrow_mut().pop();
        if let Some(path) = previous {
            self.current_dir_cell(slot).replace(path.clone());
            self.sync_active_tab_state();
            if slot == PaneSlot::Primary {
                self.rebuild_tab_strip();
            }
            self.load_directory(slot, path);
        }
    }

    fn go_up(self: &Rc<Self>) {
        let slot = self.active_slot();
        let current = self.current_dir_for(slot);
        if let Some(parent) = current.parent() {
            self.navigate_to(slot, parent.to_path_buf(), true);
        }
    }

    fn refresh(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.load_directory(slot, self.current_dir_for(slot));
        if self.split_enabled.get() {
            let other = Self::other_slot(slot);
            self.load_directory(other, self.current_dir_for(other));
        }
    }

    fn set_show_hidden(self: &Rc<Self>, show_hidden: bool) {
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

    fn set_preview_visible(self: &Rc<Self>, visible: bool) {
        self.preview_visible.set(visible);
        self.preview_host.set_visible(visible);
        self.cancel_active_preview();

        if visible {
            self.refresh_preview();
        }
    }

    fn load_directory(self: &Rc<Self>, slot: PaneSlot, path: PathBuf) {
        self.cancel_active_load(slot);
        if slot == self.active_slot() {
            self.cancel_active_preview();
        }
        self.dismiss_context_menu();

        let display_path = self.display_path(&path);
        let pane = self.pane_widgets(slot);
        pane.path_label.set_label(&display_path);
        pane.file_grid.set_loading();
        pane.file_grid.clear_selection();
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
        self.items_cell(slot).replace(items.clone());
        self.pane_widgets(slot).file_grid.set_items(&items);
        self.attach_context_handlers(slot);
        let revealed = self.reveal_pending_selection(slot);

        let display_path = self.display_path(path);
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
                self.preview.show_current_folder(&display_path, items.len());
                self.status.set_counts(items.len(), 0);
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
            .set_empty_message("Unable to read this folder.");

        let display_path = self.display_path(path);
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

        let menu_box = GtkBox::new(Orientation::Vertical, 4);
        menu_box.set_margin_top(8);
        menu_box.set_margin_bottom(8);
        menu_box.set_margin_start(8);
        menu_box.set_margin_end(8);

        append_menu_button(&menu_box, "Open", false, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.open_item_in_slot(slot, &item)
        });
        append_menu_button(&menu_box, "Open With", false, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.open_with_placeholder(slot, &item)
        });
        if item.is_dir {
            append_menu_button(&menu_box, "Open Folder in New Tab", false, {
                let controller = Rc::clone(self);
                let item = item.clone();
                move || controller.open_new_tab(Some(item.path.clone()))
            });
            if self.split_enabled.get() {
                append_menu_button(&menu_box, "Open Folder in Other Pane", false, {
                    let controller = Rc::clone(self);
                    let item = item.clone();
                    move || controller.open_folder_in_other_pane(slot, item.path.clone())
                });
            } else {
                append_menu_button(&menu_box, "Open Folder in Split Pane", false, {
                    let controller = Rc::clone(self);
                    let item = item.clone();
                    move || controller.open_folder_in_split(item.path.clone())
                });
            }
        }
        menu_box.append(&Separator::new(Orientation::Horizontal));
        append_menu_button(&menu_box, "Rename", false, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.show_rename_dialog(item.path.clone(), item.name.clone())
        });
        append_menu_button(&menu_box, "Copy Path", false, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.copy_paths_to_clipboard(vec![item.path.clone()])
        });
        append_menu_button(&menu_box, "Open Terminal Here", false, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.open_terminal_for_path(item.path.clone(), item.is_dir)
        });
        menu_box.append(&Separator::new(Orientation::Horizontal));
        append_menu_button(&menu_box, "Move to Trash", false, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.move_paths_to_trash(vec![item.path.clone()])
        });

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
        self.dismiss_context_menu();
        self.set_active_pane(slot);
        self.pane_widgets(slot).file_grid.clear_selection();

        let popover = Popover::new();
        popover.add_css_class("context-menu");
        popover.set_has_arrow(true);
        popover.set_autohide(true);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.set_parent(&self.pane_widgets(slot).file_grid.flow);

        let menu_box = GtkBox::new(Orientation::Vertical, 4);
        menu_box.set_margin_top(8);
        menu_box.set_margin_bottom(8);
        menu_box.set_margin_start(8);
        menu_box.set_margin_end(8);

        append_menu_button(&menu_box, "New Folder", false, {
            let controller = Rc::clone(self);
            move || controller.create_new_folder()
        });
        append_menu_button(&menu_box, "Open Terminal Here", false, {
            let controller = Rc::clone(self);
            move || controller.open_current_folder_terminal()
        });
        append_menu_button(&menu_box, "Copy Path", false, {
            let controller = Rc::clone(self);
            move || controller.copy_paths_to_clipboard(vec![controller.current_dir_for(slot)])
        });

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

    fn dismiss_context_menu(&self) {
        if let Some(popover) = self.context_popover.borrow_mut().take() {
            if popover.parent().is_some() {
                popover.unparent();
            }
        }
    }

    fn update_selection(self: &Rc<Self>) {
        let slot = self.active_slot();
        let selected_items = self.selected_items_for(slot);
        let item_count = self.current_item_count_for(slot);
        let selected_count = selected_items.len();

        self.status.set_counts(item_count, selected_count);
        self.status
            .set_path(&self.display_path(&self.current_dir_for(slot)));
        self.update_action_state();
        self.refresh_preview();
    }

    fn update_action_state(&self) {
        let selected_count = self
            .pane_widgets(self.active_slot())
            .file_grid
            .selected_indices()
            .len();
        self.toolbar
            .rename_button
            .set_sensitive(selected_count == 1);
        self.toolbar.trash_button.set_sensitive(selected_count > 0);
        self.toolbar.new_folder_button.set_sensitive(true);
    }

    fn refresh_preview(self: &Rc<Self>) {
        self.cancel_active_preview();

        if !self.preview_visible.get() {
            return;
        }

        let selected_items = self.selected_items();
        match selected_items.len() {
            0 => {
                let current = self.current_dir_for(self.active_slot());
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
                    self.current_item_count_for(self.active_slot()),
                );
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
        let type_label = preview_type_label(&item, content_type.as_deref());

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

        if item.kind == crate::ui::file_grid::FileKind::Image {
            self.preview_cancellable.borrow_mut().take();
            self.preview.show_image(
                &item.kind,
                &item.name,
                &display_path,
                size.as_deref(),
                modified.as_deref(),
                None,
            );
            self.preview
                .set_image_file(Some(&gio::File::for_path(&item.path)));
            let dimensions = self
                .preview
                .image_dimensions()
                .map(|(width, height)| format!("{width} × {height}"));
            self.preview.set_dimensions(dimensions.as_deref());
            self.preview.set_action_state(true, true, true);
            return;
        }

        if matches!(
            item.kind,
            crate::ui::file_grid::FileKind::Text | crate::ui::file_grid::FileKind::ConfigCode
        ) {
            self.load_text_preview(
                generation,
                item,
                display_path,
                type_label,
                size,
                modified,
                cancellable,
            );
            return;
        }

        self.preview_cancellable.borrow_mut().take();
        self.preview.show_basic_file(
            &item.kind,
            &type_label,
            &item.name,
            &display_path,
            size.as_deref(),
            modified.as_deref(),
            Some("No rich preview for this file type yet."),
        );
        self.preview.set_action_state(true, true, true);
    }

    fn load_text_preview(
        self: &Rc<Self>,
        generation: u64,
        item: FileItem,
        display_path: String,
        type_label: String,
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

    fn open_with_placeholder(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
        self.status
            .set_message("Open With chooser is coming later. Using the default app.");
        self.open_item_in_slot(slot, item);
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

    fn rename_selected(self: &Rc<Self>) {
        if let Some(item) = self.selected_single_item() {
            self.show_rename_dialog(item.path, item.name);
        }
    }

    #[allow(deprecated)]
    fn show_rename_dialog(self: &Rc<Self>, path: PathBuf, current_name: String) {
        let dialog = build_input_dialog(
            &self.window,
            "Rename",
            "Choose a new name for the selected item.",
            &current_name,
            "Rename",
        );
        let entry = dialog.entry.clone();
        let controller = Rc::clone(self);

        dialog.dialog.run_async(move |dialog, response| {
            if response == ResponseType::Accept {
                let name = entry.text().trim().to_string();
                controller.rename_path(path.clone(), name);
            }
            dialog.close();
        });
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
        let new_name_for_callback = new_name.clone();
        file.set_display_name_async(
            &new_name,
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(_) => {
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

    #[allow(deprecated)]
    fn create_new_folder(self: &Rc<Self>) {
        let suggested_name = next_new_folder_path(&self.current_dir_for(self.active_slot()))
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("New Folder")
            .to_string();

        let dialog = build_input_dialog(
            &self.window,
            "New Folder",
            "Choose a name for the new folder.",
            &suggested_name,
            "Create",
        );
        let entry = dialog.entry.clone();
        let controller = Rc::clone(self);

        dialog.dialog.run_async(move |dialog, response| {
            if response == ResponseType::Accept {
                let name = entry.text().trim().to_string();
                controller.create_folder_named(name);
            }
            dialog.close();
        });
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

    fn trash_selected(self: &Rc<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }

        self.move_paths_to_trash(paths);
    }

    fn move_paths_to_trash(self: &Rc<Self>, paths: Vec<PathBuf>) {
        self.status.set_message("Moving items to trash…");
        self.run_trash_batch(paths, 0, Rc::new(RefCell::new(BatchResult::default())));
    }

    fn run_trash_batch(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        index: usize,
        result: Rc<RefCell<BatchResult>>,
    ) {
        if index >= paths.len() {
            self.finish_batch("moved to trash", result);
            return;
        }

        let current_path = paths[index].clone();
        let controller = Rc::clone(self);
        gio::File::for_path(&current_path).trash_async(
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |operation| {
                match operation {
                    Ok(_) => result.borrow_mut().success_count += 1,
                    Err(error) => result.borrow_mut().failures.push(format!(
                        "{}: {}",
                        current_path.display(),
                        friendly_error_detail(&error)
                    )),
                }

                controller.run_trash_batch(paths.clone(), index + 1, result.clone());
            },
        );
    }

    fn finish_batch(self: &Rc<Self>, success_message: &str, result: Rc<RefCell<BatchResult>>) {
        let result = result.borrow();

        if result.failures.is_empty() {
            self.pending_status_message.replace(Some(format!(
                "{} item(s) {}.",
                result.success_count, success_message
            )));
        } else {
            let summary = if result.success_count > 0 {
                format!(
                    "{} succeeded, {} failed.",
                    result.success_count,
                    result.failures.len()
                )
            } else {
                format!("{} operation(s) failed.", result.failures.len())
            };
            self.pending_status_message.replace(Some(summary));
            self.show_error_dialog("Operation Failed", &result.failures.join("\n"));
        }

        self.refresh();
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
        self.status.set_message("Copied path to clipboard.");
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

    fn open_preview_target(self: &Rc<Self>) {
        if let Some(item) = self.selected_single_item() {
            self.open_item(&item);
        }
    }

    fn copy_preview_target_path(self: &Rc<Self>) {
        if let Some(item) = self.selected_single_item() {
            self.copy_paths_to_clipboard(vec![item.path]);
        } else {
            self.copy_paths_to_clipboard(vec![self.current_dir_for(self.active_slot())]);
        }
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
        self.toolbar.back_button.set_sensitive(
            !self
                .back_history_cell(self.active_slot())
                .borrow()
                .is_empty(),
        );
        self.toolbar.up_button.set_sensitive(
            self.current_dir_for(self.active_slot())
                .parent()
                .map(Path::exists)
                .unwrap_or(false),
        );
        self.toolbar.refresh_button.set_sensitive(true);
    }

    fn update_sidebar_state(&self) {
        let current = self.current_dir_for(self.active_slot());

        let active = self
            .places
            .projects
            .as_ref()
            .filter(|path| current.starts_with(path))
            .map(|_| SidebarLocation::Projects)
            .or_else(|| {
                current
                    .starts_with(&self.places.downloads)
                    .then_some(SidebarLocation::Downloads)
            })
            .or_else(|| {
                current
                    .starts_with(&self.places.documents)
                    .then_some(SidebarLocation::Documents)
            })
            .or_else(|| {
                current
                    .starts_with(&self.places.home)
                    .then_some(SidebarLocation::Home)
            });

        self.sidebar.set_active(active);
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
        true
    }

    fn display_path(&self, path: &Path) -> String {
        format_path(path, &self.places.home)
    }

    fn sync_path_entry_to_display(&self) {
        let current = self.current_dir_for(self.active_slot());
        let display_path = self.display_path(&current);
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

    fn cancel_active_load(&self, slot: PaneSlot) {
        if let Some(cancellable) = self.load_cancellable_cell(slot).borrow_mut().take() {
            cancellable.cancel();
        }
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
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(title)
            .detail(detail)
            .build();
        dialog.show(Some(&self.window));
    }
}

#[allow(deprecated)]
struct InputDialog {
    dialog: gtk::Dialog,
    entry: Entry,
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

fn append_menu_button<F: Fn() + 'static>(
    menu_box: &GtkBox,
    label: &str,
    dangerous: bool,
    action: F,
) {
    let button = Button::with_label(label);
    button.add_css_class("context-menu-button");
    if dangerous {
        button.add_css_class("context-menu-danger");
    }
    button.set_halign(Align::Fill);
    button.connect_clicked(move |_| action());
    menu_box.append(&button);
}

#[allow(deprecated)]
fn build_input_dialog(
    parent: &ApplicationWindow,
    title: &str,
    prompt: &str,
    initial_text: &str,
    confirm_label: &str,
) -> InputDialog {
    let dialog = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .build();
    dialog.add_css_class("input-dialog");
    dialog.set_resizable(false);
    dialog.set_default_size(460, -1);

    dialog.add_button("Cancel", ResponseType::Cancel);
    let accept_button = dialog.add_button(confirm_label, ResponseType::Accept);
    dialog.set_default_response(ResponseType::Accept);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let column = GtkBox::new(Orientation::Vertical, 12);
    column.set_halign(Align::Fill);
    column.set_hexpand(true);
    column.add_css_class("dialog-column");
    content.append(&column);

    let prompt_label = Label::new(Some(prompt));
    prompt_label.set_wrap(false);
    prompt_label.set_halign(Align::Start);
    prompt_label.set_hexpand(true);
    prompt_label.add_css_class("dialog-prompt");
    column.append(&prompt_label);

    let entry = Entry::new();
    entry.set_text(initial_text);
    entry.select_region(0, -1);
    entry.set_width_chars(36);
    entry.set_hexpand(true);
    entry.add_css_class("dialog-entry");
    column.append(&entry);

    accept_button.set_sensitive(!initial_text.trim().is_empty());
    let accept_button = accept_button.clone();
    entry.connect_changed(move |entry| {
        accept_button.set_sensitive(!entry.text().trim().is_empty());
    });

    entry.grab_focus();

    InputDialog { dialog, entry }
}

fn sort_items(items: &mut [FileItem]) {
    items.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
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

fn preview_type_label(item: &FileItem, content_type: Option<&str>) -> String {
    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.starts_with("audio/") {
        "Audio".to_string()
    } else {
        item.kind.label().to_string()
    }
}
