use crate::ui::{
    file_grid::{FileGrid, FileItem},
    preview_pane::PreviewPane,
    sidebar::{Sidebar, SidebarLocation},
    status_bar::StatusBar,
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
const TERMINAL_ENV_VAR: &str = "LATTICE_TERMINAL";

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
        let file_grid = FileGrid::build();
        let preview = PreviewPane::build();
        let status = StatusBar::build();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&toolbar.root);

        let body = build_body(&sidebar, &file_grid, &preview);
        body.set_vexpand(true);
        root.append(&body);

        root.append(&status.root);
        window.set_child(Some(&root));

        let controller = BrowserController::new(
            window.clone(),
            places,
            toolbar.clone(),
            sidebar.clone(),
            file_grid.clone(),
            preview.clone(),
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
    file_grid: FileGrid,
    preview: PreviewPane,
    status: StatusBar,
    context_popover: RefCell<Option<Popover>>,
    current_dir: RefCell<PathBuf>,
    back_history: RefCell<Vec<PathBuf>>,
    items: RefCell<Vec<FileItem>>,
    pending_status_message: RefCell<Option<String>>,
    show_hidden: Cell<bool>,
    load_generation: Cell<u64>,
    load_cancellable: RefCell<Option<gio::Cancellable>>,
}

impl BrowserController {
    fn new(
        window: ApplicationWindow,
        places: Places,
        toolbar: Toolbar,
        sidebar: Sidebar,
        file_grid: FileGrid,
        preview: PreviewPane,
        status: StatusBar,
    ) -> Rc<Self> {
        Rc::new(Self {
            window,
            current_dir: RefCell::new(places.home.clone()),
            terminal_command: detect_terminal_command(),
            places,
            toolbar,
            sidebar,
            file_grid,
            preview,
            status,
            context_popover: RefCell::new(None),
            back_history: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            pending_status_message: RefCell::new(None),
            show_hidden: Cell::new(false),
            load_generation: Cell::new(0),
            load_cancellable: RefCell::new(None),
        })
    }

    fn bootstrap(self: &Rc<Self>) {
        self.connect_navigation();
        self.connect_sidebar();
        self.connect_file_grid();
        self.update_action_state();
        self.navigate_to(self.places.home.clone(), false);
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
            .show_hidden_toggle
            .connect_toggled(move |toggle| controller.set_show_hidden(toggle.is_active()));

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

    fn connect_file_grid(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.file_grid
            .flow
            .connect_selected_children_changed(move |_| controller.update_selection());

        let controller = Rc::clone(self);
        self.file_grid
            .flow
            .connect_child_activated(move |_, child| controller.activate_index(child.index()));

        let controller = Rc::clone(self);
        let flow = self.file_grid.flow.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        gesture.connect_pressed(move |_, _, x, y| {
            if flow.child_at_pos(x as i32, y as i32).is_none() {
                controller.show_current_folder_menu(x, y);
            }
        });
        self.file_grid.flow.add_controller(gesture);
    }

    fn navigate_to(self: &Rc<Self>, path: PathBuf, remember_current: bool) {
        if remember_current {
            let current = self.current_dir.borrow().clone();
            if current != path {
                self.back_history.borrow_mut().push(current);
            }
        }

        self.current_dir.replace(path.clone());
        self.load_directory(path);
    }

    fn go_back(self: &Rc<Self>) {
        let previous = self.back_history.borrow_mut().pop();
        if let Some(path) = previous {
            self.current_dir.replace(path.clone());
            self.load_directory(path);
        }
    }

    fn go_up(self: &Rc<Self>) {
        let current = self.current_dir.borrow().clone();
        if let Some(parent) = current.parent() {
            self.navigate_to(parent.to_path_buf(), true);
        }
    }

    fn refresh(self: &Rc<Self>) {
        self.load_directory(self.current_dir.borrow().clone());
    }

    fn set_show_hidden(self: &Rc<Self>, show_hidden: bool) {
        self.show_hidden.set(show_hidden);
        self.refresh();
    }

    fn load_directory(self: &Rc<Self>, path: PathBuf) {
        self.cancel_active_load();
        self.dismiss_context_menu();

        let display_path = self.display_path(&path);
        self.toolbar.path_entry.set_text(&display_path);
        self.status.set_path(&display_path);
        self.status.clear_message();
        self.file_grid.set_loading();
        self.file_grid.clear_selection();
        self.items.borrow_mut().clear();
        self.preview.show_current_folder(&display_path, 0);
        self.update_sidebar_state();
        self.update_navigation_state();
        self.update_action_state();

        let generation = self.load_generation.get() + 1;
        self.load_generation.set(generation);

        let cancellable = gio::Cancellable::new();
        self.load_cancellable.replace(Some(cancellable.clone()));

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
                if !controller.is_current_load(generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                match result {
                    Ok(enumerator) => {
                        let collected = Rc::new(RefCell::new(Vec::new()));
                        controller.read_directory_batch(
                            directory_for_callback.clone(),
                            enumerator,
                            collected,
                            generation,
                            path.clone(),
                            cancellable_for_callback.clone(),
                        );
                    }
                    Err(error) => {
                        controller.finish_load_error(generation, &path, &error);
                    }
                }
            },
        );
    }

    fn read_directory_batch(
        self: &Rc<Self>,
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
                if !controller.is_current_load(generation)
                    || cancellable_for_callback.is_cancelled()
                {
                    return;
                }

                match result {
                    Ok(batch) if batch.is_empty() => {
                        let mut items = collected.borrow().clone();
                        sort_items(&mut items);
                        controller.finish_load(generation, &path, items);
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
                            directory.clone(),
                            enumerator_for_callback.clone(),
                            collected.clone(),
                            generation,
                            path.clone(),
                            cancellable_for_callback.clone(),
                        );
                    }
                    Err(error) => {
                        controller.finish_load_error(generation, &path, &error);
                    }
                }
            },
        );
    }

    fn finish_load(self: &Rc<Self>, generation: u64, path: &Path, items: Vec<FileItem>) {
        if !self.is_current_load(generation) {
            return;
        }

        self.load_cancellable.borrow_mut().take();
        self.items.replace(items.clone());
        self.file_grid.set_items(&items);
        self.attach_context_handlers();

        let display_path = self.display_path(path);
        self.preview.show_current_folder(&display_path, items.len());
        self.status.set_counts(items.len(), 0);
        self.status.set_path(&display_path);
        self.toolbar.path_entry.set_text(&display_path);
        if let Some(message) = self.pending_status_message.borrow_mut().take() {
            self.status.set_message(&message);
        }
        self.update_navigation_state();
        self.update_action_state();
    }

    fn finish_load_error(self: &Rc<Self>, generation: u64, path: &Path, error: &glib::Error) {
        if !self.is_current_load(generation) {
            return;
        }

        self.load_cancellable.borrow_mut().take();
        self.items.borrow_mut().clear();
        self.file_grid
            .set_empty_message("Unable to read this folder.");

        let display_path = self.display_path(path);
        self.preview
            .show_error(&display_path, &friendly_error_detail(error));
        self.status.set_counts(0, 0);
        self.status.set_message("Unable to read this folder");
        self.status.set_path(&display_path);
        self.toolbar.path_entry.set_text(&display_path);
        self.update_navigation_state();
        self.update_action_state();
    }

    fn attach_context_handlers(self: &Rc<Self>) {
        for index in 0..self.items.borrow().len() {
            if let Some(child) = self.file_grid.flow.child_at_index(index as i32) {
                let gesture = gtk::GestureClick::new();
                gesture.set_button(3);

                let controller = Rc::clone(self);
                let anchor: gtk::Widget = child.clone().upcast();
                gesture.connect_pressed(move |_, _, x, y| {
                    controller.show_context_menu(index as i32, anchor.clone(), x, y);
                });

                child.add_controller(gesture);
            }
        }
    }

    fn show_context_menu(self: &Rc<Self>, index: i32, anchor: gtk::Widget, x: f64, y: f64) {
        self.dismiss_context_menu();
        self.file_grid.select_only_index(index);

        let item = match self.item_for_index(index) {
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
            move || controller.open_item(&item)
        });
        append_menu_button(&menu_box, "Open With", false, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.open_with_placeholder(&item)
        });
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
        append_menu_button(&menu_box, "Permanent Delete", true, {
            let controller = Rc::clone(self);
            let item = item.clone();
            move || controller.confirm_permanent_delete(vec![item.path.clone()])
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

    fn show_current_folder_menu(self: &Rc<Self>, x: f64, y: f64) {
        self.dismiss_context_menu();
        self.file_grid.clear_selection();

        let popover = Popover::new();
        popover.add_css_class("context-menu");
        popover.set_has_arrow(true);
        popover.set_autohide(true);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.set_parent(&self.file_grid.flow);

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
            move || {
                controller.copy_paths_to_clipboard(vec![controller.current_dir.borrow().clone()])
            }
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

    fn update_selection(&self) {
        let selected_items = self.selected_items();
        let item_count = self.items.borrow().len();
        let selected_count = selected_items.len();

        if let Some(item) = selected_items.first() {
            self.preview.show_item(item, &self.display_path(&item.path));
        } else {
            let current = self.current_dir.borrow().clone();
            self.preview
                .show_current_folder(&self.display_path(&current), item_count);
        }

        self.status.set_counts(item_count, selected_count);
        self.update_action_state();
    }

    fn update_action_state(&self) {
        let selected_count = self.file_grid.selected_indices().len();
        self.toolbar
            .rename_button
            .set_sensitive(selected_count == 1);
        self.toolbar.trash_button.set_sensitive(selected_count > 0);
        self.toolbar.new_folder_button.set_sensitive(true);
    }

    fn activate_index(self: &Rc<Self>, index: i32) {
        if let Some(item) = self.item_for_index(index) {
            self.open_item(&item);
        }
    }

    fn open_item(self: &Rc<Self>, item: &FileItem) {
        if item.is_dir {
            self.navigate_to(item.path.clone(), true);
        } else {
            self.open_file(&item.path);
        }
    }

    fn open_with_placeholder(self: &Rc<Self>, item: &FileItem) {
        self.status
            .set_message("Open With chooser is coming later. Using the default app.");
        self.open_item(item);
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
        let suggested_name = next_new_folder_path(&self.current_dir.borrow())
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

        let current_dir = self.current_dir.borrow().clone();
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
            self.finish_batch("Moved item(s) to trash.", result);
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

    fn confirm_permanent_delete(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        let noun = if paths.len() == 1 { "item" } else { "items" };
        let detail =
            format!("This will permanently delete the selected {noun}. This cannot be undone.");
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message("Permanently delete?")
            .detail(&detail)
            .build();
        dialog.set_buttons(&["Cancel", "Delete Permanently"]);
        dialog.set_cancel_button(0);
        dialog.set_default_button(0);

        let controller = Rc::clone(self);
        dialog.choose(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if matches!(result, Ok(1)) {
                    controller.delete_paths_permanently(paths);
                }
            },
        );
    }

    fn delete_paths_permanently(self: &Rc<Self>, paths: Vec<PathBuf>) {
        self.status.set_message("Deleting items permanently…");
        self.run_delete_batch(paths, 0, Rc::new(RefCell::new(BatchResult::default())));
    }

    fn run_delete_batch(
        self: &Rc<Self>,
        paths: Vec<PathBuf>,
        index: usize,
        result: Rc<RefCell<BatchResult>>,
    ) {
        if index >= paths.len() {
            self.finish_batch("Deleted item(s) permanently.", result);
            return;
        }

        let current_path = paths[index].clone();
        let controller = Rc::clone(self);
        gio::File::for_path(&current_path).delete_async(
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

                controller.run_delete_batch(paths.clone(), index + 1, result.clone());
            },
        );
    }

    fn finish_batch(self: &Rc<Self>, success_message: &str, result: Rc<RefCell<BatchResult>>) {
        let result = result.borrow();

        if result.failures.is_empty() {
            self.pending_status_message.replace(Some(format!(
                "{} {} completed.",
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
                .unwrap_or_else(|| self.current_dir.borrow().clone())
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
        self.open_terminal_for_path(self.current_dir.borrow().clone(), true);
    }

    fn selected_items(&self) -> Vec<FileItem> {
        self.file_grid
            .selected_indices()
            .into_iter()
            .filter_map(|index| self.item_for_index(index))
            .collect()
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_items()
            .into_iter()
            .map(|item| item.path)
            .collect()
    }

    fn selected_single_item(&self) -> Option<FileItem> {
        let items = self.selected_items();
        if items.len() == 1 {
            items.into_iter().next()
        } else {
            None
        }
    }

    fn update_navigation_state(&self) {
        self.toolbar
            .back_button
            .set_sensitive(!self.back_history.borrow().is_empty());
        self.toolbar.up_button.set_sensitive(
            self.current_dir
                .borrow()
                .parent()
                .map(Path::exists)
                .unwrap_or(false),
        );
        self.toolbar.refresh_button.set_sensitive(true);
    }

    fn update_sidebar_state(&self) {
        let current = self.current_dir.borrow();

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

    fn item_for_index(&self, index: i32) -> Option<FileItem> {
        if index < 0 {
            return None;
        }

        self.items.borrow().get(index as usize).cloned()
    }

    fn display_path(&self, path: &Path) -> String {
        format_path(path, &self.places.home)
    }

    fn cancel_active_load(&self) {
        if let Some(cancellable) = self.load_cancellable.borrow_mut().take() {
            cancellable.cancel();
        }
    }

    fn is_current_load(&self, generation: u64) -> bool {
        self.load_generation.get() == generation
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

fn build_body(sidebar: &Sidebar, file_grid: &FileGrid, preview: &PreviewPane) -> Paned {
    let outer = Paned::new(Orientation::Horizontal);
    outer.set_wide_handle(false);
    outer.set_start_child(Some(&sidebar.root));
    outer.set_position(220);

    let center_and_preview = Paned::new(Orientation::Horizontal);
    center_and_preview.set_wide_handle(false);

    let center = build_center(file_grid);
    center_and_preview.set_start_child(Some(&center));
    center_and_preview.set_end_child(Some(&preview.root));
    center_and_preview.set_position(700);

    outer.set_end_child(Some(&center_and_preview));
    outer
}

fn build_center(file_grid: &FileGrid) -> GtkBox {
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&build_tab_strip());

    file_grid.root.set_vexpand(true);
    file_grid.root.set_hexpand(true);
    vbox.append(&file_grid.root);

    vbox
}

fn build_tab_strip() -> GtkBox {
    let strip = GtkBox::new(Orientation::Horizontal, 0);
    strip.add_css_class("tab-strip");

    for (index, label) in ["Home", "Downloads", "Projects"].iter().enumerate() {
        let button = Button::with_label(label);
        button.add_css_class("tab-button");
        if index == 0 {
            button.add_css_class("active");
        } else {
            button.set_sensitive(false);
        }
        strip.append(&button);
    }

    let add_button = Button::with_label("+");
    add_button.add_css_class("tab-add-button");
    add_button.set_sensitive(false);
    strip.append(&add_button);

    strip
}

fn connect_directory_button(controller: &Rc<BrowserController>, button: &Button, path: PathBuf) {
    let controller = Rc::clone(controller);
    button.connect_clicked(move |_| controller.navigate_to(path.clone(), true));
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
    column.set_size_request(420, -1);
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
    entry.set_size_request(420, -1);
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
