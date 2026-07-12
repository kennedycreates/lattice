//! Fourth quarter of the BrowserController inherent impl, split out of mod.rs.
//! Shares mod.rs's imports via globs; allow(unused_imports) since each half
//! uses only a subset.
#![allow(unused_imports)]

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
use super::*;
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

impl BrowserController {
    pub(super) fn show_add_to_tray_by_shape_popover(
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

    pub(super) fn show_action_plan<F>(self: &Rc<Self>, plan: ConfirmationPreview, on_accept: F)
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

    pub(super) fn apply_tag_to_tray_paths(self: &Rc<Self>, paths: Vec<PathBuf>, tag_name: String) {
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

    pub(super) fn move_tray_paths_to_trash(self: &Rc<Self>, paths: Vec<PathBuf>) {
        self.move_paths_to_trash_with_completion(
            paths,
            Some(TrayCompletion {
                action: "Move Tray to Trash".to_string(),
                clear_successful_paths: true,
            }),
        );
    }

    pub(super) fn record_tray_receipt(&self, action: &str, success_count: usize, failure_count: usize) {
        let detail = format!("{} succeeded · {} failed", success_count, failure_count);
        self.ops_panel
            .add_receipt(action, &detail, failure_count > 0);
        let errors = if failure_count > 0 {
            vec![format!("{failure_count} staged item(s) failed.")]
        } else {
            Vec::new()
        };
        log_activity_result(self.metadata.borrow().log_activity(
            "holding_tray",
            success_count as i32,
            action,
            "Holding Tray",
            None,
            &errors,
        ));
    }

    pub(super) fn update_selection(self: &Rc<Self>) {
        let slot = self.active_slot();
        let selected_items = self.selected_items_for(slot);
        let item_count = self.current_item_count_for(slot);
        let selected_count = selected_items.len();

        self.status.set_counts(item_count, selected_count);
        self.status.set_path(&self.display_label_for(slot));
        self.update_action_state();
        self.refresh_preview();
    }

    pub(super) fn update_action_state(&self) {
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

    pub(super) fn refresh_preview(self: &Rc<Self>) {
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
                if let Some(item) = selected_items.into_iter().next() {
                    self.preview
                        .show_loading(&item.name, &item.kind, "Loading preview…");
                    self.preview.set_action_state(true, true, true);
                    self.load_item_preview(item);
                }
            }
            count => {
                self.preview.show_multi_selection(count);
                self.preview.set_action_state(false, false, false);
            }
        }
    }

    pub(super) fn load_current_directory_preview(
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

    pub(super) fn load_item_preview(self: &Rc<Self>, item: FileItem) {
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

    pub(super) fn render_item_preview(
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

    pub(super) fn load_text_preview(
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

    pub(super) fn load_terroir_context_for_preview(self: &Rc<Self>, generation: u64, path: PathBuf) {
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

    pub(super) fn activate_index(self: &Rc<Self>, slot: PaneSlot, index: i32) {
        if let Some(item) = self.item_for_index(slot, index) {
            self.open_item_in_slot(slot, &item);
        }
    }

    pub(super) fn open_item(self: &Rc<Self>, item: &FileItem) {
        self.open_item_in_slot(self.active_slot(), item);
    }

    pub(super) fn open_item_in_slot(self: &Rc<Self>, slot: PaneSlot, item: &FileItem) {
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

    pub(super) fn open_folder_in_split(self: &Rc<Self>, path: PathBuf) {
        self.set_pane_layout(PaneLayout::Two);
        self.set_active_pane(PaneSlot::Secondary);
        self.navigate_to(PaneSlot::Secondary, path, true);
    }

    pub(super) fn open_folder_in_other_pane(self: &Rc<Self>, slot: PaneSlot, path: PathBuf) {
        if self.pane_layout.get() == PaneLayout::Single {
            self.open_folder_in_split(path);
            return;
        }

        let target = self.next_visible_slot(slot);
        self.set_active_pane(target);
        self.navigate_to(target, path, true);
    }

    pub(super) fn open_file(self: &Rc<Self>, path: &Path) {
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

    pub(super) fn show_open_with_dialog(self: &Rc<Self>, path: PathBuf) {
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
                    controller.show_error_dialog("Open With Failed", e.message());
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

    pub(super) fn open_created_text_document(self: &Rc<Self>, path: PathBuf) {
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

    pub(super) fn wire_tag_filters(self: &Rc<Self>) {
        self.wire_tag_filter_for_slot(PaneSlot::Primary);
        self.wire_tag_filter_for_slot(PaneSlot::Secondary);
        self.wire_tag_filter_for_slot(PaneSlot::Tertiary);
    }

    pub(super) fn wire_tag_filter_for_slot(self: &Rc<Self>, slot: PaneSlot) {
        let controller = Rc::clone(self);
        self.pane_widgets(slot)
            .tag_filter
            .connect_changed(move |spec| {
                controller.apply_tag_filter(slot, &spec);
                controller.sync_filter_button_state(slot);
            });
    }

    pub(super) fn set_filter_panel_open(self: &Rc<Self>, open: bool) {
        let slot = self.active_slot();
        self.set_filter_panel_open_for_slot(slot, open);
    }

    pub(super) fn set_filter_panel_open_for_slot(self: &Rc<Self>, slot: PaneSlot, open: bool) {
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

    pub(super) fn sync_filter_button_state(&self, slot: PaneSlot) {
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

    pub(super) fn apply_tag_filter(self: &Rc<Self>, slot: PaneSlot, spec: &TagFilterSpec) {
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

    pub(super) fn rename_selected(self: &Rc<Self>) {
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
                if let Some(item) = items.into_iter().next() {
                    self.show_rename_dialog(item.path, item.name);
                }
            }
            _ => self.open_bulk_naming_with_items(items),
        }
    }

    pub(super) fn apply_bulk_rename(self: &Rc<Self>, renames: Vec<(PathBuf, String)>) {
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

    pub(super) fn run_bulk_rename_step(
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
            log_activity_result(self.metadata.borrow().log_activity_with_items(
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
            ));
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
                            log_warn!("bulk rename metadata update failed: {e}");
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

    pub(super) fn show_rename_dialog(self: &Rc<Self>, path: PathBuf, current_name: String) {
        let controller = Rc::clone(self);
        self.modal_host.show_input(
            "Rename",
            "Choose a new name for the selected item.",
            &current_name,
            "Rename",
            move |name| controller.rename_path(path.clone(), name),
        );
    }

    pub(super) fn rename_path(self: &Rc<Self>, path: PathBuf, new_name: String) {
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
                    log_activity_result(controller.metadata.borrow().log_activity_with_items(
                        "rename",
                        1,
                        "Renamed item",
                        source,
                        new_path.parent().and_then(|parent| parent.to_str()),
                        &[],
                        &[(path.clone(), Some(new_path.clone()))],
                    ));
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

    pub(super) fn duplicate_selected(self: &Rc<Self>) {
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

    pub(super) fn do_duplicate_files(self: &Rc<Self>, sources: Vec<PathBuf>) {
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
        self.start_copy_move_op(items, true, &summary, None, Some("duplicate"), false);
    }

    pub(super) fn compress_selection_from_menu(self: &Rc<Self>, slot: PaneSlot, fallback_path: PathBuf) {
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

    pub(super) fn extract_archive_from_menu(self: &Rc<Self>, _slot: PaneSlot, archive: PathBuf) {
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

    pub(super) fn finish_archive_op(self: &Rc<Self>, op_id: OpId, result: ArchiveOpResult, op_kind: &str) {
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
                log_activity_result(self.metadata.borrow().log_activity_with_items(
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
                ));
            }
            self.refresh();
        } else {
            self.show_error_dialog("Archive Operation Failed", &errors.join("\n"));
        }
    }

    pub(super) fn create_new_folder(self: &Rc<Self>) {
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

    pub(super) fn create_new_text_document(self: &Rc<Self>) {
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

    pub(super) fn create_folder_named(self: &Rc<Self>, folder_name: String) {
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

    pub(super) fn exec_create_folder(self: &Rc<Self>, parent: PathBuf, folder_name: String) {
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
                    log_activity_result(controller.metadata.borrow().log_activity_with_items(
                        "new_folder",
                        1,
                        "Created folder",
                        parent.to_str().unwrap_or(""),
                        Some(parent.to_str().unwrap_or("")),
                        &[],
                        &[(parent.clone(), Some(folder_path.clone()))],
                    ));
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

    pub(super) fn create_text_document_named(self: &Rc<Self>, document_name: String) {
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

    pub(super) fn exec_create_text_document(
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
                                log_activity_result(controller.metadata.borrow().log_activity_with_items(
                                    "new_file",
                                    1,
                                    "Created text document",
                                    parent.to_str().unwrap_or(""),
                                    Some(parent.to_str().unwrap_or("")),
                                    &[],
                                    &[(parent.clone(), Some(document_path_for_close.clone()))],
                                ));
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

    pub(super) fn show_pin_project_dialog(self: &Rc<Self>, path: PathBuf) {
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

    pub(super) fn pin_project(self: &Rc<Self>, path: PathBuf, name: String) {
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

    pub(super) fn pin_place(self: &Rc<Self>, path: PathBuf) {
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

    pub(super) fn remove_place(self: &Rc<Self>, place_id: i64) {
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

    pub(super) fn show_add_tag_dialog(self: &Rc<Self>, paths: Vec<PathBuf>) {
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

    pub(super) fn apply_tag_to_paths(self: &Rc<Self>, paths: Vec<PathBuf>, tag_name: String) {
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

    pub(super) fn show_remove_tag_dialog(self: &Rc<Self>, paths: Vec<PathBuf>) {
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

    pub(super) fn remove_tags_from_paths(self: &Rc<Self>, paths: Vec<PathBuf>, tag_ids: Vec<i64>) {
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

    pub(super) fn show_send_to_project_dialog(self: &Rc<Self>, paths: Vec<PathBuf>) {
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

    pub(super) fn send_paths_to_project(
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

    pub(super) fn run_project_transfer(
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

    pub(super) fn show_project_conflict_dialog(
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

    pub(super) fn perform_project_transfer(
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

    pub(super) fn copy_selected_to_file_clipboard(&self, mode: ClipboardMode) {
        let slot = self.active_slot();
        let selected = self.selected_paths_for(slot);
        self.copy_paths_to_file_clipboard(selected, mode);
    }

    pub(super) fn copy_paths_to_file_clipboard(&self, paths: Vec<PathBuf>, mode: ClipboardMode) {
        let slot = self.active_slot();
        if paths.is_empty() {
            self.status
                .set_message("Select one or more items before using Cut or Copy.");
            return;
        }

        if mode == ClipboardMode::Cut
            && !action_availability(
                &self.current_view_for(slot),
                paths.len(),
                self.file_clipboard.borrow().is_some(),
            )
            .can_cut_files
        {
            self.status
                .set_message("Cut is not available in this view.");
            return;
        }

        self.file_clipboard
            .replace(FileClipboardState::new(paths.clone(), mode));
        let verb = match mode {
            ClipboardMode::Copy => "Copied",
            ClipboardMode::Cut => "Ready to move",
        };
        self.status.set_message(&format!(
            "{verb} {} item{}.",
            paths.len(),
            if paths.len() == 1 { "" } else { "s" }
        ));
    }

    pub(super) fn paste_file_clipboard_into_active_pane(self: &Rc<Self>) {
        let slot = self.active_slot();
        if self.file_clipboard.borrow().is_none() {
            self.status.set_message("Nothing is waiting to be pasted.");
            return;
        }

        let Some(destination) = self.paste_destination_for_slot(slot) else {
            self.status
                .set_message("Paste is not available in this view.");
            return;
        };
        self.paste_file_clipboard_into_destination(destination);
    }

    pub(super) fn paste_file_clipboard_into_destination(self: &Rc<Self>, destination: PathBuf) {
        let Some(clipboard) = self.file_clipboard.borrow().clone() else {
            self.status.set_message("Nothing is waiting to be pasted.");
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

    pub(super) fn paste_destination_for_slot(&self, slot: PaneSlot) -> Option<PathBuf> {
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

    pub(super) fn update_file_clipboard_after_batch(
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

    pub(super) fn trash_selected(self: &Rc<Self>) {
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

    pub(super) fn move_paths_to_trash(self: &Rc<Self>, paths: Vec<PathBuf>) {
        self.move_paths_to_trash_with_completion(paths, None);
    }

    pub(super) fn move_paths_to_trash_with_completion(
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

    pub(super) fn run_trash_op(
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
            log_activity_result(self.metadata.borrow().log_activity_with_items(
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
            ));
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

    pub(super) fn confirm_permanent_delete(self: &Rc<Self>, paths: Vec<PathBuf>) {
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

    pub(super) fn delete_items_permanently(self: &Rc<Self>, paths: Vec<PathBuf>) {
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

    pub(super) fn run_delete_op(
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
            log_activity_result(self.metadata.borrow().log_activity_with_items(
                "permanent_delete",
                total as i32,
                &summary,
                source,
                None,
                &errs,
                &activity_items,
            ));
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

    pub(super) fn copy_paths_to_clipboard(self: &Rc<Self>, paths: Vec<PathBuf>) {
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

    pub(super) fn copy_path_text_for_active_context(self: &Rc<Self>) {
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

    pub(super) fn open_terminal_for_path(self: &Rc<Self>, path: PathBuf, is_dir: bool) {
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

    pub(super) fn open_current_folder_terminal(self: &Rc<Self>) {
        self.open_terminal_for_path(self.current_dir_for(self.active_slot()), true);
    }

    pub(super) fn run_custom_action_by_id(self: &Rc<Self>, action_id: &str) {
        let Some(action) = self.config.custom_action(action_id).cloned() else {
            let message = format!("Custom action '{action_id}' is not configured.");
            self.status.set_message(&message);
            return;
        };
        let paths = custom_action_paths_for_context(self, self.active_slot());
        self.run_custom_action(&action, paths);
    }

    pub(super) fn run_custom_action(self: &Rc<Self>, action: &CustomActionConfig, paths: Vec<PathBuf>) {
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

    pub(super) fn open_preview_target(self: &Rc<Self>) {
        if let Some(item) = self.selected_single_item() {
            self.open_item(&item);
        }
    }

    pub(super) fn copy_preview_target_path(self: &Rc<Self>) {
        self.copy_path_text_for_active_context();
    }

    pub(super) fn open_preview_parent(self: &Rc<Self>) {
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

    pub(super) fn selected_items(&self) -> Vec<FileItem> {
        self.selected_items_for(self.active_slot())
    }

    pub(super) fn selected_items_for(&self, slot: PaneSlot) -> Vec<FileItem> {
        self.pane_widgets(slot)
            .file_grid
            .selected_indices()
            .into_iter()
            .filter_map(|index| self.item_for_index(slot, index))
            .collect()
    }

    pub(super) fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_paths_for(self.active_slot())
    }

    pub(super) fn selected_paths_for(&self, slot: PaneSlot) -> Vec<PathBuf> {
        self.selected_items_for(slot)
            .into_iter()
            .filter(|item| !item.path.as_os_str().is_empty())
            .map(|item| item.path)
            .collect()
    }

    pub(super) fn selected_single_item(&self) -> Option<FileItem> {
        self.selected_single_item_for(self.active_slot())
    }

    pub(super) fn selected_single_item_for(&self, slot: PaneSlot) -> Option<FileItem> {
        let items = self.selected_items_for(slot);
        if items.len() == 1 {
            items.into_iter().next()
        } else {
            None
        }
    }

    pub(super) fn update_navigation_state(&self) {
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

    pub(super) fn update_sidebar_state(&self) {
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
                            .find(|s| {
                                s.entry
                                    .path
                                    .as_ref()
                                    .is_some_and(|p| current.starts_with(p))
                            })
                            .and_then(|s| s.entry.path.clone())
                            .map(SidebarTarget::Drive)
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

    pub(super) fn item_for_index(&self, slot: PaneSlot, index: i32) -> Option<FileItem> {
        if index < 0 {
            return None;
        }

        self.items_cell(slot).borrow().get(index as usize).cloned()
    }

    pub(super) fn reveal_pending_selection(&self, slot: PaneSlot) -> bool {
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

    pub(super) fn display_path(&self, path: &Path) -> String {
        format_path(path, &self.places.home)
    }

    pub(super) fn sync_path_entry_to_display(&self) {
        let display_path = self.display_label_for(self.active_slot());
        self.toolbar.path_entry.set_text(&display_path);
        self.toolbar.set_breadcrumb_path(&display_path);
        self.toolbar.show_breadcrumb_mode();
    }

    pub(super) fn resolve_path_input(&self, input: &str) -> Option<gio::File> {
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

    pub(super) fn thumb_loader_for(&self, slot: PaneSlot) -> &crate::thumbnail::ThumbnailLoader {
        match slot {
            PaneSlot::Primary => &self.primary_thumb_loader,
            PaneSlot::Secondary => &self.secondary_thumb_loader,
            PaneSlot::Tertiary => &self.tertiary_thumb_loader,
        }
    }

    pub(super) fn cancel_active_load(&self, slot: PaneSlot) {
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

    pub(super) fn cancel_active_preview(&self) {
        if let Some(cancellable) = self.preview_cancellable.borrow_mut().take() {
            cancellable.cancel();
        }
    }

    pub(super) fn is_current_load(&self, slot: PaneSlot, generation: u64) -> bool {
        self.load_generation_cell(slot).get() == generation
    }

    pub(super) fn is_current_preview(&self, generation: u64) -> bool {
        self.preview_generation.get() == generation
    }

    pub(super) fn show_error_dialog(&self, title: &str, detail: &str) {
        self.modal_host.show_error(title, detail);
    }

    // ── Drag-and-drop ────────────────────────────────────────────────

    /// Called once from bootstrap. Adds a pane-level DropTarget that catches
    /// drops anywhere in the pane and deposits files into the current folder.
    pub(super) fn attach_pane_dnd(self: &Rc<Self>, slot: PaneSlot) {
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
    pub(super) fn attach_sidebar_place_dnd(self: &Rc<Self>, button: gtk::Button, dest_path: PathBuf) {
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
    pub(super) fn attach_holding_tray_dnd(self: &Rc<Self>) {
        // Tray staging is handled from each grid/list DragSource's end/cancel
        // callbacks. Avoiding tray DropTargets keeps GTK's internal
        // drag-autoscroll path out of this non-file-operation staging flow.
    }

    pub(super) fn finish_drag_to_holding_tray(self: &Rc<Self>, drag: &gdk::Drag, paths: &[PathBuf]) -> bool {
        if paths.is_empty() || !self.drag_is_over_holding_tray(drag) {
            return false;
        }

        self.add_paths_to_holding_tray(paths.to_vec());
        drag.drop_done(true);
        true
    }

    pub(super) fn drag_is_over_holding_tray(&self, drag: &gdk::Drag) -> bool {
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

    pub(super) fn set_holding_tray_drag_active(&self, active: bool) {
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
    pub(super) fn attach_item_dnd(self: &Rc<Self>, slot: PaneSlot) {
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

                let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
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
                        paths.iter().map(gio::File::for_path).collect();
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
    pub(super) fn handle_dnd_drop(
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
            false,
        );
    }

    /// Check for conflicts, show the resolver if needed, then start the batch op.
    pub(super) fn start_copy_move_with_conflict_check(
        self: &Rc<Self>,
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        is_copy: bool,
        label_hint: String,
        clipboard_state: Option<FileClipboardState>,
        clear_tray_on_move: bool,
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
            self.start_copy_move_op(
                non_conflicting,
                is_copy,
                &label,
                clipboard_state,
                None,
                clear_tray_on_move,
            );
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
            ctrl.start_copy_move_op(
                items,
                is_copy,
                &label,
                clipboard_state.clone(),
                None,
                clear_tray_on_move,
            );
        });
    }

    pub(super) fn start_copy_move_op(
        self: &Rc<Self>,
        items: Vec<(PathBuf, PathBuf, gio::FileCopyFlags)>,
        is_copy: bool,
        label: &str,
        clipboard_state: Option<FileClipboardState>,
        activity_operation: Option<&'static str>,
        clear_tray_on_move: bool,
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
            clear_tray_on_move,
        );
    }

    pub(super) fn run_copy_move_batch(
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
        clear_tray_on_move: bool,
    ) {
        let total = items.len();

        if index >= total {
            let errs = errors.borrow().clone();
            self.update_file_clipboard_after_batch(
                clipboard_state.as_ref(),
                &moved_sources.borrow(),
            );
            if clear_tray_on_move && !is_copy {
                self.remove_holding_tray_paths(&moved_sources.borrow());
            }
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
            log_activity_result(self.metadata.borrow().log_activity_with_items(
                op,
                n,
                &summary,
                &source,
                dest.as_deref(),
                &errs,
                &activity_items,
            ));
            self.reload_visible_panes();
            return;
        }

        if cancellable.is_cancelled() {
            self.update_file_clipboard_after_batch(
                clipboard_state.as_ref(),
                &moved_sources.borrow(),
            );
            if clear_tray_on_move && !is_copy {
                self.remove_holding_tray_paths(&moved_sources.borrow());
            }
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
                            log_warn!("metadata move update failed: {error}");
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
                clear_tray_on_move,
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
                    clear_tray_on_move,
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
