use crate::config::{shortcut_tooltip, AppConfig};
use crate::converter::{BatchProgress, ConversionJob, ConversionJobId, ConversionJobStatus};
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Label, Orientation, ProgressBar, Revealer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

struct JobRow {
    status_label: Label,
    job_pb: ProgressBar,
    job_pb_revealer: Revealer,
    error_label: Label,
    error_revealer: Revealer,
    copy_wired: bool,
    job: ConversionJob,
}

struct Inner {
    jobs_box: GtkBox,
    batch_pb: ProgressBar,
    header_label: Label,
    cancel_btn: Button,
    retry_btn: Button,
    open_btn: Button,
    dismiss_btn: Button,
    job_rows: HashMap<ConversionJobId, JobRow>,
    output_dir: Option<PathBuf>,
    on_cancel: Option<Box<dyn Fn()>>,
    on_retry_failed: Option<Box<dyn Fn(Vec<ConversionJob>)>>,
    on_open_output: Option<Box<dyn Fn(PathBuf)>>,
    on_copy_error: Option<Rc<dyn Fn(String)>>,
}

/// A slide-up panel (like OpsPanel) showing detailed per-job conversion status.
/// Lives at the bottom of the window; stays visible while the user browses.
#[derive(Clone)]
pub struct ConvertProgressPanel {
    pub root: Revealer,
    inner: Rc<RefCell<Inner>>,
}

impl ConvertProgressPanel {
    pub fn build(config: &AppConfig) -> Self {
        let panel = GtkBox::new(Orientation::Vertical, 0);
        panel.add_css_class("convert-progress-panel");

        // Header: title + overall progress bar + counts
        let header = GtkBox::new(Orientation::Horizontal, 8);
        header.add_css_class("convert-progress-header");

        let title = Label::new(Some("Converting"));
        title.add_css_class("convert-progress-title");
        title.set_halign(Align::Start);
        header.append(&title);

        let batch_pb = ProgressBar::new();
        batch_pb.add_css_class("convert-progress-batch-bar");
        batch_pb.set_hexpand(true);
        batch_pb.set_valign(Align::Center);
        header.append(&batch_pb);

        let header_label = Label::new(Some(""));
        header_label.add_css_class("convert-progress-counts");
        header_label.set_halign(Align::End);
        header.append(&header_label);

        panel.append(&header);

        // Scrollable per-job list
        let jobs_box = GtkBox::new(Orientation::Vertical, 0);
        jobs_box.add_css_class("convert-progress-jobs");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&jobs_box)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(200)
            .propagate_natural_height(true)
            .build();
        scroll.add_css_class("convert-progress-scroll");
        panel.append(&scroll);

        // Footer controls
        let footer = GtkBox::new(Orientation::Horizontal, 6);
        footer.add_css_class("convert-progress-footer");

        let cancel_btn = Button::with_label("Cancel");
        cancel_btn.add_css_class("convert-progress-btn");
        crate::ui::attach_tooltip(
            &cancel_btn,
            shortcut_tooltip(config, "Cancel conversion", "convert_cancel"),
        );
        footer.append(&cancel_btn);

        let retry_btn = Button::with_label("Retry Failed");
        retry_btn.add_css_class("convert-progress-btn");
        retry_btn.add_css_class("convert-progress-btn-retry");
        retry_btn.set_visible(false);
        crate::ui::attach_tooltip(
            &retry_btn,
            shortcut_tooltip(config, "Retry failed jobs", "convert_retry_failed"),
        );
        footer.append(&retry_btn);

        let spacer = Label::new(None);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let open_btn = Button::with_label("Open Output");
        open_btn.add_css_class("convert-progress-btn");
        open_btn.set_visible(false);
        crate::ui::attach_tooltip(
            &open_btn,
            shortcut_tooltip(config, "Open output folder", "convert_open_output"),
        );
        footer.append(&open_btn);

        let dismiss_btn = Button::with_label("Dismiss");
        dismiss_btn.add_css_class("convert-progress-btn");
        dismiss_btn.set_visible(false);
        crate::ui::attach_tooltip(
            &dismiss_btn,
            shortcut_tooltip(config, "Dismiss panel", "convert_dismiss"),
        );
        footer.append(&dismiss_btn);

        panel.append(&footer);

        let root = Revealer::new();
        root.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        root.set_transition_duration(200);
        root.set_child(Some(&panel));
        root.set_reveal_child(false);

        let inner = Rc::new(RefCell::new(Inner {
            jobs_box,
            batch_pb,
            header_label,
            cancel_btn: cancel_btn.clone(),
            retry_btn: retry_btn.clone(),
            open_btn: open_btn.clone(),
            dismiss_btn: dismiss_btn.clone(),
            job_rows: HashMap::new(),
            output_dir: None,
            on_cancel: None,
            on_retry_failed: None,
            on_open_output: None,
            on_copy_error: None,
        }));

        {
            let inner = Rc::clone(&inner);
            cancel_btn.connect_clicked(move |btn| {
                // Disable immediately and show "Cancelling…" so the user knows the
                // request was received (conversion jobs finish their current 50ms poll)
                btn.set_label("Cancelling…");
                btn.set_sensitive(false);
                if let Some(cb) = inner.borrow().on_cancel.as_ref() {
                    cb();
                }
            });
        }

        {
            let inner = Rc::clone(&inner);
            retry_btn.connect_clicked(move |btn| {
                btn.set_visible(false);
                let failed: Vec<ConversionJob> = inner
                    .borrow()
                    .job_rows
                    .values()
                    .filter(|r| matches!(r.job.status, ConversionJobStatus::Failed(_)))
                    .map(|r| r.job.clone())
                    .collect();
                if let Some(cb) = inner.borrow().on_retry_failed.as_ref() {
                    cb(failed);
                }
            });
        }

        {
            let inner = Rc::clone(&inner);
            open_btn.connect_clicked(move |_| {
                let dir = inner.borrow().output_dir.clone();
                if let Some(path) = dir {
                    if let Some(cb) = inner.borrow().on_open_output.as_ref() {
                        cb(path);
                    }
                }
            });
        }

        {
            let root_d = root.clone();
            dismiss_btn.connect_clicked(move |_| {
                root_d.set_reveal_child(false);
            });
        }

        ConvertProgressPanel { root, inner }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Prepare panel for a new batch.
    pub fn start_batch(&self, jobs: &[ConversionJob], output_dir: Option<PathBuf>) {
        let mut inn = self.inner.borrow_mut();

        while let Some(child) = inn.jobs_box.first_child() {
            inn.jobs_box.remove(&child);
        }
        inn.job_rows.clear();

        inn.output_dir = output_dir;
        inn.batch_pb.set_fraction(0.0);
        inn.header_label.set_label("");
        inn.cancel_btn.set_label("Cancel");
        inn.cancel_btn.set_visible(true);
        inn.cancel_btn.set_sensitive(true);
        inn.retry_btn.set_visible(false);
        inn.open_btn.set_visible(false);
        inn.dismiss_btn.set_visible(false);

        for job in jobs {
            let row_root = GtkBox::new(Orientation::Vertical, 0);
            row_root.add_css_class("convert-job-row");

            let top = GtkBox::new(Orientation::Horizontal, 6);
            top.add_css_class("convert-job-row-top");

            let source_name = job
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".to_string());

            let name_label = Label::new(Some(&source_name));
            name_label.add_css_class("convert-job-name");
            name_label.set_halign(Align::Start);
            name_label.set_hexpand(true);
            name_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            top.append(&name_label);

            let dest_name = job
                .dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dest_label = Label::new(Some(&format!("→ {dest_name}")));
            dest_label.add_css_class("convert-job-dest");
            dest_label.set_halign(Align::End);
            dest_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            dest_label.set_max_width_chars(24);
            top.append(&dest_label);

            let status_label = Label::new(Some("⏳ Queued"));
            status_label.add_css_class("convert-job-status");
            status_label.set_halign(Align::End);
            top.append(&status_label);

            row_root.append(&top);

            // Per-job progress bar
            let job_pb = ProgressBar::new();
            job_pb.add_css_class("convert-job-pb");
            job_pb.set_margin_start(10);
            job_pb.set_margin_end(10);
            let job_pb_revealer = Revealer::new();
            job_pb_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
            job_pb_revealer.set_transition_duration(120);
            job_pb_revealer.set_child(Some(&job_pb));
            job_pb_revealer.set_reveal_child(false);
            row_root.append(&job_pb_revealer);

            // Error detail area
            let error_area = GtkBox::new(Orientation::Vertical, 4);
            error_area.add_css_class("convert-job-error-area");
            error_area.set_margin_start(10);
            error_area.set_margin_end(10);
            error_area.set_margin_bottom(4);

            let error_label = Label::new(None);
            error_label.add_css_class("convert-job-error-label");
            error_label.set_halign(Align::Start);
            error_label.set_wrap(true);
            error_label.set_selectable(true);
            error_area.append(&error_label);

            let error_revealer = Revealer::new();
            error_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
            error_revealer.set_transition_duration(150);
            error_revealer.set_child(Some(&error_area));
            error_revealer.set_reveal_child(false);
            row_root.append(&error_revealer);

            inn.jobs_box.append(&row_root);
            inn.job_rows.insert(
                job.id,
                JobRow {
                    status_label,
                    job_pb,
                    job_pb_revealer,
                    error_label,
                    error_revealer,
                    copy_wired: false,
                    job: job.clone(),
                },
            );
        }

        drop(inn);
        self.root.set_reveal_child(true);
    }

    pub fn update_job_status(&self, job_id: ConversionJobId, status: ConversionJobStatus) {
        // Extract info needed before borrowing inner mutably
        let (error_text, error_area_opt) = {
            let inn = self.inner.borrow();
            let row = inn.job_rows.get(&job_id);
            match row {
                Some(r) => {
                    if let ConversionJobStatus::Failed(ref msg) = status {
                        let area = r
                            .error_revealer
                            .child()
                            .and_then(|w| w.downcast::<GtkBox>().ok());
                        (Some(msg.clone()), area)
                    } else {
                        (None, None)
                    }
                }
                None => return,
            }
        };

        {
            let mut inn = self.inner.borrow_mut();
            let Some(row) = inn.job_rows.get_mut(&job_id) else {
                return;
            };
            row.job.status = status.clone();

            match &status {
                ConversionJobStatus::Pending => {
                    row.status_label.set_label("⏳ Queued");
                    row.status_label.remove_css_class("cjr-done");
                    row.status_label.remove_css_class("cjr-failed");
                    row.status_label.remove_css_class("cjr-running");
                    row.status_label.remove_css_class("cjr-skipped");
                    row.job_pb_revealer.set_reveal_child(false);
                    row.error_revealer.set_reveal_child(false);
                }
                ConversionJobStatus::Running => {
                    row.status_label.set_label("↻ Running…");
                    row.status_label.remove_css_class("cjr-done");
                    row.status_label.remove_css_class("cjr-failed");
                    row.status_label.add_css_class("cjr-running");
                    row.status_label.remove_css_class("cjr-skipped");
                    row.job_pb.set_fraction(0.0);
                    row.job_pb_revealer.set_reveal_child(true);
                    row.error_revealer.set_reveal_child(false);
                }
                ConversionJobStatus::Done => {
                    row.status_label.set_label("✓ Done");
                    row.status_label.add_css_class("cjr-done");
                    row.status_label.remove_css_class("cjr-failed");
                    row.status_label.remove_css_class("cjr-running");
                    row.status_label.remove_css_class("cjr-skipped");
                    row.job_pb_revealer.set_reveal_child(false);
                }
                ConversionJobStatus::Failed(msg) => {
                    row.status_label.set_label("✗ Failed");
                    row.status_label.remove_css_class("cjr-done");
                    row.status_label.add_css_class("cjr-failed");
                    row.status_label.remove_css_class("cjr-running");
                    row.status_label.remove_css_class("cjr-skipped");
                    row.job_pb_revealer.set_reveal_child(false);
                    row.error_label.set_label(msg);
                    row.error_revealer.set_reveal_child(true);
                    row.copy_wired = false; // will wire below
                }
                ConversionJobStatus::Cancelled => {
                    row.status_label.set_label("⊘ Cancelled");
                    row.status_label.remove_css_class("cjr-done");
                    row.status_label.remove_css_class("cjr-failed");
                    row.status_label.remove_css_class("cjr-running");
                    row.status_label.remove_css_class("cjr-skipped");
                    row.job_pb_revealer.set_reveal_child(false);
                }
                ConversionJobStatus::Skipped(reason) => {
                    // Use a short label for the chip; put detail in the tooltip so
                    // the row doesn't grow wide for long skip reasons.
                    let short = if reason.contains("wrong type") || reason.contains("converts") {
                        "↷ Wrong type"
                    } else if reason.contains("already exists") {
                        "↷ Already exists"
                    } else if reason.contains("not installed") {
                        "↷ Tool missing"
                    } else {
                        "↷ Skipped"
                    };
                    row.status_label.set_label(short);
                    row.status_label.set_tooltip_text(Some(reason.as_str()));
                    row.status_label.remove_css_class("cjr-done");
                    row.status_label.remove_css_class("cjr-failed");
                    row.status_label.remove_css_class("cjr-running");
                    row.status_label.add_css_class("cjr-skipped");
                    row.job_pb_revealer.set_reveal_child(false);
                }
            }
        }

        // Wire copy button for failed rows (after releasing the borrow above)
        if let (Some(msg), Some(error_area)) = (error_text, error_area_opt) {
            let copy_cb = self.inner.borrow().on_copy_error.clone();
            if let Some(cb) = copy_cb {
                // Check not already wired
                let already = self
                    .inner
                    .borrow()
                    .job_rows
                    .get(&job_id)
                    .map(|r| r.copy_wired)
                    .unwrap_or(true);
                if !already {
                    let copy_btn = Button::with_label("Copy error");
                    copy_btn.add_css_class("convert-job-copy-btn");
                    copy_btn.set_halign(Align::Start);
                    crate::ui::attach_tooltip(&copy_btn, "Copy error text");
                    let cb = Rc::clone(&cb);
                    copy_btn.connect_clicked(move |_| cb(msg.clone()));
                    error_area.append(&copy_btn);
                    if let Some(row) = self.inner.borrow_mut().job_rows.get_mut(&job_id) {
                        row.copy_wired = true;
                    }
                }
            }
        }
    }

    pub fn update_batch_progress(&self, progress: BatchProgress) {
        let inn = self.inner.borrow();
        inn.batch_pb
            .set_fraction(progress.fraction_done().clamp(0.0, 1.0));

        let done_count =
            progress.completed + progress.failed + progress.cancelled + progress.skipped;
        let mut parts = Vec::new();
        if progress.completed > 0 {
            parts.push(format!("✓ {}", progress.completed));
        }
        if progress.failed > 0 {
            parts.push(format!("✗ {}", progress.failed));
        }
        if progress.skipped > 0 {
            parts.push(format!("↷ {}", progress.skipped));
        }
        let counts = if parts.is_empty() {
            format!("{done_count} / {}", progress.total)
        } else {
            format!("{done_count} / {}  ({})", progress.total, parts.join("  "))
        };
        inn.header_label.set_label(&counts);

        let has_failed = progress.failed > 0;
        inn.retry_btn
            .set_visible(has_failed && progress.is_finished());

        if progress.is_finished() {
            inn.cancel_btn.set_visible(false);
            inn.open_btn.set_visible(inn.output_dir.is_some());
            inn.dismiss_btn.set_visible(true);

            if progress.failed == 0 && progress.cancelled == 0 {
                let root = self.root.clone();
                glib::timeout_add_local_once(Duration::from_secs(4), move || {
                    root.set_reveal_child(false);
                });
            }
        }
    }

    pub fn update_job_progress(&self, job_id: ConversionJobId, fraction: f64) {
        let inn = self.inner.borrow();
        if let Some(row) = inn.job_rows.get(&job_id) {
            row.job_pb.set_fraction(fraction.clamp(0.0, 1.0));
        }
    }

    pub fn connect_cancel(&self, cb: impl Fn() + 'static) {
        self.inner.borrow_mut().on_cancel = Some(Box::new(cb));
    }

    pub fn trigger_cancel(&self) -> bool {
        {
            let inn = self.inner.borrow();
            if !inn.cancel_btn.is_visible() || !inn.cancel_btn.is_sensitive() {
                return false;
            }
            inn.cancel_btn.set_label("Cancelling…");
            inn.cancel_btn.set_sensitive(false);
        }
        let cb = self.inner.borrow_mut().on_cancel.take();
        let handled = if let Some(cb_ref) = cb.as_ref() {
            cb_ref();
            true
        } else {
            false
        };
        self.inner.borrow_mut().on_cancel = cb;
        handled
    }

    pub fn connect_retry_failed(&self, cb: impl Fn(Vec<ConversionJob>) + 'static) {
        self.inner.borrow_mut().on_retry_failed = Some(Box::new(cb));
    }

    pub fn trigger_retry_failed(&self) -> bool {
        let failed: Vec<ConversionJob> = {
            let inn = self.inner.borrow();
            if !inn.retry_btn.is_visible() {
                return false;
            }
            inn.job_rows
                .values()
                .filter(|r| matches!(r.job.status, ConversionJobStatus::Failed(_)))
                .map(|r| r.job.clone())
                .collect()
        };
        self.inner.borrow().retry_btn.set_visible(false);
        let cb = self.inner.borrow_mut().on_retry_failed.take();
        let handled = if let Some(cb_ref) = cb.as_ref() {
            cb_ref(failed);
            true
        } else {
            false
        };
        self.inner.borrow_mut().on_retry_failed = cb;
        handled
    }

    pub fn connect_open_output(&self, cb: impl Fn(PathBuf) + 'static) {
        self.inner.borrow_mut().on_open_output = Some(Box::new(cb));
    }

    pub fn trigger_open_output(&self) -> bool {
        let path = {
            let inn = self.inner.borrow();
            if !inn.open_btn.is_visible() {
                return false;
            }
            inn.output_dir.clone()
        };
        let cb = self.inner.borrow_mut().on_open_output.take();
        let handled = if let (Some(path), Some(cb_ref)) = (path, cb.as_ref()) {
            cb_ref(path);
            true
        } else {
            false
        };
        self.inner.borrow_mut().on_open_output = cb;
        handled
    }

    pub fn trigger_dismiss(&self) -> bool {
        if !self.root.reveals_child() {
            return false;
        }
        self.root.set_reveal_child(false);
        true
    }

    /// Set the clipboard callback used by per-job "Copy error" buttons.
    pub fn set_copy_error_fn(&self, cb: impl Fn(String) + 'static) {
        self.inner.borrow_mut().on_copy_error = Some(Rc::new(cb));
    }

    /// Call after `connect_retry_failed` to wire copy buttons on any already-failed rows.
    pub fn wire_copy_buttons(&self) {}
}
