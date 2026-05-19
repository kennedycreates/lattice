use crate::metadata::TintRecord;
use crate::thumbnail::{ThumbnailKind, ThumbnailTarget};
use crate::ui::file_grid::FileItem;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, DrawingArea, Label, Orientation, Picture, Revealer, Separator, Stack,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone)]
pub struct HoldingTray {
    pub root: Revealer,
    pub item_box: GtkBox,
    pub empty_label: Label,
    pub add_selection_button: Button,
    pub add_by_tint_button: Button,
    pub add_by_shape_button: Button,
    pub move_to_project_button: Button,
    pub copy_to_project_button: Button,
    pub tag_button: Button,
    pub apply_mark_button: Button,
    pub reset_mark_button: Button,
    pub trash_button: Button,
    pub copy_path_button: Button,
    pub clear_button: Button,
    count_label: Label,
    thumb_targets: RefCell<Vec<ThumbnailTarget>>,
}

impl HoldingTray {
    pub fn build() -> Self {
        let panel = GtkBox::new(Orientation::Vertical, 0);
        panel.add_css_class("holding-tray");

        let header = GtkBox::new(Orientation::Horizontal, 8);
        header.add_css_class("holding-tray-header");

        let title = Label::new(Some("Holding Tray"));
        title.add_css_class("holding-tray-title");
        title.set_halign(gtk::Align::Start);
        header.append(&title);

        let count_label = Label::new(Some("0 items"));
        count_label.add_css_class("holding-tray-count");
        count_label.set_halign(gtk::Align::Start);
        count_label.set_hexpand(true);
        header.append(&count_label);

        let (add_selection_button, add_selection_host) =
            action_button("Add selection (Ctrl+Alt+A)", "list-add-symbolic");
        header.append(&add_selection_host);

        let (add_by_tint_button, add_by_tint_host) =
            action_button("Add by Tint", "color-select-symbolic");
        header.append(&add_by_tint_host);

        let (add_by_shape_button, add_by_shape_host) =
            action_button("Add by Shape", "shapes-symbolic");
        header.append(&add_by_shape_host);

        let (move_to_project_button, move_to_project_host) =
            action_button("Move to project (Ctrl+Alt+M)", "go-next-symbolic");
        header.append(&move_to_project_host);

        let (copy_to_project_button, copy_to_project_host) =
            action_button("Copy to project (Ctrl+Alt+C)", "edit-copy-symbolic");
        header.append(&copy_to_project_host);

        let (tag_button, tag_host) = action_button("Tag tray (Ctrl+Alt+T)", "tag-symbolic");
        header.append(&tag_host);

        let (apply_mark_button, apply_mark_host) =
            action_button("Apply Mark to tray items", "emblem-symbolic");
        header.append(&apply_mark_host);

        let (reset_mark_button, reset_mark_host) =
            action_button("Reset tray items to Beige Square", "edit-undo-symbolic");
        header.append(&reset_mark_host);

        let (trash_button, trash_host) =
            action_button("Trash tray (Ctrl+Alt+Delete)", "user-trash-symbolic");
        trash_button.add_css_class("holding-tray-danger-action");
        header.append(&trash_host);

        let (copy_path_button, copy_path_host) =
            action_button("Copy paths (Ctrl+Alt+P)", "edit-copy-symbolic");
        header.append(&copy_path_host);

        let (clear_button, clear_host) =
            action_button("Clear tray (Ctrl+Alt+K)", "edit-clear-symbolic");
        header.append(&clear_host);

        panel.append(&header);

        let body = GtkBox::new(Orientation::Horizontal, 0);
        body.add_css_class("holding-tray-body");

        let item_box = GtkBox::new(Orientation::Horizontal, 6);
        item_box.add_css_class("holding-tray-items");
        item_box.set_hexpand(true);
        body.append(&item_box);

        let empty_label = Label::new(Some("No staged items"));
        empty_label.add_css_class("holding-tray-empty");
        empty_label.set_halign(gtk::Align::Center);
        empty_label.set_hexpand(true);
        body.append(&empty_label);

        panel.append(&body);

        let root = Revealer::new();
        root.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        root.set_transition_duration(200);
        root.set_reveal_child(false);
        root.set_child(Some(&panel));

        let tray = Self {
            root,
            item_box,
            empty_label,
            add_selection_button,
            add_by_tint_button,
            add_by_shape_button,
            move_to_project_button,
            copy_to_project_button,
            tag_button,
            apply_mark_button,
            reset_mark_button,
            trash_button,
            copy_path_button,
            clear_button,
            count_label,
            thumb_targets: RefCell::new(Vec::new()),
        };
        tray.set_action_sensitive(false);
        tray
    }

    pub fn set_items<F, G, H>(
        &self,
        items: &[FileItem],
        selected_paths: &[PathBuf],
        tint_colors: &HashMap<i64, String>,
        on_remove: F,
        on_select: G,
        on_open: H,
    ) where
        F: Fn(PathBuf) + Clone + 'static,
        G: Fn(PathBuf) + Clone + 'static,
        H: Fn(PathBuf) + Clone + 'static,
    {
        clear_box(&self.item_box);
        self.thumb_targets.borrow_mut().clear();

        let count = items.len();
        self.count_label.set_label(&format!(
            "{count} item{}",
            if count == 1 { "" } else { "s" }
        ));
        self.empty_label.set_visible(items.is_empty());
        self.item_box.set_visible(!items.is_empty());
        self.set_action_sensitive(!items.is_empty());

        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                let sep = Separator::new(Orientation::Vertical);
                sep.add_css_class("holding-tray-item-separator");
                self.item_box.append(&sep);
            }
            let selected = selected_paths.iter().any(|path| path == &item.path);
            let tint_color = tint_colors.get(&item.mark_tint_id).cloned();
            let (row, target) = tray_item(
                item,
                selected,
                tint_color,
                on_remove.clone(),
                on_select.clone(),
                on_open.clone(),
            );
            self.item_box.append(&row);
            if let Some(target) = target {
                self.thumb_targets.borrow_mut().push(target);
            }
        }
    }

    pub fn tint_color_map(tints: &[TintRecord]) -> HashMap<i64, String> {
        tints
            .iter()
            .filter_map(|t| t.color.clone().map(|c| (t.id, c)))
            .collect()
    }

    pub fn drain_thumb_targets(&self) -> Vec<ThumbnailTarget> {
        self.thumb_targets.borrow_mut().drain(..).collect()
    }

    fn set_action_sensitive(&self, sensitive: bool) {
        self.add_selection_button.set_sensitive(true);
        self.add_by_tint_button.set_sensitive(true);
        self.add_by_shape_button.set_sensitive(true);
        self.move_to_project_button.set_sensitive(sensitive);
        self.copy_to_project_button.set_sensitive(sensitive);
        self.tag_button.set_sensitive(sensitive);
        self.apply_mark_button.set_sensitive(sensitive);
        self.reset_mark_button.set_sensitive(sensitive);
        self.trash_button.set_sensitive(sensitive);
        self.copy_path_button.set_sensitive(sensitive);
        self.clear_button.set_sensitive(sensitive);
    }
}

fn action_button(label: &str, icon_name: &str) -> (Button, GtkBox) {
    let button = Button::from_icon_name(icon_name);
    button.add_css_class("holding-tray-action");
    button.add_css_class("toolbar-icon-btn");
    let host = super::tooltip_host(&button, label);
    (button, host)
}

fn tray_item<F, G, H>(
    item: &FileItem,
    selected: bool,
    tint_color: Option<String>,
    on_remove: F,
    on_select: G,
    on_open: H,
) -> (GtkBox, Option<ThumbnailTarget>)
where
    F: Fn(PathBuf) + Clone + 'static,
    G: Fn(PathBuf) + Clone + 'static,
    H: Fn(PathBuf) + Clone + 'static,
{
    let row = GtkBox::new(Orientation::Horizontal, 5);
    row.add_css_class("holding-tray-item");
    row.add_css_class(item.kind.css_class());
    if selected {
        row.add_css_class("holding-tray-item-selected");
    }
    row.set_tooltip_text(Some(&format!(
        "{}\nClick to select. Double-click or Enter opens.",
        item.path.display()
    )));
    row.set_focusable(true);

    let (media, target) = tray_item_media(item);
    row.append(&media);

    let name = Label::new(Some(&item.name));
    name.add_css_class("holding-tray-item-name");
    name.set_single_line_mode(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(18);
    row.append(&name);

    // Mark badge: color chip + shape glyph
    if let Some(color) = tint_color {
        let badge = GtkBox::new(Orientation::Horizontal, 2);
        badge.add_css_class("ht-mark-badge");
        let chip = DrawingArea::new();
        chip.set_content_width(8);
        chip.set_content_height(8);
        chip.set_valign(gtk::Align::Center);
        {
            let color = color.clone();
            chip.set_draw_func(move |_, cr, w, h| {
                if let Ok(rgba) = gtk::gdk::RGBA::parse(&color) {
                    cr.set_source_rgba(
                        rgba.red() as f64,
                        rgba.green() as f64,
                        rgba.blue() as f64,
                        1.0,
                    );
                    cr.rectangle(0.0, 0.0, w as f64, h as f64);
                    let _ = cr.fill();
                }
            });
        }
        badge.append(&chip);
        let shape_lbl = Label::new(Some(item.mark_shape.glyph()));
        shape_lbl.add_css_class("ht-mark-shape");
        badge.append(&shape_lbl);
        row.append(&badge);
    }

    let remove_button = Button::from_icon_name("window-close-symbolic");
    remove_button.add_css_class("holding-tray-remove");
    remove_button.add_css_class("toolbar-icon-btn");
    super::attach_tooltip(&remove_button, "Remove item (Delete)");
    let path = item.path.clone();
    remove_button.connect_clicked(move |_| on_remove(path.clone()));
    row.append(&remove_button);

    let click_path = item.path.clone();
    let click_select = on_select.clone();
    let click_open = on_open.clone();
    let row_focus = row.clone();
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.connect_released(move |gesture, n_press, _, _| {
        row_focus.grab_focus();
        if n_press >= 2 {
            click_open(click_path.clone());
        } else {
            click_select(click_path.clone());
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    row.add_controller(click);

    (row, target)
}

fn tray_item_media(item: &FileItem) -> (Stack, Option<ThumbnailTarget>) {
    let stack = Stack::new();
    stack.add_css_class("holding-tray-item-media");
    stack.set_size_request(24, 24);

    let icon = Label::new(Some(item.kind.badge()));
    icon.add_css_class("holding-tray-item-icon");
    stack.add_named(&icon, Some("badge"));

    let target_kind = match item.kind {
        crate::ui::file_grid::FileKind::Image => Some(ThumbnailKind::Image),
        crate::ui::file_grid::FileKind::Video => Some(ThumbnailKind::Video),
        crate::ui::file_grid::FileKind::Audio => Some(ThumbnailKind::Audio),
        _ => None,
    };

    let Some(kind) = target_kind else {
        stack.set_visible_child_name("badge");
        return (stack, None);
    };

    let picture = Picture::new();
    picture.add_css_class("holding-tray-item-thumb");
    picture.set_size_request(24, 24);
    picture.set_content_fit(gtk::ContentFit::Cover);
    stack.add_named(&picture, Some("thumb"));
    stack.set_visible_child_name("badge");

    (
        stack.clone(),
        Some(ThumbnailTarget {
            path: item.path.clone(),
            mtime: item.modified_unix.unwrap_or(0),
            stack,
            picture,
            kind,
        }),
    )
}

fn clear_box(container: &GtkBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
