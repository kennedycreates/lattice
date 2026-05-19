use crate::metadata::{Shape, TintRecord};
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, DrawingArea, Label, Orientation, Popover, Revealer,
    RevealerTransitionType, Separator, ToggleButton,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PaintTool {
    #[default]
    Brush,
    Eraser,
    Eyedropper,
    FillSelection,
}

struct Callbacks {
    on_tint_changed: Box<dyn Fn(i64)>,
    on_shape_changed: Box<dyn Fn(Shape)>,
    on_tool_changed: Box<dyn Fn(PaintTool)>,
    on_paint_contents_changed: Box<dyn Fn(bool)>,
    on_undo: Box<dyn Fn()>,
    on_redo: Box<dyn Fn()>,
}

impl Default for Callbacks {
    fn default() -> Self {
        Self {
            on_tint_changed: Box::new(|_| {}),
            on_shape_changed: Box::new(|_| {}),
            on_tool_changed: Box::new(|_| {}),
            on_paint_contents_changed: Box::new(|_| {}),
            on_undo: Box::new(|| {}),
            on_redo: Box::new(|| {}),
        }
    }
}

struct Inner {
    tint_swatch: DrawingArea,
    tint_label: Label,
    tint_list: GtkBox,
    tint_popover: Popover,
    shape_label: Label,
    brush_btn: ToggleButton,
    eraser_btn: ToggleButton,
    eyedropper_btn: ToggleButton,
    fill_btn: ToggleButton,
    paint_contents_btn: ToggleButton,
    undo_btn: Button,
    redo_btn: Button,
    active_tint_color: Rc<RefCell<String>>,
    active_shape: Cell<Shape>,
    cbs: Callbacks,
}

#[derive(Clone)]
pub struct PaintingToolbar {
    pub revealer: Revealer,
    inner: Rc<RefCell<Inner>>,
}

impl PaintingToolbar {
    pub fn build() -> Self {
        let revealer = Revealer::new();
        revealer.set_transition_type(RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(150);
        revealer.set_reveal_child(false);

        let bar = GtkBox::new(Orientation::Horizontal, 4);
        bar.add_css_class("paint-toolbar");
        bar.set_margin_start(8);
        bar.set_margin_end(8);
        bar.set_margin_top(3);
        bar.set_margin_bottom(3);

        // Shared color Rc — both the swatch draw func and Inner hold a clone.
        let active_tint_color: Rc<RefCell<String>> = Rc::new(RefCell::new("#806040".to_string()));

        // ── Tint selector ──────────────────────────────────────────────
        let tint_swatch = DrawingArea::new();
        tint_swatch.set_content_width(14);
        tint_swatch.set_content_height(14);
        tint_swatch.add_css_class("paint-tint-swatch");
        {
            let color_rc = Rc::clone(&active_tint_color);
            tint_swatch.set_draw_func(move |_, cr, w, h| {
                let hex = color_rc.borrow();
                let (r, g, b) = parse_hex_color(&hex).unwrap_or((0.5, 0.38, 0.25));
                cr.arc(
                    w as f64 / 2.0,
                    h as f64 / 2.0,
                    (w.min(h) as f64 / 2.0) * 0.82,
                    0.0,
                    2.0 * std::f64::consts::PI,
                );
                cr.set_source_rgba(r, g, b, 1.0);
                let _ = cr.fill();
            });
        }

        let tint_label = Label::new(Some("Beige"));
        tint_label.add_css_class("paint-selector-label");
        tint_label.set_single_line_mode(true);

        let tint_btn_inner = GtkBox::new(Orientation::Horizontal, 4);
        tint_btn_inner.append(&tint_swatch);
        tint_btn_inner.append(&tint_label);
        let tint_btn = Button::new();
        tint_btn.set_child(Some(&tint_btn_inner));
        tint_btn.add_css_class("paint-selector-btn");
        crate::ui::attach_tooltip(&tint_btn, "Active tint — click to change");
        bar.append(&tint_btn);

        let tint_popover = Popover::new();
        tint_popover.add_css_class("paint-tint-popover");
        tint_popover.set_has_arrow(true);
        tint_popover.set_position(gtk::PositionType::Bottom);
        tint_popover.set_parent(&tint_btn);

        let tint_list = GtkBox::new(Orientation::Vertical, 2);
        tint_list.set_margin_top(4);
        tint_list.set_margin_bottom(4);
        tint_list.set_margin_start(4);
        tint_list.set_margin_end(4);
        tint_popover.set_child(Some(&tint_list));

        tint_btn.connect_clicked({
            let pop = tint_popover.clone();
            move |_| pop.popup()
        });

        // ── Shape selector ─────────────────────────────────────────────
        let shape_label = Label::new(Some("■ Square"));
        shape_label.add_css_class("paint-selector-label");
        shape_label.set_single_line_mode(true);
        let shape_btn = Button::new();
        shape_btn.set_child(Some(&shape_label));
        shape_btn.add_css_class("paint-selector-btn");
        crate::ui::attach_tooltip(&shape_btn, "Active shape — click to change");
        bar.append(&shape_btn);

        let shape_popover = Popover::new();
        shape_popover.add_css_class("paint-shape-popover");
        shape_popover.set_has_arrow(true);
        shape_popover.set_position(gtk::PositionType::Bottom);
        shape_popover.set_parent(&shape_btn);

        let shape_grid = GtkBox::new(Orientation::Horizontal, 4);
        shape_grid.set_margin_top(6);
        shape_grid.set_margin_bottom(6);
        shape_grid.set_margin_start(6);
        shape_grid.set_margin_end(6);
        shape_popover.set_child(Some(&shape_grid));

        shape_btn.connect_clicked({
            let pop = shape_popover.clone();
            move |_| pop.popup()
        });

        // ── Separator ──────────────────────────────────────────────────
        let sep1 = Separator::new(Orientation::Vertical);
        sep1.add_css_class("paint-toolbar-sep");
        bar.append(&sep1);

        // ── Tool buttons ───────────────────────────────────────────────
        let brush_btn = build_tool_toggle("🖌", "Brush — click or drag to apply mark");
        let eraser_btn = build_tool_toggle("◻", "Eraser — click or drag to reset mark");
        let eyedropper_btn = build_tool_toggle("💧", "Eyedropper — pick mark from a file");
        let fill_btn = build_tool_toggle("⬛", "Fill Selection — apply mark to all selected");

        eraser_btn.set_group(Some(&brush_btn));
        eyedropper_btn.set_group(Some(&brush_btn));
        fill_btn.set_group(Some(&brush_btn));
        brush_btn.set_active(true);

        bar.append(&brush_btn);
        bar.append(&eraser_btn);
        bar.append(&eyedropper_btn);
        bar.append(&fill_btn);

        // ── Separator ──────────────────────────────────────────────────
        let sep2 = Separator::new(Orientation::Vertical);
        sep2.add_css_class("paint-toolbar-sep");
        bar.append(&sep2);

        // ── Paint Contents toggle ───────────────────────────────────────
        let paint_contents_btn = ToggleButton::new();
        paint_contents_btn.set_label("📂 Contents");
        paint_contents_btn.add_css_class("paint-tool-btn");
        paint_contents_btn.set_active(false);
        crate::ui::attach_tooltip(
            &paint_contents_btn,
            "Paint Contents — when on, painting a folder marks its contents recursively",
        );
        bar.append(&paint_contents_btn);

        // ── Separator ──────────────────────────────────────────────────
        let sep3 = Separator::new(Orientation::Vertical);
        sep3.add_css_class("paint-toolbar-sep");
        bar.append(&sep3);

        // ── Undo / Redo ─────────────────────────────────────────────────
        let undo_btn = Button::with_label("↩");
        undo_btn.add_css_class("paint-tool-btn");
        undo_btn.set_sensitive(false);
        crate::ui::attach_tooltip(&undo_btn, "Undo last paint action (Ctrl+Z)");
        bar.append(&undo_btn);

        let redo_btn = Button::with_label("↪");
        redo_btn.add_css_class("paint-tool-btn");
        redo_btn.set_sensitive(false);
        crate::ui::attach_tooltip(&redo_btn, "Redo paint action (Ctrl+Y)");
        bar.append(&redo_btn);

        revealer.set_child(Some(&bar));

        let inner = Rc::new(RefCell::new(Inner {
            tint_swatch: tint_swatch.clone(),
            tint_label: tint_label.clone(),
            tint_list: tint_list.clone(),
            tint_popover: tint_popover.clone(),
            shape_label: shape_label.clone(),
            brush_btn: brush_btn.clone(),
            eraser_btn: eraser_btn.clone(),
            eyedropper_btn: eyedropper_btn.clone(),
            fill_btn: fill_btn.clone(),
            paint_contents_btn: paint_contents_btn.clone(),
            undo_btn: undo_btn.clone(),
            redo_btn: redo_btn.clone(),
            active_tint_color,
            active_shape: Cell::new(Shape::Square),
            cbs: Callbacks::default(),
        }));

        // Wire shape buttons
        for (shape, label_text) in SHAPES {
            let btn = Button::with_label(label_text);
            btn.add_css_class("paint-shape-option");
            let inner_rc = Rc::clone(&inner);
            let pop = shape_popover.clone();
            btn.connect_clicked(move |_| {
                pop.popdown();
                inner_rc.borrow().cbs.on_shape_changed.as_ref()(shape);
            });
            shape_grid.append(&btn);
        }

        // Wire tool buttons
        wire_tool_btn(&brush_btn, PaintTool::Brush, Rc::clone(&inner));
        wire_tool_btn(&eraser_btn, PaintTool::Eraser, Rc::clone(&inner));
        wire_tool_btn(&eyedropper_btn, PaintTool::Eyedropper, Rc::clone(&inner));
        wire_tool_btn(&fill_btn, PaintTool::FillSelection, Rc::clone(&inner));

        // Wire paint_contents
        {
            let inner_rc = Rc::clone(&inner);
            paint_contents_btn.connect_toggled(move |btn| {
                inner_rc.borrow().cbs.on_paint_contents_changed.as_ref()(btn.is_active());
            });
        }

        // Wire undo/redo
        {
            let inner_rc = Rc::clone(&inner);
            undo_btn.connect_clicked(move |_| {
                inner_rc.borrow().cbs.on_undo.as_ref()();
            });
        }
        {
            let inner_rc = Rc::clone(&inner);
            redo_btn.connect_clicked(move |_| {
                inner_rc.borrow().cbs.on_redo.as_ref()();
            });
        }

        PaintingToolbar { revealer, inner }
    }

    pub fn connect_tint_changed(&self, f: impl Fn(i64) + 'static) {
        self.inner.borrow_mut().cbs.on_tint_changed = Box::new(f);
    }

    pub fn connect_shape_changed(&self, f: impl Fn(Shape) + 'static) {
        self.inner.borrow_mut().cbs.on_shape_changed = Box::new(f);
    }

    pub fn connect_tool_changed(&self, f: impl Fn(PaintTool) + 'static) {
        self.inner.borrow_mut().cbs.on_tool_changed = Box::new(f);
    }

    pub fn connect_paint_contents_changed(&self, f: impl Fn(bool) + 'static) {
        self.inner.borrow_mut().cbs.on_paint_contents_changed = Box::new(f);
    }

    pub fn connect_undo(&self, f: impl Fn() + 'static) {
        self.inner.borrow_mut().cbs.on_undo = Box::new(f);
    }

    pub fn connect_redo(&self, f: impl Fn() + 'static) {
        self.inner.borrow_mut().cbs.on_redo = Box::new(f);
    }

    pub fn set_undo_enabled(&self, enabled: bool) {
        self.inner.borrow().undo_btn.set_sensitive(enabled);
    }

    pub fn set_redo_enabled(&self, enabled: bool) {
        self.inner.borrow().redo_btn.set_sensitive(enabled);
    }

    pub fn set_tints(&self, tints: &[TintRecord], active_id: i64) {
        let inner = self.inner.borrow();
        // Rebuild tint option list
        while let Some(child) = inner.tint_list.first_child() {
            inner.tint_list.remove(&child);
        }
        for tint in tints {
            let color = tint.color.as_deref().unwrap_or("#806040").to_string();
            let row = GtkBox::new(Orientation::Horizontal, 6);
            row.set_margin_start(2);
            row.set_margin_end(2);

            let swatch = tint_color_dot(&color);
            row.append(&swatch);

            let lbl = Label::new(Some(&tint.name));
            lbl.set_halign(gtk::Align::Start);
            lbl.set_hexpand(true);
            row.append(&lbl);

            if tint.id == active_id {
                let check = Label::new(Some("✓"));
                check.add_css_class("paint-tint-check");
                row.append(&check);
            }

            let btn = Button::new();
            btn.set_child(Some(&row));
            btn.add_css_class("paint-tint-option");

            let tint_id = tint.id;
            let pop = inner.tint_popover.clone();
            let inner_rc = Rc::clone(&self.inner);
            btn.connect_clicked(move |_| {
                pop.popdown();
                inner_rc.borrow().cbs.on_tint_changed.as_ref()(tint_id);
            });
            inner.tint_list.append(&btn);
        }

        if let Some(t) = tints.iter().find(|t| t.id == active_id) {
            let color = t.color.as_deref().unwrap_or("#806040");
            drop(inner);
            self.set_active_tint_display(color, &t.name);
        }
    }

    pub fn set_active_tint_display(&self, hex_color: &str, name: &str) {
        let inner = self.inner.borrow();
        *inner.active_tint_color.borrow_mut() = hex_color.to_string();
        inner.tint_label.set_text(name);
        inner.tint_swatch.queue_draw();
    }

    pub fn set_active_shape(&self, shape: Shape) {
        let inner = self.inner.borrow();
        inner.active_shape.set(shape);
        inner.shape_label.set_text(shape_label_text(shape));
    }

    pub fn set_active_tool(&self, tool: PaintTool) {
        let inner = self.inner.borrow();
        let btn = match tool {
            PaintTool::Brush => &inner.brush_btn,
            PaintTool::Eraser => &inner.eraser_btn,
            PaintTool::Eyedropper => &inner.eyedropper_btn,
            PaintTool::FillSelection => &inner.fill_btn,
        };
        if !btn.is_active() {
            btn.set_active(true);
        }
    }

    pub fn set_paint_contents(&self, on: bool) {
        let inner = self.inner.borrow();
        if inner.paint_contents_btn.is_active() != on {
            inner.paint_contents_btn.set_active(on);
        }
    }

    pub fn set_reveal(&self, reveal: bool) {
        self.revealer.set_reveal_child(reveal);
    }
}

const SHAPES: [(Shape, &str); 7] = [
    (Shape::Circle, "● Circle"),
    (Shape::Square, "■ Square"),
    (Shape::Triangle, "▲ Triangle"),
    (Shape::Pentagon, "⬠ Pentagon"),
    (Shape::Hexagon, "⬡ Hexagon"),
    (Shape::Octagon, "⯃ Octagon"),
    (Shape::Trapezoid, "⏢ Trapezoid"),
];

fn shape_label_text(shape: Shape) -> &'static str {
    match shape {
        Shape::Circle => "● Circle",
        Shape::Square => "■ Square",
        Shape::Triangle => "▲ Triangle",
        Shape::Pentagon => "⬠ Pentagon",
        Shape::Hexagon => "⬡ Hexagon",
        Shape::Octagon => "⯃ Octagon",
        Shape::Trapezoid => "⏢ Trapezoid",
    }
}

fn build_tool_toggle(icon: &str, tooltip: &str) -> ToggleButton {
    let btn = ToggleButton::with_label(icon);
    btn.add_css_class("paint-tool-btn");
    crate::ui::attach_tooltip(&btn, tooltip);
    btn
}

fn wire_tool_btn(btn: &ToggleButton, tool: PaintTool, inner: Rc<RefCell<Inner>>) {
    btn.connect_toggled(move |b| {
        if b.is_active() {
            inner.borrow().cbs.on_tool_changed.as_ref()(tool);
        }
    });
}

fn tint_color_dot(hex: &str) -> DrawingArea {
    let color = hex.to_string();
    let area = DrawingArea::new();
    area.set_content_width(12);
    area.set_content_height(12);
    area.set_draw_func(move |_, cr, w, h| {
        let (r, g, b) = parse_hex_color(&color).unwrap_or((0.5, 0.38, 0.25));
        cr.arc(
            w as f64 / 2.0,
            h as f64 / 2.0,
            (w.min(h) as f64 / 2.0) * 0.82,
            0.0,
            2.0 * std::f64::consts::PI,
        );
        cr.set_source_rgba(r, g, b, 1.0);
        let _ = cr.fill();
    });
    area
}

fn parse_hex_color(hex: &str) -> Option<(f64, f64, f64)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}
