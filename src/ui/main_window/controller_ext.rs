//! Second half of the BrowserController inherent impl, split out of mod.rs.
//! Shares mod.rs's imports via globs; allow(unused_imports) since each half
//! uses only a subset.
#![allow(unused_imports)]

use super::action_preview::*;
use super::activity::*;
use super::archive::*;
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
use crate::action_plan::{ActionPlan as FileOpPlan, OpKind as FileOpKind, RestoreSpec};
use crate::config::{shortcut_tooltip, AppConfig, CustomActionConfig};
use crate::converter::{
    cleanup_orphaned_temps_in, ConversionQueue, ConvertItem, ConvertSettings, MediaKind,
};
use crate::metadata::{
    ActivityLogEntry, CloudRecord, FolderViewState, MetadataStore, PlaceRecord, ProjectRecord,
    Shape, TagRecord,
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
    search_panel::{SearchPanel, SearchQuery},
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
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea, Entry,
    FlowBox, HeaderBar, Image, Label, ListBox, ListBoxRow, Orientation, Paned, Popover, Revealer,
    Separator,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

impl BrowserController {
    pub(super) fn queue_plan(self: &Rc<Self>, mut plan: crate::action_plan::ActionPlan) {
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

    pub(super) fn should_queue_actions(&self) -> bool {
        should_queue_actions_state(self.plan_mode_active.get(), self.executing_plan_queue.get())
    }

    pub(super) fn refresh_plan_queue_panel(self: &Rc<Self>) {
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

    pub(super) fn execute_plan_queue(self: &Rc<Self>) {
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
                        false,
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
                            self.start_copy_move_op(
                                items,
                                is_copy,
                                &plan.summary,
                                None,
                                None,
                                false,
                            );
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

    pub(super) fn apply_mark_to_paths_direct(
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
            log_activity_result(
                meta.log_activity(
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
                ),
            );
        }
        self.reload_active_tab();
    }

    pub(super) fn reset_mark_for_paths_direct(self: &Rc<Self>, paths: Vec<PathBuf>) {
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
            log_activity_result(
                meta.log_activity(
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
                ),
            );
        }
        self.reload_active_tab();
    }

    pub(super) fn reset_mark_recursive(self: &Rc<Self>, folder_paths: Vec<PathBuf>) {
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

    pub(super) fn apply_folder_view_state(self: &Rc<Self>, slot: PaneSlot, path: &Path) {
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

    pub(super) fn save_folder_view_state_for(&self, slot: PaneSlot) {
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

    pub(super) fn load_directory(self: &Rc<Self>, slot: PaneSlot, path: PathBuf) {
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

    pub(super) fn read_directory_batch(
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

    pub(super) fn finish_load(
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
                log_warn!("recent-location update failed: {error}");
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
        let displayed_len = displayed.len();
        // Render from the borrow, then move the Vec into the cell (no clone).
        self.pane_widgets(slot).file_grid.set_items(&displayed);
        self.items_cell(slot).replace(displayed);
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
                self.show_empty_selection_preview(slot, &display_path, displayed_len);
                self.status.set_counts(displayed_len, 0);
                self.refresh_preview();
                // Give the grid keyboard focus so Ctrl+A / arrows work without a click
                // first — but only when focus isn't in a text field, sidebar, or tray
                // (e.g. don't steal focus while the user is typing in the path box).
                if matches!(
                    self.focused_context(),
                    FocusedContext::Window | FocusedContext::FileGrid
                ) {
                    self.pane_widgets(slot).file_grid.grab_focus_on_active();
                }
            }
        }
    }

    pub(super) fn finish_load_error(
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

    pub(super) fn load_triage(self: &Rc<Self>, slot: PaneSlot, root: &Path) {
        self.current_dir_cell(slot).replace(root.to_path_buf());
        self.load_directory(slot, root.to_path_buf());
    }

    pub(super) fn open_system_drives(self: &Rc<Self>) {
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

    pub(super) fn load_system_drives_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn open_recent(self: &Rc<Self>) {
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

    pub(super) fn load_recent_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn open_trash(self: &Rc<Self>) {
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

    pub(super) fn load_trash_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn read_trash_batch(
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

    pub(super) fn finish_trash_load(
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

    pub(super) fn finish_trash_load_error(
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

    pub(super) fn restore_items_from_trash(self: &Rc<Self>, items: Vec<FileItem>) {
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

    pub(super) fn run_restore_batch(
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

    pub(super) fn empty_trash(self: &Rc<Self>) {
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

    pub(super) fn do_empty_trash(self: &Rc<Self>) {
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

    pub(super) fn collect_trash_children_then_delete(
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

    pub(super) fn delete_trash_batch(
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

    pub(super) fn exit_search_if_empty(self: &Rc<Self>, slot: PaneSlot) -> bool {
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

    pub(super) fn run_path_box_search(self: &Rc<Self>) {
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

    pub(super) fn open_search_in_current_dir(self: &Rc<Self>) {
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

    pub(super) fn open_search(self: &Rc<Self>, slot: PaneSlot, query: SearchQuery) {
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

    pub(super) fn rerun_search(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn load_search_view(self: &Rc<Self>, slot: PaneSlot, query: SearchQuery) {
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

    pub(super) fn finish_search_load(
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

    pub(super) fn connect_search_panels(self: &Rc<Self>) {
        self.connect_search_panel(PaneSlot::Primary);
        self.connect_search_panel(PaneSlot::Secondary);
        self.connect_search_panel(PaneSlot::Tertiary);
    }

    pub(super) fn connect_search_panel(self: &Rc<Self>, slot: PaneSlot) {
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
    pub(super) fn wire_search_tag_buttons(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn wire_search_mark_buttons(self: &Rc<Self>, slot: PaneSlot) {
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
    pub(super) fn refresh_search_tag_buttons(self: &Rc<Self>, slot: PaneSlot) {
        if !matches!(self.current_view_for(slot), PaneView::Search(_)) {
            return;
        }
        let tags = self.tags.borrow().clone();
        self.pane_widgets(slot).search_panel.set_tags(&tags);
        self.wire_search_tag_buttons(slot);
    }

    pub(super) fn load_tag_view(self: &Rc<Self>, slot: PaneSlot, tag: TagRecord) {
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

    pub(super) fn query_tagged_paths(
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
                                    .map(|value| value.to_unix()),
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

    pub(super) fn finish_virtual_load(
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

    pub(super) fn enrich_items_with_tags(&self, items: Vec<FileItem>) -> Vec<FileItem> {
        self.enrich_items(items)
    }

    pub(super) fn enrich_items(&self, mut items: Vec<FileItem>) -> Vec<FileItem> {
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

    pub(super) fn preview_identity_for_item(
        &self,
        item: &FileItem,
    ) -> (String, Shape, Option<String>, Vec<TagRecord>) {
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        let (tint_name, tint_color) =
            tint_name_and_color(&tints, item.mark_tint_id, item.mark_tint_color.clone());
        (tint_name, item.mark_shape, tint_color, item.tags.clone())
    }

    pub(super) fn preview_identity_for_path(
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

    pub(super) fn init_tint_css(self: &Rc<Self>) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        gtk::style_context_add_provider_for_display(
            &display,
            &self.tint_css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    }

    pub(super) fn apply_tint_css(self: &Rc<Self>) {
        let tints = self.metadata.borrow().list_tints().unwrap_or_default();
        let css = generate_tint_css(&tints);
        self.tint_css_provider.load_from_string(&css);
    }

    pub(super) fn show_empty_selection_preview(
        &self,
        slot: PaneSlot,
        display_label: &str,
        item_count: usize,
    ) {
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

    pub(super) fn attach_context_handlers(self: &Rc<Self>, slot: PaneSlot) {
        let _ = slot;
        // Right-click is handled once at the FlowBox/ListBox container level in
        // `connect_pane()`. Keeping it there avoids GTK selection/focus races
        // between row wrappers and child widgets.
    }

    pub(super) fn show_context_menu(
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

    pub(super) fn icon_item_at_scroll_point(
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

    pub(super) fn list_item_at_scroll_point(
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

    pub(super) fn show_current_folder_menu(
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

    pub(super) fn item_context_entries(&self, is_dir: bool) -> Vec<String> {
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
                    "cut",
                    "copy",
                    "paste",
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
                    "cut",
                    "copy",
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

    pub(super) fn background_context_entries(&self) -> Vec<String> {
        self.config
            .context_menu
            .background
            .clone()
            .unwrap_or_else(|| {
                [
                    "new_folder",
                    "new_text_document",
                    "paste",
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

    pub(super) fn append_item_context_menu_entries(
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
                "cut" => append_menu_button(menu_box, "Cut", Some("edit-cut-symbolic"), false, {
                    let controller = Rc::clone(self);
                    let item = item.clone();
                    move || {
                        controller.copy_paths_to_file_clipboard(
                            vec![item.path.clone()],
                            ClipboardMode::Cut,
                        )
                    }
                }),
                "copy" => {
                    append_menu_button(menu_box, "Copy", Some("edit-copy-symbolic"), false, {
                        let controller = Rc::clone(self);
                        let item = item.clone();
                        move || {
                            controller.copy_paths_to_file_clipboard(
                                vec![item.path.clone()],
                                ClipboardMode::Copy,
                            )
                        }
                    })
                }
                "paste" if item.is_dir && self.file_clipboard.borrow().is_some() => {
                    append_menu_button(
                        menu_box,
                        "Paste Into Folder",
                        Some("edit-paste-symbolic"),
                        false,
                        {
                            let controller = Rc::clone(self);
                            let item = item.clone();
                            move || {
                                controller.paste_file_clipboard_into_destination(item.path.clone())
                            }
                        },
                    )
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
                                        std::slice::from_ref(&item.path),
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

    pub(super) fn append_background_context_menu_entries(
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
                "paste" if self.file_clipboard.borrow().is_some() => {
                    append_menu_button(menu_box, "Paste", Some("edit-paste-symbolic"), false, {
                        let controller = Rc::clone(self);
                        move || controller.paste_file_clipboard_into_active_pane()
                    })
                }
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

    pub(super) fn append_custom_context_action(
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

    pub(super) fn append_custom_background_action(
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

    pub(super) fn dismiss_context_menu(&self) -> bool {
        if let Some(popover) = self.context_popover.borrow_mut().take() {
            if popover.parent().is_some() {
                popover.unparent();
            }
            return true;
        }
        false
    }

    pub(super) fn add_selection_to_holding_tray(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn add_paths_to_holding_tray(self: &Rc<Self>, paths: Vec<PathBuf>) {
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

    pub(super) fn paste_file_clipboard_into_holding_tray(self: &Rc<Self>) {
        let Some(clipboard) = self.file_clipboard.borrow().clone() else {
            self.status
                .set_message("Nothing is waiting to be added to the Holding Tray.");
            return;
        };
        self.add_paths_to_holding_tray(clipboard.paths);
    }

    /// Move or copy the staged tray items into the currently viewed folder.
    /// On a move, successfully moved items are removed from the tray.
    pub(super) fn send_tray_to_current_folder(self: &Rc<Self>, is_copy: bool) {
        let slot = self.active_slot();
        let paths = self.holding_tray_paths();
        if paths.is_empty() {
            self.status.set_message("The Holding Tray is empty.");
            return;
        }

        let Some(dest) = self.paste_destination_for_slot(slot) else {
            self.status
                .set_message("Open a folder first — tray items can't be sent to this view.");
            return;
        };

        // Skip items already sitting in the destination or that would move into
        // themselves (mirrors handle_dnd_drop).
        let sources: Vec<PathBuf> = paths
            .into_iter()
            .filter(|src| {
                let already_there = src.parent().map(|p| p == dest).unwrap_or(false);
                let into_self = dest.starts_with(src);
                !already_there && !into_self
            })
            .collect();
        if sources.is_empty() {
            self.status
                .set_message("Tray items are already in this folder.");
            return;
        }

        let title = if is_copy {
            "Copy to current folder"
        } else {
            "Move to current folder"
        };

        if self.should_queue_actions() {
            self.queue_plan(
                FileOpPlan::for_paste(&sources, &dest, is_copy)
                    .with_tray_completion(title, /* clear_successful_paths = */ !is_copy),
            );
            return;
        }

        self.start_copy_move_with_conflict_check(
            sources,
            dest,
            is_copy,
            String::new(),
            None,
            /* clear_tray_on_move = */ !is_copy,
        );
    }

    pub(super) fn file_item_for_known_or_local_path(&self, path: &Path) -> Option<FileItem> {
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

    pub(super) fn refresh_holding_tray(self: &Rc<Self>) {
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

    pub(super) fn remove_holding_tray_path(self: &Rc<Self>, path: &Path) {
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

    pub(super) fn remove_holding_tray_paths(self: &Rc<Self>, paths: &[PathBuf]) {
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

    pub(super) fn clear_holding_tray(self: &Rc<Self>) {
        self.holding_tray_items.borrow_mut().clear();
        self.holding_tray_selection.borrow_mut().clear();
        self.refresh_holding_tray();
        self.status
            .set_message("Holding Tray cleared. No files were deleted.");
    }

    pub(super) fn holding_tray_paths(&self) -> Vec<PathBuf> {
        self.holding_tray_items
            .borrow()
            .iter()
            .map(|item| item.path.clone())
            .collect()
    }

    pub(super) fn selected_holding_tray_paths(&self) -> Vec<PathBuf> {
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

    pub(super) fn select_holding_tray_path(self: &Rc<Self>, path: PathBuf) {
        self.holding_tray_selection.replace(vec![path]);
        self.refresh_holding_tray();
    }

    pub(super) fn clear_holding_tray_selection(self: &Rc<Self>) {
        if self.holding_tray_selection.borrow().is_empty() {
            return;
        }
        self.holding_tray_selection.borrow_mut().clear();
        self.refresh_holding_tray();
        self.status.set_message("Holding Tray selection cleared.");
    }

    pub(super) fn remove_selected_holding_tray_items(self: &Rc<Self>) {
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

    pub(super) fn open_selected_holding_tray_item(self: &Rc<Self>) {
        let Some(path) = self.holding_tray_selection.borrow().first().cloned() else {
            self.status
                .set_message("Select a staged item before opening it.");
            return;
        };
        self.open_holding_tray_path(path);
    }

    pub(super) fn open_holding_tray_path(self: &Rc<Self>, path: PathBuf) {
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

    pub(super) fn show_tray_project_dialog(self: &Rc<Self>, action: TrayProjectAction) {
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

    pub(super) fn show_tray_tag_preview(self: &Rc<Self>) {
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

    pub(super) fn show_tray_trash_preview(self: &Rc<Self>) {
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

    pub(super) fn copy_holding_tray_paths(self: &Rc<Self>) {
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

    pub(super) fn show_tray_apply_mark_preview(self: &Rc<Self>) {
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

    pub(super) fn show_tray_reset_mark_preview(self: &Rc<Self>) {
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

    pub(super) fn show_add_to_tray_by_tint_popover(
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
}
