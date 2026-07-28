//! Third quarter of the BrowserController inherent impl, split out of mod.rs.
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
    pub(super) fn unmount_cloud_profile(self: &Rc<Self>, cloud_id: i64) {
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
                    controller
                        .ops_panel
                        .finish_op(op_id, std::slice::from_ref(&err));
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

    pub(super) fn show_cloud_context_menu(
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

    pub(super) fn is_in_cloud_location(&self, path: &Path) -> bool {
        self.cloud_locations.borrow().iter().any(|loc| {
            let loc_path = std::path::Path::new(&loc.path);
            path.starts_with(loc_path)
        })
    }

    pub(super) fn cloud_name_for_path(&self, path: &Path) -> Option<(String, String)> {
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

    pub(super) fn cloud_summary(&self, summary: &str, path: &Path) -> String {
        if self.cloud_name_for_path(path).is_some() {
            format!("☁ {summary}")
        } else {
            summary.to_string()
        }
    }

    pub(super) fn open_project(self: &Rc<Self>, project_id: i64) {
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

    pub(super) fn open_tag(self: &Rc<Self>, tag_id: i64) {
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

    pub(super) fn open_triage(self: &Rc<Self>, root: PathBuf, filter: TriageFilter) {
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

    pub(super) fn set_triage_filter(self: &Rc<Self>, slot: PaneSlot, filter: TriageFilter) {
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

    pub(super) fn triage_active_folder(self: &Rc<Self>) {
        let slot = self.active_slot();
        let root = self.tool_scope_dir_for(slot);
        self.open_triage(root, TriageFilter::All);
    }

    pub(super) fn open_bulk_naming_tool(self: &Rc<Self>) {
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

    pub(super) fn open_bulk_naming_with_items(self: &Rc<Self>, items: Vec<FileItem>) {
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

    pub(super) fn open_media_convert_with_items(
        self: &Rc<Self>,
        slot: PaneSlot,
        items: Vec<FileItem>,
    ) {
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
    pub(super) fn cleanup_convert_temps(self: &Rc<Self>) {
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

    pub(super) fn connect_media_convert_actions(self: &Rc<Self>) {
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
                cp.wire_copy_buttons();
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

    pub(super) fn load_bulk_naming_view(self: &Rc<Self>, slot: PaneSlot, root: PathBuf) {
        let recursive = self.pane_widgets(slot).bulk_naming_panel.recursive_active();
        self.load_bulk_naming_folder(slot, root, recursive);
    }

    pub(super) fn load_bulk_naming_folder(
        self: &Rc<Self>,
        slot: PaneSlot,
        root: PathBuf,
        recursive: bool,
    ) {
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

    pub(super) fn finish_bulk_naming_load(
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

    pub(super) fn load_bulk_naming_items(
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

    pub(super) fn connect_bulk_naming_panel(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn sibling_names_outside_selection(
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

    pub(super) fn start_duplicate_scan(self: &Rc<Self>, slot: PaneSlot, root: PathBuf) {
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

    pub(super) fn update_view_strip(&self, slot: PaneSlot) {
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

    pub(super) fn load_current_view(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn connect_tab_strip(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.tab_strip
            .new_tab_button
            .connect_clicked(move |_| controller.open_new_tab(None));
    }

    pub(super) fn connect_preview_actions(self: &Rc<Self>) {
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

    pub(super) fn connect_panes(self: &Rc<Self>) {
        self.connect_pane(PaneSlot::Primary);
        self.connect_pane(PaneSlot::Secondary);
        self.connect_pane(PaneSlot::Tertiary);
    }

    pub(super) fn connect_pane(self: &Rc<Self>, slot: PaneSlot) {
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

        // Custom rubber-band (marquee) selection for both view modes.
        self.attach_marquee(slot, false);
        self.attach_marquee(slot, true);

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

    pub(super) fn handle_window_command(self: &Rc<Self>, command: WindowCommand) -> bool {
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

    pub(super) fn handle_escape(self: &Rc<Self>, focus: FocusedContext) -> bool {
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

    pub(super) fn handle_holding_tray_key(
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

    /// Attach a Capture-phase drag gesture to a pane's grid container to drive
    /// custom rubber-band selection. `is_list` picks the ListBox vs FlowBox.
    pub(super) fn attach_marquee(self: &Rc<Self>, slot: PaneSlot, is_list: bool) {
        let container: gtk::Widget = if is_list {
            self.pane_widgets(slot).file_grid.list_box.clone().upcast()
        } else {
            self.pane_widgets(slot).file_grid.flow.clone().upcast()
        };

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);

        let controller = Rc::clone(self);
        drag.connect_drag_begin(move |gesture, x, y| {
            controller.marquee_drag_begin(slot, is_list, gesture, x, y);
        });

        let controller = Rc::clone(self);
        drag.connect_drag_update(move |gesture, ox, oy| {
            controller.marquee_drag_update(slot, gesture, ox, oy);
        });

        let controller = Rc::clone(self);
        drag.connect_drag_end(move |_, _, _| {
            controller.marquee_drag_end(slot);
        });

        container.add_controller(drag);
    }

    pub(super) fn marquee_drag_begin(
        self: &Rc<Self>,
        slot: PaneSlot,
        is_list: bool,
        gesture: &gtk::GestureDrag,
        x: f64,
        y: f64,
    ) {
        // Paint mode owns left-drags when a stroke tool is active.
        if self.paint_mode_active.get() && self.active_paint_tool.get() != PaintTool::Cursor {
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }

        let grid = &self.pane_widgets(slot).file_grid;
        let on_item = if is_list {
            grid.list_box.row_at_y(y as i32).is_some()
        } else {
            grid.flow.child_at_pos(x as i32, y as i32).is_some()
        };
        if on_item {
            // Let the per-item DragSource and native click-select handle presses on
            // items; only empty space starts a marquee.
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }

        // Empty space: claim the sequence to suppress the native FlowBox rubber-band
        // and drive our own selection.
        gesture.set_state(gtk::EventSequenceState::Claimed);
        self.set_active_pane(slot);
        let state = gesture.current_event_state();
        let additive = state.contains(gdk::ModifierType::CONTROL_MASK)
            || state.contains(gdk::ModifierType::SHIFT_MASK);
        let base = grid.selected_indices();
        *self.marquee.borrow_mut() = Some(MarqueeSession {
            start: (x, y),
            base,
            additive,
            moved: false,
        });
    }

    pub(super) fn marquee_drag_update(
        self: &Rc<Self>,
        slot: PaneSlot,
        _gesture: &gtk::GestureDrag,
        offset_x: f64,
        offset_y: f64,
    ) {
        let (start, base, additive) = {
            let mut guard = self.marquee.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            if offset_x.abs() + offset_y.abs() > 4.0 {
                session.moved = true;
            }
            (session.start, session.base.clone(), session.additive)
        };

        let cx = start.0 + offset_x;
        let cy = start.1 + offset_y;
        let x0 = start.0.min(cx);
        let y0 = start.1.min(cy);
        let w = (cx - start.0).abs();
        let h = (cy - start.1).abs();

        let grid = self.pane_widgets(slot).file_grid.clone();
        let rect = gtk::graphene::Rect::new(x0 as f32, y0 as f32, w as f32, h as f32);
        let hits = grid.children_in_rect(&rect);
        let mut selection: Vec<i32> = if additive { base } else { Vec::new() };
        for hit in hits {
            if !selection.contains(&hit) {
                selection.push(hit);
            }
        }
        grid.set_selected_indices(&selection);

        // Map the container-space rectangle to overlay coordinates for painting.
        let container = grid.active_container();
        if let Some(p0) =
            container.compute_point(&grid.root, &gtk::graphene::Point::new(x0 as f32, y0 as f32))
        {
            grid.set_marquee_rect(Some((p0.x() as f64, p0.y() as f64, w, h)));
        }
    }

    pub(super) fn marquee_drag_end(self: &Rc<Self>, slot: PaneSlot) {
        let Some(session) = self.marquee.borrow_mut().take() else {
            return;
        };
        let grid = self.pane_widgets(slot).file_grid.clone();
        grid.set_marquee_rect(None);
        // A bare click on empty space (no drag) clears the selection — this restores
        // the native behavior we suppressed by claiming the gesture.
        if !session.moved && !session.additive {
            grid.clear_selection();
        }
        self.sync_keyboard_state_from_selection(slot);
        if slot == self.active_slot() {
            self.update_selection();
        }
    }

    pub(super) fn handle_file_grid_key(
        self: &Rc<Self>,
        focus: FocusedContext,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> bool {
        // Also accept the bare Window context so Ctrl+A / arrows / space / Enter work
        // immediately after navigation even when no grid widget holds GTK focus yet.
        if !matches!(focus, FocusedContext::FileGrid | FocusedContext::Window) {
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

    pub(super) fn activate_keyboard_target(self: &Rc<Self>) {
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

    pub(super) fn move_grid_keyboard_selection(
        self: &Rc<Self>,
        offset: i32,
        ctrl: bool,
        shift: bool,
    ) {
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

    pub(super) fn apply_space_selection(self: &Rc<Self>, ctrl: bool, shift: bool) {
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

    pub(super) fn set_keyboard_focus(&self, slot: PaneSlot, index: i32, update_anchor: bool) {
        self.keyboard_current_cell(slot).set(Some(index));
        if update_anchor {
            self.keyboard_anchor_cell(slot).set(Some(index));
        }
        self.pane_widgets(slot).file_grid.focus_index(index);
    }

    pub(super) fn select_only_in_slot(self: &Rc<Self>, slot: PaneSlot, index: i32) {
        self.pane_widgets(slot).file_grid.select_only_index(index);
        self.set_keyboard_focus(slot, index, true);
        if slot == self.active_slot() {
            self.update_selection();
        }
    }

    pub(super) fn select_range_in_slot(
        &self,
        slot: PaneSlot,
        anchor: i32,
        target: i32,
        clear_first: bool,
    ) {
        self.pane_widgets(slot)
            .file_grid
            .select_range(anchor, target, clear_first);
        self.pane_widgets(slot).file_grid.focus_index(target);
    }

    pub(super) fn select_all_in_active_pane(self: &Rc<Self>) {
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

    pub(super) fn clear_selection_in_active_pane(self: &Rc<Self>) {
        let slot = self.active_slot();
        self.pane_widgets(slot).file_grid.clear_selection();
        self.reset_keyboard_state(slot);
        self.update_selection();
        self.pane_widgets(slot).file_grid.grab_focus_on_active();
    }

    /// Log a database/metadata write failure and, for user-initiated actions,
    /// surface a short message so the failure isn't silent. Pass `None` for
    /// `user_msg` when the write is background bookkeeping.
    pub(super) fn report_db_error(&self, context: &str, err: &str, user_msg: Option<&str>) {
        log_err!("{context}: {err}");
        if let Some(msg) = user_msg {
            self.status.set_message(msg);
        }
    }

    pub(super) fn reset_keyboard_state(&self, slot: PaneSlot) {
        self.keyboard_anchor_cell(slot).set(None);
        self.keyboard_current_cell(slot).set(None);
    }

    pub(super) fn sync_keyboard_state_from_selection(&self, slot: PaneSlot) {
        let selected = self.pane_widgets(slot).file_grid.selected_indices();
        if let Some(index) = selected.first().copied() {
            self.keyboard_current_cell(slot).set(Some(index));
            if selected.len() == 1 || self.keyboard_anchor_cell(slot).get().is_none() {
                self.keyboard_anchor_cell(slot).set(Some(index));
            }
        } else {
            self.reset_keyboard_state(slot);
        }
    }

    pub(super) fn cycle_active_pane(self: &Rc<Self>) {
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

    pub(super) fn switch_tab_relative(self: &Rc<Self>, offset: i32) {
        let count = self.tabs.borrow().len();
        if count <= 1 {
            return;
        }

        let current = self.active_tab.get() as i32;
        let next = (current + offset).rem_euclid(count as i32) as usize;
        self.switch_to_tab(next);
    }

    pub(super) fn pane_widgets(&self, slot: PaneSlot) -> &PaneWidgets {
        match slot {
            PaneSlot::Primary => &self.primary_pane,
            PaneSlot::Secondary => &self.secondary_pane,
            PaneSlot::Tertiary => &self.tertiary_pane,
        }
    }

    pub(super) fn duplicate_set_cell(
        &self,
        slot: PaneSlot,
    ) -> &RefCell<Option<std::collections::HashSet<PathBuf>>> {
        match slot {
            PaneSlot::Primary => &self.primary_duplicate_set,
            PaneSlot::Secondary => &self.secondary_duplicate_set,
            PaneSlot::Tertiary => &self.tertiary_duplicate_set,
        }
    }

    pub(super) fn duplicate_scan_pending_for(&self, slot: PaneSlot) -> bool {
        match slot {
            PaneSlot::Primary => self.primary_duplicate_scan_pending.get(),
            PaneSlot::Secondary => self.secondary_duplicate_scan_pending.get(),
            PaneSlot::Tertiary => self.tertiary_duplicate_scan_pending.get(),
        }
    }

    pub(super) fn set_duplicate_scan_pending(&self, slot: PaneSlot, pending: bool) {
        match slot {
            PaneSlot::Primary => self.primary_duplicate_scan_pending.set(pending),
            PaneSlot::Secondary => self.secondary_duplicate_scan_pending.set(pending),
            PaneSlot::Tertiary => self.tertiary_duplicate_scan_pending.set(pending),
        }
    }

    pub(super) fn current_dir_cell(&self, slot: PaneSlot) -> &RefCell<PathBuf> {
        match slot {
            PaneSlot::Primary => &self.current_dir,
            PaneSlot::Secondary => &self.secondary_current_dir,
            PaneSlot::Tertiary => &self.tertiary_current_dir,
        }
    }

    pub(super) fn current_view_cell(&self, slot: PaneSlot) -> &RefCell<PaneView> {
        match slot {
            PaneSlot::Primary => &self.current_view,
            PaneSlot::Secondary => &self.secondary_view,
            PaneSlot::Tertiary => &self.tertiary_view,
        }
    }

    pub(super) fn back_history_cell(&self, slot: PaneSlot) -> &RefCell<Vec<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.back_history,
            PaneSlot::Secondary => &self.secondary_back_history,
            PaneSlot::Tertiary => &self.tertiary_back_history,
        }
    }

    // Save the current directory to back history when entering a tool view from a directory.
    // Tool-to-tool transitions are intentionally ignored so back always returns to the last
    // real folder, not a phantom home path set by a previous tool.
    pub(super) fn save_dir_to_history_if_in_directory(&self, slot: PaneSlot) {
        if let PaneView::Directory(path) = self.current_view_for(slot) {
            self.back_history_cell(slot).borrow_mut().push(path);
            self.forward_history_cell(slot).borrow_mut().clear();
        }
    }

    pub(super) fn forward_history_cell(&self, slot: PaneSlot) -> &RefCell<Vec<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.forward_history,
            PaneSlot::Secondary => &self.secondary_forward_history,
            PaneSlot::Tertiary => &self.tertiary_forward_history,
        }
    }

    pub(super) fn items_cell(&self, slot: PaneSlot) -> &RefCell<Vec<FileItem>> {
        match slot {
            PaneSlot::Primary => &self.items,
            PaneSlot::Secondary => &self.secondary_items,
            PaneSlot::Tertiary => &self.tertiary_items,
        }
    }

    pub(super) fn all_items_cell(&self, slot: PaneSlot) -> &RefCell<Vec<FileItem>> {
        match slot {
            PaneSlot::Primary => &self.primary_all_items,
            PaneSlot::Secondary => &self.secondary_all_items,
            PaneSlot::Tertiary => &self.tertiary_all_items,
        }
    }

    pub(super) fn pending_reveal_cell(&self, slot: PaneSlot) -> &RefCell<Option<PathBuf>> {
        match slot {
            PaneSlot::Primary => &self.pending_reveal_path,
            PaneSlot::Secondary => &self.secondary_pending_reveal_path,
            PaneSlot::Tertiary => &self.tertiary_pending_reveal_path,
        }
    }

    pub(super) fn load_generation_cell(&self, slot: PaneSlot) -> &Cell<u64> {
        match slot {
            PaneSlot::Primary => &self.load_generation,
            PaneSlot::Secondary => &self.secondary_load_generation,
            PaneSlot::Tertiary => &self.tertiary_load_generation,
        }
    }

    pub(super) fn load_cancellable_cell(
        &self,
        slot: PaneSlot,
    ) -> &RefCell<Option<gio::Cancellable>> {
        match slot {
            PaneSlot::Primary => &self.load_cancellable,
            PaneSlot::Secondary => &self.secondary_load_cancellable,
            PaneSlot::Tertiary => &self.tertiary_load_cancellable,
        }
    }

    pub(super) fn search_cancel_cell(&self, slot: PaneSlot) -> &RefCell<Option<Arc<AtomicBool>>> {
        match slot {
            PaneSlot::Primary => &self.primary_search_cancel,
            PaneSlot::Secondary => &self.secondary_search_cancel,
            PaneSlot::Tertiary => &self.tertiary_search_cancel,
        }
    }

    pub(super) fn keyboard_anchor_cell(&self, slot: PaneSlot) -> &Cell<Option<i32>> {
        match slot {
            PaneSlot::Primary => &self.primary_keyboard_anchor,
            PaneSlot::Secondary => &self.secondary_keyboard_anchor,
            PaneSlot::Tertiary => &self.tertiary_keyboard_anchor,
        }
    }

    pub(super) fn keyboard_current_cell(&self, slot: PaneSlot) -> &Cell<Option<i32>> {
        match slot {
            PaneSlot::Primary => &self.primary_keyboard_current,
            PaneSlot::Secondary => &self.secondary_keyboard_current,
            PaneSlot::Tertiary => &self.tertiary_keyboard_current,
        }
    }

    pub(super) fn current_dir_for(&self, slot: PaneSlot) -> PathBuf {
        self.current_dir_cell(slot).borrow().clone()
    }

    pub(super) fn current_view_for(&self, slot: PaneSlot) -> PaneView {
        self.current_view_cell(slot).borrow().clone()
    }

    pub(super) fn tool_scope_dir_for(&self, slot: PaneSlot) -> PathBuf {
        resolve_tool_scope_dir(
            &self.current_view_for(slot),
            &self.current_dir_for(slot),
            &self.places.home,
        )
    }

    pub(super) fn is_directory_view(&self, slot: PaneSlot) -> bool {
        matches!(self.current_view_for(slot), PaneView::Directory(_))
    }

    pub(super) fn display_label_for(&self, slot: PaneSlot) -> String {
        view_display_label(&self.current_view_for(slot), &self.places.home)
    }

    pub(super) fn current_item_count_for(&self, slot: PaneSlot) -> usize {
        self.items_cell(slot).borrow().len()
    }

    pub(super) fn active_slot(&self) -> PaneSlot {
        self.active_pane.get()
    }

    pub(super) fn view_mode_cell(&self, slot: PaneSlot) -> &Cell<ViewMode> {
        match slot {
            PaneSlot::Primary => &self.primary_view_mode,
            PaneSlot::Secondary => &self.secondary_view_mode,
            PaneSlot::Tertiary => &self.tertiary_view_mode,
        }
    }

    pub(super) fn show_hidden_cell(&self, slot: PaneSlot) -> &Cell<bool> {
        match slot {
            PaneSlot::Primary => &self.primary_show_hidden,
            PaneSlot::Secondary => &self.secondary_show_hidden,
            PaneSlot::Tertiary => &self.tertiary_show_hidden,
        }
    }

    pub(super) fn show_shape_badges_cell(&self, slot: PaneSlot) -> &Cell<bool> {
        match slot {
            PaneSlot::Primary => &self.primary_show_shape_badges,
            PaneSlot::Secondary => &self.secondary_show_shape_badges,
            PaneSlot::Tertiary => &self.tertiary_show_shape_badges,
        }
    }

    pub(super) fn badges_hidden_by_paint_cell(&self, slot: PaneSlot) -> &Cell<bool> {
        match slot {
            PaneSlot::Primary => &self.primary_badges_hidden_by_paint,
            PaneSlot::Secondary => &self.secondary_badges_hidden_by_paint,
            PaneSlot::Tertiary => &self.tertiary_badges_hidden_by_paint,
        }
    }

    pub(super) fn sort_field_cell(&self, slot: PaneSlot) -> &Cell<SortField> {
        match slot {
            PaneSlot::Primary => &self.primary_sort_field,
            PaneSlot::Secondary => &self.secondary_sort_field,
            PaneSlot::Tertiary => &self.tertiary_sort_field,
        }
    }

    pub(super) fn sort_direction_cell(&self, slot: PaneSlot) -> &Cell<SortDirection> {
        match slot {
            PaneSlot::Primary => &self.primary_sort_direction,
            PaneSlot::Secondary => &self.secondary_sort_direction,
            PaneSlot::Tertiary => &self.tertiary_sort_direction,
        }
    }

    pub(super) fn set_view_mode(self: &Rc<Self>, slot: PaneSlot, mode: ViewMode) {
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

    pub(super) fn set_show_hidden_for_slot(self: &Rc<Self>, slot: PaneSlot, show_hidden: bool) {
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

    pub(super) fn sync_show_hidden_button_state(&self, slot: PaneSlot) {
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

    pub(super) fn set_show_shape_badges_for_slot(
        self: &Rc<Self>,
        slot: PaneSlot,
        show_badges: bool,
    ) {
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

    pub(super) fn sync_show_shape_badges_button_state(&self, slot: PaneSlot) {
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

    pub(super) fn visible_slots(&self) -> Vec<PaneSlot> {
        [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary]
            .into_iter()
            .filter(|slot| self.pane_layout.get().includes(*slot))
            .collect()
    }

    pub(super) fn next_visible_slot(&self, slot: PaneSlot) -> PaneSlot {
        let visible = self.visible_slots();
        let current_index = visible
            .iter()
            .position(|candidate| *candidate == slot)
            .unwrap_or(0);
        visible[(current_index + 1) % visible.len()]
    }

    pub(super) fn sync_active_tab_state(&self) {
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

    pub(super) fn rebuild_tab_strip(self: &Rc<Self>) {
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

    pub(super) fn open_new_tab(self: &Rc<Self>, path: Option<PathBuf>) {
        self.sync_active_tab_state();
        let target = path.unwrap_or_else(|| self.current_dir_for(self.active_slot()));
        self.tabs.borrow_mut().push(TabState::new(target));
        let new_index = self.tabs.borrow().len().saturating_sub(1);
        self.active_tab.set(new_index);
        self.reload_active_tab();
    }

    pub(super) fn close_tab(self: &Rc<Self>, index: usize) {
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

    pub(super) fn switch_to_tab(self: &Rc<Self>, index: usize) {
        if index == self.active_tab.get() || index >= self.tabs.borrow().len() {
            return;
        }

        self.sync_active_tab_state();
        self.active_tab.set(index);
        self.reload_active_tab();
    }

    pub(super) fn reload_active_tab(self: &Rc<Self>) {
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

    pub(super) fn update_active_pane_visuals(&self) {
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

    pub(super) fn sync_pane_layout_visibility(&self) {
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

    pub(super) fn update_split_button_state(&self) {
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

    pub(super) fn cycle_pane_layout(self: &Rc<Self>) {
        let next = self.pane_layout.get().next();
        self.set_pane_layout(next);
    }

    pub(super) fn set_pane_layout(self: &Rc<Self>, layout: PaneLayout) {
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

    pub(super) fn set_active_pane(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn update_selection_for(self: &Rc<Self>, slot: PaneSlot) {
        self.sync_keyboard_state_from_selection(slot);
        if slot == self.active_slot() {
            self.update_selection();
        }
    }

    pub(super) fn navigate_to(
        self: &Rc<Self>,
        slot: PaneSlot,
        path: PathBuf,
        remember_current: bool,
    ) {
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

    pub(super) fn navigate_to_active(self: &Rc<Self>, path: PathBuf) {
        self.navigate_to(self.active_slot(), path, true);
    }

    pub(super) fn go_back(self: &Rc<Self>) {
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

    pub(super) fn go_forward(self: &Rc<Self>) {
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

    pub(super) fn go_up(self: &Rc<Self>) {
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

    pub(super) fn refresh(self: &Rc<Self>) {
        self.reload_visible_panes();
    }

    pub(super) fn reload_visible_panes(self: &Rc<Self>) {
        for slot in self.visible_slots() {
            self.load_current_view(slot);
        }
    }

    pub(super) fn navigate_from_path_entry(self: &Rc<Self>) {
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

    pub(super) fn begin_path_entry_editing(self: &Rc<Self>) {
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

    pub(super) fn finish_path_entry_editing(&self) {
        if let Some(id) = self.path_box_debounce.borrow_mut().take() {
            id.remove();
        }
        self.sync_path_entry_to_display();
    }

    pub(super) fn cancel_path_entry_editing(self: &Rc<Self>) {
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

    pub(super) fn apply_sidebar_visibility(&self, visible: bool) {
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

    pub(super) fn set_sidebar_visible(self: &Rc<Self>, visible: bool) {
        self.apply_sidebar_visibility(visible);
    }

    pub(super) fn apply_preview_visibility(self: &Rc<Self>, visible: bool) {
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

    pub(super) fn set_preview_visible(self: &Rc<Self>, visible: bool) {
        self.apply_preview_visibility(visible);
    }

    pub(super) fn set_holding_tray_visible(self: &Rc<Self>, visible: bool) {
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

    pub(super) fn set_plan_mode(self: &Rc<Self>, active: bool) {
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

    pub(super) fn set_paint_mode(self: &Rc<Self>, active: bool) {
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

    pub(super) fn on_paint_tint_changed(self: &Rc<Self>, tint_id: i64) {
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

    pub(super) fn on_paint_tag_changed(self: &Rc<Self>, tag_id: i64) {
        let tags = self.metadata.borrow().list_tags().unwrap_or_default();
        if let Some(t) = tags.iter().find(|t| t.id == tag_id) {
            self.active_paint_tag_id.set(tag_id);
            *self.active_paint_tag_name.borrow_mut() = t.name.clone();
            self.painting_toolbar.set_active_tag_display(&t.name);
            self.painting_toolbar.set_tags(&tags, tag_id);
        }
    }

    pub(super) fn dispatch_paint_tool(self: &Rc<Self>, slot: PaneSlot, index: i32) {
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

    pub(super) fn paint_brush_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
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
            self.log_paint_mark(slot, std::slice::from_ref(&item.path), tint_id, shape);
        }
    }

    pub(super) fn paint_eraser_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
        if self.should_queue_actions() {
            self.queue_plan(FileOpPlan::for_reset_mark(
                std::slice::from_ref(&item.path),
                false,
            ));
            return;
        }
        let prev = self.read_explicit_mark(&item.path);
        if let Err(e) = self.metadata.borrow_mut().clear_file_mark(&item.path) {
            self.report_db_error("clear_file_mark", &e, Some("Couldn't clear the mark."));
        }
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
        self.log_erase_mark(slot, std::slice::from_ref(&item.path));
    }

    pub(super) fn paint_eyedropper_item(self: &Rc<Self>, item: &FileItem) {
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

    pub(super) fn paint_fill_selection(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn paint_tag_brush_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
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

    pub(super) fn paint_tag_eraser_item(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
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

    pub(super) fn paint_tag_eyedropper_item(self: &Rc<Self>, item: &FileItem) {
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

    pub(super) fn paint_tag_fill_selection(self: &Rc<Self>, slot: PaneSlot) {
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

    pub(super) fn update_item_mark_in_grid(
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

    pub(super) fn update_item_tags_in_grid(
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

    pub(super) fn tint_color_for_id(&self, tint_id: i64) -> Option<String> {
        self.metadata
            .borrow()
            .list_tints()
            .unwrap_or_default()
            .into_iter()
            .find(|tint| tint.id == tint_id)
            .and_then(|tint| tint.color)
    }

    pub(super) fn log_paint_mark(
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

    pub(super) fn log_erase_mark(self: &Rc<Self>, slot: PaneSlot, paths: &[PathBuf]) {
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
    pub(super) fn read_explicit_mark(&self, path: &PathBuf) -> Option<(i64, Shape)> {
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
    pub(super) fn append_or_commit_history(&self, entry: PaintHistoryEntry) -> bool {
        let mut acc = self.drag_history_accumulator.borrow_mut();
        if let Some(ref mut entries) = *acc {
            entries.push(entry);
            true
        } else {
            false
        }
    }

    /// Push a completed step onto the undo stack and clear the redo stack.
    pub(super) fn commit_paint_history(&self, step: PaintHistoryStep) {
        let mut undo = self.paint_undo_stack.borrow_mut();
        undo.push(step);
        if undo.len() > PAINT_HISTORY_LIMIT {
            undo.remove(0);
        }
        drop(undo);
        self.paint_redo_stack.borrow_mut().clear();
        self.refresh_paint_undo_redo_state();
    }

    pub(super) fn refresh_paint_undo_redo_state(&self) {
        let can_undo = !self.paint_undo_stack.borrow().is_empty();
        let can_redo = !self.paint_redo_stack.borrow().is_empty();
        self.painting_toolbar.set_undo_enabled(can_undo);
        self.painting_toolbar.set_redo_enabled(can_redo);
    }

    pub(super) fn paint_undo(self: &Rc<Self>) {
        let step = self.paint_undo_stack.borrow_mut().pop();
        let Some(step) = step else { return };
        let slot = self.active_slot();
        self.apply_paint_history_step(slot, &step, true);
        self.paint_redo_stack.borrow_mut().push(step);
        self.refresh_paint_undo_redo_state();
        self.status.set_message("Paint undo.");
    }

    pub(super) fn paint_redo(self: &Rc<Self>) {
        let step = self.paint_redo_stack.borrow_mut().pop();
        let Some(step) = step else { return };
        let slot = self.active_slot();
        self.apply_paint_history_step(slot, &step, false);
        self.paint_undo_stack.borrow_mut().push(step);
        self.refresh_paint_undo_redo_state();
        self.status.set_message("Paint redo.");
    }

    /// Apply a history step (undo = restore `prev`; redo = restore `next`).
    pub(super) fn apply_paint_history_step(
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

    pub(super) fn paint_folder_with_preview(self: &Rc<Self>, slot: PaneSlot, folder_path: PathBuf) {
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

    pub(super) fn do_paint_folder_recursive(
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
}
