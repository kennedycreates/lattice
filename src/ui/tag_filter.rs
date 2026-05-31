use crate::metadata::{Shape, TagRecord, TintRecord};
use crate::ui::file_grid::FileItem;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, DrawingArea, FlowBox, Label, Orientation, Separator, ToggleButton,
};
use std::cell::RefCell;
use std::rc::Rc;

// ── Public data model ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CombineMode {
    #[default]
    And,
    Or,
}

impl CombineMode {
    fn label(self) -> &'static str {
        match self {
            Self::And => "All match",
            Self::Or => "Any match",
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::And => Self::Or,
            Self::Or => Self::And,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TagFilterSpec {
    pub active_ids: Vec<i64>,
    pub mode: CombineMode,
    pub active_tint_ids: Vec<i64>,
    pub active_shapes: Vec<Shape>,
}

impl TagFilterSpec {
    pub fn is_empty(&self) -> bool {
        self.active_ids.is_empty()
            && self.active_tint_ids.is_empty()
            && self.active_shapes.is_empty()
    }

    pub fn active_count(&self) -> usize {
        self.active_ids.len() + self.active_tint_ids.len() + self.active_shapes.len()
    }

    pub fn matches(&self, item: &FileItem) -> bool {
        // Tag filter (existing And/Or logic)
        if !self.active_ids.is_empty() {
            let tag_match = match self.mode {
                CombineMode::And => self
                    .active_ids
                    .iter()
                    .all(|id| item.tags.iter().any(|t| t.id == *id)),
                CombineMode::Or => self
                    .active_ids
                    .iter()
                    .any(|id| item.tags.iter().any(|t| t.id == *id)),
            };
            if !tag_match {
                return false;
            }
        }
        // Tint filter (OR within selected tints)
        if !self.active_tint_ids.is_empty() && !self.active_tint_ids.contains(&item.mark_tint_id) {
            return false;
        }
        // Shape filter (OR within selected shapes)
        if !self.active_shapes.is_empty() && !self.active_shapes.contains(&item.mark_shape) {
            return false;
        }
        true
    }
}

// ── Internal state ─────────────────────────────────────────────────────────────

struct State {
    spec: RefCell<TagFilterSpec>,
    // Tags
    tags: RefCell<Vec<TagRecord>>,
    chip_flow: FlowBox,
    chip_btns: RefCell<Vec<(i64, ToggleButton)>>,
    empty_hint: Label,
    // Tints
    tints: RefCell<Vec<TintRecord>>,
    tint_section: GtkBox,
    tint_flow: FlowBox,
    tint_chip_btns: RefCell<Vec<(i64, ToggleButton)>>,
    // Shapes
    shape_flow: FlowBox,
    shape_chip_btns: RefCell<Vec<(Shape, ToggleButton)>>,
    // Header controls
    mode_btn: Button,
    clear_btn: Button,
    header_label: Label,
    active_chips_row: GtkBox,
    active_chips_flow: FlowBox,
    on_change: RefCell<Option<Box<dyn Fn(TagFilterSpec)>>>,
}

impl State {
    fn refresh_header(&self) {
        let spec = self.spec.borrow();
        let tag_count = spec.active_ids.len();
        let total = spec.active_count();

        if total == 0 {
            self.header_label.set_label("🏷  Filter by Marks & Tags");
            self.mode_btn.set_visible(false);
            self.clear_btn.set_visible(false);
            self.active_chips_row.set_visible(false);
        } else {
            self.header_label
                .set_label(&format!("🏷  Filter  ·  {} active", total));
            self.mode_btn.set_label(spec.mode.label());
            self.mode_btn.set_visible(tag_count >= 2);
            self.clear_btn.set_visible(true);
            self.rebuild_active_chips(&spec);
            self.active_chips_row.set_visible(true);
        }
    }

    fn rebuild_active_chips(&self, spec: &TagFilterSpec) {
        while let Some(child) = self.active_chips_flow.first_child() {
            self.active_chips_flow.remove(&child);
        }
        let tags = self.tags.borrow();
        for id in &spec.active_ids {
            if let Some(tag) = tags.iter().find(|t| t.id == *id) {
                let chip = Label::new(Some(&format!("#{}", tag.name)));
                chip.add_css_class("tf-active-chip");
                self.active_chips_flow.append(&chip);
            }
        }
        drop(tags);
        let tints = self.tints.borrow();
        for id in &spec.active_tint_ids {
            if let Some(tint) = tints.iter().find(|t| t.id == *id) {
                let chip = Label::new(Some(&format!("● {}", tint.name)));
                chip.add_css_class("tf-active-chip");
                self.active_chips_flow.append(&chip);
            }
        }
        drop(tints);
        for shape in &spec.active_shapes {
            let chip = Label::new(Some(shape_chip_label(*shape)));
            chip.add_css_class("tf-active-chip");
            self.active_chips_flow.append(&chip);
        }
    }

    fn notify_change(&self) {
        let spec = self.spec.borrow().clone();
        if let Some(cb) = self.on_change.borrow().as_ref() {
            cb(spec);
        }
    }
}

// ── Public widget ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TagFilterPanel {
    pub root: GtkBox,
    state: Rc<State>,
}

impl TagFilterPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("tf-panel");

        let inner = GtkBox::new(Orientation::Vertical, 0);
        inner.add_css_class("tf-inner");
        root.append(&inner);

        // ── Header row ─────────────────────────────────────────────────────────
        let header_row = GtkBox::new(Orientation::Horizontal, 8);
        header_row.add_css_class("tf-header-row");

        let header_label = Label::new(Some("🏷  Filter by Marks & Tags"));
        header_label.add_css_class("tf-header-label");
        header_label.set_halign(Align::Start);
        header_label.set_hexpand(true);
        header_row.append(&header_label);

        let mode_btn = Button::with_label("All match");
        mode_btn.add_css_class("tf-mode-btn");
        mode_btn.set_visible(false);
        crate::ui::attach_tooltip(&mode_btn, "Switch tag match mode");
        header_row.append(&mode_btn);

        let clear_btn = Button::with_label("Clear All");
        clear_btn.add_css_class("tf-clear-btn");
        clear_btn.set_visible(false);
        crate::ui::attach_tooltip(&clear_btn, "Clear all filters");
        header_row.append(&clear_btn);

        inner.append(&header_row);

        // ── Active chips strip ──────────────────────────────────────────────────
        let active_chips_row = GtkBox::new(Orientation::Horizontal, 8);
        active_chips_row.add_css_class("tf-active-row");
        active_chips_row.set_visible(false);

        let active_label = Label::new(Some("Active:"));
        active_label.add_css_class("tf-active-label");
        active_chips_row.append(&active_label);

        let active_chips_flow = FlowBox::new();
        active_chips_flow.add_css_class("tf-active-chips");
        active_chips_flow.set_selection_mode(gtk::SelectionMode::None);
        active_chips_flow.set_homogeneous(false);
        active_chips_flow.set_column_spacing(4);
        active_chips_flow.set_row_spacing(0);
        active_chips_flow.set_max_children_per_line(64);
        active_chips_flow.set_hexpand(true);
        active_chips_row.append(&active_chips_flow);

        inner.append(&active_chips_row);

        // ── Divider ────────────────────────────────────────────────────────────
        let sep = Separator::new(Orientation::Horizontal);
        sep.add_css_class("tf-sep");
        inner.append(&sep);

        // ── Chips area (tags + tints + shapes) ─────────────────────────────────
        let chips_wrap = GtkBox::new(Orientation::Vertical, 0);
        chips_wrap.add_css_class("tf-chips-wrap");

        // Tags subsection
        let tag_section_label = Label::new(Some("Tags"));
        tag_section_label.add_css_class("tf-section-label");
        tag_section_label.set_halign(Align::Start);
        chips_wrap.append(&tag_section_label);

        let chip_flow = FlowBox::new();
        chip_flow.add_css_class("tf-chips");
        chip_flow.set_selection_mode(gtk::SelectionMode::None);
        chip_flow.set_homogeneous(false);
        chip_flow.set_column_spacing(6);
        chip_flow.set_row_spacing(6);
        chip_flow.set_max_children_per_line(64);
        chips_wrap.append(&chip_flow);

        let empty_hint = Label::new(Some(
            "No tags yet. Right-click any file and choose Add Tag to get started.",
        ));
        empty_hint.add_css_class("tf-empty-hint");
        empty_hint.set_halign(Align::Start);
        empty_hint.set_wrap(true);
        empty_hint.set_visible(false);
        chips_wrap.append(&empty_hint);

        // Tints subsection (hidden until tints are available)
        let tint_section = GtkBox::new(Orientation::Vertical, 0);
        tint_section.set_visible(false);

        let tint_sep = Separator::new(Orientation::Horizontal);
        tint_sep.add_css_class("tf-sep");
        tint_section.append(&tint_sep);

        let tint_section_label = Label::new(Some("Tints"));
        tint_section_label.add_css_class("tf-section-label");
        tint_section_label.set_halign(Align::Start);
        tint_section.append(&tint_section_label);

        let tint_flow = FlowBox::new();
        tint_flow.add_css_class("tf-chips");
        tint_flow.set_selection_mode(gtk::SelectionMode::None);
        tint_flow.set_homogeneous(false);
        tint_flow.set_column_spacing(6);
        tint_flow.set_row_spacing(6);
        tint_flow.set_max_children_per_line(64);
        tint_section.append(&tint_flow);

        chips_wrap.append(&tint_section);

        // Shapes subsection
        let shape_sep = Separator::new(Orientation::Horizontal);
        shape_sep.add_css_class("tf-sep");
        chips_wrap.append(&shape_sep);

        let shape_section_label = Label::new(Some("Shapes"));
        shape_section_label.add_css_class("tf-section-label");
        shape_section_label.set_halign(Align::Start);
        chips_wrap.append(&shape_section_label);

        let shape_flow = FlowBox::new();
        shape_flow.add_css_class("tf-chips");
        shape_flow.set_selection_mode(gtk::SelectionMode::None);
        shape_flow.set_homogeneous(false);
        shape_flow.set_column_spacing(6);
        shape_flow.set_row_spacing(6);
        shape_flow.set_max_children_per_line(64);
        chips_wrap.append(&shape_flow);

        inner.append(&chips_wrap);

        // ── Wire state ─────────────────────────────────────────────────────────
        let state = Rc::new(State {
            spec: RefCell::new(TagFilterSpec::default()),
            tags: RefCell::new(Vec::new()),
            chip_flow: chip_flow.clone(),
            chip_btns: RefCell::new(Vec::new()),
            empty_hint,
            tints: RefCell::new(Vec::new()),
            tint_section: tint_section.clone(),
            tint_flow: tint_flow.clone(),
            tint_chip_btns: RefCell::new(Vec::new()),
            shape_flow: shape_flow.clone(),
            shape_chip_btns: RefCell::new(Vec::new()),
            mode_btn: mode_btn.clone(),
            clear_btn: clear_btn.clone(),
            header_label: header_label.clone(),
            active_chips_row,
            active_chips_flow,
            on_change: RefCell::new(None),
        });

        {
            let state = Rc::clone(&state);
            mode_btn.connect_clicked(move |_| {
                {
                    let mut spec = state.spec.borrow_mut();
                    spec.mode = spec.mode.toggled();
                }
                state.refresh_header();
                state.notify_change();
            });
        }

        {
            let state = Rc::clone(&state);
            clear_btn.connect_clicked(move |_| {
                {
                    let mut spec = state.spec.borrow_mut();
                    spec.active_ids.clear();
                    spec.active_tint_ids.clear();
                    spec.active_shapes.clear();
                }
                for (_, btn) in state.chip_btns.borrow().iter() {
                    btn.set_active(false);
                    btn.remove_css_class("tf-chip-active");
                }
                for (_, btn) in state.tint_chip_btns.borrow().iter() {
                    btn.set_active(false);
                    btn.remove_css_class("tf-chip-active");
                }
                for (_, btn) in state.shape_chip_btns.borrow().iter() {
                    btn.set_active(false);
                    btn.remove_css_class("tf-chip-active");
                }
                state.refresh_header();
                state.notify_change();
            });
        }

        let panel = Self { root, state };
        panel.build_shape_chips();
        panel
    }

    // ── Public API ─────────────────────────────────────────────────────────────

    pub fn set_tags(&self, tags: &[TagRecord]) {
        *self.state.tags.borrow_mut() = tags.to_vec();
        self.rebuild_tag_chips();
    }

    pub fn set_tints(&self, tints: &[TintRecord]) {
        *self.state.tints.borrow_mut() = tints.to_vec();
        self.rebuild_tint_chips();
    }

    pub fn spec(&self) -> TagFilterSpec {
        self.state.spec.borrow().clone()
    }

    pub fn is_filtering(&self) -> bool {
        !self.state.spec.borrow().is_empty()
    }

    pub fn active_count(&self) -> usize {
        self.state.spec.borrow().active_count()
    }

    #[allow(dead_code)]
    pub fn clear(&self) {
        {
            let mut spec = self.state.spec.borrow_mut();
            spec.active_ids.clear();
            spec.active_tint_ids.clear();
            spec.active_shapes.clear();
        }
        for (_, btn) in self.state.chip_btns.borrow().iter() {
            btn.set_active(false);
            btn.remove_css_class("tf-chip-active");
        }
        for (_, btn) in self.state.tint_chip_btns.borrow().iter() {
            btn.set_active(false);
            btn.remove_css_class("tf-chip-active");
        }
        for (_, btn) in self.state.shape_chip_btns.borrow().iter() {
            btn.set_active(false);
            btn.remove_css_class("tf-chip-active");
        }
        self.state.refresh_header();
    }

    pub fn connect_changed(&self, callback: impl Fn(TagFilterSpec) + 'static) {
        *self.state.on_change.borrow_mut() = Some(Box::new(callback));
    }

    // ── Internal rebuilds ──────────────────────────────────────────────────────

    fn rebuild_tag_chips(&self) {
        let flow = &self.state.chip_flow;
        while let Some(child) = flow.first_child() {
            flow.remove(&child);
        }

        let tags = self.state.tags.borrow().clone();
        let mut btns = Vec::with_capacity(tags.len());

        for tag in &tags {
            let label = format!("#{}", tag.name);
            let btn = ToggleButton::with_label(&label);
            btn.add_css_class("tf-chip");

            let is_active = self.state.spec.borrow().active_ids.contains(&tag.id);
            btn.set_active(is_active);
            if is_active {
                btn.add_css_class("tf-chip-active");
            }

            let tag_id = tag.id;
            let state = Rc::clone(&self.state);
            btn.connect_toggled(move |b| {
                {
                    let mut spec = state.spec.borrow_mut();
                    if b.is_active() {
                        b.add_css_class("tf-chip-active");
                        if !spec.active_ids.contains(&tag_id) {
                            spec.active_ids.push(tag_id);
                        }
                    } else {
                        b.remove_css_class("tf-chip-active");
                        spec.active_ids.retain(|id| *id != tag_id);
                    }
                }
                state.refresh_header();
                state.notify_change();
            });

            flow.append(&btn);
            btns.push((tag.id, btn));
        }

        *self.state.chip_btns.borrow_mut() = btns;

        let has_tags = !tags.is_empty();
        self.state.chip_flow.set_visible(has_tags);
        self.state.empty_hint.set_visible(!has_tags);
        self.state.refresh_header();
    }

    fn rebuild_tint_chips(&self) {
        let flow = &self.state.tint_flow;
        while let Some(child) = flow.first_child() {
            flow.remove(&child);
        }

        let tints = self.state.tints.borrow().clone();
        let mut btns = Vec::with_capacity(tints.len());

        for tint in &tints {
            let hex = tint.color.as_deref().unwrap_or("#806040").to_string();
            let hex_rc = Rc::new(hex);

            let swatch = DrawingArea::new();
            swatch.set_content_width(10);
            swatch.set_content_height(10);
            swatch.set_draw_func({
                let hex_rc = Rc::clone(&hex_rc);
                move |_, cr, w, h| {
                    let (r, g, b) = parse_hex_color(&hex_rc).unwrap_or((0.5, 0.38, 0.25));
                    cr.arc(
                        w as f64 / 2.0,
                        h as f64 / 2.0,
                        (w.min(h) as f64 / 2.0) * 0.85,
                        0.0,
                        2.0 * std::f64::consts::PI,
                    );
                    cr.set_source_rgba(r, g, b, 1.0);
                    let _ = cr.fill();
                }
            });

            let lbl = Label::new(Some(&tint.name));
            let row = GtkBox::new(Orientation::Horizontal, 4);
            row.append(&swatch);
            row.append(&lbl);

            let btn = ToggleButton::new();
            btn.set_child(Some(&row));
            btn.add_css_class("tf-chip");

            let tint_id = tint.id;
            let is_active = self.state.spec.borrow().active_tint_ids.contains(&tint_id);
            btn.set_active(is_active);
            if is_active {
                btn.add_css_class("tf-chip-active");
            }

            let state = Rc::clone(&self.state);
            btn.connect_toggled(move |b| {
                {
                    let mut spec = state.spec.borrow_mut();
                    if b.is_active() {
                        b.add_css_class("tf-chip-active");
                        if !spec.active_tint_ids.contains(&tint_id) {
                            spec.active_tint_ids.push(tint_id);
                        }
                    } else {
                        b.remove_css_class("tf-chip-active");
                        spec.active_tint_ids.retain(|id| *id != tint_id);
                    }
                }
                state.refresh_header();
                state.notify_change();
            });

            flow.append(&btn);
            btns.push((tint_id, btn));
        }

        *self.state.tint_chip_btns.borrow_mut() = btns;
        self.state.tint_section.set_visible(!tints.is_empty());
        self.state.refresh_header();
    }

    fn build_shape_chips(&self) {
        let flow = &self.state.shape_flow;
        let mut btns = Vec::new();

        for (shape, label) in SHAPE_CHIPS {
            let btn = ToggleButton::with_label(label);
            btn.add_css_class("tf-chip");

            let state = Rc::clone(&self.state);
            btn.connect_toggled(move |b| {
                {
                    let mut spec = state.spec.borrow_mut();
                    if b.is_active() {
                        b.add_css_class("tf-chip-active");
                        if !spec.active_shapes.contains(&shape) {
                            spec.active_shapes.push(shape);
                        }
                    } else {
                        b.remove_css_class("tf-chip-active");
                        spec.active_shapes.retain(|s| *s != shape);
                    }
                }
                state.refresh_header();
                state.notify_change();
            });

            flow.append(&btn);
            btns.push((shape, btn));
        }

        *self.state.shape_chip_btns.borrow_mut() = btns;
    }
}

const SHAPE_CHIPS: [(Shape, &str); 7] = [
    (Shape::Circle, "● Circle"),
    (Shape::Square, "■ Square"),
    (Shape::Triangle, "▲ Triangle"),
    (Shape::Pentagon, "⬠ Pentagon"),
    (Shape::Hexagon, "⬡ Hexagon"),
    (Shape::Octagon, "⯃ Octagon"),
    (Shape::Trapezoid, "⏢ Trapezoid"),
];

fn shape_chip_label(shape: Shape) -> &'static str {
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
