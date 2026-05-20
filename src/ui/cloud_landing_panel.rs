use crate::metadata::CloudRecord;
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Label, Orientation, Separator};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct CloudLandingPanel {
    pub root: GtkBox,
    inner: GtkBox,
    // Holds the current status label for async availability updates.
    live_status: Rc<RefCell<Option<Label>>>,
    // Open Drive / Space Viewer / Triage — disabled when unavailable.
    live_action_btns: Rc<RefCell<Option<[Button; 3]>>>,
    // Mount / Unmount — visibility toggled by set_availability().
    live_mount_btns: Rc<RefCell<Option<(Button, Button)>>>,
}

impl CloudLandingPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("project-landing");
        root.set_vexpand(false);
        root.set_hexpand(true);
        root.set_visible(false);

        let inner = GtkBox::new(Orientation::Vertical, 0);
        inner.set_hexpand(true);
        root.append(&inner);

        Self {
            root,
            inner,
            live_status: Rc::new(RefCell::new(None)),
            live_action_btns: Rc::new(RefCell::new(None)),
            live_mount_btns: Rc::new(RefCell::new(None)),
        }
    }

    pub fn populate<FDrive, FSpaceViewer, FTriage, FEdit, FRemove>(
        &self,
        record: &CloudRecord,
        on_open_drive: FDrive,
        on_space_viewer: FSpaceViewer,
        on_triage: FTriage,
        on_edit: FEdit,
        on_remove: FRemove,
        on_mount: Option<Box<dyn Fn()>>,
        on_unmount: Option<Box<dyn Fn()>>,
    ) where
        FDrive: Fn() + 'static,
        FSpaceViewer: Fn() + 'static,
        FTriage: Fn() + 'static,
        FEdit: Fn() + 'static,
        FRemove: Fn() + 'static,
    {
        while let Some(child) = self.inner.first_child() {
            self.inner.remove(&child);
        }
        *self.live_status.borrow_mut() = None;
        *self.live_action_btns.borrow_mut() = None;
        *self.live_mount_btns.borrow_mut() = None;

        let section = GtkBox::new(Orientation::Vertical, 8);
        section.add_css_class("landing-section");
        section.set_margin_start(12);
        section.set_margin_end(12);
        section.set_margin_top(12);
        section.set_margin_bottom(12);

        // Header row: icon + name + kind badge
        let header_row = GtkBox::new(Orientation::Horizontal, 8);
        header_row.add_css_class("cloud-landing-header");

        let icon = Label::new(Some("☁"));
        icon.add_css_class("cloud-landing-icon");
        header_row.append(&icon);

        let name_label = Label::new(Some(&record.name));
        name_label.add_css_class("landing-section-heading");
        name_label.set_halign(Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header_row.append(&name_label);

        let kind_badge = Label::new(Some(&record.kind));
        kind_badge.add_css_class("cloud-landing-kind-badge");
        kind_badge.set_halign(Align::End);
        header_row.append(&kind_badge);

        section.append(&header_row);

        // Path label
        let path_label = Label::new(Some(&record.path));
        path_label.add_css_class("cloud-landing-path");
        path_label.set_halign(Align::Start);
        path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path_label.set_max_width_chars(52);
        section.append(&path_label);

        // Remote name row (only for rclone entries with a remote_name set)
        if let Some(rn) = &record.remote_name {
            if !rn.trim().is_empty() {
                let rn_label = Label::new(Some(&format!("remote: {rn}")));
                rn_label.add_css_class("cloud-landing-remote-name");
                rn_label.set_halign(Align::Start);
                section.append(&rn_label);
            }
        }

        // Status label — created fresh each populate to avoid reparent issues
        let status_label = Label::new(Some("Checking…"));
        status_label.add_css_class("cloud-landing-status");
        status_label.set_halign(Align::Start);
        section.append(&status_label);
        *self.live_status.borrow_mut() = Some(status_label);

        // Notes
        if let Some(notes) = &record.notes {
            if !notes.trim().is_empty() {
                let notes_label = Label::new(Some(notes.trim()));
                notes_label.add_css_class("cloud-landing-notes");
                notes_label.set_halign(Align::Start);
                notes_label.set_wrap(true);
                section.append(&notes_label);
            }
        }

        section.append(&Separator::new(Orientation::Horizontal));

        // Contextual notice
        let notice_text = if record.kind == "rclone" && record.remote_name.is_some() {
            "rclone manages this mount. Credentials are in rclone config — not in Lattice."
        } else {
            "Cloud support uses externally mounted locations. No provider API is used."
        };
        let notice = Label::new(Some(notice_text));
        notice.add_css_class("cloud-landing-notice");
        notice.set_halign(Align::Start);
        notice.set_wrap(true);
        section.append(&notice);

        section.append(&Separator::new(Orientation::Horizontal));

        // Primary action buttons (Open Drive / Space Viewer / Triage)
        let primary_actions = GtkBox::new(Orientation::Horizontal, 8);
        primary_actions.add_css_class("cloud-landing-actions");

        let drive_btn = Button::with_label("Open Drive");
        drive_btn.add_css_class("landing-add-btn");
        drive_btn.connect_clicked(move |_| on_open_drive());
        crate::ui::attach_tooltip(&drive_btn, "Browse this cloud drive in the file grid");
        primary_actions.append(&drive_btn);

        let sv_btn = Button::with_label("Space Viewer");
        sv_btn.add_css_class("landing-add-btn");
        sv_btn.connect_clicked(move |_| on_space_viewer());
        crate::ui::attach_tooltip(
            &sv_btn,
            "Analyse disk usage on this cloud drive (can be slow for remote paths)",
        );
        primary_actions.append(&sv_btn);

        let triage_btn = Button::with_label("Triage");
        triage_btn.add_css_class("landing-add-btn");
        triage_btn.connect_clicked(move |_| on_triage());
        crate::ui::attach_tooltip(&triage_btn, "Sort and triage files on this cloud drive");
        primary_actions.append(&triage_btn);

        section.append(&primary_actions);

        *self.live_action_btns.borrow_mut() =
            Some([drive_btn.clone(), sv_btn.clone(), triage_btn.clone()]);

        // Mount / Unmount row (rclone profiles only)
        if on_mount.is_some() || on_unmount.is_some() {
            let mount_row = GtkBox::new(Orientation::Horizontal, 8);
            mount_row.add_css_class("cloud-landing-actions");

            let mount_btn = Button::with_label("Mount");
            mount_btn.add_css_class("cloud-mount-btn");
            if let Some(cb) = on_mount {
                mount_btn.connect_clicked(move |_| cb());
            }
            crate::ui::attach_tooltip(
                &mount_btn,
                "Mount this rclone remote at the configured path",
            );
            mount_row.append(&mount_btn);

            let unmount_btn = Button::with_label("Unmount");
            unmount_btn.add_css_class("cloud-unmount-btn");
            if let Some(cb) = on_unmount {
                unmount_btn.connect_clicked(move |_| cb());
            }
            crate::ui::attach_tooltip(&unmount_btn, "Unmount this rclone remote");
            mount_row.append(&unmount_btn);

            section.append(&mount_row);
            *self.live_mount_btns.borrow_mut() = Some((mount_btn, unmount_btn));
        }

        // Management row (Edit / Remove — always enabled)
        let mgmt_row = GtkBox::new(Orientation::Horizontal, 8);
        mgmt_row.add_css_class("cloud-landing-actions");

        let spacer = Label::new(None);
        spacer.set_hexpand(true);
        mgmt_row.append(&spacer);

        let edit_btn = Button::with_label("Edit");
        edit_btn.add_css_class("landing-add-btn");
        edit_btn.connect_clicked(move |_| on_edit());
        crate::ui::attach_tooltip(
            &edit_btn,
            "Edit the name, path, kind, or notes for this entry",
        );
        mgmt_row.append(&edit_btn);

        let remove_btn = Button::with_label("Remove");
        remove_btn.add_css_class("cloud-landing-remove-btn");
        remove_btn.connect_clicked(move |_| on_remove());
        crate::ui::attach_tooltip(
            &remove_btn,
            "Remove this Cloud entry from Lattice — does not delete any files",
        );
        mgmt_row.append(&remove_btn);

        section.append(&mgmt_row);

        self.inner.append(&section);
        self.root.set_visible(true);
    }

    pub fn set_availability(&self, available: Option<bool>) {
        if let Some(label) = self.live_status.borrow().as_ref() {
            label.remove_css_class("cloud-status-available");
            label.remove_css_class("cloud-status-unavailable");
            match available {
                Some(true) => {
                    label.set_label("● Mounted / Available");
                    label.add_css_class("cloud-status-available");
                }
                Some(false) => {
                    label.set_label("○ Unavailable — not mounted or path not found");
                    label.add_css_class("cloud-status-unavailable");
                }
                None => {
                    label.set_label("Checking…");
                }
            }
        }

        // Enable/disable Open Drive / Space Viewer / Triage based on availability
        let sensitive = available.unwrap_or(true);
        if let Some(btns) = self.live_action_btns.borrow().as_ref() {
            for btn in btns {
                btn.set_sensitive(sensitive);
            }
        }

        // Flip Mount / Unmount visibility: mounted → show Unmount; not mounted → show Mount
        if let Some((mount_btn, unmount_btn)) = self.live_mount_btns.borrow().as_ref() {
            let mounted = available.unwrap_or(false);
            mount_btn.set_visible(!mounted);
            unmount_btn.set_visible(mounted);
        }
    }

    /// Disable Mount and Unmount buttons while a mount/unmount operation is in progress.
    pub fn set_mount_busy(&self, busy: bool) {
        if let Some((mount_btn, unmount_btn)) = self.live_mount_btns.borrow().as_ref() {
            mount_btn.set_sensitive(!busy);
            unmount_btn.set_sensitive(!busy);
        }
    }
}
