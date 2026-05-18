use crate::metadata::ProjectRecord;
use crate::ui::tag_panel::{tag_color_class, TAG_PALETTE};
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, Entry, EventControllerKey, Label, Orientation, Popover, Revealer,
    RevealerTransitionType, ScrolledWindow, Stack,
};
use std::cell::RefCell;
use std::rc::Rc;

struct Callbacks {
    on_project_clicked: RefCell<Option<Box<dyn Fn(i64)>>>,
    on_project_created: RefCell<Option<Box<dyn Fn(String, String)>>>,
    on_project_renamed: RefCell<Option<Box<dyn Fn(i64, String)>>>,
    on_project_recolored: RefCell<Option<Box<dyn Fn(i64, String)>>>,
    on_project_deleted: RefCell<Option<Box<dyn Fn(i64)>>>,
}

impl Callbacks {
    fn new() -> Self {
        Self {
            on_project_clicked: RefCell::new(None),
            on_project_created: RefCell::new(None),
            on_project_renamed: RefCell::new(None),
            on_project_recolored: RefCell::new(None),
            on_project_deleted: RefCell::new(None),
        }
    }
}

struct Inner {
    list_box: GtkBox,
    empty_hint: Label,
    create_revealer: Revealer,
    create_name_entry: Entry,
    create_color_selected: RefCell<String>,
    cbs: Callbacks,
}

#[derive(Clone)]
pub struct ProjectManagerPanel {
    pub root: GtkBox,
    inner: Rc<Inner>,
}

impl ProjectManagerPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("pm-panel");
        root.set_visible(false);

        // ── Header ──────────────────────────────────────────────────────────────
        let header = GtkBox::new(Orientation::Horizontal, 8);
        header.add_css_class("tm-header");

        let title = Label::new(Some("🗂  Projects"));
        title.add_css_class("tm-title");
        title.set_halign(Align::Start);
        title.set_hexpand(true);
        header.append(&title);

        let new_btn = Button::with_label("+ New Project");
        new_btn.add_css_class("tm-new-btn");
        header.append(&new_btn);

        root.append(&header);

        // ── Create row ───────────────────────────────────────────────────────────
        let create_revealer = Revealer::new();
        create_revealer.set_transition_type(RevealerTransitionType::SlideDown);
        create_revealer.set_transition_duration(160);
        create_revealer.set_reveal_child(false);

        let create_row = GtkBox::new(Orientation::Vertical, 6);
        create_row.add_css_class("tm-create-row");

        let create_name_entry = Entry::new();
        create_name_entry.add_css_class("tm-create-entry");
        create_name_entry.set_placeholder_text(Some("Project name…"));
        create_row.append(&create_name_entry);

        let swatch_row = GtkBox::new(Orientation::Horizontal, 6);
        swatch_row.add_css_class("tm-swatch-row");

        let create_color_selected = RefCell::new(TAG_PALETTE[0].0.to_string());
        let mut create_swatch_btns: Vec<(String, Button)> = Vec::new();

        for (i, (hex, _)) in TAG_PALETTE.iter().enumerate() {
            let btn = Button::new();
            btn.add_css_class("tm-swatch-btn");
            btn.add_css_class(&format!("tm-swatch-{}", i));
            if i == 0 {
                btn.add_css_class("selected");
            }
            swatch_row.append(&btn);
            create_swatch_btns.push((hex.to_string(), btn));
        }

        create_row.append(&swatch_row);

        let create_actions = GtkBox::new(Orientation::Horizontal, 6);
        create_actions.add_css_class("tm-create-actions");

        let create_btn = Button::with_label("Create");
        create_btn.add_css_class("tm-create-btn");
        let cancel_btn = Button::with_label("Cancel");
        cancel_btn.add_css_class("tm-cancel-btn");

        create_actions.append(&create_btn);
        create_actions.append(&cancel_btn);
        create_row.append(&create_actions);

        create_revealer.set_child(Some(&create_row));
        root.append(&create_revealer);

        // ── Project list ─────────────────────────────────────────────────────────
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled.set_vscrollbar_policy(gtk::PolicyType::Automatic);

        let list_box = GtkBox::new(Orientation::Vertical, 0);
        list_box.add_css_class("tm-list");
        scrolled.set_child(Some(&list_box));
        root.append(&scrolled);

        let empty_hint = Label::new(Some(
            "No projects yet. Create one with + New Project above.",
        ));
        empty_hint.add_css_class("tm-empty");
        empty_hint.set_halign(Align::Start);
        empty_hint.set_wrap(true);
        empty_hint.set_visible(false);
        root.append(&empty_hint);

        let inner = Rc::new(Inner {
            list_box,
            empty_hint,
            create_revealer: create_revealer.clone(),
            create_name_entry: create_name_entry.clone(),
            create_color_selected,
            cbs: Callbacks::new(),
        });

        // ── Wire create swatch buttons ───────────────────────────────────────────
        for (idx, (hex, btn)) in create_swatch_btns.iter().enumerate() {
            let hex = hex.clone();
            let inner = Rc::clone(&inner);
            let all_btns: Vec<Button> = create_swatch_btns.iter().map(|(_, b)| b.clone()).collect();
            btn.connect_clicked(move |clicked| {
                *inner.create_color_selected.borrow_mut() = hex.clone();
                for (i, b) in all_btns.iter().enumerate() {
                    if i == idx {
                        b.add_css_class("selected");
                    } else {
                        b.remove_css_class("selected");
                    }
                }
                let _ = clicked;
            });
        }

        // ── Wire new-project button ──────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            new_btn.connect_clicked(move |_| {
                let already = inner.create_revealer.reveals_child();
                inner.create_revealer.set_reveal_child(!already);
                if !already {
                    inner.create_name_entry.set_text("");
                    inner.create_name_entry.grab_focus();
                }
            });
        }

        // ── Wire cancel button ───────────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            cancel_btn.connect_clicked(move |_| {
                inner.create_revealer.set_reveal_child(false);
                inner.create_name_entry.set_text("");
            });
        }

        // ── Wire create button ───────────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            create_btn.connect_clicked(move |_| {
                fire_create(&inner);
            });
        }

        // ── Wire entry activate ──────────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            create_name_entry.connect_activate(move |_| {
                fire_create(&inner);
            });
        }

        Self { root, inner }
    }

    // ── Data loading ─────────────────────────────────────────────────────────────

    pub fn set_projects(&self, projects: &[ProjectRecord]) {
        while let Some(child) = self.inner.list_box.first_child() {
            self.inner.list_box.remove(&child);
        }

        if projects.is_empty() {
            self.inner.empty_hint.set_visible(true);
            self.inner.list_box.set_visible(false);
            return;
        }

        self.inner.empty_hint.set_visible(false);
        self.inner.list_box.set_visible(true);

        for project in projects {
            let row = build_project_row(project, &self.inner);
            self.inner.list_box.append(&row);
        }
    }

    // ── Callback wiring ───────────────────────────────────────────────────────────

    pub fn connect_project_clicked<F: Fn(i64) + 'static>(&self, f: F) {
        *self.inner.cbs.on_project_clicked.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_project_created<F: Fn(String, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_project_created.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_project_renamed<F: Fn(i64, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_project_renamed.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_project_recolored<F: Fn(i64, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_project_recolored.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_project_deleted<F: Fn(i64) + 'static>(&self, f: F) {
        *self.inner.cbs.on_project_deleted.borrow_mut() = Some(Box::new(f));
    }
}

fn fire_create(inner: &Rc<Inner>) {
    let name = inner.create_name_entry.text().to_string();
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let color = inner.create_color_selected.borrow().clone();
    inner.create_revealer.set_reveal_child(false);
    inner.create_name_entry.set_text("");
    if let Some(cb) = inner.cbs.on_project_created.borrow().as_ref() {
        cb(name, color);
    }
}

fn build_project_row(project: &ProjectRecord, inner: &Rc<Inner>) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.add_css_class("tm-row");

    // Color dot
    let dot = Label::new(Some("●"));
    dot.add_css_class("tm-dot");
    dot.add_css_class(tag_color_class(project.color.as_deref()));
    dot.set_valign(Align::Center);
    row.append(&dot);

    // Name stack (label / entry)
    let name_stack = Stack::new();
    name_stack.set_transition_duration(0);

    let name_label = Label::new(Some(&project.name));
    name_label.add_css_class("tm-tag-name");
    name_label.set_halign(Align::Start);
    name_label.set_hexpand(true);
    name_stack.add_named(&name_label, Some("label"));

    let name_entry = Entry::new();
    name_entry.add_css_class("tm-tag-entry");
    name_entry.set_text(&project.name);
    name_stack.add_named(&name_entry, Some("edit"));

    name_stack.set_visible_child_name("label");
    row.append(&name_stack);

    // Click name label to open project
    {
        let project_id = project.id;
        let inner = Rc::clone(inner);
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(cb) = inner.cbs.on_project_clicked.borrow().as_ref() {
                cb(project_id);
            }
        });
        name_label.add_controller(click);
    }

    // Edit (rename) button
    let edit_btn = Button::with_label("✏");
    edit_btn.add_css_class("tm-edit-btn");
    {
        let name_stack = name_stack.clone();
        let name_entry = name_entry.clone();
        let project_name = project.name.clone();
        edit_btn.connect_clicked(move |_| {
            name_entry.set_text(&project_name);
            name_stack.set_visible_child_name("edit");
            name_entry.grab_focus();
        });
    }
    row.append(&edit_btn);

    // Wire rename commit (Enter)
    {
        let project_id = project.id;
        let project_name = project.name.clone();
        let inner = Rc::clone(inner);
        let name_stack = name_stack.clone();
        let name_entry_ref = name_entry.clone();
        name_entry.connect_activate(move |entry| {
            commit_rename(project_id, &project_name, entry, &name_stack, &inner);
            let _ = name_entry_ref;
        });
    }

    // Wire rename commit (focus-out)
    {
        let project_id = project.id;
        let project_name = project.name.clone();
        let inner = Rc::clone(inner);
        let name_stack_fo = name_stack.clone();
        let name_entry_fo = name_entry.clone();
        name_entry.connect_has_focus_notify(move |entry| {
            if !entry.has_focus()
                && name_stack_fo
                    .visible_child_name()
                    .map(|s| s == "edit")
                    .unwrap_or(false)
            {
                commit_rename(project_id, &project_name, entry, &name_stack_fo, &inner);
                let _ = name_entry_fo;
            }
        });
    }

    // Wire Escape to cancel rename
    {
        let name_stack = name_stack.clone();
        let key_ctrl = EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                name_stack.set_visible_child_name("label");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        name_entry.add_controller(key_ctrl);
    }

    // Color change button → Popover with swatch row
    let color_btn = Button::with_label("●");
    color_btn.add_css_class("tm-color-btn");
    color_btn.add_css_class(tag_color_class(project.color.as_deref()));
    {
        let project_id = project.id;
        let inner = Rc::clone(inner);
        let color_btn_ref = color_btn.clone();
        let current_color = project
            .color
            .clone()
            .unwrap_or_else(|| TAG_PALETTE[0].0.to_string());

        color_btn.connect_clicked(move |btn| {
            let popover = Popover::new();
            popover.add_css_class("tm-color-popover");
            popover.set_has_arrow(true);

            let swatch_row = GtkBox::new(Orientation::Horizontal, 6);
            swatch_row.add_css_class("tm-swatch-popover-row");

            for (i, (hex, _)) in TAG_PALETTE.iter().enumerate() {
                let swatch = Button::new();
                swatch.add_css_class("tm-swatch-btn");
                swatch.add_css_class(&format!("tm-swatch-{}", i));
                if hex.eq_ignore_ascii_case(&current_color) {
                    swatch.add_css_class("selected");
                }
                let hex_str = hex.to_string();
                let inner = Rc::clone(&inner);
                let popover_ref = popover.clone();
                let color_btn_inner = color_btn_ref.clone();
                swatch.connect_clicked(move |_| {
                    popover_ref.popdown();
                    for (_, c) in TAG_PALETTE.iter() {
                        color_btn_inner.remove_css_class(c);
                    }
                    color_btn_inner.add_css_class(tag_color_class(Some(&hex_str)));
                    if let Some(cb) = inner.cbs.on_project_recolored.borrow().as_ref() {
                        cb(project_id, hex_str.clone());
                    }
                });
                swatch_row.append(&swatch);
            }

            popover.set_child(Some(&swatch_row));
            popover.set_parent(btn);
            popover.popup();

            let popover_clone = popover.clone();
            popover.connect_closed(move |_| {
                popover_clone.unparent();
            });
        });
    }
    row.append(&color_btn);

    // Delete button
    let delete_btn = Button::with_label("🗑");
    delete_btn.add_css_class("tm-delete-btn");
    {
        let project_id = project.id;
        let inner = Rc::clone(inner);
        delete_btn.connect_clicked(move |_| {
            if let Some(cb) = inner.cbs.on_project_deleted.borrow().as_ref() {
                cb(project_id);
            }
        });
    }
    row.append(&delete_btn);

    row
}

fn commit_rename(
    project_id: i64,
    original_name: &str,
    entry: &Entry,
    name_stack: &Stack,
    inner: &Rc<Inner>,
) {
    let new_name = entry.text().to_string();
    let new_name = new_name.trim().to_string();
    name_stack.set_visible_child_name("label");
    if !new_name.is_empty() && new_name != original_name {
        if let Some(cb) = inner.cbs.on_project_renamed.borrow().as_ref() {
            cb(project_id, new_name);
        }
    }
}
