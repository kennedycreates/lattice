/// In-window modal system for Lattice.
///
/// All internal Lattice dialogs (rename, new folder, tag, palette, errors,
/// conflict prompts) are hosted here as in-window overlays instead of separate
/// GtkWindow / GtkDialog toplevels.  This eliminates the first-frame squash
/// bug that plagued separate popup windows.
///
/// Architecture
/// ─────────────────────────────────────────────────────────────────────────
/// ApplicationWindow
/// └── GtkOverlay  (app_overlay — exposed as `ModalHost::overlay`)
///     ├── [main child] GtkBox  (the normal Lattice UI: toolbar, body, etc.)
///     └── [overlay]   GtkOverlay  (modal_outer — hidden when no dialog open)
///         ├── [main child] GtkBox  (.modal-scrim — dim backdrop, click to dismiss)
///         └── [overlay]   GtkBox  (.modal-panel — centered dialog card)
///             ├── title row    (.modal-title)
///             ├── separator    (.modal-title-sep)
///             ├── content slot (.modal-content)
///             ├── separator    (.modal-actions-sep)
///             └── actions row  (.modal-actions)
///
/// Usage
/// ─────────────────────────────────────────────────────────────────────────
/// • Wrap the main UI with `modal_host.overlay` as the window child.
/// • Store `modal_host` in `BrowserController`.
/// • Call `modal_host.show_input / show_confirm / show_error / show_with_custom_ui`.
/// • Never open `gtk::Dialog`, `gtk::AlertDialog`, or any new `ApplicationWindow`
///   for normal Lattice dialogs — use this host instead.
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, Entry, EventControllerKey, GestureClick, Label, Orientation,
    Overlay, Separator,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ── Public helpers ────────────────────────────────────────────────────────────

/// Button role used to apply the correct CSS class.
#[derive(Clone, Copy)]
pub enum ButtonKind {
    Primary,
    Danger,
    Secondary,
}

/// Build a styled modal action button.
pub fn build_modal_button(label: &str, kind: ButtonKind, on_click: impl Fn() + 'static) -> Button {
    let btn = Button::with_label(label);
    btn.add_css_class(match kind {
        ButtonKind::Primary => "modal-primary-button",
        ButtonKind::Danger => "modal-danger-button",
        ButtonKind::Secondary => "modal-secondary-button",
    });
    btn.connect_clicked(move |_| on_click());
    btn
}

/// Build the horizontal row that holds modal action buttons.
pub fn build_modal_actions() -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.add_css_class("modal-actions");
    row.set_halign(Align::End);
    row
}

/// Build a wrapped, word-wrapping prompt label styled for modal content.
pub fn build_modal_prompt(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("dialog-prompt");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(48);
    label.set_halign(Align::Start);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label
}

// ── ModalHost ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ModalHost {
    /// The GtkOverlay that wraps the entire app UI.
    /// Set this as the window's child; place the normal app UI as its main child.
    pub overlay: Overlay,
    inner: Rc<Inner>,
}

struct Inner {
    /// The outer overlay layer (scrim + panel).  Hidden when no dialog is open.
    modal_outer: Overlay,
    title_label: Label,
    content_slot: GtkBox,
    actions_wrapper: GtkBox,
    /// When true, clicking the scrim or pressing Escape closes the modal.
    scrim_dismisses: Cell<bool>,
    /// Called when the modal is dismissed via scrim/Escape.
    dismiss_cb: RefCell<Option<Box<dyn Fn()>>>,
}

impl ModalHost {
    pub fn new() -> Self {
        // ── App-level overlay ─────────────────────────────────────────────
        let overlay = Overlay::new();

        // ── Modal outer layer (scrim + panel) ─────────────────────────────
        let modal_outer = Overlay::new();
        modal_outer.add_css_class("modal-layer");
        modal_outer.set_halign(Align::Fill);
        modal_outer.set_valign(Align::Fill);
        modal_outer.set_hexpand(true);
        modal_outer.set_vexpand(true);
        modal_outer.set_visible(false);

        // Scrim: fills the full layer, provides the dim backdrop
        let scrim = GtkBox::new(Orientation::Vertical, 0);
        scrim.add_css_class("modal-scrim");
        modal_outer.set_child(Some(&scrim));

        // Panel: centered dialog card, overlaid on the scrim
        let panel = GtkBox::new(Orientation::Vertical, 0);
        panel.add_css_class("modal-panel");
        panel.set_halign(Align::Center);
        panel.set_valign(Align::Center);

        // Title row
        let title_label = Label::new(None);
        title_label.add_css_class("modal-title");
        title_label.set_halign(Align::Start);
        title_label.set_xalign(0.0);
        title_label.set_hexpand(true);

        let title_row = GtkBox::new(Orientation::Horizontal, 0);
        title_row.add_css_class("modal-title-row");
        title_row.append(&title_label);
        panel.append(&title_row);

        let title_sep = Separator::new(Orientation::Horizontal);
        title_sep.add_css_class("modal-title-sep");
        panel.append(&title_sep);

        // Content area (replaced each time a modal is shown)
        let content_slot = GtkBox::new(Orientation::Vertical, 0);
        content_slot.add_css_class("modal-content");
        panel.append(&content_slot);

        // Actions area
        let actions_sep = Separator::new(Orientation::Horizontal);
        actions_sep.add_css_class("modal-actions-sep");
        panel.append(&actions_sep);

        let actions_wrapper = GtkBox::new(Orientation::Horizontal, 0);
        actions_wrapper.add_css_class("modal-actions-wrapper");
        panel.append(&actions_wrapper);

        // Add panel as an overlay on the scrim.
        // In GTK4, overlay children naturally capture events in their allocated area
        // — no pass-through flag needed.
        modal_outer.add_overlay(&panel);

        // Add modal_outer as an overlay on the app overlay.
        overlay.add_overlay(&modal_outer);

        // panel stays alive through GTK's ref-count as an overlay child of modal_outer
        let _ = panel;

        let inner = Rc::new(Inner {
            modal_outer,
            title_label,
            content_slot,
            actions_wrapper,
            scrim_dismisses: Cell::new(false),
            dismiss_cb: RefCell::new(None),
        });

        // ── Scrim click → dismiss ─────────────────────────────────────────
        {
            let iw = Rc::downgrade(&inner);
            let scrim_click = GestureClick::new();
            scrim_click.connect_pressed(move |_, _, _, _| {
                let Some(inn) = iw.upgrade() else { return };
                if inn.scrim_dismisses.get() {
                    let cb = inn.dismiss_cb.borrow();
                    if let Some(f) = cb.as_ref() {
                        f();
                    }
                }
            });
            scrim.add_controller(scrim_click);
        }

        // Note: no GestureClick is needed on the panel to "block" the scrim.
        // In a GtkOverlay, the overlay child (panel) receives events in its area
        // and the main child (scrim) receives events only outside that area.
        // Adding a capture-phase GestureClick on the panel would prevent child
        // buttons from ever receiving click events.

        // ── Escape key → dismiss (if scrim_dismisses) ────────────────────
        {
            let iw = Rc::downgrade(&inner);
            let key_ctrl = EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    if let Some(inn) = iw.upgrade() {
                        if inn.scrim_dismisses.get() {
                            let cb = inn.dismiss_cb.borrow();
                            if let Some(f) = cb.as_ref() {
                                f();
                            }
                            return glib::Propagation::Stop;
                        }
                    }
                }
                glib::Propagation::Proceed
            });
            inner.modal_outer.add_controller(key_ctrl);
        }

        Self { overlay, inner }
    }

    // ── Public show methods ───────────────────────────────────────────────────

    /// Core method. Populate the modal with pre-built `content` and `actions`
    /// widgets, then reveal it.  Call `host.hide()` from within action
    /// callbacks when the dialog should close.
    ///
    /// `scrim_dismisses`: clicking the scrim or pressing Escape closes the modal.
    /// `dismiss_cb`: optional callback invoked on scrim/Escape dismiss.
    pub fn show_with_custom_ui(
        &self,
        title: &str,
        content: &impl IsA<gtk::Widget>,
        actions: &GtkBox,
        scrim_dismisses: bool,
        dismiss_cb: Option<Box<dyn Fn()>>,
    ) {
        self.clear_slots();
        self.inner.title_label.set_label(title);
        self.inner.scrim_dismisses.set(scrim_dismisses);
        *self.inner.dismiss_cb.borrow_mut() = dismiss_cb;

        self.inner.content_slot.append(content);
        self.inner.actions_wrapper.append(actions);

        self.inner.modal_outer.set_visible(true);
    }

    /// Show a text-input modal (e.g. Rename, New Folder, Pin Palette, Add Tag).
    /// `on_accept` receives the trimmed entry text. The modal auto-closes on
    /// accept or cancel, and the scrim also dismisses it.
    pub fn show_input(
        &self,
        title: &str,
        prompt: &str,
        initial: &str,
        confirm_label: &str,
        on_accept: impl Fn(String) + 'static,
    ) {
        let content = GtkBox::new(Orientation::Vertical, 10);
        content.add_css_class("modal-input-content");
        content.append(&build_modal_prompt(prompt));

        let entry = Entry::new();
        entry.add_css_class("dialog-entry");
        entry.set_text(initial);
        entry.select_region(0, -1);
        entry.set_hexpand(true);
        content.append(&entry);

        let actions = build_modal_actions();

        let host = self.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.clone();
        let entry_ref = entry.clone();
        let on_accept = Rc::new(on_accept);
        let accept_btn = build_modal_button(confirm_label, ButtonKind::Primary, move || {
            let text = entry_ref.text().trim().to_string();
            on_accept(text);
            host.hide();
        });
        accept_btn.set_sensitive(!initial.trim().is_empty());
        actions.append(&accept_btn);

        // Keep accept button sensitive only when entry has non-whitespace text
        {
            let accept_btn = accept_btn.clone();
            entry.connect_changed(move |e| {
                accept_btn.set_sensitive(!e.text().trim().is_empty());
            });
        }

        // Enter in the entry activates accept
        {
            let accept_btn = accept_btn.clone();
            entry.connect_activate(move |_| {
                if accept_btn.is_sensitive() {
                    accept_btn.emit_clicked();
                }
            });
        }

        let host = self.clone();
        self.show_with_custom_ui(
            title,
            &content,
            &actions,
            true,
            Some(Box::new(move || host.hide())),
        );

        entry.grab_focus();
    }

    /// Show a confirmation modal with Cancel and a primary/danger action button.
    /// `scrim_dismisses`: whether clicking outside the panel closes the dialog.
    /// For destructive confirmations pass `scrim_dismisses = false`.
    pub fn show_confirm(
        &self,
        title: &str,
        prompt: &str,
        confirm_label: &str,
        dangerous: bool,
        scrim_dismisses: bool,
        on_accept: impl Fn() + 'static,
    ) {
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.add_css_class("modal-confirm-content");
        content.append(&build_modal_prompt(prompt));

        let actions = build_modal_actions();

        let host = self.clone();
        let cancel_btn = build_modal_button("Cancel", ButtonKind::Secondary, move || host.hide());
        actions.append(&cancel_btn);

        let host = self.clone();
        let on_accept = Rc::new(on_accept);
        let kind = if dangerous {
            ButtonKind::Danger
        } else {
            ButtonKind::Primary
        };
        let accept_btn = build_modal_button(confirm_label, kind, move || {
            on_accept();
            host.hide();
        });
        actions.append(&accept_btn);

        let dismiss_cb: Option<Box<dyn Fn()>> = if scrim_dismisses {
            let host = self.clone();
            Some(Box::new(move || host.hide()))
        } else {
            None
        };

        self.show_with_custom_ui(title, &content, &actions, scrim_dismisses, dismiss_cb);
        accept_btn.grab_focus();
    }

    /// Show an error notification with an OK button.
    /// Scrim click and Escape also dismiss it.
    pub fn show_error(&self, title: &str, detail: &str) {
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.add_css_class("modal-error-content");
        content.append(&build_modal_prompt(detail));

        let actions = build_modal_actions();
        let host = self.clone();
        let ok_btn = build_modal_button("OK", ButtonKind::Primary, move || host.hide());
        actions.append(&ok_btn);

        let host = self.clone();
        self.show_with_custom_ui(
            title,
            &content,
            &actions,
            true,
            Some(Box::new(move || host.hide())),
        );
        ok_btn.grab_focus();
    }

    /// Hide the current modal and clear its contents.
    pub fn hide(&self) {
        self.inner.modal_outer.set_visible(false);
        self.clear_slots();
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn clear_slots(&self) {
        while let Some(child) = self.inner.content_slot.first_child() {
            self.inner.content_slot.remove(&child);
        }
        while let Some(child) = self.inner.actions_wrapper.first_child() {
            self.inner.actions_wrapper.remove(&child);
        }
        *self.inner.dismiss_cb.borrow_mut() = None;
        self.inner.scrim_dismisses.set(false);
    }
}
