use crate::metadata::TagRecord;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, Entry, EventControllerKey, Label, Orientation, Revealer,
    RevealerTransitionType, ScrolledWindow, Stack,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// ── Callbacks ─────────────────────────────────────────────────────────────────

struct Callbacks {
    on_tag_clicked: RefCell<Option<Box<dyn Fn(i64)>>>,
    on_tag_created: RefCell<Option<Box<dyn Fn(String)>>>,
    on_tag_renamed: RefCell<Option<Box<dyn Fn(i64, String)>>>,
    on_tag_deleted: RefCell<Option<Box<dyn Fn(i64)>>>,
}

impl Callbacks {
    fn new() -> Self {
        Self {
            on_tag_clicked: RefCell::new(None),
            on_tag_created: RefCell::new(None),
            on_tag_renamed: RefCell::new(None),
            on_tag_deleted: RefCell::new(None),
        }
    }
}

// ── Inner state ───────────────────────────────────────────────────────────────

struct Inner {
    list_box: GtkBox,
    empty_hint: Label,
    create_revealer: Revealer,
    create_name_entry: Entry,
    cbs: Callbacks,
}

// ── Public widget ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TagManagerPanel {
    pub root: GtkBox,
    inner: Rc<Inner>,
}

impl TagManagerPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("tm-panel");
        root.set_visible(false);

        // ── Header ─────────────────────────────────────────────────────────────
        let header = GtkBox::new(Orientation::Horizontal, 8);
        header.add_css_class("tm-header");

        let title = Label::new(Some("🏷  Tags"));
        title.add_css_class("tm-title");
        title.set_halign(Align::Start);
        title.set_hexpand(true);
        header.append(&title);

        let new_btn = Button::with_label("+ New Tag");
        new_btn.add_css_class("tm-new-btn");
        header.append(&new_btn);

        root.append(&header);

        // ── Create row (slide-down revealer) ────────────────────────────────────
        let create_revealer = Revealer::new();
        create_revealer.set_transition_type(RevealerTransitionType::SlideDown);
        create_revealer.set_transition_duration(160);
        create_revealer.set_reveal_child(false);

        let create_row = GtkBox::new(Orientation::Vertical, 6);
        create_row.add_css_class("tm-create-row");

        let create_name_entry = Entry::new();
        create_name_entry.add_css_class("tm-create-entry");
        create_name_entry.set_placeholder_text(Some("Tag name…"));
        create_row.append(&create_name_entry);

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

        // ── Tag list ────────────────────────────────────────────────────────────
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled.set_vscrollbar_policy(gtk::PolicyType::Automatic);

        let list_box = GtkBox::new(Orientation::Vertical, 0);
        list_box.add_css_class("tm-list");
        scrolled.set_child(Some(&list_box));
        root.append(&scrolled);

        let empty_hint = Label::new(Some("No tags yet. Create one with + New Tag above."));
        empty_hint.add_css_class("tm-empty");
        empty_hint.set_halign(Align::Start);
        empty_hint.set_wrap(true);
        empty_hint.set_visible(false);
        root.append(&empty_hint);

        // ── Assemble inner state ────────────────────────────────────────────────
        let inner = Rc::new(Inner {
            list_box,
            empty_hint,
            create_revealer: create_revealer.clone(),
            create_name_entry: create_name_entry.clone(),
            cbs: Callbacks::new(),
        });

        // ── Wire new-tag button ─────────────────────────────────────────────────
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

        // ── Wire cancel button ──────────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            cancel_btn.connect_clicked(move |_| {
                inner.create_revealer.set_reveal_child(false);
                inner.create_name_entry.set_text("");
            });
        }

        // ── Wire create button ──────────────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            create_btn.connect_clicked(move |_| {
                fire_create(&inner);
            });
        }

        // ── Wire entry activate (Enter) ─────────────────────────────────────────
        {
            let inner = Rc::clone(&inner);
            create_name_entry.connect_activate(move |_| {
                fire_create(&inner);
            });
        }

        Self { root, inner }
    }

    // ── Data loading ───────────────────────────────────────────────────────────

    pub fn set_tags(&self, tags: &[TagRecord], counts: &HashMap<i64, usize>) {
        while let Some(child) = self.inner.list_box.first_child() {
            self.inner.list_box.remove(&child);
        }

        if tags.is_empty() {
            self.inner.empty_hint.set_visible(true);
            self.inner.list_box.set_visible(false);
            return;
        }

        self.inner.empty_hint.set_visible(false);
        self.inner.list_box.set_visible(true);

        for tag in tags {
            let count = counts.get(&tag.id).copied().unwrap_or(0);
            let row = build_tag_row(tag, count, &self.inner);
            self.inner.list_box.append(&row);
        }
    }

    // ── Callback wiring ────────────────────────────────────────────────────────

    pub fn connect_tag_clicked<F: Fn(i64) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_clicked.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_tag_created<F: Fn(String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_created.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_tag_renamed<F: Fn(i64, String) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_renamed.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_tag_deleted<F: Fn(i64) + 'static>(&self, f: F) {
        *self.inner.cbs.on_tag_deleted.borrow_mut() = Some(Box::new(f));
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn fire_create(inner: &Rc<Inner>) {
    let name = inner.create_name_entry.text().to_string();
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    inner.create_revealer.set_reveal_child(false);
    inner.create_name_entry.set_text("");
    if let Some(cb) = inner.cbs.on_tag_created.borrow().as_ref() {
        cb(name);
    }
}

fn build_tag_row(tag: &TagRecord, count: usize, inner: &Rc<Inner>) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.add_css_class("tm-row");

    // Name stack (label / entry)
    let name_stack = Stack::new();
    name_stack.set_transition_duration(0);

    let name_label = Label::new(Some(&tag.name));
    name_label.add_css_class("tm-tag-name");
    name_label.set_halign(Align::Start);
    name_label.set_hexpand(true);
    name_stack.add_named(&name_label, Some("label"));

    let name_entry = Entry::new();
    name_entry.add_css_class("tm-tag-entry");
    name_entry.set_text(&tag.name);
    name_stack.add_named(&name_entry, Some("edit"));

    name_stack.set_visible_child_name("label");
    row.append(&name_stack);

    // Click name label to browse tag files
    {
        let tag_id = tag.id;
        let inner = Rc::clone(inner);
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(cb) = inner.cbs.on_tag_clicked.borrow().as_ref() {
                cb(tag_id);
            }
        });
        name_label.add_controller(click);
    }

    // File count label
    let file_word = if count == 1 { "file" } else { "files" };
    let count_label = Label::new(Some(&format!("{count} {file_word}")));
    count_label.add_css_class("tm-count");
    count_label.set_hexpand(true);
    count_label.set_halign(Align::End);
    row.append(&count_label);

    // Edit (rename) button
    let edit_btn = Button::with_label("✏");
    edit_btn.add_css_class("tm-edit-btn");
    {
        let name_stack = name_stack.clone();
        let name_entry = name_entry.clone();
        let tag_name = tag.name.clone();
        edit_btn.connect_clicked(move |_| {
            name_entry.set_text(&tag_name);
            name_stack.set_visible_child_name("edit");
            name_entry.grab_focus();
        });
    }
    row.append(&edit_btn);

    // Wire rename commit (Enter key)
    {
        let tag_id = tag.id;
        let tag_name = tag.name.clone();
        let inner = Rc::clone(inner);
        let name_stack = name_stack.clone();
        let name_entry_ref = name_entry.clone();
        name_entry.connect_activate(move |entry| {
            commit_rename(tag_id, &tag_name, entry, &name_stack, &inner);
            let _ = name_entry_ref;
        });
    }

    // Wire rename commit (focus-out)
    {
        let tag_id = tag.id;
        let tag_name = tag.name.clone();
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
                commit_rename(tag_id, &tag_name, entry, &name_stack_fo, &inner);
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

    // Delete button
    let delete_btn = Button::with_label("🗑");
    delete_btn.add_css_class("tm-delete-btn");
    {
        let tag_id = tag.id;
        let inner = Rc::clone(inner);
        delete_btn.connect_clicked(move |_| {
            if let Some(cb) = inner.cbs.on_tag_deleted.borrow().as_ref() {
                cb(tag_id);
            }
        });
    }
    row.append(&delete_btn);

    row
}

fn commit_rename(
    tag_id: i64,
    original_name: &str,
    entry: &Entry,
    name_stack: &Stack,
    inner: &Rc<Inner>,
) {
    let new_name = entry.text().to_string();
    let new_name = new_name.trim().to_string();
    name_stack.set_visible_child_name("label");
    if !new_name.is_empty() && new_name != original_name {
        if let Some(cb) = inner.cbs.on_tag_renamed.borrow().as_ref() {
            cb(tag_id, new_name);
        }
    }
}
