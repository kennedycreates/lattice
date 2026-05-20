use crate::config::{shortcut_tooltip, AppConfig};
use crate::converter::{
    all_presets, plan_batch, ConversionBatch, ConversionPreset, ConvertItem, ConvertSettings,
    MediaKind, OutputConflictPolicy, OutputLocationMode, ToolAvailability,
};
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, DropDown, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow, StringList, ToggleButton,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum ConvertSourceMode {
    #[default]
    Auto,
    Tray,
    Selection,
}

struct State {
    items: RefCell<Vec<ConvertItem>>,
    selected_preset_idx: Cell<usize>,
    output_mode: RefCell<OutputLocationMode>,
    conflict_policy: RefCell<OutputConflictPolicy>,
    tools: RefCell<ToolAvailability>,
    source_mode: Cell<ConvertSourceMode>,
    tool_warning_row: GtkBox,
    tool_warning_label: Label,
    file_list: ListBox,
    preset_dropdown: DropDown,
    convert_button: Button,
    summary_label: Label,
    output_chosen_label: Label,
    /// Kept to programmatically revert the toggle group when folder pick is cancelled.
    btn_next: ToggleButton,
    on_start: RefCell<Option<Box<dyn Fn(ConversionBatch)>>>,
    on_folder_pick: RefCell<Option<Box<dyn Fn()>>>,
    on_settings_changed:
        RefCell<Option<Box<dyn Fn(&str, &OutputLocationMode, &OutputConflictPolicy)>>>,
    on_source_mode_changed: RefCell<Option<Box<dyn Fn(ConvertSourceMode)>>>,
}

impl State {
    fn update_preview(self: &Rc<Self>) {
        while let Some(child) = self.file_list.first_child() {
            self.file_list.remove(&child);
        }

        let items = self.items.borrow();
        let presets = all_presets();
        let preset_idx = self.selected_preset_idx.get();
        let preset = presets.get(preset_idx);

        if items.is_empty() {
            let empty = Label::new(Some("No media files selected."));
            empty.add_css_class("convert-empty");
            empty.set_halign(Align::Start);
            empty.set_margin_top(12);
            empty.set_margin_start(12);
            let row = ListBoxRow::new();
            row.set_selectable(false);
            row.set_child(Some(&empty));
            self.file_list.append(&row);
            self.summary_label.set_label("No files.");
            self.convert_button.set_sensitive(false);
            return;
        }

        let compatible_count = if let Some(preset) = preset {
            items.iter().filter(|item| item.kind == preset.kind).count()
        } else {
            0
        };

        const PREVIEW_LIMIT: usize = 200;
        let output_mode = self.output_mode.borrow();
        let hidden = items.len().saturating_sub(PREVIEW_LIMIT);

        for item in items.iter().take(PREVIEW_LIMIT) {
            let row = ListBoxRow::new();
            row.add_css_class("convert-file-row");
            row.set_selectable(false);

            let body = GtkBox::new(Orientation::Horizontal, 8);
            body.add_css_class("convert-file-row-body");
            body.set_margin_start(10);
            body.set_margin_end(10);
            body.set_margin_top(4);
            body.set_margin_bottom(4);

            let kind_badge = Label::new(Some(kind_badge(item.kind)));
            kind_badge.add_css_class("convert-file-row-badge");
            body.append(&kind_badge);

            let source_name = item
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let src_label = Label::new(Some(&source_name));
            src_label.add_css_class("convert-file-row-name");
            src_label.set_halign(Align::Start);
            src_label.set_hexpand(true);
            src_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            body.append(&src_label);

            let arrow = Label::new(Some("→"));
            arrow.add_css_class("convert-file-row-arrow");
            body.append(&arrow);

            let dest_text = if let Some(preset) = preset {
                if item.kind == preset.kind {
                    dest_preview_name(&item.path, preset, &output_mode)
                } else {
                    "(skipped)".to_string()
                }
            } else {
                "—".to_string()
            };

            let dest_label = Label::new(Some(&dest_text));
            dest_label.add_css_class("convert-file-row-dest");
            dest_label.set_halign(Align::End);
            dest_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            body.append(&dest_label);

            row.set_child(Some(&body));
            self.file_list.append(&row);
        }

        if hidden > 0 {
            let more_row = ListBoxRow::new();
            more_row.set_selectable(false);
            let more_label = Label::new(Some(&format!(
                "…and {hidden} more file{} not shown",
                if hidden == 1 { "" } else { "s" }
            )));
            more_label.add_css_class("convert-more-label");
            more_label.set_halign(Align::Start);
            more_label.set_margin_start(12);
            more_label.set_margin_top(4);
            more_label.set_margin_bottom(4);
            more_row.set_child(Some(&more_label));
            self.file_list.append(&more_row);
        }

        let total = items.len();
        let skipped = total - compatible_count;
        let summary = if compatible_count == 0 {
            if skipped > 0 {
                format!(
                    "None of the {skipped} selected file{} match this preset — choose a different preset.",
                    if skipped == 1 { "" } else { "s" }
                )
            } else {
                "No files selected.".to_string()
            }
        } else if skipped > 0 {
            format!(
                "{compatible_count} file{} will convert; {skipped} skipped (wrong type for this preset). Originals are never modified.",
                if compatible_count == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "{compatible_count} file{} ready to convert. Originals are never modified.",
                if compatible_count == 1 { "" } else { "s" }
            )
        };
        self.summary_label.set_label(&summary);

        let tools = self.tools.borrow();
        let tool_ok = preset.map_or(true, |p| p.tool_available(&tools));
        self.convert_button
            .set_sensitive(compatible_count > 0 && tool_ok);
    }

    fn update_tool_warning(self: &Rc<Self>) {
        let presets = all_presets();
        let preset = presets.get(self.selected_preset_idx.get());
        let tools = self.tools.borrow();

        let warning: Option<String> = match preset {
            None => None,
            Some(p) if p.tool_available(&tools) => None,
            Some(p) => {
                let name = p.tool.display_name();
                let hint = p.tool.install_hint();
                Some(format!(
                    "{name} not found — conversion unavailable.\n{hint}"
                ))
            }
        };

        if let Some(msg) = warning {
            self.tool_warning_label.set_label(&msg);
            self.tool_warning_row.set_visible(true);
        } else {
            self.tool_warning_row.set_visible(false);
        }
    }

    fn build_batch(&self) -> ConversionBatch {
        let items = self.items.borrow();
        let presets = all_presets();
        let preset = match presets.get(self.selected_preset_idx.get()) {
            Some(p) => p,
            None => {
                // Return empty batch with a dummy preset
                return ConversionBatch {
                    jobs: Vec::new(),
                    preset: &presets[0],
                    output_mode: OutputLocationMode::NextToSource,
                    conflict_policy: OutputConflictPolicy::AutoRename,
                };
            }
        };

        let output_mode = self.output_mode.borrow().clone();
        let conflict_policy = self.conflict_policy.borrow().clone();
        let tools = self.tools.borrow().clone();
        let paths: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();

        // Create output subdirectory when using subfolder mode
        if let OutputLocationMode::Subfolder(ref name) = output_mode {
            let dirs: HashSet<PathBuf> = paths
                .iter()
                .filter_map(|p| p.parent())
                .map(|p| p.join(name))
                .collect();
            for dir in dirs {
                let _ = std::fs::create_dir_all(&dir);
            }
        }

        plan_batch(
            &paths,
            preset,
            output_mode,
            conflict_policy,
            &tools,
            &HashMap::new(),
        )
    }
}

#[derive(Clone)]
pub struct MediaConvertPanel {
    pub root: GtkBox,
    state: Rc<State>,
}

impl MediaConvertPanel {
    pub fn build(config: &AppConfig) -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("convert-panel");
        root.set_vexpand(true);
        root.set_hexpand(true);

        // Tool warning row
        let tool_warning_row = GtkBox::new(Orientation::Horizontal, 6);
        tool_warning_row.add_css_class("convert-tool-warning");
        let tool_warning_label = Label::new(Some("ffmpeg not found."));
        tool_warning_label.add_css_class("convert-tool-warning-label");
        tool_warning_label.set_halign(Align::Start);
        tool_warning_label.set_hexpand(true);
        tool_warning_label.set_wrap(true);
        tool_warning_row.append(&tool_warning_label);
        tool_warning_row.set_visible(false);
        root.append(&tool_warning_row);

        // Source row
        let source_row = GtkBox::new(Orientation::Horizontal, 6);
        source_row.add_css_class("convert-source-row");

        let src_label = Label::new(Some("Source:"));
        src_label.add_css_class("convert-section-label");
        source_row.append(&src_label);

        let btn_src_auto = ToggleButton::with_label("Auto");
        btn_src_auto.add_css_class("convert-option-btn");
        btn_src_auto.set_active(true);
        crate::ui::attach_tooltip(
            &btn_src_auto,
            "Use tray if items are present, otherwise file selection",
        );

        let btn_src_tray = ToggleButton::with_label("Tray");
        btn_src_tray.add_css_class("convert-option-btn");
        btn_src_tray.set_group(Some(&btn_src_auto));
        crate::ui::attach_tooltip(&btn_src_tray, "Always use holding tray items");

        let btn_src_sel = ToggleButton::with_label("Selection");
        btn_src_sel.add_css_class("convert-option-btn");
        btn_src_sel.set_group(Some(&btn_src_auto));
        crate::ui::attach_tooltip(&btn_src_sel, "Always use file grid selection");

        source_row.append(&btn_src_auto);
        source_row.append(&btn_src_tray);
        source_row.append(&btn_src_sel);
        root.append(&source_row);

        // Preset toolbar
        let toolbar = GtkBox::new(Orientation::Horizontal, 8);
        toolbar.add_css_class("convert-toolbar");

        let preset_label = Label::new(Some("Preset:"));
        preset_label.add_css_class("convert-preset-label");
        toolbar.append(&preset_label);

        let preset_names: Vec<&str> = all_presets().iter().map(|p| p.label).collect();
        let preset_dropdown = DropDown::builder()
            .model(&StringList::new(&preset_names))
            .hexpand(true)
            .build();
        preset_dropdown.add_css_class("convert-preset-dropdown");
        crate::ui::attach_tooltip(&preset_dropdown, "Choose conversion preset");
        toolbar.append(&preset_dropdown);
        root.append(&toolbar);

        // Output location row
        let output_row = GtkBox::new(Orientation::Horizontal, 6);
        output_row.add_css_class("convert-output-row");

        let out_label = Label::new(Some("Output:"));
        out_label.add_css_class("convert-section-label");
        output_row.append(&out_label);

        let btn_next = ToggleButton::with_label("Next to originals");
        btn_next.add_css_class("convert-option-btn");
        btn_next.set_active(true);
        crate::ui::attach_tooltip(&btn_next, "Save beside originals");

        let btn_subfolder = ToggleButton::with_label("Converted subfolder");
        btn_subfolder.add_css_class("convert-option-btn");
        btn_subfolder.set_group(Some(&btn_next));
        crate::ui::attach_tooltip(&btn_subfolder, "Save in Converted folders");

        let btn_choose = ToggleButton::with_label("Choose folder…");
        btn_choose.add_css_class("convert-option-btn");
        btn_choose.set_group(Some(&btn_next));
        crate::ui::attach_tooltip(&btn_choose, "Choose output folder");

        let output_chosen_label = Label::new(None);
        output_chosen_label.add_css_class("convert-chosen-folder-label");
        output_chosen_label.set_halign(Align::Start);
        output_chosen_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        output_chosen_label.set_max_width_chars(28);
        output_chosen_label.set_visible(false);

        output_row.append(&btn_next);
        output_row.append(&btn_subfolder);
        output_row.append(&btn_choose);
        output_row.append(&output_chosen_label);
        root.append(&output_row);

        // Conflict policy row
        let conflict_row = GtkBox::new(Orientation::Horizontal, 6);
        conflict_row.add_css_class("convert-conflict-row");

        let conf_label = Label::new(Some("Conflict:"));
        conf_label.add_css_class("convert-section-label");
        conflict_row.append(&conf_label);

        let btn_rename = ToggleButton::with_label("Auto-rename");
        btn_rename.add_css_class("convert-option-btn");
        btn_rename.set_active(true);
        crate::ui::attach_tooltip(&btn_rename, "Keep both files");

        let btn_skip = ToggleButton::with_label("Skip existing");
        btn_skip.add_css_class("convert-option-btn");
        btn_skip.set_group(Some(&btn_rename));
        crate::ui::attach_tooltip(&btn_skip, "Skip conflicts");

        let btn_overwrite = ToggleButton::with_label("Overwrite");
        btn_overwrite.add_css_class("convert-option-btn");
        btn_overwrite.set_group(Some(&btn_rename));
        crate::ui::attach_tooltip(&btn_overwrite, "Replace conflicts");

        conflict_row.append(&btn_rename);
        conflict_row.append(&btn_skip);
        conflict_row.append(&btn_overwrite);
        root.append(&conflict_row);

        // File list
        let file_list = ListBox::new();
        file_list.add_css_class("convert-file-list");
        file_list.set_selection_mode(gtk::SelectionMode::None);
        let scroll = ScrolledWindow::builder()
            .child(&file_list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .build();
        scroll.add_css_class("convert-scroll");
        root.append(&scroll);

        // Footer
        let footer = GtkBox::new(Orientation::Horizontal, 8);
        footer.add_css_class("convert-action-row");

        let summary_label = Label::new(Some("No files loaded."));
        summary_label.add_css_class("convert-summary");
        summary_label.set_halign(Align::Start);
        summary_label.set_hexpand(true);
        footer.append(&summary_label);

        let convert_button = Button::with_label("Convert");
        convert_button.add_css_class("convert-start-button");
        convert_button.set_sensitive(false);
        crate::ui::attach_tooltip(
            &convert_button,
            shortcut_tooltip(config, "Start conversion", "convert_start"),
        );
        footer.append(&convert_button);
        root.append(&footer);

        let state = Rc::new(State {
            items: RefCell::new(Vec::new()),
            selected_preset_idx: Cell::new(0),
            output_mode: RefCell::new(OutputLocationMode::NextToSource),
            conflict_policy: RefCell::new(OutputConflictPolicy::AutoRename),
            tools: RefCell::new(ToolAvailability {
                ffmpeg: true,
                ffprobe: false,
                imagemagick: false,
                vips: false,
            }),
            tool_warning_row,
            tool_warning_label,
            file_list,
            preset_dropdown,
            convert_button,
            summary_label,
            output_chosen_label,
            btn_next: btn_next.clone(),
            source_mode: Cell::new(ConvertSourceMode::Auto),
            on_start: RefCell::new(None),
            on_folder_pick: RefCell::new(None),
            on_settings_changed: RefCell::new(None),
            on_source_mode_changed: RefCell::new(None),
        });

        // Wire source mode buttons
        for (btn, mode) in [
            (&btn_src_auto, ConvertSourceMode::Auto),
            (&btn_src_tray, ConvertSourceMode::Tray),
            (&btn_src_sel, ConvertSourceMode::Selection),
        ] {
            let state = Rc::clone(&state);
            btn.connect_toggled(move |b| {
                if b.is_active() {
                    state.source_mode.set(mode);
                    if let Some(cb) = state.on_source_mode_changed.borrow().as_ref() {
                        cb(mode);
                    }
                }
            });
        }

        // Wire preset dropdown change
        {
            let state = Rc::clone(&state);
            let dropdown = state.preset_dropdown.clone();
            dropdown.connect_selected_notify(move |dd| {
                state.selected_preset_idx.set(dd.selected() as usize);
                state.update_tool_warning();
                state.update_preview();
                state.fire_settings_changed();
            });
        }

        // Wire output location buttons
        {
            let state = Rc::clone(&state);
            let chosen_label = state.output_chosen_label.clone();
            btn_next.connect_toggled(move |btn| {
                if btn.is_active() {
                    *state.output_mode.borrow_mut() = OutputLocationMode::NextToSource;
                    chosen_label.set_visible(false);
                    state.update_preview();
                    state.fire_settings_changed();
                }
            });
        }
        {
            let state = Rc::clone(&state);
            let chosen_label = state.output_chosen_label.clone();
            btn_subfolder.connect_toggled(move |btn| {
                if btn.is_active() {
                    *state.output_mode.borrow_mut() =
                        OutputLocationMode::Subfolder("Converted".to_string());
                    chosen_label.set_visible(false);
                    state.update_preview();
                    state.fire_settings_changed();
                }
            });
        }
        {
            let state = Rc::clone(&state);
            btn_choose.connect_toggled(move |btn| {
                if btn.is_active() {
                    if let Some(cb) = state.on_folder_pick.borrow().as_ref() {
                        cb();
                    }
                    // Don't update output_mode yet — wait for set_chosen_folder
                }
            });
        }

        // Wire conflict policy buttons
        {
            let state = Rc::clone(&state);
            btn_rename.connect_toggled(move |btn| {
                if btn.is_active() {
                    *state.conflict_policy.borrow_mut() = OutputConflictPolicy::AutoRename;
                    state.fire_settings_changed();
                }
            });
        }
        {
            let state = Rc::clone(&state);
            btn_skip.connect_toggled(move |btn| {
                if btn.is_active() {
                    *state.conflict_policy.borrow_mut() = OutputConflictPolicy::Skip;
                    state.fire_settings_changed();
                }
            });
        }
        {
            let state = Rc::clone(&state);
            btn_overwrite.connect_toggled(move |btn| {
                if btn.is_active() {
                    *state.conflict_policy.borrow_mut() = OutputConflictPolicy::Overwrite;
                    state.fire_settings_changed();
                }
            });
        }

        // Wire convert button
        {
            let state = Rc::clone(&state);
            let btn = state.convert_button.clone();
            btn.connect_clicked(move |btn| {
                let batch = state.build_batch();
                if batch.active_count() == 0 {
                    return;
                }
                btn.set_sensitive(false);
                if let Some(cb) = state.on_start.borrow().as_ref() {
                    cb(batch);
                }
            });
        }

        Self { root, state }
    }

    /// Load items into the panel, update tool warning, and rebuild the file list.
    pub fn set_items(
        &self,
        items: Vec<ConvertItem>,
        tools: &ToolAvailability,
        _output_dir: Option<PathBuf>,
    ) {
        *self.state.tools.borrow_mut() = tools.clone();
        *self.state.items.borrow_mut() = items;
        self.state.convert_button.set_sensitive(true);

        // Auto-select most relevant preset based on dominant kind
        let dominant = dominant_kind(&self.state.items.borrow());
        let target_idx = best_preset_idx_for(dominant);
        self.state.preset_dropdown.set_selected(target_idx as u32);
        self.state.selected_preset_idx.set(target_idx);

        self.state.update_tool_warning();
        self.state.update_preview();
    }

    pub fn connect_start(&self, callback: impl Fn(ConversionBatch) + 'static) {
        *self.state.on_start.borrow_mut() = Some(Box::new(callback));
    }

    pub fn source_mode(&self) -> ConvertSourceMode {
        self.state.source_mode.get()
    }

    pub fn connect_source_mode_changed(&self, callback: impl Fn(ConvertSourceMode) + 'static) {
        *self.state.on_source_mode_changed.borrow_mut() = Some(Box::new(callback));
    }

    pub fn start_current_batch(&self) -> bool {
        let batch = self.state.build_batch();
        if batch.active_count() == 0 || !self.state.convert_button.is_sensitive() {
            return false;
        }
        self.state.convert_button.set_sensitive(false);
        if let Some(cb) = self.state.on_start.borrow().as_ref() {
            cb(batch);
            true
        } else {
            false
        }
    }

    /// Triggered when "Choose folder…" is toggled. Wire to a GTK FileDialog in main_window.
    pub fn connect_folder_pick(&self, callback: impl Fn() + 'static) {
        *self.state.on_folder_pick.borrow_mut() = Some(Box::new(callback));
    }

    /// Called when the user cancels the folder picker dialog without choosing a folder.
    /// Reverts the output toggle back to "Next to originals" and updates output_mode.
    pub fn folder_pick_cancelled(&self) {
        *self.state.output_mode.borrow_mut() = OutputLocationMode::NextToSource;
        self.state.output_chosen_label.set_visible(false);
        self.state.btn_next.set_active(true);
        self.state.update_preview();
    }

    /// Called back from main_window after the user picks a folder.
    pub fn set_chosen_folder(&self, path: PathBuf) {
        let label = path.display().to_string();
        self.state
            .output_chosen_label
            .set_label(&format!("→ {label}"));
        self.state.output_chosen_label.set_visible(true);
        *self.state.output_mode.borrow_mut() = OutputLocationMode::ChosenFolder(path);
        self.state.update_preview();
        self.state.fire_settings_changed();
    }

    /// Called when preset/output/conflict changes. Receives (preset_id, output_mode, conflict_policy).
    pub fn connect_settings_changed(
        &self,
        callback: impl Fn(&str, &OutputLocationMode, &OutputConflictPolicy) + 'static,
    ) {
        *self.state.on_settings_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Apply saved settings on startup.
    pub fn apply_settings(&self, settings: &ConvertSettings) {
        // Apply output mode
        let mode_str = settings.output_mode.as_str();
        match mode_str {
            "converted_subfolder" => {
                *self.state.output_mode.borrow_mut() =
                    OutputLocationMode::Subfolder("Converted".to_string());
            }
            _ => {
                *self.state.output_mode.borrow_mut() = OutputLocationMode::NextToSource;
            }
        }

        // Apply conflict policy
        match settings.conflict_policy.as_str() {
            "skip" => {
                *self.state.conflict_policy.borrow_mut() = OutputConflictPolicy::Skip;
            }
            "overwrite" => {
                *self.state.conflict_policy.borrow_mut() = OutputConflictPolicy::Overwrite;
            }
            _ => {
                *self.state.conflict_policy.borrow_mut() = OutputConflictPolicy::AutoRename;
            }
        }
        // Note: preset is applied via set_items which auto-selects based on file kinds.
        // The last_preset_image/audio/video are used there via best_preset_for_kind.
    }

    #[allow(dead_code)]
    pub fn reset_convert_button(&self) {
        let items = self.state.items.borrow();
        let presets = all_presets();
        let preset = presets.get(self.state.selected_preset_idx.get());
        let tools = self.state.tools.borrow();
        let has_compatible = preset.map_or(false, |p| {
            p.tool_available(&tools) && items.iter().any(|item| item.kind == p.kind)
        });
        self.state.convert_button.set_sensitive(has_compatible);
    }
}

impl State {
    fn fire_settings_changed(self: &Rc<Self>) {
        let cb = self.on_settings_changed.borrow();
        if let Some(cb) = cb.as_ref() {
            let presets = all_presets();
            let preset_id = presets
                .get(self.selected_preset_idx.get())
                .map(|p| p.id)
                .unwrap_or("");
            let mode = self.output_mode.borrow();
            let policy = self.conflict_policy.borrow();
            cb(preset_id, &mode, &policy);
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn kind_badge(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "🖼",
        MediaKind::Audio => "🎵",
        MediaKind::Video => "🎬",
        MediaKind::Unknown => "❓",
    }
}

fn dest_preview_name(
    source: &std::path::Path,
    preset: &ConversionPreset,
    output_mode: &OutputLocationMode,
) -> String {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let filename = format!("{stem}.{}", preset.ext);
    match output_mode {
        OutputLocationMode::NextToSource => filename,
        OutputLocationMode::Subfolder(name) => format!("{name}/{filename}"),
        OutputLocationMode::ChosenFolder(path) => {
            format!("{}/{filename}", path.display())
        }
    }
}

fn dominant_kind(items: &[ConvertItem]) -> Option<MediaKind> {
    let images = items.iter().filter(|i| i.kind == MediaKind::Image).count();
    let audio = items.iter().filter(|i| i.kind == MediaKind::Audio).count();
    let video = items.iter().filter(|i| i.kind == MediaKind::Video).count();
    if images == 0 && audio == 0 && video == 0 {
        return None;
    }
    if images >= audio && images >= video {
        Some(MediaKind::Image)
    } else if audio >= video {
        Some(MediaKind::Audio)
    } else {
        Some(MediaKind::Video)
    }
}

fn best_preset_idx_for(kind: Option<MediaKind>) -> usize {
    let presets = all_presets();
    match kind {
        Some(k) => presets.iter().position(|p| p.kind == k).unwrap_or(0),
        None => 0,
    }
}
