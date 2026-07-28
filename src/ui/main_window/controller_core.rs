//! First half of the BrowserController inherent impl, split out of mod.rs.
//! Shares mod.rs's imports via globs (see controller_ext.rs).
#![allow(unused_imports)]

use crate::action_plan::ActionPlan as FileOpPlan;
use crate::config::{shortcut_tooltip, AppConfig, CustomActionConfig};
use crate::converter::{
    cleanup_orphaned_temps_in, ConversionQueue, ConvertItem, ConvertSettings, MediaKind,
};
use crate::metadata::{
    ActivityLogEntry, CloudRecord, MetadataStore, PlaceRecord, ProjectRecord, Shape, TagRecord,
};
use crate::terroir_client;
use crate::ui::{
    activity_log_panel::{ActivityLogAction, ActivityLogPanel},
    bulk_naming_panel::BulkNamingPanel,
    cloud_landing_panel::CloudLandingPanel,
    convert_progress_panel::ConvertProgressPanel,
    file_grid::{FileGrid, FileItem, FileKind, ViewMode},
    holding_tray::HoldingTray,
    media_convert_panel::{ConvertSourceMode, MediaConvertPanel},
    modal_host::{
        build_modal_actions, build_modal_button, build_modal_prompt, ButtonKind, ModalHost,
    },
    ops_panel::OpsPanel,
    painting_toolbar::{PaintTool, PaintType, PaintingToolbar},
    palette_board_panel::PaletteBoardPanel,
    picker::{show_picker_modal, PickerConfig, PickerResult},
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
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea, Entry,
    FlowBox, HeaderBar, Image, Label, ListBox, ListBoxRow, Orientation, Paned, Popover, Revealer,
    Separator,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{atomic::AtomicBool, mpsc, Arc};

use super::action_preview::*;
use super::activity::*;
use super::archive::*;
use super::controller_ext::*;
use super::copy_util::*;
use super::dedup::*;
use super::drive::*;
use super::format::*;
use super::keys::*;
use super::path_complete::*;
use super::paths::*;
use super::search::*;
use super::sort::*;
use super::tint_css::*;
use super::triage::*;
use super::view_label::*;
use super::*;

impl BrowserController {
    pub(super) fn new(
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
            log_warn!("metadata store unavailable, using in-memory fallback: {error}");
            MetadataStore::open_in_memory()
        });
        let metadata = metadata.unwrap_or_else(|error| {
            // Both the on-disk store and the in-memory fallback failed — there is
            // nothing usable to continue with, so exit cleanly with a clear log
            // line instead of an opaque panic backtrace.
            log_err!("could not initialize metadata storage (even in-memory): {error}");
            std::process::exit(1);
        });
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
            marquee: RefCell::new(None),
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

    pub(super) fn bootstrap(self: &Rc<Self>) {
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

    pub(super) fn connect_navigation(self: &Rc<Self>) {
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

    pub(super) fn connect_holding_tray(self: &Rc<Self>) {
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
            .send_to_folder_button
            .connect_clicked(move |_| controller.send_tray_to_current_folder(false));

        let controller = Rc::clone(self);
        self.holding_tray
            .copy_to_folder_button
            .connect_clicked(move |_| controller.send_tray_to_current_folder(true));

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

    pub(super) fn connect_window_shortcuts(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        let win_keys = gtk::EventControllerKey::new();
        win_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        win_keys.connect_key_pressed(move |_, key, _, modifiers| {
            controller.handle_window_key(key, modifiers)
        });
        self.window.add_controller(win_keys);
    }

    #[allow(deprecated)]
    pub(super) fn attach_path_completion(self: &Rc<Self>) {
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

    pub(super) fn attach_path_live_search(self: &Rc<Self>) {
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
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(280), move || {
                    c.path_box_debounce.borrow_mut().take();
                    c.run_path_box_search();
                });
            *controller.path_box_debounce.borrow_mut() = Some(id);
        });
    }

    pub(super) fn handle_window_key(
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

    pub(super) fn window_command_from_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<WindowCommand> {
        configured_window_command_from_key(&self.config, key, modifiers)
    }

    pub(super) fn focused_context(&self) -> FocusedContext {
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

    pub(super) fn search_entries(&self) -> [Entry; 3] {
        [
            self.primary_pane.search_panel.name_entry.clone(),
            self.secondary_pane.search_panel.name_entry.clone(),
            self.tertiary_pane.search_panel.name_entry.clone(),
        ]
    }

    pub(super) fn sidebar_buttons(&self) -> Vec<Button> {
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

    pub(super) fn handle_sidebar_navigation(
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

    pub(super) fn connect_sidebar(self: &Rc<Self>) {
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
        let ctrl = Rc::clone(self);
        volume_monitor.connect_volume_added(move |_, _| ctrl.refresh_drive_sidebar());
        let ctrl = Rc::clone(self);
        volume_monitor.connect_volume_removed(move |_, _| ctrl.refresh_drive_sidebar());
    }

    pub(super) fn open_convert_from_sidebar(self: &Rc<Self>) {
        let slot = self.active_slot();
        let items = self.resolve_convert_source_items(slot);
        self.open_media_convert_with_items(slot, items);
    }

    pub(super) fn resolve_convert_source_items(self: &Rc<Self>, slot: PaneSlot) -> Vec<FileItem> {
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

    pub(super) fn reload_convert_items(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn open_space_viewer(self: &Rc<Self>) {
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

    pub(super) fn load_space_viewer_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn load_space_viewer_mark_stats(
        self: &Rc<Self>,
        root: &std::path::Path,
        slot: PaneSlot,
    ) {
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

    pub(super) fn open_activity_log(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.save_dir_to_history_if_in_directory(slot);
        self.current_view_cell(slot).replace(PaneView::ActivityLog);
        self.sync_active_tab_state();
        if slot == PaneSlot::Primary {
            self.rebuild_tab_strip();
        }
        self.load_activity_log_view(slot);
    }

    pub(super) fn open_watercolor(self: &Rc<Self>, view: WatercolorView) {
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

    pub(super) fn load_watercolor_view(self: &Rc<Self>, slot: PaneSlot, view: WatercolorView) {
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

    pub(super) fn open_watercolor_path(self: &Rc<Self>, path: PathBuf) {
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

    pub(super) fn refresh_watercolor_context(self: &Rc<Self>) {
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

    pub(super) fn refresh_watercolor_sidebar_visibility(self: &Rc<Self>) {
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

    pub(super) fn open_project_manager(self: &Rc<Self>) {
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

    pub(super) fn load_project_manager_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn handle_project_created(self: &Rc<Self>, name: String) {
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

    pub(super) fn handle_project_renamed(self: &Rc<Self>, id: i64, name: String) {
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

    pub(super) fn handle_project_deleted(self: &Rc<Self>, project_id: i64) {
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
                if let Err(e) = controller.metadata.borrow_mut().delete_project(project_id) {
                    controller.report_db_error(
                        "delete_project",
                        &e,
                        Some("Couldn't delete the palette."),
                    );
                }
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

    pub(super) fn reload_project_manager_if_visible(self: &Rc<Self>) {
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::ProjectManager) {
                self.load_project_manager_view(slot);
            }
        }
    }

    pub(super) fn open_tag_manager(self: &Rc<Self>) {
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

    pub(super) fn load_tag_manager_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn handle_tag_created(self: &Rc<Self>, name: String) {
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

    pub(super) fn handle_tag_renamed(self: &Rc<Self>, id: i64, new_name: String) {
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

    pub(super) fn handle_tag_deleted(self: &Rc<Self>, id: i64) {
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

    pub(super) fn reload_tag_manager_if_visible(self: &Rc<Self>) {
        self.apply_tint_css();
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::TagManager) {
                self.load_tag_manager_view(slot);
            }
        }
    }

    pub(super) fn handle_tint_created(self: &Rc<Self>, name: String, color: String) {
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

    pub(super) fn handle_tint_renamed(self: &Rc<Self>, id: i64, new_name: String) {
        let result = self.metadata.borrow_mut().rename_tint(id, &new_name);
        match result {
            Ok(()) => self.reload_tag_manager_if_visible(),
            Err(e) => self.modal_host.show_error("Rename Tint Failed", &e),
        }
    }

    pub(super) fn handle_tint_color_changed(self: &Rc<Self>, id: i64, color: String) {
        let result = self.metadata.borrow_mut().update_tint_color(id, &color);
        match result {
            Ok(()) => self.reload_tag_manager_if_visible(),
            Err(e) => self.modal_host.show_error("Update Tint Color Failed", &e),
        }
    }

    pub(super) fn show_tint_color_picker(
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

    pub(super) fn handle_tint_deleted(self: &Rc<Self>, id: i64) {
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

    pub(super) fn load_activity_log_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn load_project_landing_view(self: &Rc<Self>, slot: PaneSlot, project_id: i64) {
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

    pub(super) fn populate_palette_board(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
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
                            if let Err(e) = ctrl.metadata.borrow_mut().update_palette_item_geometry(
                                id,
                                x,
                                y,
                                item.width,
                                item.height,
                            ) {
                                // Fires on every drag frame — log only, don't nag.
                                ctrl.report_db_error("update_palette_item_geometry", &e, None);
                            }
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
                        if let Err(e) = ctrl.metadata.borrow_mut().delete_palette_item(id) {
                            ctrl.report_db_error(
                                "delete_palette_item",
                                &e,
                                Some("Couldn't remove the item."),
                            );
                        }
                    }
                },
                // on_note_edited
                {
                    let ctrl = Rc::clone(&controller);
                    move |id, title: Option<String>, body: Option<String>| {
                        if let Err(e) = ctrl.metadata.borrow_mut().update_palette_item_content(
                            id,
                            title.as_deref(),
                            body.as_deref(),
                        ) {
                            ctrl.report_db_error(
                                "update_palette_item_content",
                                &e,
                                Some("Couldn't save the note."),
                            );
                        }
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
                        match result {
                            Ok(_link) => {
                                // Refresh links on the board
                                let links = ctrl
                                    .metadata
                                    .borrow()
                                    .list_palette_links(palette_id)
                                    .unwrap_or_default();
                                brd.set_links(links);
                            }
                            Err(e) => ctrl.report_db_error(
                                "create_palette_link",
                                &e,
                                Some("Couldn't create the link."),
                            ),
                        }
                    }
                },
                // on_link_deleted
                {
                    let ctrl = Rc::clone(&controller);
                    move |id| {
                        if let Err(e) = ctrl.metadata.borrow_mut().delete_palette_link(id) {
                            ctrl.report_db_error(
                                "delete_palette_link",
                                &e,
                                Some("Couldn't remove the link."),
                            );
                        }
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

    pub(super) fn show_add_file_card_dialog(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
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

    pub(super) fn show_add_folder_card_dialog(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
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

    pub(super) fn add_note_card_to_board(self: &Rc<Self>, slot: PaneSlot, palette_id: i64) {
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

    pub(super) fn show_pin_folder_dialog(
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

    pub(super) fn remove_project_destination(
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

    pub(super) fn handle_activity_log_action(
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

    pub(super) fn show_place_context_menu(
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

    pub(super) fn show_sort_popover(self: &Rc<Self>, slot: PaneSlot, anchor: gtk::Widget) {
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

    pub(super) fn apply_sort(self: &Rc<Self>, slot: PaneSlot) {
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
        // Render from the borrow, then move the Vec into the cell (no second clone).
        self.pane_widgets(slot).file_grid.set_items(&items);
        self.items_cell(slot).replace(items);
        self.sync_active_tab_state();
    }

    pub(super) fn repeat_activity_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
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
                    false,
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
                    false,
                );
            }
            "duplicate" => self.do_duplicate_files(sources),
            "rename" | "bulk_rename" => {
                let renames = activity_renames(&entry);
                if renames.is_empty() {
                    self.status
                        .set_message("This rename entry has no names to repeat.");
                } else if renames.len() == 1 {
                    if let Some((path, name)) = renames.into_iter().next() {
                        self.rename_path(path, name);
                    }
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

    pub(super) fn undo_activity_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
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
                self.start_copy_move_op(undo_items, false, "Undo operation", None, None, false);
            }
            "trash" => self.restore_activity_trash_entry(entry),
            _ => self
                .status
                .set_message("Undo is not available for this activity entry."),
        }
    }

    pub(super) fn reveal_activity_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
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

    pub(super) fn copy_activity_entry_paths(self: &Rc<Self>, entry: ActivityLogEntry) {
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

    pub(super) fn restore_activity_trash_entry(self: &Rc<Self>, entry: ActivityLogEntry) {
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

    pub(super) fn refresh_metadata_sidebar(self: &Rc<Self>) {
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

    pub(super) fn refresh_cloud_sidebar(self: &Rc<Self>) {
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

    pub(super) fn refresh_drive_sidebar(self: &Rc<Self>) {
        let states = collect_removable_drives();
        let entries: Vec<DriveEntry> = states.iter().map(|s| s.entry.clone()).collect();
        self.removable_drives.replace(states);
        self.sidebar.set_removable_drives(&entries);

        for (entry, button) in self.sidebar.drive_buttons.borrow().clone() {
            let controller = Rc::clone(self);
            if entry.is_mounted {
                if let Some(path) = entry.path.clone() {
                    button.connect_clicked(move |_| controller.navigate_to_active(path.clone()));
                }
            } else {
                let name = entry.name.clone();
                button.connect_clicked(move |_| controller.mount_removable_by_name(name.clone()));
            }

            let controller = Rc::clone(self);
            let entry_for_menu = entry.clone();
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            gesture.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(widget) = gesture.widget() else {
                    return;
                };
                controller.show_drive_context_menu(entry_for_menu.clone(), widget, x, y);
            });
            button.add_controller(gesture);
        }
    }

    pub(super) fn mount_removable_by_name(self: &Rc<Self>, name: String) {
        let volume = self
            .removable_drives
            .borrow()
            .iter()
            .find(|s| s.entry.name == name && !s.entry.is_mounted)
            .and_then(|s| s.volume.clone());
        let Some(volume) = volume else { return };
        let mount_op = gio::MountOperation::new();
        let controller = Rc::clone(self);
        volume.mount(
            gio::MountMountFlags::NONE,
            Some(&mount_op),
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(()) => {
                    controller.refresh_drive_sidebar();
                    controller.status.set_message("Drive mounted.");
                }
                Err(e) => controller.status.set_message(&format!("Mount failed: {e}")),
            },
        );
    }

    pub(super) fn show_drive_context_menu(
        self: &Rc<Self>,
        entry: DriveEntry,
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

        let popover = Popover::new();
        popover.add_css_class("context-popover");
        popover.set_has_arrow(false);
        popover.set_parent(widget.upcast_ref::<gtk::Widget>());
        let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));

        if entry.is_mounted {
            let mount = self
                .removable_drives
                .borrow()
                .iter()
                .find(|s| s.entry.name == entry.name && s.entry.is_mounted)
                .and_then(|s| s.mount.clone());

            if let Some(mount) = mount {
                let unmount_btn = make_item("Unmount");
                let eject_btn = if mount.can_eject() {
                    Some(make_item("Eject"))
                } else {
                    None
                };
                menu.append(&unmount_btn);
                if let Some(ref b) = eject_btn {
                    menu.append(b);
                }

                let controller = Rc::clone(self);
                let m = mount.clone();
                let popover_c = popover.clone();
                unmount_btn.connect_clicked(move |_| {
                    popover_c.popdown();
                    let controller = Rc::clone(&controller);
                    let m = m.clone();
                    m.unmount_with_operation(
                        gio::MountUnmountFlags::NONE,
                        None::<&gio::MountOperation>,
                        None::<&gio::Cancellable>,
                        move |result| match result {
                            Ok(()) => {
                                controller.refresh_drive_sidebar();
                                controller.status.set_message("Drive unmounted.");
                            }
                            Err(e) => controller
                                .status
                                .set_message(&format!("Unmount failed: {e}")),
                        },
                    );
                });

                if let Some(eject_btn) = eject_btn {
                    let controller = Rc::clone(self);
                    let m = mount.clone();
                    let popover_c = popover.clone();
                    eject_btn.connect_clicked(move |_| {
                        popover_c.popdown();
                        let controller = Rc::clone(&controller);
                        let m = m.clone();
                        m.eject_with_operation(
                            gio::MountUnmountFlags::NONE,
                            None::<&gio::MountOperation>,
                            None::<&gio::Cancellable>,
                            move |result| match result {
                                Ok(()) => {
                                    controller.refresh_drive_sidebar();
                                    controller.status.set_message("Drive ejected.");
                                }
                                Err(e) => {
                                    controller.status.set_message(&format!("Eject failed: {e}"))
                                }
                            },
                        );
                    });
                }
            }
        } else {
            let mount_btn = make_item("Mount");
            menu.append(&mount_btn);
            let controller = Rc::clone(self);
            let name = entry.name.clone();
            let popover_c = popover.clone();
            mount_btn.connect_clicked(move |_| {
                popover_c.popdown();
                controller.mount_removable_by_name(name.clone());
            });
        }

        if menu.first_child().is_none() {
            return;
        }

        popover.set_child(Some(&menu));
        *self.context_popover.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    pub(super) fn open_cloud(self: &Rc<Self>, cloud_id: i64) {
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

    pub(super) fn load_cloud_landing_view(self: &Rc<Self>, slot: PaneSlot, cloud_id: i64) {
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

    pub(super) fn show_add_cloud_dialog(self: &Rc<Self>, prefill_path: Option<String>) {
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

    pub(super) fn show_edit_cloud_dialog(
        self: &Rc<Self>,
        cloud_id: i64,
        prefill_path: Option<String>,
    ) {
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

    pub(super) fn show_cloud_form_dialog<F>(
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

    pub(super) fn show_rclone_setup_dialog(self: &Rc<Self>) {
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
                    crate::ui::attach_tooltip(&copy_btn, format!("Copy: {mount_cmd}"));
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
                        format!("Pre-fill Add Cloud Drive with path: {mount_path}"),
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

    pub(super) fn show_remove_cloud_confirm(self: &Rc<Self>, cloud_id: i64) {
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

    pub(super) fn refresh_cloud_landing_availability(
        self: &Rc<Self>,
        cloud_id: i64,
        available: bool,
    ) {
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::CloudLanding(id) if id == cloud_id) {
                self.pane_widgets(slot)
                    .cloud_landing_panel
                    .set_availability(Some(available));
            }
        }
    }

    pub(super) fn set_cloud_landing_mount_busy(self: &Rc<Self>, cloud_id: i64, busy: bool) {
        for slot in [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary] {
            if matches!(self.current_view_for(slot), PaneView::CloudLanding(id) if id == cloud_id) {
                self.pane_widgets(slot)
                    .cloud_landing_panel
                    .set_mount_busy(busy);
            }
        }
    }

    pub(super) fn mount_cloud_profile(self: &Rc<Self>, cloud_id: i64) {
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
                    controller
                        .ops_panel
                        .finish_op(op_id, std::slice::from_ref(&err));
                    controller.refresh_cloud_landing_availability(cloud_id, false);
                    controller
                        .status
                        .set_message(&format!("Mount failed: {err}"));
                }
            }
        });
    }
}
