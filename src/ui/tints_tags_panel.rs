use crate::metadata::{Shape, TagRecord, TintRecord};
use gtk::cairo;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, DrawingArea, Entry, EventControllerKey, FlowBox, Label,
    Orientation, Revealer, RevealerTransitionType, ScrolledWindow, Stack,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;
use std::rc::Rc;

// ── Callbacks ──────────────────────────────────────────────────────────────────

type TagClickedCallback = Rc<dyn Fn(i64)>;
type TagCreatedCallback = Rc<dyn Fn(String)>;
type TagRenamedCallback = Rc<dyn Fn(i64, String)>;
type TagDeletedCallback = Rc<dyn Fn(i64)>;
type TintCreatedCallback = Rc<dyn Fn(String, String)>;
type TintRenamedCallback = Rc<dyn Fn(i64, String)>;
type TintColorChangedCallback = Rc<dyn Fn(i64, String)>;
type TintColorPickCallback = Rc<dyn Fn(String, String, Box<dyn Fn(String)>)>;
type TintDeletedCallback = Rc<dyn Fn(i64)>;

struct Callbacks {
    on_tag_clicked: RefCell<Option<TagClickedCallback>>,
    on_tag_created: RefCell<Option<TagCreatedCallback>>,
    on_tag_renamed: RefCell<Option<TagRenamedCallback>>,
    on_tag_deleted: RefCell<Option<TagDeletedCallback>>,
    on_tint_created: RefCell<Option<TintCreatedCallback>>,
    on_tint_renamed: RefCell<Option<TintRenamedCallback>>,
    on_tint_color_changed: RefCell<Option<TintColorChangedCallback>>,
    on_tint_color_pick_requested: RefCell<Option<TintColorPickCallback>>,
    on_tint_deleted: RefCell<Option<TintDeletedCallback>>,
}

impl Callbacks {
    fn new() -> Self {
        Self {
            on_tag_clicked: RefCell::new(None),
            on_tag_created: RefCell::new(None),
            on_tag_renamed: RefCell::new(None),
            on_tag_deleted: RefCell::new(None),
            on_tint_created: RefCell::new(None),
            on_tint_renamed: RefCell::new(None),
            on_tint_color_changed: RefCell::new(None),
            on_tint_color_pick_requested: RefCell::new(None),
            on_tint_deleted: RefCell::new(None),
        }
    }
}

// ── Inner state ────────────────────────────────────────────────────────────────

struct Inner {
    tints_list: GtkBox,
    tints_empty_hint: Label,
    tint_create_revealer: Revealer,
    tint_name_entry: Entry,
    tint_color_swatch: DrawingArea,
    tint_color_hex: Rc<RefCell<String>>,
    tint_color_label: Label,
    tags_list: GtkBox,
    tags_empty_hint: Label,
    tag_create_revealer: Revealer,
    tag_name_entry: Entry,
    cbs: Callbacks,
}

// ── Public widget ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TintsTagsPanel {
    pub root: GtkBox,
    inner: Rc<Inner>,
}

impl TintsTagsPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("tt-panel");
        root.set_visible(false);

        // ── Scrollable body ─────────────────────────────────────────────────────
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled.set_vscrollbar_policy(gtk::PolicyType::Automatic);
        let body = GtkBox::new(Orientation::Vertical, 0);
        scrolled.set_child(Some(&body));
        root.append(&scrolled);

        // ── Tints section ───────────────────────────────────────────────────────
        let (tints_sec, tints_content, tints_new_btn) = build_section("TINTS", Some("+ New Tint"));
        body.append(&tints_sec);

        let tint_create_revealer = Revealer::new();
        tint_create_revealer.set_transition_type(RevealerTransitionType::SlideDown);
        tint_create_revealer.set_transition_duration(160);
        tint_create_revealer.set_reveal_child(false);

        let tint_create_row = GtkBox::new(Orientation::Vertical, 6);
        tint_create_row.add_css_class("tt-create-row");

        let tint_name_entry = Entry::new();
        tint_name_entry.set_placeholder_text(Some("Tint name…"));
        tint_name_entry.add_css_class("tt-create-entry");
        tint_create_row.append(&tint_name_entry);

        let tint_color_row = GtkBox::new(Orientation::Horizontal, 8);
        tint_color_row.add_css_class("tt-picker-row");
        let (tint_color_button, tint_color_swatch, tint_color_hex) =
            build_color_swatch_button("#806040", 44, 30, "tt-picker-swatch");
        tint_color_row.append(&tint_color_button);
        let tint_color_text = GtkBox::new(Orientation::Vertical, 2);
        tint_color_text.set_hexpand(true);
        let tint_color_title = Label::new(Some("Tint color"));
        tint_color_title.add_css_class("tt-picker-title");
        tint_color_title.set_halign(Align::Start);
        let tint_color_label = Label::new(Some("#806040"));
        tint_color_label.add_css_class("tt-picker-hex");
        tint_color_label.set_halign(Align::Start);
        tint_color_text.append(&tint_color_title);
        tint_color_text.append(&tint_color_label);
        tint_color_row.append(&tint_color_text);
        tint_create_row.append(&tint_color_row);

        let tint_form_actions = GtkBox::new(Orientation::Horizontal, 6);
        tint_form_actions.add_css_class("tt-create-actions");
        let tint_create_btn = Button::with_label("Create");
        tint_create_btn.add_css_class("tt-create-btn");
        let tint_cancel_btn = Button::with_label("Cancel");
        tint_cancel_btn.add_css_class("tt-cancel-btn");
        tint_form_actions.append(&tint_create_btn);
        tint_form_actions.append(&tint_cancel_btn);
        tint_create_row.append(&tint_form_actions);
        tint_create_revealer.set_child(Some(&tint_create_row));
        tints_content.append(&tint_create_revealer);

        let tints_list = GtkBox::new(Orientation::Vertical, 0);
        tints_list.add_css_class("tt-list");
        tints_content.append(&tints_list);

        let tints_empty_hint = Label::new(Some("No tints yet. Create one with + New Tint above."));
        tints_empty_hint.add_css_class("tt-empty");
        tints_empty_hint.set_halign(Align::Start);
        tints_empty_hint.set_wrap(true);
        tints_empty_hint.set_visible(false);
        tints_content.append(&tints_empty_hint);

        // ── Tags section ────────────────────────────────────────────────────────
        let (tags_sec, tags_content, tags_new_btn) = build_section("TAGS", Some("+ New Tag"));
        body.append(&tags_sec);

        let tag_create_revealer = Revealer::new();
        tag_create_revealer.set_transition_type(RevealerTransitionType::SlideDown);
        tag_create_revealer.set_transition_duration(160);
        tag_create_revealer.set_reveal_child(false);

        let tag_create_row = GtkBox::new(Orientation::Vertical, 6);
        tag_create_row.add_css_class("tt-create-row");

        let tag_name_entry = Entry::new();
        tag_name_entry.set_placeholder_text(Some("Tag name…"));
        tag_name_entry.add_css_class("tt-create-entry");
        tag_create_row.append(&tag_name_entry);

        let tag_form_actions = GtkBox::new(Orientation::Horizontal, 6);
        tag_form_actions.add_css_class("tt-create-actions");
        let tag_create_btn = Button::with_label("Create");
        tag_create_btn.add_css_class("tt-create-btn");
        let tag_cancel_btn = Button::with_label("Cancel");
        tag_cancel_btn.add_css_class("tt-cancel-btn");
        tag_form_actions.append(&tag_create_btn);
        tag_form_actions.append(&tag_cancel_btn);
        tag_create_row.append(&tag_form_actions);
        tag_create_revealer.set_child(Some(&tag_create_row));
        tags_content.append(&tag_create_revealer);

        let tags_list = GtkBox::new(Orientation::Vertical, 0);
        tags_list.add_css_class("tt-list");
        tags_content.append(&tags_list);

        let tags_empty_hint = Label::new(Some("No tags yet. Create one with + New Tag above."));
        tags_empty_hint.add_css_class("tt-empty");
        tags_empty_hint.set_halign(Align::Start);
        tags_empty_hint.set_wrap(true);
        tags_empty_hint.set_visible(false);
        tags_content.append(&tags_empty_hint);

        // ── Shapes section ──────────────────────────────────────────────────────
        let (shapes_sec, shapes_content, _) = build_section("SHAPES", None);
        body.append(&shapes_sec);

        let shapes_flow = FlowBox::new();
        shapes_flow.add_css_class("tt-shapes-grid");
        shapes_flow.set_selection_mode(gtk::SelectionMode::None);
        shapes_flow.set_homogeneous(true);
        shapes_flow.set_column_spacing(4);
        shapes_flow.set_row_spacing(8);
        shapes_flow.set_max_children_per_line(4);
        shapes_flow.set_margin_start(14);
        shapes_flow.set_margin_end(14);
        shapes_flow.set_margin_top(10);
        shapes_flow.set_margin_bottom(14);
        for shape in [
            Shape::Circle,
            Shape::Square,
            Shape::Triangle,
            Shape::Pentagon,
            Shape::Hexagon,
            Shape::Octagon,
            Shape::Trapezoid,
        ] {
            shapes_flow.append(&build_shape_badge(shape));
        }
        shapes_content.append(&shapes_flow);

        // ── Assemble inner ──────────────────────────────────────────────────────
        let inner = Rc::new(Inner {
            tints_list,
            tints_empty_hint,
            tint_create_revealer: tint_create_revealer.clone(),
            tint_name_entry: tint_name_entry.clone(),
            tint_color_swatch: tint_color_swatch.clone(),
            tint_color_hex: Rc::clone(&tint_color_hex),
            tint_color_label: tint_color_label.clone(),
            tags_list,
            tags_empty_hint,
            tag_create_revealer: tag_create_revealer.clone(),
            tag_name_entry: tag_name_entry.clone(),
            cbs: Callbacks::new(),
        });

        // ── Wire + New Tint ─────────────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            let btn = tints_new_btn.expect("tints new btn");
            btn.connect_clicked(move |_| {
                let open = !inner.tint_create_revealer.reveals_child();
                inner.tint_create_revealer.set_reveal_child(open);
                if open {
                    inner.tint_name_entry.set_text("");
                    *inner.tint_color_hex.borrow_mut() = "#806040".to_string();
                    inner.tint_color_swatch.queue_draw();
                    inner.tint_color_label.set_text("#806040");
                    inner.tint_name_entry.grab_focus();
                }
            });
        }
        {
            let inner = Rc::clone(&inner);
            let tint_color_hex = Rc::clone(&tint_color_hex);
            let tint_color_swatch = tint_color_swatch.clone();
            let tint_color_label = tint_color_label.clone();
            tint_color_button.set_tooltip_text(Some("Choose Tint color"));
            tint_color_button.connect_clicked(move |_| {
                let initial = tint_color_hex.borrow().clone();
                let tint_color_hex = Rc::clone(&tint_color_hex);
                let tint_color_swatch = tint_color_swatch.clone();
                let tint_color_label = tint_color_label.clone();
                let cb = inner.cbs.on_tint_color_pick_requested.borrow().clone();
                if let Some(cb) = cb {
                    cb(
                        "Choose New Tint Color".to_string(),
                        initial,
                        Box::new(move |hex| {
                            *tint_color_hex.borrow_mut() = hex.clone();
                            tint_color_swatch.queue_draw();
                            tint_color_label.set_text(&hex);
                        }),
                    );
                }
            });
        }
        {
            let inner = Rc::clone(&inner);
            tint_create_btn.connect_clicked(move |_| fire_tint_create(&inner));
        }
        {
            let inner = Rc::clone(&inner);
            tint_name_entry.connect_activate(move |_| fire_tint_create(&inner));
        }
        {
            let inner = Rc::clone(&inner);
            tint_cancel_btn.connect_clicked(move |_| {
                inner.tint_create_revealer.set_reveal_child(false);
            });
        }

        // ── Wire + New Tag ──────────────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            let btn = tags_new_btn.expect("tags new btn");
            btn.connect_clicked(move |_| {
                let open = !inner.tag_create_revealer.reveals_child();
                inner.tag_create_revealer.set_reveal_child(open);
                if open {
                    inner.tag_name_entry.set_text("");
                    inner.tag_name_entry.grab_focus();
                }
            });
        }
        {
            let inner = Rc::clone(&inner);
            tag_create_btn.connect_clicked(move |_| fire_tag_create(&inner));
        }
        {
            let inner = Rc::clone(&inner);
            tag_name_entry.connect_activate(move |_| fire_tag_create(&inner));
        }
        {
            let inner = Rc::clone(&inner);
            tag_cancel_btn.connect_clicked(move |_| {
                inner.tag_create_revealer.set_reveal_child(false);
            });
        }

        Self { root, inner }
    }

    // ── Data loading ───────────────────────────────────────────────────────────

    pub fn set_tints(&self, tints: &[TintRecord]) {
        clear_box(&self.inner.tints_list);
        if tints.is_empty() {
            self.inner.tints_empty_hint.set_visible(true);
            self.inner.tints_list.set_visible(false);
            return;
        }
        self.inner.tints_empty_hint.set_visible(false);
        self.inner.tints_list.set_visible(true);
        for tint in tints {
            let row = build_tint_row(tint, &self.inner);
            self.inner.tints_list.append(&row);
        }
    }

    pub fn set_tags(&self, tags: &[TagRecord], counts: &HashMap<i64, usize>, tints: &[TintRecord]) {
        clear_box(&self.inner.tags_list);
        if tags.is_empty() {
            self.inner.tags_empty_hint.set_visible(true);
            self.inner.tags_list.set_visible(false);
            return;
        }
        let tint_names: HashMap<i64, &str> =
            tints.iter().map(|t| (t.id, t.name.as_str())).collect();
        self.inner.tags_empty_hint.set_visible(false);
        self.inner.tags_list.set_visible(true);
        for tag in tags {
            let count = counts.get(&tag.id).copied().unwrap_or(0);
            let mark_hint = build_mark_hint(tag, &tint_names);
            let row = build_tag_row(tag, count, mark_hint.as_deref(), &self.inner);
            self.inner.tags_list.append(&row);
        }
    }

    // ── Callback wiring ────────────────────────────────────────────────────────

    pub fn connect_tag_clicked<F: Fn(i64) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_clicked.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tag_created<F: Fn(String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_created.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tag_renamed<F: Fn(i64, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_renamed.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tag_deleted<F: Fn(i64) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_deleted.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tint_created<F: Fn(String, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tint_created.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tint_renamed<F: Fn(i64, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tint_renamed.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tint_color_changed<F: Fn(i64, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tint_color_changed.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tint_color_pick_requested<F>(&self, f: F)
    where
        F: Fn(String, String, Box<dyn Fn(String)>) + 'static,
    {
        *self.inner.cbs.on_tint_color_pick_requested.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_tint_deleted<F: Fn(i64) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tint_deleted.borrow_mut() = Some(Rc::new(f));
    }
}

// ── Section builder ────────────────────────────────────────────────────────────
//
// The toggle button and the action button are siblings inside a header Box —
// NOT nested — to prevent click events from propagating to the wrong handler.

fn build_section(heading: &str, action: Option<&str>) -> (GtkBox, GtkBox, Option<Button>) {
    let section = GtkBox::new(Orientation::Vertical, 0);
    section.add_css_class("tt-section");

    // Header row: toggle button (hexpand) + optional action button as siblings
    let hdr_row = GtkBox::new(Orientation::Horizontal, 0);
    hdr_row.add_css_class("tt-section-hdr");

    let toggle_btn = Button::new();
    toggle_btn.add_css_class("tt-section-toggle");
    toggle_btn.set_hexpand(true);
    toggle_btn.set_focus_on_click(false);

    let toggle_content = GtkBox::new(Orientation::Horizontal, 0);
    let arrow = Label::new(Some("▾"));
    arrow.add_css_class("tt-section-arrow");
    let title_lbl = Label::new(Some(heading));
    title_lbl.add_css_class("tt-section-heading");
    title_lbl.set_halign(Align::Start);
    toggle_content.append(&arrow);
    toggle_content.append(&title_lbl);
    toggle_btn.set_child(Some(&toggle_content));
    hdr_row.append(&toggle_btn);

    let action_btn = action.map(|lbl| {
        let btn = Button::with_label(lbl);
        btn.add_css_class("tt-new-btn");
        hdr_row.append(&btn);
        btn
    });

    section.append(&hdr_row);

    let content_box = GtkBox::new(Orientation::Vertical, 0);
    let content_rev = Revealer::new();
    content_rev.set_transition_type(RevealerTransitionType::SlideDown);
    content_rev.set_transition_duration(150);
    content_rev.set_reveal_child(true);
    content_rev.set_child(Some(&content_box));
    section.append(&content_rev);

    let rev_c = content_rev.clone();
    let arrow_c = arrow.clone();
    toggle_btn.connect_clicked(move |_| {
        let expanding = !rev_c.reveals_child();
        rev_c.set_reveal_child(expanding);
        arrow_c.set_label(if expanding { "▾" } else { "▸" });
    });

    (section, content_box, action_btn)
}

// ── Tint row ───────────────────────────────────────────────────────────────────

fn build_tint_row(tint: &TintRecord, inner: &Rc<Inner>) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.add_css_class("tt-row");

    // Color chip
    let chip = make_color_chip(tint.color.as_deref(), 16);
    chip.add_css_class("tt-tint-chip");
    row.append(&chip);

    // Name stack (label ↔ entry)
    let name_stack = Stack::new();
    name_stack.set_transition_duration(0);
    name_stack.set_hexpand(true);

    let name_label = Label::new(Some(&tint.name));
    name_label.add_css_class("tt-item-name");
    name_label.set_halign(Align::Start);
    name_label.set_hexpand(true);
    name_stack.add_named(&name_label, Some("label"));

    let name_entry = Entry::new();
    name_entry.add_css_class("tt-tag-entry");
    name_entry.set_text(&tint.name);
    name_entry.set_hexpand(true);
    name_stack.add_named(&name_entry, Some("edit"));
    name_stack.set_visible_child_name("label");
    row.append(&name_stack);

    // Default badge
    if tint.is_default {
        let badge = Label::new(Some("default"));
        badge.add_css_class("tt-default-badge");
        row.append(&badge);
    }

    // Rename button
    let rename_btn = Button::with_label("✏");
    rename_btn.add_css_class("tt-edit-btn");
    {
        let ns = name_stack.clone();
        let ne = name_entry.clone();
        let orig = tint.name.clone();
        rename_btn.connect_clicked(move |_| {
            ne.set_text(&orig);
            ns.set_visible_child_name("edit");
            ne.grab_focus();
        });
    }
    row.append(&rename_btn);

    // Wire rename Enter
    {
        let tint_id = tint.id;
        let orig = tint.name.clone();
        let inner = Rc::clone(inner);
        let ns = name_stack.clone();
        name_entry.connect_activate(move |entry| {
            commit_tint_rename(tint_id, &orig, entry, &ns, &inner);
        });
    }

    // Wire rename focus-out
    {
        let tint_id = tint.id;
        let orig = tint.name.clone();
        let inner = Rc::clone(inner);
        let ns_fo = name_stack.clone();
        name_entry.connect_has_focus_notify(move |entry| {
            if !entry.has_focus()
                && ns_fo
                    .visible_child_name()
                    .map(|s| s == "edit")
                    .unwrap_or(false)
            {
                commit_tint_rename(tint_id, &orig, entry, &ns_fo, &inner);
            }
        });
    }

    // Wire Escape cancel
    {
        let ns = name_stack.clone();
        let key_ctrl = EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                ns.set_visible_child_name("label");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        name_entry.add_controller(key_ctrl);
    }

    // Color edit button
    let initial_color = tint.color.as_deref().unwrap_or("#806040");
    let (color_btn, color_swatch, color_hex) =
        build_color_swatch_button(initial_color, 26, 22, "tt-row-color-btn");
    {
        let tint_id = tint.id;
        let inner = Rc::clone(inner);
        color_btn.set_tooltip_text(Some("Choose Tint color"));
        color_btn.connect_clicked(move |_| {
            let initial = color_hex.borrow().clone();
            let color_hex = Rc::clone(&color_hex);
            let color_swatch = color_swatch.clone();
            let inner_for_change = Rc::clone(&inner);
            let cb = inner.cbs.on_tint_color_pick_requested.borrow().clone();
            if let Some(cb) = cb {
                cb(
                    "Choose Tint Color".to_string(),
                    initial,
                    Box::new(move |hex| {
                        *color_hex.borrow_mut() = hex.clone();
                        color_swatch.queue_draw();
                        let cb = inner_for_change.cbs.on_tint_color_changed.borrow().clone();
                        if let Some(cb) = cb {
                            cb(tint_id, hex);
                        }
                    }),
                );
            }
        });
    }
    row.append(&color_btn);

    // Delete button (hidden for default tint)
    if !tint.is_default {
        let del_btn = Button::with_label("🗑");
        del_btn.add_css_class("tt-delete-btn");
        {
            let tint_id = tint.id;
            let inner = Rc::clone(inner);
            del_btn.connect_clicked(move |_| {
                let cb = inner.cbs.on_tint_deleted.borrow().clone();
                if let Some(cb) = cb {
                    cb(tint_id);
                }
            });
        }
        row.append(&del_btn);
    }

    row
}

// ── Tag row ────────────────────────────────────────────────────────────────────

fn build_mark_hint(tag: &TagRecord, tint_names: &HashMap<i64, &str>) -> Option<String> {
    let has_tint = tag.associated_tint_id.is_some();
    let has_shape = tag.associated_shape.is_some();
    if !has_tint && !has_shape {
        return None;
    }
    let tint_part = tag
        .associated_tint_id
        .and_then(|id| tint_names.get(&id).copied())
        .unwrap_or("");
    let shape_part = tag.associated_shape.map(|s| s.glyph()).unwrap_or("");
    Some(format!("→ {tint_part} {shape_part}").trim().to_string())
}

fn build_tag_row(
    tag: &TagRecord,
    count: usize,
    mark_hint: Option<&str>,
    inner: &Rc<Inner>,
) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.add_css_class("tt-row");

    let name_stack = Stack::new();
    name_stack.set_transition_duration(0);

    let name_label = Label::new(Some(&tag.name));
    name_label.add_css_class("tt-item-name");
    name_label.add_css_class("tt-tag-clickable");
    name_label.set_halign(Align::Start);
    name_label.set_hexpand(true);
    name_stack.add_named(&name_label, Some("label"));

    let name_entry = Entry::new();
    name_entry.add_css_class("tt-tag-entry");
    name_entry.set_text(&tag.name);
    name_stack.add_named(&name_entry, Some("edit"));
    name_stack.set_visible_child_name("label");
    row.append(&name_stack);

    // Click name to browse
    {
        let tag_id = tag.id;
        let inner = Rc::clone(inner);
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            let cb = inner.cbs.on_tag_clicked.borrow().clone();
            if let Some(cb) = cb {
                cb(tag_id);
            }
        });
        name_label.add_controller(click);
    }

    // File count
    let word = if count == 1 { "file" } else { "files" };
    let count_lbl = Label::new(Some(&format!("{count} {word}")));
    count_lbl.add_css_class("tt-count");
    count_lbl.set_hexpand(true);
    count_lbl.set_halign(Align::End);
    row.append(&count_lbl);

    // Mark hint (shows associated Tint + Shape when set)
    if let Some(hint) = mark_hint {
        let hint_lbl = Label::new(Some(hint));
        hint_lbl.add_css_class("tt-tag-mark-hint");
        hint_lbl.set_halign(Align::End);
        row.append(&hint_lbl);
    }

    // Edit button
    let edit_btn = Button::with_label("✏");
    edit_btn.add_css_class("tt-edit-btn");
    {
        let ns = name_stack.clone();
        let ne = name_entry.clone();
        let orig = tag.name.clone();
        edit_btn.connect_clicked(move |_| {
            ne.set_text(&orig);
            ns.set_visible_child_name("edit");
            ne.grab_focus();
        });
    }
    row.append(&edit_btn);

    // Wire rename Enter
    {
        let tag_id = tag.id;
        let orig = tag.name.clone();
        let inner = Rc::clone(inner);
        let ns = name_stack.clone();
        name_entry.connect_activate(move |entry| {
            commit_tag_rename(tag_id, &orig, entry, &ns, &inner);
        });
    }

    // Wire rename focus-out
    {
        let tag_id = tag.id;
        let orig = tag.name.clone();
        let inner = Rc::clone(inner);
        let ns_fo = name_stack.clone();
        name_entry.connect_has_focus_notify(move |entry| {
            if !entry.has_focus()
                && ns_fo
                    .visible_child_name()
                    .map(|s| s == "edit")
                    .unwrap_or(false)
            {
                commit_tag_rename(tag_id, &orig, entry, &ns_fo, &inner);
            }
        });
    }

    // Wire Escape cancel
    {
        let ns = name_stack.clone();
        let key_ctrl = EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                ns.set_visible_child_name("label");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        name_entry.add_controller(key_ctrl);
    }

    // Delete button
    let del_btn = Button::with_label("🗑");
    del_btn.add_css_class("tt-delete-btn");
    {
        let tag_id = tag.id;
        let inner = Rc::clone(inner);
        del_btn.connect_clicked(move |_| {
            let cb = inner.cbs.on_tag_deleted.borrow().clone();
            if let Some(cb) = cb {
                cb(tag_id);
            }
        });
    }
    row.append(&del_btn);

    row
}

// ── Shape badge ────────────────────────────────────────────────────────────────

fn build_shape_badge(shape: Shape) -> GtkBox {
    let vbox = GtkBox::new(Orientation::Vertical, 4);
    vbox.add_css_class("tt-shape-badge");
    vbox.set_halign(Align::Center);
    vbox.set_margin_top(2);
    vbox.set_margin_bottom(2);

    let area = DrawingArea::new();
    area.set_content_width(26);
    area.set_content_height(26);
    area.set_halign(Align::Center);
    area.set_draw_func(move |_, cr, w, h| {
        let cx = w as f64 / 2.0;
        let cy = h as f64 / 2.0;
        let r = (w.min(h) as f64 / 2.0) * 0.80;
        cr.set_source_rgb(0.718, 0.627, 0.490);
        draw_shape(cr, shape, cx, cy, r);
        let _ = cr.fill();
    });
    vbox.append(&area);

    let name = capitalize(shape.as_str());
    let lbl = Label::new(Some(&name));
    lbl.add_css_class("tt-shape-name");
    lbl.set_halign(Align::Center);
    vbox.append(&lbl);

    vbox
}

pub(crate) fn draw_shape(cr: &cairo::Context, shape: Shape, cx: f64, cy: f64, r: f64) {
    match shape {
        Shape::Circle => {
            cr.arc(cx, cy, r, 0.0, 2.0 * std::f64::consts::PI);
        }
        Shape::Square => {
            cr.rectangle(cx - r, cy - r, r * 2.0, r * 2.0);
        }
        Shape::Triangle => {
            let offset = r * 0.15;
            cr.move_to(cx, cy - r + offset);
            cr.line_to(cx + r * 0.866, cy + r * 0.5 + offset);
            cr.line_to(cx - r * 0.866, cy + r * 0.5 + offset);
            cr.close_path();
        }
        Shape::Pentagon => draw_polygon(cr, cx, cy, r, 5, FRAC_PI_2),
        Shape::Hexagon => draw_polygon(cr, cx, cy, r, 6, FRAC_PI_2),
        Shape::Octagon => draw_polygon(cr, cx, cy, r, 8, FRAC_PI_2 - std::f64::consts::PI / 8.0),
        Shape::Trapezoid => {
            let top = r * 0.62;
            cr.move_to(cx - top, cy - r * 0.45);
            cr.line_to(cx + top, cy - r * 0.45);
            cr.line_to(cx + r, cy + r * 0.55);
            cr.line_to(cx - r, cy + r * 0.55);
            cr.close_path();
        }
    }
}

pub(crate) fn draw_polygon(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    r: f64,
    n: i32,
    start_angle: f64,
) {
    for i in 0..n {
        let angle = start_angle + 2.0 * std::f64::consts::PI * i as f64 / n as f64;
        let x = cx + r * angle.cos();
        let y = cy - r * angle.sin();
        if i == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
    cr.close_path();
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

// ── Color chip ─────────────────────────────────────────────────────────────────

fn make_color_chip(hex: Option<&str>, size: i32) -> DrawingArea {
    let (r, g, b) = parse_hex_color(hex.unwrap_or("#806040"));
    let area = DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_valign(Align::Center);
    area.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgb(r, g, b);
        let rad = 2.5_f64;
        let w = w as f64;
        let h = h as f64;
        cr.new_sub_path();
        cr.arc(rad, rad, rad, std::f64::consts::PI, 3.0 * FRAC_PI_2);
        cr.arc(w - rad, rad, rad, 3.0 * FRAC_PI_2, 0.0);
        cr.arc(w - rad, h - rad, rad, 0.0, FRAC_PI_2);
        cr.arc(rad, h - rad, rad, FRAC_PI_2, std::f64::consts::PI);
        cr.close_path();
        let _ = cr.fill();
    });
    area
}

fn build_color_swatch_button(
    initial_hex: &str,
    width: i32,
    height: i32,
    css_class: &str,
) -> (Button, DrawingArea, Rc<RefCell<String>>) {
    let hex = Rc::new(RefCell::new(initial_hex.to_string()));
    let swatch = DrawingArea::new();
    swatch.set_content_width(width);
    swatch.set_content_height(height);
    swatch.set_size_request(width, height);
    swatch.set_halign(Align::Center);
    swatch.set_valign(Align::Center);
    {
        let hex = Rc::clone(&hex);
        swatch.set_draw_func(move |_, cr, w, h| {
            let (r, g, b) = parse_hex_color(&hex.borrow());
            cr.set_source_rgb(r, g, b);
            rounded_rect(cr, 1.0, 1.0, (w - 2) as f64, (h - 2) as f64, 4.0);
            let _ = cr.fill();
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
            cr.set_line_width(1.0);
            rounded_rect(cr, 0.5, 0.5, (w - 1) as f64, (h - 1) as f64, 4.0);
            let _ = cr.stroke();
        });
    }

    let button = Button::new();
    button.add_css_class(css_class);
    button.set_focus_on_click(false);
    button.set_size_request(width + 8, height + 8);
    button.set_child(Some(&swatch));

    (button, swatch, hex)
}

pub(crate) fn parse_hex_color(hex: &str) -> (f64, f64, f64) {
    let s = hex.trim().trim_start_matches('#');
    if s.len() != 6 {
        return (0.502, 0.376, 0.251);
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(128) as f64 / 255.0;
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(96) as f64 / 255.0;
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(64) as f64 / 255.0;
    (r, g, b)
}

// ── Commit helpers ─────────────────────────────────────────────────────────────

fn commit_tint_rename(id: i64, orig: &str, entry: &Entry, stack: &Stack, inner: &Rc<Inner>) {
    let new_name = entry.text().to_string();
    let new_name = new_name.trim().to_string();
    stack.set_visible_child_name("label");
    if !new_name.is_empty() && new_name != orig {
        let cb = inner.cbs.on_tint_renamed.borrow().clone();
        if let Some(cb) = cb {
            cb(id, new_name);
        }
    }
}

fn commit_tag_rename(id: i64, orig: &str, entry: &Entry, stack: &Stack, inner: &Rc<Inner>) {
    let new_name = entry.text().to_string();
    let new_name = new_name.trim().to_string();
    stack.set_visible_child_name("label");
    if !new_name.is_empty() && new_name != orig {
        let cb = inner.cbs.on_tag_renamed.borrow().clone();
        if let Some(cb) = cb {
            cb(id, new_name);
        }
    }
}

fn fire_tint_create(inner: &Rc<Inner>) {
    let name = inner.tint_name_entry.text().to_string();
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let color = inner.tint_color_hex.borrow().clone();
    inner.tint_create_revealer.set_reveal_child(false);
    inner.tint_name_entry.set_text("");
    *inner.tint_color_hex.borrow_mut() = "#806040".to_string();
    inner.tint_color_swatch.queue_draw();
    inner.tint_color_label.set_text("#806040");
    let cb = inner.cbs.on_tint_created.borrow().clone();
    if let Some(cb) = cb {
        cb(name, color);
    }
}

fn fire_tag_create(inner: &Rc<Inner>) {
    let name = inner.tag_name_entry.text().to_string();
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    inner.tag_create_revealer.set_reveal_child(false);
    inner.tag_name_entry.set_text("");
    let cb = inner.cbs.on_tag_created.borrow().clone();
    if let Some(cb) = cb {
        cb(name);
    }
}

// ── Misc helpers ───────────────────────────────────────────────────────────────

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + chars.as_str(),
    }
}

fn clear_box(container: &GtkBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
