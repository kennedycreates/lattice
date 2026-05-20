use crate::metadata::TagRecord;
use crate::ui::file_grid::FileItem;
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, FlowBox, Label, Orientation, Separator, ToggleButton};
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
}

impl TagFilterSpec {
    pub fn is_empty(&self) -> bool {
        self.active_ids.is_empty()
    }

    pub fn active_count(&self) -> usize {
        self.active_ids.len()
    }

    pub fn matches(&self, item: &FileItem) -> bool {
        if self.active_ids.is_empty() {
            return true;
        }
        match self.mode {
            CombineMode::And => self
                .active_ids
                .iter()
                .all(|id| item.tags.iter().any(|t| t.id == *id)),
            CombineMode::Or => self
                .active_ids
                .iter()
                .any(|id| item.tags.iter().any(|t| t.id == *id)),
        }
    }
}

// ── Internal state ─────────────────────────────────────────────────────────────

struct State {
    spec: RefCell<TagFilterSpec>,
    tags: RefCell<Vec<TagRecord>>,
    chip_flow: FlowBox,
    chip_btns: RefCell<Vec<(i64, ToggleButton)>>,
    mode_btn: Button,
    clear_btn: Button,
    header_label: Label,
    active_chips_row: GtkBox,
    active_chips_flow: FlowBox,
    empty_hint: Label,
    on_change: RefCell<Option<Box<dyn Fn(TagFilterSpec)>>>,
}

impl State {
    fn refresh_header(&self) {
        let spec = self.spec.borrow();
        let count = spec.active_ids.len();

        if count == 0 {
            self.header_label.set_label("🏷  Filter by Tags");
            self.mode_btn.set_visible(false);
            self.clear_btn.set_visible(false);
            self.active_chips_row.set_visible(false);
        } else {
            self.header_label
                .set_label(&format!("🏷  Filter  ·  {} active", count));
            self.mode_btn.set_label(spec.mode.label());
            self.mode_btn.set_visible(count >= 2);
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

        let header_label = Label::new(Some("🏷  Filter by Tags"));
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
        crate::ui::attach_tooltip(&clear_btn, "Clear tag filters");
        header_row.append(&clear_btn);

        inner.append(&header_row);

        // ── Active chips strip (compact display of selected tags) ────────────
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

        // ── Tag chips area ─────────────────────────────────────────────────────
        let chips_wrap = GtkBox::new(Orientation::Vertical, 0);
        chips_wrap.add_css_class("tf-chips-wrap");

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

        inner.append(&chips_wrap);

        // ── Wire state ─────────────────────────────────────────────────────────
        let state = Rc::new(State {
            spec: RefCell::new(TagFilterSpec::default()),
            tags: RefCell::new(Vec::new()),
            chip_flow: chip_flow.clone(),
            chip_btns: RefCell::new(Vec::new()),
            mode_btn: mode_btn.clone(),
            clear_btn: clear_btn.clone(),
            header_label: header_label.clone(),
            active_chips_row,
            active_chips_flow,
            empty_hint,
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
                    state.spec.borrow_mut().active_ids.clear();
                }
                for (_, btn) in state.chip_btns.borrow().iter() {
                    btn.set_active(false);
                    btn.remove_css_class("tf-chip-active");
                }
                state.refresh_header();
                state.notify_change();
            });
        }

        Self { root, state }
    }

    // ── Public API ─────────────────────────────────────────────────────────────

    pub fn set_tags(&self, tags: &[TagRecord]) {
        *self.state.tags.borrow_mut() = tags.to_vec();
        self.rebuild_chips();
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
        self.state.spec.borrow_mut().active_ids.clear();
        for (_, btn) in self.state.chip_btns.borrow().iter() {
            btn.set_active(false);
            btn.remove_css_class("tf-chip-active");
        }
        self.state.refresh_header();
    }

    pub fn connect_changed(&self, callback: impl Fn(TagFilterSpec) + 'static) {
        *self.state.on_change.borrow_mut() = Some(Box::new(callback));
    }

    // ── Internal rebuild ───────────────────────────────────────────────────────

    fn rebuild_chips(&self) {
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
}
