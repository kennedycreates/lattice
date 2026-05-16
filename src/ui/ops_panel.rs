use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, Orientation, ProgressBar};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

pub type OpId = u64;

struct OpEntry {
    root: GtkBox,
    progress_bar: ProgressBar,
    detail_label: Label,
    action_button: gtk::Button,
}

struct Inner {
    ops_box: GtkBox,
    entries: HashMap<OpId, OpEntry>,
    next_id: u64,
}

/// A panel that shows active file operations with progress bars and cancel buttons.
/// Hidden automatically when no operations are active.
#[derive(Clone)]
pub struct OpsPanel {
    pub root: GtkBox,
    inner: Rc<RefCell<Inner>>,
}

impl OpsPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("ops-panel");
        root.set_visible(false);

        let ops_box = GtkBox::new(Orientation::Vertical, 0);
        ops_box.add_css_class("ops-panel-list");
        root.append(&ops_box);

        Self {
            root,
            inner: Rc::new(RefCell::new(Inner {
                ops_box,
                entries: HashMap::new(),
                next_id: 0,
            })),
        }
    }

    /// Register a new operation. Returns an `OpId` used to update or finish it.
    /// Passing a `Cancellable` wires the Cancel button to it automatically.
    pub fn add_op(&self, label: &str, cancellable: Option<gio::Cancellable>) -> OpId {
        let id = {
            let mut inn = self.inner.borrow_mut();
            let id = inn.next_id;
            inn.next_id += 1;
            id
        };

        // ── Row container ─────────────────────────────────────────────────
        let row = GtkBox::new(Orientation::Vertical, 3);
        row.add_css_class("op-row");
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(10);
        row.set_margin_end(10);

        // Top line: label + cancel button
        let top = GtkBox::new(Orientation::Horizontal, 8);

        let lbl = Label::new(Some(label));
        lbl.add_css_class("op-label");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        lbl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        top.append(&lbl);

        let action_btn = gtk::Button::with_label("Cancel");
        action_btn.add_css_class("op-action-button");
        if let Some(ref c) = cancellable {
            let c = c.clone();
            action_btn.connect_clicked(move |_| c.cancel());
        } else {
            action_btn.set_sensitive(false);
        }
        top.append(&action_btn);
        row.append(&top);

        // Progress bar
        let pb = ProgressBar::new();
        pb.add_css_class("op-progress");
        pb.set_pulse_step(0.08);
        row.append(&pb);

        // Detail line (current filename / bytes / error)
        let detail = Label::new(None);
        detail.add_css_class("op-detail");
        detail.set_halign(gtk::Align::Start);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        detail.set_visible(false);
        row.append(&detail);

        {
            let mut inn = self.inner.borrow_mut();
            inn.ops_box.append(&row);
            inn.entries.insert(
                id,
                OpEntry {
                    root: row,
                    progress_bar: pb,
                    detail_label: detail,
                    action_button: action_btn,
                },
            );
        }

        self.root.set_visible(true);
        id
    }

    /// Update the progress bar and detail text for a running operation.
    /// `fraction` is 0.0–1.0; pass an empty string to hide the detail line.
    pub fn update_progress(&self, id: OpId, fraction: f64, detail: &str) {
        let inn = self.inner.borrow();
        let Some(entry) = inn.entries.get(&id) else {
            return;
        };
        entry.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
        if detail.is_empty() {
            entry.detail_label.set_visible(false);
        } else {
            entry.detail_label.set_label(detail);
            entry.detail_label.set_visible(true);
        }
    }

    /// Mark an operation as finished.
    /// - Empty `errors` → success: shows "Done", auto-dismisses after 2 s.
    /// - Non-empty `errors` → failure: shows errors, requires manual Dismiss.
    pub fn finish_op(&self, id: OpId, errors: &[String]) {
        // Grab widget handles before releasing borrow
        let (pb, btn, detail, row) = {
            let inn = self.inner.borrow();
            let Some(e) = inn.entries.get(&id) else {
                return;
            };
            (
                e.progress_bar.clone(),
                e.action_button.clone(),
                e.detail_label.clone(),
                e.root.clone(),
            )
        };

        if errors.is_empty() {
            pb.set_fraction(1.0);
            btn.set_label("Done");
            btn.set_sensitive(false);
            detail.set_visible(false);

            let panel = self.clone();
            glib::timeout_add_local_once(Duration::from_secs(2), move || {
                panel.remove_op(id);
            });
        } else {
            pb.set_visible(false);
            btn.set_label("Dismiss");
            btn.set_sensitive(true);
            detail.set_label(&errors.join("\n"));
            detail.set_visible(true);
            row.add_css_class("op-row-failed");

            let panel = self.clone();
            btn.connect_clicked(move |_| panel.remove_op(id));
        }
    }

    fn remove_op(&self, id: OpId) {
        let entry = self.inner.borrow_mut().entries.remove(&id);
        if let Some(e) = entry {
            self.inner.borrow().ops_box.remove(&e.root);
        }
        if self.inner.borrow().entries.is_empty() {
            self.root.set_visible(false);
        }
    }
}
