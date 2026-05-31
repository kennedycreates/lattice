use crate::metadata::{Shape, TagRecord};
use crate::terroir_client::TerroirContext;
use crate::ui::file_grid::FileKind;
use crate::ui::mark_badge::make_shape_badge;
use gtk::prelude::*;
use gtk::{
    Align, Box, Button, Label, Orientation, Picture, ScrolledWindow, Separator, TextBuffer,
    TextView,
};

const PREVIEW_MARK_BADGE_SIZE: i32 = 18;
const PREVIEW_VISIBLE_TAGS: usize = 3;

#[derive(Clone)]
struct MetaRow {
    row: Box,
    key: Label,
    value: Label,
}

impl MetaRow {
    fn build(parent: &Box) -> Self {
        let row = Box::new(Orientation::Vertical, 4);
        row.add_css_class("preview-meta");
        row.set_visible(false);

        let key = Label::new(None);
        key.add_css_class("preview-meta-key");
        key.set_halign(Align::Start);
        row.append(&key);

        let value = Label::new(None);
        value.add_css_class("preview-meta-value");
        value.set_halign(Align::Start);
        value.set_wrap(true);
        value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        value.set_selectable(true);
        row.append(&value);

        parent.append(&row);

        Self { row, key, value }
    }

    fn set(&self, key: &str, value: &str) {
        self.key.set_label(key);
        self.value.set_label(value);
        self.row.set_visible(true);
    }

    fn clear(&self) {
        self.key.set_label("");
        self.value.set_label("");
        self.row.set_visible(false);
    }
}

#[derive(Clone)]
pub struct PreviewPane {
    pub root: Box,
    pub open_button: Button,
    pub copy_path_button: Button,
    pub open_parent_button: Button,
    icon: Label,
    title: Label,
    identity: Box,
    mark_badge_slot: Box,
    mark_label: Label,
    tag_box: Box,
    picture: Picture,
    text_buffer: TextBuffer,
    text_scroll: ScrolledWindow,
    type_row: MetaRow,
    mime_row: MetaRow,
    path_row: MetaRow,
    size_row: MetaRow,
    modified_row: MetaRow,
    dimensions_row: MetaRow,
    duration_row: MetaRow,
    watercolor_workspace_row: MetaRow,
    watercolor_palette_row: MetaRow,
    watercolor_object_row: MetaRow,
    watercolor_health_row: MetaRow,
    note_row: MetaRow,
}

impl PreviewPane {
    pub fn build() -> Self {
        let root = Box::new(Orientation::Vertical, 0);
        root.add_css_class("preview-pane");

        let header = Label::new(Some("Preview"));
        header.add_css_class("preview-header");
        header.set_halign(Align::Start);
        header.set_margin_top(16);
        header.set_margin_bottom(8);
        header.set_margin_start(16);
        header.set_margin_end(16);
        root.append(&header);

        let sep = Separator::new(Orientation::Horizontal);
        root.append(&sep);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        scroll.set_vexpand(true);
        scroll.add_css_class("preview-scroll");
        root.append(&scroll);

        let content = Box::new(Orientation::Vertical, 12);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        scroll.set_child(Some(&content));

        let icon = Label::new(Some("DIR"));
        icon.add_css_class("preview-icon");
        icon.set_halign(Align::Center);
        content.append(&icon);

        let picture = Picture::new();
        picture.add_css_class("preview-image");
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_hexpand(true);
        picture.set_size_request(-1, 220);
        picture.set_visible(false);
        content.append(&picture);

        let title = Label::new(Some("No Selection"));
        title.add_css_class("preview-title");
        title.set_halign(Align::Start);
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        content.append(&title);

        let identity = Box::new(Orientation::Vertical, 6);
        identity.add_css_class("preview-identity");
        identity.set_halign(Align::Fill);
        identity.set_visible(false);
        content.append(&identity);

        let mark_row = Box::new(Orientation::Horizontal, 8);
        mark_row.add_css_class("preview-mark-row");
        mark_row.set_halign(Align::Start);
        identity.append(&mark_row);

        let mark_badge_slot = Box::new(Orientation::Horizontal, 0);
        mark_badge_slot.add_css_class("preview-mark-badge-slot");
        mark_badge_slot.set_halign(Align::Start);
        mark_badge_slot.set_valign(Align::Center);
        mark_row.append(&mark_badge_slot);

        let mark_label = Label::new(None);
        mark_label.add_css_class("preview-mark-label");
        mark_label.set_halign(Align::Start);
        mark_label.set_valign(Align::Center);
        mark_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        mark_label.set_single_line_mode(true);
        mark_row.append(&mark_label);

        let tag_box = Box::new(Orientation::Horizontal, 4);
        tag_box.add_css_class("preview-tags");
        tag_box.set_halign(Align::Start);
        identity.append(&tag_box);

        let actions = Box::new(Orientation::Horizontal, 8);
        actions.add_css_class("preview-actions");
        actions.set_halign(Align::Start);
        content.append(&actions);

        let open_button = Button::with_label("Open");
        open_button.add_css_class("toolbar-action-btn");
        actions.append(&open_button);

        let copy_path_button = Button::with_label("Copy Path");
        copy_path_button.add_css_class("toolbar-action-btn");
        actions.append(&copy_path_button);

        let open_parent_button = Button::with_label("Open Parent Folder");
        open_parent_button.add_css_class("toolbar-action-btn");
        actions.append(&open_parent_button);

        let sep2 = Separator::new(Orientation::Horizontal);
        sep2.add_css_class("preview-meta-sep");
        content.append(&sep2);

        let type_row = MetaRow::build(&content);
        let mime_row = MetaRow::build(&content);
        let path_row = MetaRow::build(&content);
        path_row.value.set_wrap_mode(gtk::pango::WrapMode::Char);
        path_row.value.add_css_class("preview-path-value");
        let size_row = MetaRow::build(&content);
        let modified_row = MetaRow::build(&content);
        let dimensions_row = MetaRow::build(&content);
        let duration_row = MetaRow::build(&content);
        let watercolor_workspace_row = MetaRow::build(&content);
        let watercolor_palette_row = MetaRow::build(&content);
        let watercolor_object_row = MetaRow::build(&content);
        let watercolor_health_row = MetaRow::build(&content);
        let note_row = MetaRow::build(&content);

        let text_buffer = TextBuffer::new(None);
        let text_view = TextView::with_buffer(&text_buffer);
        text_view.add_css_class("preview-text");
        text_view.set_editable(false);
        text_view.set_cursor_visible(false);
        text_view.set_wrap_mode(gtk::WrapMode::WordChar);
        text_view.set_monospace(true);
        text_view.set_can_focus(false);

        let text_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(180)
            .build();
        text_scroll.set_visible(false);
        text_scroll.set_child(Some(&text_view));
        content.append(&text_scroll);

        let pane = Self {
            root,
            open_button,
            copy_path_button,
            open_parent_button,
            icon,
            title,
            identity,
            mark_badge_slot,
            mark_label,
            tag_box,
            picture,
            text_buffer,
            text_scroll,
            type_row,
            mime_row,
            path_row,
            size_row,
            modified_row,
            dimensions_row,
            duration_row,
            watercolor_workspace_row,
            watercolor_palette_row,
            watercolor_object_row,
            watercolor_health_row,
            note_row,
        };
        pane.show_current_folder("~", 0);
        pane
    }

    pub fn set_action_state(&self, open_enabled: bool, copy_enabled: bool, parent_enabled: bool) {
        self.open_button.set_sensitive(open_enabled);
        self.copy_path_button.set_sensitive(copy_enabled);
        self.open_parent_button.set_sensitive(parent_enabled);
    }

    pub fn set_icon_kind(&self, kind: &FileKind) {
        for class in FileKind::ALL_CSS_CLASSES {
            self.icon.remove_css_class(class);
        }
        self.icon.add_css_class(kind.css_class());
        self.icon.set_label(kind.badge());
    }

    pub fn set_image_file(&self, file: Option<&gio::File>) {
        self.picture.set_file(file);
    }

    pub fn image_dimensions(&self) -> Option<(i32, i32)> {
        let paintable = self.picture.paintable()?;
        let width = paintable.intrinsic_width();
        let height = paintable.intrinsic_height();
        (width > 0 && height > 0).then_some((width, height))
    }

    pub fn set_mime_type(&self, mime: Option<&str>) {
        if let Some(m) = mime.filter(|s| !s.is_empty()) {
            self.mime_row.set("MIME Type", m);
        } else {
            self.mime_row.clear();
        }
    }

    pub fn set_watercolor_context(&self, context: &TerroirContext) {
        if context.workspaces.is_empty()
            && context.palettes.is_empty()
            && context.objects.is_empty()
            && context.refs.is_empty()
        {
            self.clear_watercolor_context();
            return;
        }

        if !context.workspaces.is_empty() {
            self.watercolor_workspace_row.set(
                "Watercolor Workspaces",
                &join_limited(
                    context
                        .workspaces
                        .iter()
                        .map(|workspace| workspace.workspace_name.as_str()),
                    3,
                ),
            );
        } else {
            self.watercolor_workspace_row.clear();
        }

        if !context.palettes.is_empty() {
            self.watercolor_palette_row.set(
                "Watercolor Palettes",
                &join_limited(
                    context
                        .palettes
                        .iter()
                        .map(|palette| palette.palette_name.as_str()),
                    4,
                ),
            );
        } else {
            self.watercolor_palette_row.clear();
        }

        if !context.objects.is_empty() {
            let labels = context.objects.iter().map(|object| {
                format!(
                    "{} ({})",
                    object.title,
                    if object.object_type.is_empty() {
                        "object"
                    } else {
                        object.object_type.as_str()
                    }
                )
            });
            self.watercolor_object_row
                .set("Referenced By", &join_limited(labels, 4));
        } else {
            self.watercolor_object_row.clear();
        }

        if context.broken || context.health.as_deref() == Some("broken") {
            self.watercolor_health_row.set(
                "Watercolor Reference",
                "Broken reference: .water mentions this path, but Terroir reports a missing target.",
            );
        } else {
            self.watercolor_health_row.clear();
        }
    }

    pub fn set_watercolor_unavailable(&self) {
        self.clear_watercolor_context();
        self.watercolor_health_row
            .set("Watercolor Context", "Watercolor context unavailable.");
    }

    pub fn clear_watercolor_context(&self) {
        self.watercolor_workspace_row.clear();
        self.watercolor_palette_row.clear();
        self.watercolor_object_row.clear();
        self.watercolor_health_row.clear();
    }

    pub fn set_identity(
        &self,
        tint_name: &str,
        shape: Shape,
        tint_color: Option<&str>,
        tags: &[TagRecord],
    ) {
        clear_children(&self.mark_badge_slot);
        clear_children(&self.tag_box);

        let badge = make_shape_badge(shape, PREVIEW_MARK_BADGE_SIZE, tint_color);
        self.mark_badge_slot.append(&badge);
        self.mark_label
            .set_label(&format!("{tint_name} {}", shape.display_name()));

        for tag in tags.iter().take(PREVIEW_VISIBLE_TAGS) {
            let chip = Label::new(Some(&format!("#{}", tag.name)));
            chip.add_css_class("preview-tag-chip");
            chip.set_ellipsize(gtk::pango::EllipsizeMode::End);
            chip.set_single_line_mode(true);
            chip.set_max_width_chars(18);
            self.tag_box.append(&chip);
        }

        if tags.len() > PREVIEW_VISIBLE_TAGS {
            let overflow = Label::new(Some(&format!("+{}", tags.len() - PREVIEW_VISIBLE_TAGS)));
            overflow.add_css_class("preview-tag-chip");
            overflow.add_css_class("preview-tag-chip-muted");
            self.tag_box.append(&overflow);
        }

        self.tag_box.set_visible(!tags.is_empty());
        self.identity.set_visible(true);
    }

    pub fn show_loading(&self, title: &str, kind: &FileKind, note: &str) {
        self.reset(kind, title);
        self.type_row.set("Type", kind.label());
        self.note_row.set("Status", note);
    }

    pub fn show_current_folder(&self, path: &str, item_count: usize) {
        self.reset(&FileKind::Folder, "Current Folder");
        self.type_row.set("Type", "Folder");
        self.path_row.set("Path", path);
        self.note_row
            .set("Items", &format!("{item_count} item(s) loaded."));
    }

    pub fn show_multi_selection(&self, selected_count: usize) {
        self.reset(&FileKind::Unknown, "Multiple Items Selected");
        self.type_row.set("Type", "Mixed Selection");
        self.note_row.set(
            "Status",
            &format!("{selected_count} items selected. Select one item to inspect its preview."),
        );
    }

    pub fn show_folder(
        &self,
        name: &str,
        path: &str,
        modified: Option<&str>,
        item_count: Option<usize>,
        type_label: &str,
    ) {
        self.reset(&FileKind::Folder, name);
        self.type_row.set("Type", type_label);
        self.path_row.set("Path", path);
        if let Some(modified) = modified {
            self.modified_row.set("Modified", modified);
        }
        if let Some(item_count) = item_count {
            self.note_row
                .set("Items", &format!("{item_count} item(s) loaded."));
        }
    }

    pub fn show_image(
        &self,
        kind: &FileKind,
        name: &str,
        path: &str,
        size: Option<&str>,
        modified: Option<&str>,
        dimensions: Option<&str>,
    ) {
        self.reset(kind, name);
        self.icon.set_visible(false);
        self.picture.set_visible(true);
        self.type_row.set("Type", "Image");
        self.path_row.set("Path", path);
        if let Some(size) = size {
            self.size_row.set("Size", size);
        }
        if let Some(modified) = modified {
            self.modified_row.set("Modified", modified);
        }
        if let Some(dimensions) = dimensions {
            self.dimensions_row.set("Dimensions", dimensions);
        }
    }

    pub fn show_text_preview(
        &self,
        kind: &FileKind,
        type_label: &str,
        name: &str,
        path: &str,
        size: Option<&str>,
        modified: Option<&str>,
        text: &str,
        note: Option<&str>,
    ) {
        self.reset(kind, name);
        self.type_row.set("Type", type_label);
        self.path_row.set("Path", path);
        if let Some(size) = size {
            self.size_row.set("Size", size);
        }
        if let Some(modified) = modified {
            self.modified_row.set("Modified", modified);
        }
        if let Some(note) = note {
            self.note_row.set("Preview", note);
        }
        self.text_buffer.set_text(text);
        self.text_scroll.set_visible(true);
    }

    pub fn show_basic_file(
        &self,
        kind: &FileKind,
        type_label: &str,
        name: &str,
        path: &str,
        size: Option<&str>,
        modified: Option<&str>,
        note: Option<&str>,
    ) {
        self.reset(kind, name);
        self.type_row.set("Type", type_label);
        self.path_row.set("Path", path);
        if let Some(size) = size {
            self.size_row.set("Size", size);
        }
        if let Some(modified) = modified {
            self.modified_row.set("Modified", modified);
        }
        if let Some(note) = note {
            self.note_row.set("Preview", note);
        }
    }

    pub fn show_error(&self, path: &str, message: &str) {
        self.reset(&FileKind::Unknown, "Preview Unavailable");
        self.icon.set_label("⚠");
        self.type_row.set("Type", "Error");
        self.path_row.set("Path", path);
        self.note_row.set("Status", message);
    }

    fn reset(&self, kind: &FileKind, title: &str) {
        self.set_icon_kind(kind);
        self.icon.set_visible(true);
        self.picture.set_visible(false);
        self.set_image_file(None::<&gio::File>);
        self.title.set_label(title);
        self.clear_identity();
        self.text_buffer.set_text("");
        self.text_scroll.set_visible(false);
        self.type_row.clear();
        self.mime_row.clear();
        self.path_row.clear();
        self.size_row.clear();
        self.modified_row.clear();
        self.dimensions_row.clear();
        self.duration_row.clear();
        self.clear_watercolor_context();
        self.note_row.clear();
    }

    fn clear_identity(&self) {
        clear_children(&self.mark_badge_slot);
        clear_children(&self.tag_box);
        self.mark_label.set_label("");
        self.identity.set_visible(false);
    }
}

fn join_limited<I, S>(values: I, limit: usize) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let values: Vec<String> = values
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    let shown = values
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    if values.len() > limit {
        format!("{shown}, +{}", values.len() - limit)
    } else {
        shown
    }
}

fn clear_children(container: &Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
