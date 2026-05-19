use crate::metadata::{PaletteItemRecord, PaletteLinkRecord, Shape, TintRecord};
use crate::ui::tints_tags_panel::{draw_polygon, draw_shape, parse_hex_color};
use gtk::cairo;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, DrawingArea, Entry, EventControllerFocus, GestureClick,
    GestureDrag, Label, Orientation, Overlay, Popover, ScrolledWindow, Separator, TextView,
    ToggleButton, WrapMode,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;
use std::rc::Rc;

const CANVAS_W: i32 = 2400;
const CANVAS_H: i32 = 1800;
const DEFAULT_CARD_W: i64 = 220;
const DEFAULT_CARD_H: i64 = 160;
const MIN_CARD_W: i64 = 120;
const MIN_CARD_H: i64 = 80;
const DEFAULT_BEIGE: &str = "#806040";

// ── Callbacks ──────────────────────────────────────────────────────────────────

struct BoardCallbacks {
    on_item_moved: RefCell<Option<Box<dyn Fn(i64, i64, i64)>>>,
    on_item_resized: RefCell<Option<Box<dyn Fn(i64, i64, i64)>>>,
    on_item_deleted: RefCell<Option<Box<dyn Fn(i64)>>>,
    on_note_edited: RefCell<Option<Box<dyn Fn(i64, Option<String>, Option<String>)>>>,
    on_link_created: RefCell<Option<Box<dyn Fn(i64, i64, String)>>>,
    on_link_deleted: RefCell<Option<Box<dyn Fn(i64)>>>,
    on_add_file_card: RefCell<Option<Box<dyn Fn()>>>,
    on_add_folder_card: RefCell<Option<Box<dyn Fn()>>>,
    on_add_note_card: RefCell<Option<Box<dyn Fn()>>>,
}

impl BoardCallbacks {
    fn new() -> Self {
        Self {
            on_item_moved: RefCell::new(None),
            on_item_resized: RefCell::new(None),
            on_item_deleted: RefCell::new(None),
            on_note_edited: RefCell::new(None),
            on_link_created: RefCell::new(None),
            on_link_deleted: RefCell::new(None),
            on_add_file_card: RefCell::new(None),
            on_add_folder_card: RefCell::new(None),
            on_add_note_card: RefCell::new(None),
        }
    }
}

// ── Public widget ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct PaletteBoardPanel {
    pub root: GtkBox,
    fixed: gtk::Fixed,
    drawing_area: DrawingArea,
    link_mode: Rc<Cell<bool>>,
    link_source_id: Rc<Cell<Option<i64>>>,
    pub items: Rc<RefCell<Vec<PaletteItemRecord>>>,
    links: Rc<RefCell<Vec<PaletteLinkRecord>>>,
    tints: Rc<RefCell<Vec<TintRecord>>>,
    card_widgets: Rc<RefCell<HashMap<i64, GtkBox>>>,
    cbs: Rc<BoardCallbacks>,
    link_mode_btn: ToggleButton,
    status_label: Label,
    palette_id: Rc<Cell<i64>>,
    palette_name_label: Label,
    back_cb: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl PaletteBoardPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("palette-board-root");
        root.set_vexpand(true);
        root.set_hexpand(true);
        root.set_visible(false);

        // ── Toolbar ──────────────────────────────────────────────────────────
        let toolbar = GtkBox::new(Orientation::Horizontal, 0);
        toolbar.add_css_class("board-toolbar");

        // Back navigation (filled in by set_palette_info)
        let back_btn = Button::with_label("← Palettes");
        back_btn.add_css_class("board-back-btn");
        toolbar.append(&back_btn);

        let palette_name_label = Label::new(None);
        palette_name_label.add_css_class("board-palette-name");
        toolbar.append(&palette_name_label);

        let sep_nav = Separator::new(Orientation::Vertical);
        sep_nav.set_margin_start(6);
        sep_nav.set_margin_end(6);
        sep_nav.set_margin_top(6);
        sep_nav.set_margin_bottom(6);
        toolbar.append(&sep_nav);

        let add_file_btn = Button::with_label("+ File");
        add_file_btn.add_css_class("board-toolbar-btn");
        crate::ui::attach_tooltip(&add_file_btn, "Add a file card to the board");
        toolbar.append(&add_file_btn);

        let add_folder_btn = Button::with_label("+ Folder");
        add_folder_btn.add_css_class("board-toolbar-btn");
        crate::ui::attach_tooltip(&add_folder_btn, "Add a folder card to the board");
        toolbar.append(&add_folder_btn);

        let add_note_btn = Button::with_label("+ Note");
        add_note_btn.add_css_class("board-toolbar-btn");
        crate::ui::attach_tooltip(&add_note_btn, "Add a note card to the board");
        toolbar.append(&add_note_btn);

        let sep = Separator::new(Orientation::Vertical);
        sep.set_margin_start(6);
        sep.set_margin_end(6);
        sep.set_margin_top(6);
        sep.set_margin_bottom(6);
        toolbar.append(&sep);

        let link_mode_btn = ToggleButton::with_label("Link");
        link_mode_btn.add_css_class("board-toolbar-btn");
        crate::ui::attach_tooltip(&link_mode_btn, "Toggle link creation mode");
        toolbar.append(&link_mode_btn);

        let status_label = Label::new(None);
        status_label.add_css_class("board-status-label");
        status_label.set_halign(Align::Start);
        status_label.set_hexpand(true);
        toolbar.append(&status_label);

        root.append(&toolbar);

        // ── Canvas ───────────────────────────────────────────────────────────
        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();

        let overlay = Overlay::new();

        let drawing_area = DrawingArea::new();
        drawing_area.set_size_request(CANVAS_W, CANVAS_H);
        drawing_area.set_content_width(CANVAS_W);
        drawing_area.set_content_height(CANVAS_H);
        drawing_area.add_css_class("board-canvas");
        drawing_area.set_can_target(false);

        let fixed = gtk::Fixed::new();
        fixed.set_size_request(CANVAS_W, CANVAS_H);
        fixed.add_css_class("board-fixed");

        overlay.set_child(Some(&drawing_area));
        overlay.add_overlay(&fixed);
        overlay.set_measure_overlay(&fixed, false);

        scroll.set_child(Some(&overlay));
        root.append(&scroll);

        // ── Shared state ─────────────────────────────────────────────────────
        let items: Rc<RefCell<Vec<PaletteItemRecord>>> = Rc::new(RefCell::new(Vec::new()));
        let links: Rc<RefCell<Vec<PaletteLinkRecord>>> = Rc::new(RefCell::new(Vec::new()));
        let tints: Rc<RefCell<Vec<TintRecord>>> = Rc::new(RefCell::new(Vec::new()));
        let link_mode = Rc::new(Cell::new(false));
        let link_source_id: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
        let card_widgets: Rc<RefCell<HashMap<i64, GtkBox>>> = Rc::new(RefCell::new(HashMap::new()));
        let cbs = Rc::new(BoardCallbacks::new());
        let palette_id = Rc::new(Cell::new(0i64));
        let back_cb: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        // ── Back button ──────────────────────────────────────────────────────
        {
            let back_cb = Rc::clone(&back_cb);
            back_btn.connect_clicked(move |_| {
                if let Some(cb) = back_cb.borrow().as_ref() {
                    cb();
                }
            });
        }

        // ── Link drawing ─────────────────────────────────────────────────────
        {
            let items = Rc::clone(&items);
            let links = Rc::clone(&links);
            drawing_area.set_draw_func(move |_, cr, _w, _h| {
                draw_links(cr, &items.borrow(), &links.borrow());
            });
        }

        // ── Link mode toggle ─────────────────────────────────────────────────
        {
            let link_mode = Rc::clone(&link_mode);
            let link_source_id = Rc::clone(&link_source_id);
            let status_label = status_label.clone();
            let card_widgets = Rc::clone(&card_widgets);
            link_mode_btn.connect_toggled(move |btn| {
                let active = btn.is_active();
                link_mode.set(active);
                if !active {
                    // Clear source selection
                    if let Some(src_id) = link_source_id.take() {
                        if let Some(card) = card_widgets.borrow().get(&src_id) {
                            card.remove_css_class("board-card-link-source");
                        }
                    }
                    status_label.set_text("");
                } else {
                    status_label.set_text("Click source card, then target card");
                }
            });
        }

        // ── Toolbar callbacks ────────────────────────────────────────────────
        {
            let cbs = Rc::clone(&cbs);
            add_file_btn.connect_clicked(move |_| {
                if let Some(cb) = cbs.on_add_file_card.borrow().as_ref() {
                    cb();
                }
            });
        }
        {
            let cbs = Rc::clone(&cbs);
            add_folder_btn.connect_clicked(move |_| {
                if let Some(cb) = cbs.on_add_folder_card.borrow().as_ref() {
                    cb();
                }
            });
        }
        {
            let cbs = Rc::clone(&cbs);
            add_note_btn.connect_clicked(move |_| {
                if let Some(cb) = cbs.on_add_note_card.borrow().as_ref() {
                    cb();
                }
            });
        }

        // ── Board-level card drag (on Fixed, not on individual cards) ────────
        // Fixed's coordinate system is stable even when cards move inside it.
        // Per-card GestureDrag breaks because fixed.move_() shifts the card's
        // widget-local coordinates, making subsequent drag_update offsets ≈ 0
        // and causing the card to oscillate back toward its origin.
        const CARD_HANDLE_H: f64 = 36.0;
        const CARD_MENU_BTN_W: f64 = 28.0;
        let active_drag_id: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
        let card_drag_origin: Rc<Cell<(f64, f64)>> = Rc::new(Cell::new((0.0, 0.0)));
        let board_drag = GestureDrag::new();
        board_drag.set_button(1);
        board_drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let link_mode = Rc::clone(&link_mode);
            let items = Rc::clone(&items);
            let active_drag_id = Rc::clone(&active_drag_id);
            let card_drag_origin = Rc::clone(&card_drag_origin);
            board_drag.connect_drag_begin(move |gesture, start_x, start_y| {
                if link_mode.get() {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                let items = items.borrow();
                let hit = items.iter().find(|i| {
                    let cx = i.x as f64;
                    let cy = i.y as f64;
                    let cw = i.width as f64;
                    start_x >= cx
                        && start_x <= cx + cw - CARD_MENU_BTN_W
                        && start_y >= cy
                        && start_y <= cy + CARD_HANDLE_H
                });
                if let Some(item) = hit {
                    active_drag_id.set(Some(item.id));
                    card_drag_origin.set((item.x as f64, item.y as f64));
                } else {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                }
            });
        }
        {
            let fixed = fixed.clone();
            let items = Rc::clone(&items);
            let card_widgets = Rc::clone(&card_widgets);
            let da = drawing_area.clone();
            let active_drag_id = Rc::clone(&active_drag_id);
            let card_drag_origin = Rc::clone(&card_drag_origin);
            board_drag.connect_drag_update(move |_, offset_x, offset_y| {
                let Some(card_id) = active_drag_id.get() else {
                    return;
                };
                let (orig_x, orig_y) = card_drag_origin.get();
                let new_x = (orig_x + offset_x).max(0.0);
                let new_y = (orig_y + offset_y).max(0.0);
                if let Some(widget) = card_widgets.borrow().get(&card_id) {
                    fixed.move_(widget, new_x, new_y);
                }
                if let Some(item) = items.borrow_mut().iter_mut().find(|i| i.id == card_id) {
                    item.x = new_x as i64;
                    item.y = new_y as i64;
                }
                da.queue_draw();
            });
        }
        {
            let items = Rc::clone(&items);
            let cbs = Rc::clone(&cbs);
            let active_drag_id = Rc::clone(&active_drag_id);
            board_drag.connect_drag_end(move |_, _, _| {
                let Some(card_id) = active_drag_id.take() else {
                    return;
                };
                if let Some(item) = items.borrow().iter().find(|i| i.id == card_id) {
                    if let Some(cb) = cbs.on_item_moved.borrow().as_ref() {
                        cb(item.id, item.x, item.y);
                    }
                }
            });
        }
        fixed.add_controller(board_drag);

        Self {
            root,
            fixed,
            drawing_area,
            link_mode,
            link_source_id,
            items,
            links,
            tints,
            card_widgets,
            cbs,
            link_mode_btn,
            status_label,
            palette_id,
            palette_name_label,
            back_cb,
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    pub fn set_palette_info(&self, name: &str, on_back: impl Fn() + 'static) {
        self.palette_name_label.set_text(name);
        *self.back_cb.borrow_mut() = Some(Box::new(on_back));
    }

    pub fn populate(
        &self,
        palette_id: i64,
        items: Vec<PaletteItemRecord>,
        links: Vec<PaletteLinkRecord>,
        tints: Vec<TintRecord>,
    ) {
        self.palette_id.set(palette_id);

        // Clear existing cards from fixed
        {
            let mut card_map = self.card_widgets.borrow_mut();
            for (_, widget) in card_map.drain() {
                self.fixed.remove(&widget);
            }
        }

        // Reset link mode
        self.link_mode_btn.set_active(false);
        self.link_source_id.set(None);
        self.status_label.set_text("");

        *self.tints.borrow_mut() = tints;
        *self.links.borrow_mut() = links;
        *self.items.borrow_mut() = items.clone();

        for item in &items {
            self.add_card_widget(item);
        }

        self.drawing_area.queue_draw();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_callbacks(
        &self,
        on_item_moved: impl Fn(i64, i64, i64) + 'static,
        on_item_resized: impl Fn(i64, i64, i64) + 'static,
        on_item_deleted: impl Fn(i64) + 'static,
        on_note_edited: impl Fn(i64, Option<String>, Option<String>) + 'static,
        on_link_created: impl Fn(i64, i64, String) + 'static,
        on_link_deleted: impl Fn(i64) + 'static,
        on_add_file_card: impl Fn() + 'static,
        on_add_folder_card: impl Fn() + 'static,
        on_add_note_card: impl Fn() + 'static,
    ) {
        *self.cbs.on_item_moved.borrow_mut() = Some(Box::new(on_item_moved));
        *self.cbs.on_item_resized.borrow_mut() = Some(Box::new(on_item_resized));
        *self.cbs.on_item_deleted.borrow_mut() = Some(Box::new(on_item_deleted));
        *self.cbs.on_note_edited.borrow_mut() = Some(Box::new(on_note_edited));
        *self.cbs.on_link_created.borrow_mut() = Some(Box::new(on_link_created));
        *self.cbs.on_link_deleted.borrow_mut() = Some(Box::new(on_link_deleted));
        *self.cbs.on_add_file_card.borrow_mut() = Some(Box::new(on_add_file_card));
        *self.cbs.on_add_folder_card.borrow_mut() = Some(Box::new(on_add_folder_card));
        *self.cbs.on_add_note_card.borrow_mut() = Some(Box::new(on_add_note_card));
    }

    /// Add a single new card to the live board (after creating it in the DB).
    pub fn add_card(&self, item: PaletteItemRecord) {
        self.items.borrow_mut().push(item.clone());
        self.add_card_widget(&item);
        self.drawing_area.queue_draw();
    }

    /// Update the links list and redraw (called after link create/delete in DB).
    pub fn set_links(&self, links: Vec<PaletteLinkRecord>) {
        *self.links.borrow_mut() = links;
        self.drawing_area.queue_draw();
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn add_card_widget(&self, item: &PaletteItemRecord) {
        let card = self.build_card(item);
        self.fixed.put(&card, item.x as f64, item.y as f64);
        self.card_widgets.borrow_mut().insert(item.id, card);
    }

    fn build_card(&self, item: &PaletteItemRecord) -> GtkBox {
        let item_id = item.id;
        let item_path = item.path.clone();

        // Resolve tint color
        let tint_color = resolve_tint_color(item.tint_id, &self.tints.borrow());
        let (tr, tg, tb) = parse_hex_color(&tint_color);

        // ── Outer horizontal box: tint strip + content ────────────────────
        let outer = GtkBox::new(Orientation::Horizontal, 0);
        outer.add_css_class("board-card");
        match item.item_type.as_str() {
            "file" => outer.add_css_class("board-card-file"),
            "folder" => outer.add_css_class("board-card-folder"),
            "note" => outer.add_css_class("board-card-note"),
            _ => {}
        }
        outer.set_size_request(item.width as i32, item.height as i32);

        // Tint strip (6px left edge)
        let tint_strip = DrawingArea::new();
        tint_strip.set_content_width(6);
        tint_strip.set_content_height(1);
        tint_strip.set_vexpand(true);
        tint_strip.set_can_target(false);
        tint_strip.add_css_class("card-tint-strip");
        tint_strip.set_draw_func(move |_, cr, _w, h| {
            cr.set_source_rgb(tr, tg, tb);
            cr.rectangle(0.0, 0.0, 6.0, h as f64);
            let _ = cr.fill();
        });
        outer.append(&tint_strip);

        // Content column
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.set_hexpand(true);
        content.set_vexpand(true);

        // ── Drag handle (card header) ─────────────────────────────────────
        let drag_handle = GtkBox::new(Orientation::Horizontal, 4);
        drag_handle.add_css_class("card-drag-handle");
        drag_handle.set_margin_start(6);
        drag_handle.set_margin_end(4);
        drag_handle.set_margin_top(4);
        drag_handle.set_margin_bottom(4);

        let icon_text = match item.item_type.as_str() {
            "folder" => "📁",
            "note" => "📝",
            _ => "📄",
        };
        let type_icon = Label::new(Some(icon_text));
        type_icon.add_css_class("card-type-icon");
        drag_handle.append(&type_icon);

        let name_text = card_display_name(item);
        let name_label = Label::new(Some(&name_text));
        name_label.add_css_class("card-name");
        name_label.set_halign(Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name_label.set_max_width_chars(20);
        drag_handle.append(&name_label);

        // Shape badge
        let shape_val = item.shape.unwrap_or(Shape::Square);
        let shape_badge = DrawingArea::new();
        shape_badge.set_content_width(18);
        shape_badge.set_content_height(18);
        shape_badge.set_size_request(18, 18);
        shape_badge.set_valign(Align::Center);
        shape_badge.set_can_target(false);
        shape_badge.set_draw_func(move |_, cr, w, h| {
            let cx = w as f64 / 2.0;
            let cy = h as f64 / 2.0;
            let r = 6.5_f64;
            cr.set_source_rgba(tr, tg, tb, 0.85);
            draw_shape(cr, shape_val, cx, cy, r);
            let _ = cr.fill();
        });
        drag_handle.append(&shape_badge);

        // Menu button (⋯)
        let menu_btn = Button::with_label("⋯");
        menu_btn.add_css_class("card-menu-btn");
        menu_btn.set_valign(Align::Center);
        drag_handle.append(&menu_btn);

        content.append(&drag_handle);

        // ── Card body ─────────────────────────────────────────────────────
        match item.item_type.as_str() {
            "note" => {
                let note_box = GtkBox::new(Orientation::Vertical, 2);
                note_box.set_vexpand(true);
                note_box.set_margin_start(6);
                note_box.set_margin_end(6);
                note_box.set_margin_top(4);
                note_box.set_margin_bottom(4);

                let title_entry = Entry::new();
                title_entry.add_css_class("card-note-title");
                title_entry.set_placeholder_text(Some("Title"));
                if let Some(t) = &item.title {
                    title_entry.set_text(t);
                }
                note_box.append(&title_entry);

                let body_view = TextView::new();
                body_view.add_css_class("card-note-body");
                body_view.set_wrap_mode(WrapMode::Word);
                body_view.set_vexpand(true);
                body_view.set_hexpand(true);
                body_view.set_top_margin(2);
                body_view.set_left_margin(2);
                body_view.set_right_margin(2);
                if let Some(b) = &item.body {
                    body_view.buffer().set_text(b);
                }
                note_box.append(&body_view);

                // Auto-save on focus-out.
                let cbs = Rc::clone(&self.cbs);
                let title_entry_c = title_entry.clone();
                let body_view_c = body_view.clone();
                let focus_ctrl = EventControllerFocus::new();
                focus_ctrl.connect_leave(move |_| {
                    let title = {
                        let t = title_entry_c.text().to_string();
                        if t.trim().is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    };
                    let body = {
                        let buf = body_view_c.buffer();
                        let s = buf
                            .text(&buf.start_iter(), &buf.end_iter(), false)
                            .to_string();
                        if s.trim().is_empty() {
                            None
                        } else {
                            Some(s)
                        }
                    };
                    if let Some(cb) = cbs.on_note_edited.borrow().as_ref() {
                        cb(item_id, title, body);
                    }
                });
                body_view.add_controller(focus_ctrl);

                // Also save on title activate
                let cbs = Rc::clone(&self.cbs);
                let title_entry_c2 = title_entry.clone();
                let body_view_c2 = body_view.clone();
                title_entry.connect_activate(move |_| {
                    let title = {
                        let t = title_entry_c2.text().to_string();
                        if t.trim().is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    };
                    let body = {
                        let buf = body_view_c2.buffer();
                        let s = buf
                            .text(&buf.start_iter(), &buf.end_iter(), false)
                            .to_string();
                        if s.trim().is_empty() {
                            None
                        } else {
                            Some(s)
                        }
                    };
                    if let Some(cb) = cbs.on_note_edited.borrow().as_ref() {
                        cb(item_id, title, body);
                    }
                });

                content.append(&note_box);
            }
            _ => {
                // File or folder card: show path
                let path_str = item_path.clone().unwrap_or_default();
                let path_label = Label::new(Some(&path_str));
                path_label.add_css_class("card-path");
                path_label.set_halign(Align::Start);
                path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                path_label.set_max_width_chars(28);
                path_label.set_margin_start(8);
                path_label.set_margin_top(4);
                path_label.set_margin_bottom(4);
                path_label.set_hexpand(true);
                path_label.set_vexpand(true);
                path_label.set_valign(Align::Start);
                content.append(&path_label);

                // Tooltip with full path
                if !path_str.is_empty() {
                    crate::ui::attach_tooltip(&path_label, &path_str);
                }
            }
        }

        // ── Resize handle ─────────────────────────────────────────────────
        let footer = GtkBox::new(Orientation::Horizontal, 0);
        footer.set_hexpand(true);
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let resize_da = DrawingArea::new();
        resize_da.add_css_class("card-resize-handle");
        resize_da.set_size_request(18, 18);
        resize_da.set_content_width(18);
        resize_da.set_content_height(18);
        resize_da.set_valign(Align::End);
        resize_da.set_draw_func(|_, cr, w, h| {
            cr.set_source_rgba(0.72, 0.62, 0.47, 0.6);
            cr.set_line_width(1.5);
            let margin = 3.0_f64;
            let x = w as f64 - margin;
            let y = h as f64 - margin;
            // Draw three diagonal lines as a resize indicator
            for i in 0..3i32 {
                let offset = i as f64 * 3.5;
                cr.move_to(x - offset, y);
                cr.line_to(x, y - offset);
            }
            let _ = cr.stroke();
        });
        footer.append(&resize_da);
        content.append(&footer);

        outer.append(&content);

        // ── Resize gesture ────────────────────────────────────────────────
        // Note: card dragging is handled by a board-level GestureDrag on Fixed
        // (set up in build()). Per-card drag was removed because GTK4 computes
        // GestureDrag offsets in widget-local coordinates. Once fixed.move_()
        // relocates the card, the widget-local system shifts and offsets collapse
        // to ≈ 0, causing severe oscillation jitter.
        let resize_origin: Rc<Cell<(i64, i64)>> = Rc::new(Cell::new((0, 0)));
        let resize_drag = GestureDrag::new();
        resize_drag.set_button(1);
        {
            let items = Rc::clone(&self.items);
            let resize_origin = Rc::clone(&resize_origin);
            resize_drag.connect_drag_begin(move |_, _, _| {
                let size = items
                    .borrow()
                    .iter()
                    .find(|i| i.id == item_id)
                    .map(|i| (i.width, i.height))
                    .unwrap_or((DEFAULT_CARD_W, DEFAULT_CARD_H));
                resize_origin.set(size);
            });
        }
        {
            let card_ref = outer.clone();
            let items = Rc::clone(&self.items);
            let da = self.drawing_area.clone();
            let resize_origin = Rc::clone(&resize_origin);
            resize_drag.connect_drag_update(move |_, offset_x, offset_y| {
                let (init_w, init_h) = resize_origin.get();
                let new_w = (init_w + offset_x as i64).max(MIN_CARD_W);
                let new_h = (init_h + offset_y as i64).max(MIN_CARD_H);
                card_ref.set_size_request(new_w as i32, new_h as i32);
                if let Some(item) = items.borrow_mut().iter_mut().find(|i| i.id == item_id) {
                    item.width = new_w;
                    item.height = new_h;
                }
                da.queue_draw();
            });
        }
        {
            let items = Rc::clone(&self.items);
            let cbs = Rc::clone(&self.cbs);
            resize_drag.connect_drag_end(move |_, _, _| {
                if let Some(item) = items.borrow().iter().find(|i| i.id == item_id) {
                    if let Some(cb) = cbs.on_item_resized.borrow().as_ref() {
                        cb(item.id, item.width, item.height);
                    }
                }
            });
        }
        resize_da.add_controller(resize_drag);

        // ── Link mode click ───────────────────────────────────────────────
        {
            let link_mode = Rc::clone(&self.link_mode);
            let link_source_id = Rc::clone(&self.link_source_id);
            let card_widgets = Rc::clone(&self.card_widgets);
            let cbs = Rc::clone(&self.cbs);
            let status_label = self.status_label.clone();
            let link_mode_btn = self.link_mode_btn.clone();
            let outer_ref = outer.clone();
            let click = GestureClick::new();
            click.set_button(1);
            click.connect_pressed(move |_, n, _, _| {
                if n != 1 || !link_mode.get() {
                    return;
                }
                match link_source_id.get() {
                    None => {
                        link_source_id.set(Some(item_id));
                        outer_ref.add_css_class("board-card-link-source");
                        status_label.set_text("Now click the target card");
                    }
                    Some(src_id) if src_id != item_id => {
                        // Show weak/strong popover on the target card
                        show_link_popover(
                            &outer_ref,
                            src_id,
                            item_id,
                            &cbs,
                            &link_source_id,
                            &link_mode,
                            &link_mode_btn,
                            &card_widgets,
                            &status_label,
                        );
                    }
                    _ => {}
                }
            });
            outer.add_controller(click);
        }

        // ── Context menu (⋯) ─────────────────────────────────────────────
        {
            let cbs = Rc::clone(&self.cbs);
            let links_ref = Rc::clone(&self.links);
            let card_ref = outer.clone();
            let card_widgets = Rc::clone(&self.card_widgets);
            let items_ref = Rc::clone(&self.items);
            let da = self.drawing_area.clone();
            let fixed = self.fixed.clone();

            menu_btn.connect_clicked(move |btn| {
                show_card_context_menu(
                    btn,
                    item_id,
                    &card_ref,
                    &cbs,
                    &links_ref,
                    &card_widgets,
                    &items_ref,
                    &da,
                    &fixed,
                );
            });
        }

        outer
    }
}

// ── Link drawing ────────────────────────────────────────────────────────────

fn draw_links(cr: &cairo::Context, items: &[PaletteItemRecord], links: &[PaletteLinkRecord]) {
    for link in links {
        let Some(src) = items.iter().find(|i| i.id == link.source_item_id) else {
            continue;
        };
        let Some(dst) = items.iter().find(|i| i.id == link.target_item_id) else {
            continue;
        };

        let src_cx = src.x as f64 + src.width as f64 / 2.0;
        let src_cy = src.y as f64 + src.height as f64 / 2.0;
        let dst_cx = dst.x as f64 + dst.width as f64 / 2.0;
        let dst_cy = dst.y as f64 + dst.height as f64 / 2.0;

        cr.set_line_width(2.0);
        match link.strength.as_str() {
            "weak" => {
                cr.set_dash(&[6.0, 4.0], 0.0);
                cr.set_source_rgba(0.72, 0.62, 0.47, 0.6);
            }
            "strong" => {
                cr.set_dash(&[], 0.0);
                cr.set_source_rgba(0.79, 0.59, 0.18, 0.9);
            }
            _ => {
                cr.set_dash(&[], 0.0);
                cr.set_source_rgba(0.5, 0.5, 0.5, 0.5);
            }
        }

        cr.move_to(src_cx, src_cy);
        cr.line_to(dst_cx, dst_cy);
        let _ = cr.stroke();

        // Small circle at midpoint to help distinguish links
        let mid_x = (src_cx + dst_cx) / 2.0;
        let mid_y = (src_cy + dst_cy) / 2.0;
        cr.set_dash(&[], 0.0);
        cr.arc(mid_x, mid_y, 3.0, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.fill();
    }
}

// ── Link strength popover ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn show_link_popover(
    parent: &GtkBox,
    src_id: i64,
    dst_id: i64,
    cbs: &Rc<BoardCallbacks>,
    link_source_id: &Rc<Cell<Option<i64>>>,
    link_mode: &Rc<Cell<bool>>,
    link_mode_btn: &ToggleButton,
    card_widgets: &Rc<RefCell<HashMap<i64, GtkBox>>>,
    status_label: &Label,
) {
    let popover = Popover::new();
    popover.set_parent(parent);
    popover.set_has_arrow(true);

    let vbox = GtkBox::new(Orientation::Vertical, 6);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);

    let heading = Label::new(Some("Link type"));
    heading.add_css_class("link-popover-heading");
    vbox.append(&heading);

    let weak_btn = Button::with_label("Weak  (dashed)");
    weak_btn.add_css_class("link-type-btn");
    let strong_btn = Button::with_label("Strong (solid)");
    strong_btn.add_css_class("link-type-btn");

    vbox.append(&weak_btn);
    vbox.append(&strong_btn);
    popover.set_child(Some(&vbox));

    // Shared cleanup closure
    let cleanup = {
        let link_source_id = Rc::clone(link_source_id);
        let link_mode = Rc::clone(link_mode);
        let link_mode_btn = link_mode_btn.clone();
        let card_widgets = Rc::clone(card_widgets);
        let status_label = status_label.clone();
        Rc::new(move || {
            if let Some(old_src) = link_source_id.take() {
                if let Some(card) = card_widgets.borrow().get(&old_src) {
                    card.remove_css_class("board-card-link-source");
                }
            }
            link_mode.set(false);
            link_mode_btn.set_active(false);
            status_label.set_text("");
        })
    };

    {
        let cbs = Rc::clone(cbs);
        let cleanup = Rc::clone(&cleanup);
        let popover_c = popover.clone();
        weak_btn.connect_clicked(move |_| {
            if let Some(cb) = cbs.on_link_created.borrow().as_ref() {
                cb(src_id, dst_id, "weak".to_string());
            }
            cleanup();
            popover_c.popdown();
        });
    }

    {
        let cbs = Rc::clone(cbs);
        let cleanup = Rc::clone(&cleanup);
        let popover_c = popover.clone();
        strong_btn.connect_clicked(move |_| {
            if let Some(cb) = cbs.on_link_created.borrow().as_ref() {
                cb(src_id, dst_id, "strong".to_string());
            }
            cleanup();
            popover_c.popdown();
        });
    }

    popover.popup();
}

// ── Card context menu ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn show_card_context_menu(
    anchor: &Button,
    item_id: i64,
    card_ref: &GtkBox,
    cbs: &Rc<BoardCallbacks>,
    links_ref: &Rc<RefCell<Vec<PaletteLinkRecord>>>,
    card_widgets: &Rc<RefCell<HashMap<i64, GtkBox>>>,
    items_ref: &Rc<RefCell<Vec<PaletteItemRecord>>>,
    da: &DrawingArea,
    fixed: &gtk::Fixed,
) {
    let popover = Popover::new();
    popover.set_parent(anchor);
    popover.set_has_arrow(true);

    let vbox = GtkBox::new(Orientation::Vertical, 4);
    vbox.set_margin_top(6);
    vbox.set_margin_bottom(6);
    vbox.set_margin_start(6);
    vbox.set_margin_end(6);

    // Remove from board
    let remove_btn = Button::with_label("Remove from Board");
    remove_btn.add_css_class("card-menu-item");
    {
        let cbs = Rc::clone(cbs);
        let card_widgets = Rc::clone(card_widgets);
        let items_ref = Rc::clone(items_ref);
        let links_ref = Rc::clone(links_ref);
        let da = da.clone();
        let fixed = fixed.clone();
        let card_ref = card_ref.clone();
        let popover_c = popover.clone();
        remove_btn.connect_clicked(move |_| {
            popover_c.popdown();
            // Remove widget
            fixed.remove(&card_ref);
            card_widgets.borrow_mut().remove(&item_id);
            items_ref.borrow_mut().retain(|i| i.id != item_id);
            links_ref
                .borrow_mut()
                .retain(|l| l.source_item_id != item_id && l.target_item_id != item_id);
            da.queue_draw();
            if let Some(cb) = cbs.on_item_deleted.borrow().as_ref() {
                cb(item_id);
            }
        });
    }
    vbox.append(&remove_btn);

    // Show linked cards and allow unlinking
    let item_links: Vec<PaletteLinkRecord> = links_ref
        .borrow()
        .iter()
        .filter(|l| l.source_item_id == item_id || l.target_item_id == item_id)
        .cloned()
        .collect();

    if !item_links.is_empty() {
        let sep = Separator::new(Orientation::Horizontal);
        vbox.append(&sep);
        let lbl = Label::new(Some("Delete link:"));
        lbl.add_css_class("card-menu-section-label");
        lbl.set_halign(Align::Start);
        vbox.append(&lbl);

        for link in item_links {
            let other_id = if link.source_item_id == item_id {
                link.target_item_id
            } else {
                link.source_item_id
            };
            let other_name = items_ref
                .borrow()
                .iter()
                .find(|i| i.id == other_id)
                .map(|i| card_display_name(i))
                .unwrap_or_else(|| format!("#{other_id}"));
            let link_label = format!("→ {} ({})", other_name, link.strength);
            let del_btn = Button::with_label(&link_label);
            del_btn.add_css_class("card-menu-item");
            del_btn.add_css_class("card-menu-item-delete");
            let link_id = link.id;
            let cbs = Rc::clone(cbs);
            let links_ref = Rc::clone(links_ref);
            let da = da.clone();
            let popover_c = popover.clone();
            del_btn.connect_clicked(move |_| {
                popover_c.popdown();
                links_ref.borrow_mut().retain(|l| l.id != link_id);
                da.queue_draw();
                if let Some(cb) = cbs.on_link_deleted.borrow().as_ref() {
                    cb(link_id);
                }
            });
            vbox.append(&del_btn);
        }
    }

    popover.set_child(Some(&vbox));
    popover.popup();
}

// ── Utilities ────────────────────────────────────────────────────────────────

fn resolve_tint_color(tint_id: Option<i64>, tints: &[TintRecord]) -> String {
    tint_id
        .and_then(|id| tints.iter().find(|t| t.id == id))
        .and_then(|t| t.color.clone())
        .unwrap_or_else(|| DEFAULT_BEIGE.to_string())
}

fn card_display_name(item: &PaletteItemRecord) -> String {
    if let Some(title) = &item.title {
        if !title.trim().is_empty() {
            return title.clone();
        }
    }
    if let Some(path) = &item.path {
        if let Some(name) = std::path::Path::new(path).file_name() {
            return name.to_string_lossy().to_string();
        }
        return path.clone();
    }
    match item.item_type.as_str() {
        "note" => "Untitled Note".to_string(),
        "folder" => "Folder".to_string(),
        _ => "File".to_string(),
    }
}

// Silence unused import for FRAC_PI_2 brought in via draw_polygon
#[allow(dead_code)]
const _FRAC_PI_2: f64 = FRAC_PI_2;
// Similarly silence draw_polygon if used only transitively
#[allow(dead_code)]
fn _use_helpers() {
    let _ = draw_polygon;
}
