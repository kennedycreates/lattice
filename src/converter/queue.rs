use super::{
    command::{
        build_commands, cleanup_temp, finalize_output, temp_dest, CommandRunner, RealRunner,
        RunResult,
    },
    detect_tools, format_job_error, progress, ConversionJob, ConversionJobId, ConversionJobStatus,
    MediaKind, ToolAvailability,
};
use crate::ui::ops_panel::OpId;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

#[cfg(test)]
use super::command::MockRunner;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

// ── Concurrency limits ────────────────────────────────────────────────────────

const IMAGE_CONCURRENCY: u32 = 4;
const AV_CONCURRENCY: u32 = 1;

// ── BatchProgress ─────────────────────────────────────────────────────────────

/// Snapshot of the queue's work-item counts. Cheap to clone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchProgress {
    pub total: u32,
    pub queued: u32,
    pub running: u32,
    pub completed: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub skipped: u32,
}

impl BatchProgress {
    /// Fraction of terminal-state jobs over total (0.0–1.0).
    pub fn fraction_done(&self) -> f64 {
        let done = self.completed + self.failed + self.cancelled + self.skipped;
        if self.total == 0 {
            1.0
        } else {
            done as f64 / self.total as f64
        }
    }

    pub fn is_finished(&self) -> bool {
        self.queued == 0 && self.running == 0
    }

    fn tick_running(&mut self) {
        self.queued = self.queued.saturating_sub(1);
        self.running += 1;
    }

    fn tick_completed(&mut self) {
        self.running = self.running.saturating_sub(1);
        self.completed += 1;
    }

    fn tick_failed(&mut self) {
        self.running = self.running.saturating_sub(1);
        self.failed += 1;
    }

    fn tick_cancelled(&mut self) {
        self.running = self.running.saturating_sub(1);
        self.cancelled += 1;
    }
}

// ── Queue state ───────────────────────────────────────────────────────────────

struct QueueState {
    pending: VecDeque<ConversionJob>,
    image_active: u32,
    av_active: u32,
    cancel_flag: Arc<AtomicBool>,
    ops_id: Option<OpId>,
    progress: BatchProgress,
    errors: Vec<String>,
}

impl QueueState {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            image_active: 0,
            av_active: 0,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            ops_id: None,
            progress: BatchProgress::default(),
            errors: Vec::new(),
        }
    }

    fn has_slot_for(&self, kind: MediaKind) -> bool {
        match kind {
            MediaKind::Image => self.image_active < IMAGE_CONCURRENCY,
            _ => self.av_active < AV_CONCURRENCY,
        }
    }

    fn claim_slot(&mut self, kind: MediaKind) {
        match kind {
            MediaKind::Image => self.image_active += 1,
            _ => self.av_active += 1,
        }
    }

    fn release_slot(&mut self, kind: MediaKind) {
        match kind {
            MediaKind::Image => self.image_active = self.image_active.saturating_sub(1),
            _ => self.av_active = self.av_active.saturating_sub(1),
        }
    }

    fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.image_active == 0 && self.av_active == 0
    }
}

// ── Callback type aliases ─────────────────────────────────────────────────────

type ProgressCb = Option<Box<dyn Fn(OpId, f64, &str)>>;
type DoneCb = Option<Box<dyn Fn(OpId, Vec<String>)>>;
type BatchProgressCb = Option<Box<dyn Fn(BatchProgress)>>;
type JobStatusCb = Option<Box<dyn Fn(ConversionJobId, ConversionJobStatus)>>;
type JobProgressCb = Option<Box<dyn Fn(ConversionJobId, f64)>>;

// ── ConversionQueue ───────────────────────────────────────────────────────────

/// Background conversion queue.
///
/// Use `ConversionQueue::new()` in production. Use `ConversionQueue::with_runner`
/// to inject a `MockRunner` for tests.
#[derive(Clone)]
pub struct ConversionQueue {
    pub tools: ToolAvailability,
    runner: Arc<dyn CommandRunner>,
    state: Rc<RefCell<QueueState>>,
    on_progress: Rc<RefCell<ProgressCb>>,
    on_done: Rc<RefCell<DoneCb>>,
    on_batch_progress: Rc<RefCell<BatchProgressCb>>,
    on_job_status: Rc<RefCell<JobStatusCb>>,
    on_job_progress: Rc<RefCell<JobProgressCb>>,
}

impl ConversionQueue {
    /// Create a queue using real subprocess execution.
    pub fn new() -> Self {
        let tools = detect_tools();
        let runner: Arc<dyn CommandRunner> = Arc::new(RealRunner);
        Self::with_runner(tools, runner)
    }

    /// Create a queue with an injected runner (for testing).
    pub fn with_runner(tools: ToolAvailability, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            tools,
            runner,
            state: Rc::new(RefCell::new(QueueState::new())),
            on_progress: Rc::new(RefCell::new(None)),
            on_done: Rc::new(RefCell::new(None)),
            on_batch_progress: Rc::new(RefCell::new(None)),
            on_job_status: Rc::new(RefCell::new(None)),
            on_job_progress: Rc::new(RefCell::new(None)),
        }
    }

    // ── Callbacks ─────────────────────────────────────────────────────────────

    /// Called with `(op_id, fraction 0–1, current filename)` after each job
    /// completes. Also called periodically for video/audio jobs while running.
    pub fn connect_progress(&self, f: impl Fn(OpId, f64, &str) + 'static) {
        *self.on_progress.borrow_mut() = Some(Box::new(f));
    }

    /// Called with `(op_id, error_strings)` when the entire batch finishes or
    /// is cancelled and no more jobs are in flight.
    pub fn connect_done(&self, f: impl Fn(OpId, Vec<String>) + 'static) {
        *self.on_done.borrow_mut() = Some(Box::new(f));
    }

    /// Called with the updated `BatchProgress` after every job state change.
    pub fn connect_batch_progress(&self, f: impl Fn(BatchProgress) + 'static) {
        *self.on_batch_progress.borrow_mut() = Some(Box::new(f));
    }

    /// Called whenever a job transitions to a new status.
    pub fn connect_job_status(&self, f: impl Fn(ConversionJobId, ConversionJobStatus) + 'static) {
        *self.on_job_status.borrow_mut() = Some(Box::new(f));
    }

    /// Called periodically during video/audio encoding with `(job_id, fraction 0–1)`.
    pub fn connect_job_progress(&self, f: impl Fn(ConversionJobId, f64) + 'static) {
        *self.on_job_progress.borrow_mut() = Some(Box::new(f));
    }

    // ── Control ───────────────────────────────────────────────────────────────

    /// Enqueue pre-built active jobs and begin processing.
    /// `ops_id` is the OpsPanel operation tracking this batch.
    pub fn enqueue_jobs(&self, jobs: Vec<ConversionJob>, ops_id: OpId) {
        {
            let mut state = self.state.borrow_mut();
            let n = jobs.len() as u32;
            state.cancel_flag = Arc::new(AtomicBool::new(false));
            state.ops_id = Some(ops_id);
            state.errors.clear();
            state.progress = BatchProgress {
                total: n,
                queued: n,
                ..BatchProgress::default()
            };
            for job in jobs {
                state.pending.push_back(job);
            }
        }
        Self::pump(
            Rc::clone(&self.state),
            Arc::clone(&self.runner),
            Rc::clone(&self.on_progress),
            Rc::clone(&self.on_done),
            Rc::clone(&self.on_batch_progress),
            Rc::clone(&self.on_job_status),
            Rc::clone(&self.on_job_progress),
        );
    }

    /// Cancel the current batch. In-flight jobs receive a kill signal; they will
    /// finish their current blocking call before reporting cancellation.
    pub fn cancel(&self) {
        let mut state = self.state.borrow_mut();
        state.cancel_flag.store(true, Ordering::Relaxed);
        let drained = state.pending.drain(..).count() as u32;
        state.progress.queued = state.progress.queued.saturating_sub(drained);
        state.progress.cancelled += drained;
    }

    /// Re-queue a failed job (increments attempt counter, resets status).
    pub fn retry_job(&self, mut job: ConversionJob) {
        job.attempt += 1;
        job.status = ConversionJobStatus::Pending;
        {
            let mut state = self.state.borrow_mut();
            if state.cancel_flag.load(Ordering::Relaxed) {
                return;
            }
            // Walk back the failed count, add to queued
            state.progress.failed = state.progress.failed.saturating_sub(1);
            state.progress.total += 1;
            state.progress.queued += 1;
            state.pending.push_back(job);
        }
        Self::pump(
            Rc::clone(&self.state),
            Arc::clone(&self.runner),
            Rc::clone(&self.on_progress),
            Rc::clone(&self.on_done),
            Rc::clone(&self.on_batch_progress),
            Rc::clone(&self.on_job_status),
            Rc::clone(&self.on_job_progress),
        );
    }

    pub fn is_idle(&self) -> bool {
        self.state.borrow().is_idle()
    }

    pub fn batch_progress(&self) -> BatchProgress {
        self.state.borrow().progress.clone()
    }

    // ── Pump ──────────────────────────────────────────────────────────────────

    fn pump(
        state_rc: Rc<RefCell<QueueState>>,
        runner: Arc<dyn CommandRunner>,
        on_progress: Rc<RefCell<ProgressCb>>,
        on_done: Rc<RefCell<DoneCb>>,
        on_batch_progress: Rc<RefCell<BatchProgressCb>>,
        on_job_status: Rc<RefCell<JobStatusCb>>,
        on_job_progress: Rc<RefCell<JobProgressCb>>,
    ) {
        loop {
            let (job, cancel_flag) = {
                let mut state = state_rc.borrow_mut();

                // If cancelled, drain any remaining pending jobs
                if state.cancel_flag.load(Ordering::Relaxed) {
                    if state.is_idle() {
                        if let Some(ops_id) = state.ops_id.take() {
                            let errors = std::mem::take(&mut state.errors);
                            drop(state);
                            if let Some(cb) = on_done.borrow().as_ref() {
                                cb(ops_id, errors);
                            }
                        }
                    }
                    return;
                }

                // Find first job with an available slot
                let pos = state
                    .pending
                    .iter()
                    .position(|j| state.has_slot_for(j.preset.kind));

                match pos {
                    None => return,
                    Some(i) => {
                        let mut job = state.pending.remove(i).unwrap();
                        job.status = ConversionJobStatus::Running;
                        state.claim_slot(job.preset.kind);
                        state.progress.tick_running();
                        let cancel = Arc::clone(&state.cancel_flag);
                        (job, cancel)
                    }
                }
            };

            // Notify status change
            if let Some(cb) = on_job_status.borrow().as_ref() {
                cb(job.id, ConversionJobStatus::Running);
            }
            if let Some(cb) = on_batch_progress.borrow().as_ref() {
                cb(state_rc.borrow().progress.clone());
            }

            // Clone everything needed for the async block
            let job_id = job.id;
            let job_kind = job.preset.kind;
            let temp = temp_dest(&job.dest, job.id);
            let dest = job.dest.clone();
            let source_name = job
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".to_string());

            let state_for_cb = Rc::clone(&state_rc);
            let runner_for_spawn = Arc::clone(&runner);
            let on_progress_cb = Rc::clone(&on_progress);
            let on_done_cb = Rc::clone(&on_done);
            let on_batch_for_cb = Rc::clone(&on_batch_progress);
            let on_job_for_cb = Rc::clone(&on_job_status);
            let on_job_progress_cb = Rc::clone(&on_job_progress);

            // Stdout sink shared between blocking thread and main-thread timer
            let stdout_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            // Probe total duration for video/audio (fast ffprobe call)
            let runner_for_probe = Arc::clone(&runner);
            let source_for_probe = job.source.clone();
            let cancel_for_probe = Arc::clone(&cancel_flag);

            glib::MainContext::default().spawn_local(async move {
                // ── ffprobe duration (for progress fraction) ──────────────────
                let total_us: Option<u64> =
                    if matches!(job_kind, MediaKind::Audio | MediaKind::Video) {
                        let source_p = source_for_probe.clone();
                        let runner_p = Arc::clone(&runner_for_probe);
                        let cancel_p = Arc::clone(&cancel_for_probe);
                        gio::spawn_blocking(move || {
                            progress::probe_duration_us(&source_p, runner_p.as_ref(), &cancel_p)
                        })
                        .await
                        .ok()
                        .flatten()
                    } else {
                        None
                    };

                // ── Progress timer (video/audio only, main thread) ────────────
                let timer_handle: Option<glib::SourceId> = if total_us.is_some()
                    || matches!(job_kind, MediaKind::Audio | MediaKind::Video)
                {
                    let stdout_for_timer = Arc::clone(&stdout_lines);
                    let total_us_for_timer = total_us;
                    let on_progress_timer = Rc::clone(&on_progress_cb);
                    let on_job_progress_timer = Rc::clone(&on_job_progress_cb);
                    let ops_id_for_timer = state_for_cb.borrow().ops_id;
                    let name_for_timer = source_name.clone();
                    let job_id_for_timer = job_id;

                    Some(glib::timeout_add_local(
                        Duration::from_millis(200),
                        move || {
                            let lines = stdout_for_timer.lock().unwrap().clone();
                            let fraction = total_us_for_timer
                                .and_then(|t| progress::parse_latest_fraction(&lines, t))
                                .map(|f| f as f64)
                                .unwrap_or(0.0); // 0 = indeterminate fallback

                            if let (Some(ops_id), Some(cb)) =
                                (ops_id_for_timer, on_progress_timer.borrow().as_ref())
                            {
                                cb(ops_id, fraction, &name_for_timer);
                            }
                            if let Some(cb) = on_job_progress_timer.borrow().as_ref() {
                                cb(job_id_for_timer, fraction);
                            }
                            glib::ControlFlow::Continue
                        },
                    ))
                } else {
                    None
                };

                // ── Run conversion on blocking thread ─────────────────────────
                let temp_for_blocking = temp.clone();
                let stdout_for_blocking = Arc::clone(&stdout_lines);
                let cancel_for_blocking = Arc::clone(&cancel_flag);

                let (returned_job, run_result) = gio::spawn_blocking(move || {
                    let cmds = build_commands(&job, &temp_for_blocking);
                    for spec in &cmds {
                        let result = runner_for_spawn.run(
                            spec,
                            &cancel_for_blocking,
                            Some(&stdout_for_blocking),
                        );
                        if !result.is_success() {
                            return (job, result);
                        }
                    }
                    (job, RunResult::Success)
                })
                .await
                .unwrap();

                // ── Stop timer ────────────────────────────────────────────────
                if let Some(id) = timer_handle {
                    id.remove();
                }

                // ── Finalize or clean up ──────────────────────────────────────
                let new_status = match &run_result {
                    RunResult::Success => match finalize_output(&temp, &dest) {
                        Ok(()) => ConversionJobStatus::Done,
                        Err(e) => {
                            cleanup_temp(&temp);
                            ConversionJobStatus::Failed(e)
                        }
                    },
                    RunResult::Failed { stderr, .. } => {
                        cleanup_temp(&temp);
                        ConversionJobStatus::Failed(format_job_error(stderr))
                    }
                    RunResult::Cancelled => {
                        cleanup_temp(&temp);
                        ConversionJobStatus::Cancelled
                    }
                    RunResult::ToolNotFound(name) => {
                        cleanup_temp(&temp);
                        // Look up install hint for the tool that was expected
                        let hint = returned_job.preset.tool.install_hint();
                        ConversionJobStatus::Failed(format!(
                            "{name} was not found on this system.\n{hint}"
                        ))
                    }
                };

                // ── Update queue state ────────────────────────────────────────
                let (ops_id_opt, errors_opt) = {
                    let mut state = state_for_cb.borrow_mut();
                    state.release_slot(returned_job.preset.kind);
                    match &new_status {
                        ConversionJobStatus::Done => state.progress.tick_completed(),
                        ConversionJobStatus::Cancelled => state.progress.tick_cancelled(),
                        ConversionJobStatus::Failed(msg) => {
                            state.progress.tick_failed();
                            state.errors.push(format!("{source_name}: {msg}"));
                        }
                        _ => {}
                    }

                    // Emit per-job progress update
                    if let Some(ops_id) = state.ops_id {
                        let fraction = state.progress.fraction_done();
                        if let Some(cb) = on_progress_cb.borrow().as_ref() {
                            cb(ops_id, fraction, &source_name);
                        }
                    }

                    let idle = state.is_idle();
                    (
                        if idle { state.ops_id.take() } else { None },
                        if idle {
                            Some(std::mem::take(&mut state.errors))
                        } else {
                            None
                        },
                    )
                };

                // Notify job status
                if let Some(cb) = on_job_for_cb.borrow().as_ref() {
                    cb(job_id, new_status);
                }

                // Notify batch progress
                if let Some(cb) = on_batch_for_cb.borrow().as_ref() {
                    cb(state_for_cb.borrow().progress.clone());
                }

                // Notify done
                if let (Some(ops_id), Some(errors)) = (ops_id_opt, errors_opt) {
                    if let Some(cb) = on_done_cb.borrow().as_ref() {
                        cb(ops_id, errors);
                    }
                } else {
                    // Continue pumping
                    Self::pump(
                        state_for_cb,
                        runner_for_probe,
                        on_progress_cb,
                        on_done_cb,
                        on_batch_for_cb,
                        on_job_for_cb,
                        on_job_progress_cb,
                    );
                }
            });
        }
    }
}

/// Truncate to at most `max_bytes`, walking back to a valid UTF-8 boundary.
fn truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::ConversionJobStatus;

    // ── BatchProgress ─────────────────────────────────────────────────────────

    #[test]
    fn batch_progress_fraction_zero_total() {
        let p = BatchProgress::default();
        assert_eq!(p.fraction_done(), 1.0);
        assert!(p.is_finished());
    }

    #[test]
    fn batch_progress_fraction_partial() {
        let p = BatchProgress {
            total: 4,
            queued: 2,
            running: 1,
            completed: 1,
            ..Default::default()
        };
        assert_eq!(p.fraction_done(), 0.25);
        assert!(!p.is_finished());
    }

    #[test]
    fn batch_progress_fraction_all_done() {
        let p = BatchProgress {
            total: 3,
            completed: 2,
            failed: 1,
            ..Default::default()
        };
        assert!(p.is_finished());
        assert_eq!(p.fraction_done(), 1.0);
    }

    #[test]
    fn batch_progress_is_finished_only_when_no_active() {
        let p = BatchProgress {
            total: 2,
            running: 1,
            completed: 1,
            ..Default::default()
        };
        assert!(!p.is_finished());
    }

    #[test]
    fn batch_progress_tick_running_decrements_queued() {
        let mut p = BatchProgress {
            total: 3,
            queued: 3,
            ..Default::default()
        };
        p.tick_running();
        assert_eq!(p.queued, 2);
        assert_eq!(p.running, 1);
    }

    // ── temp_dest ─────────────────────────────────────────────────────────────

    #[test]
    fn temp_dest_unique_per_job_id() {
        use std::path::PathBuf;
        let dest = PathBuf::from("/out/photo.jpg");
        let t1 = temp_dest(&dest, 1);
        let t2 = temp_dest(&dest, 2);
        assert_ne!(t1, t2);
    }

    // ── Queue state (synchronous, no GLib) ───────────────────────────────────

    fn ffmpeg_tools() -> ToolAvailability {
        ToolAvailability {
            ffmpeg: true,
            ffprobe: false,
            imagemagick: false,
            vips: false,
        }
    }

    fn pending_job(id: u64, dir: &std::path::Path) -> ConversionJob {
        let src = dir.join(format!("photo{id}.png"));
        std::fs::write(&src, b"").unwrap();
        ConversionJob {
            id,
            source: src.clone(),
            dest: dir.join(format!("photo{id}.jpg")),
            preset: crate::converter::preset_by_id("to_jpeg").unwrap(),
            status: ConversionJobStatus::Pending,
            attempt: 0,
        }
    }

    // Populate queue state directly, bypassing pump (no GLib needed).
    fn seed_state(queue: &ConversionQueue, jobs: Vec<ConversionJob>) {
        let mut state = queue.state.borrow_mut();
        let n = jobs.len() as u32;
        state.cancel_flag = Arc::new(AtomicBool::new(false));
        state.ops_id = Some(0);
        state.errors.clear();
        state.progress = BatchProgress {
            total: n,
            queued: n,
            ..BatchProgress::default()
        };
        for job in jobs {
            state.pending.push_back(job);
        }
    }

    #[test]
    fn initial_progress_fields_are_set_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::all_succeed());
        let queue = ConversionQueue::with_runner(ffmpeg_tools(), runner);
        seed_state(
            &queue,
            vec![pending_job(0, dir.path()), pending_job(1, dir.path())],
        );
        let p = queue.batch_progress();
        assert_eq!(p.total, 2);
        assert_eq!(p.queued, 2);
        assert_eq!(p.completed, 0);
        assert!(!p.is_finished());
    }

    #[test]
    fn cancel_clears_pending_and_sets_flag() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::all_succeed());
        let queue = ConversionQueue::with_runner(ffmpeg_tools(), runner);
        seed_state(
            &queue,
            vec![
                pending_job(0, dir.path()),
                pending_job(1, dir.path()),
                pending_job(2, dir.path()),
            ],
        );
        queue.cancel();
        let state = queue.state.borrow();
        assert!(state.cancel_flag.load(Ordering::Relaxed));
        assert!(state.pending.is_empty(), "cancel must drain pending");
        assert_eq!(state.progress.cancelled, 3);
    }

    #[test]
    fn cancel_prevents_retry() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner::all_succeed());
        let queue = ConversionQueue::with_runner(ffmpeg_tools(), runner);
        // Set the cancel flag without going through pump
        queue
            .state
            .borrow_mut()
            .cancel_flag
            .store(true, Ordering::Relaxed);
        let mut job = pending_job(0, dir.path());
        job.status = ConversionJobStatus::Failed("err".into());
        queue.retry_job(job); // early-returns because cancel_flag is set
        assert!(
            queue.state.borrow().pending.is_empty(),
            "retry must be ignored when cancelled"
        );
    }

    #[test]
    fn queue_state_slot_accounting() {
        let mut state = QueueState::new();
        assert!(state.has_slot_for(MediaKind::Image));
        assert!(state.has_slot_for(MediaKind::Video));
        for _ in 0..IMAGE_CONCURRENCY {
            state.claim_slot(MediaKind::Image);
        }
        assert!(!state.has_slot_for(MediaKind::Image));
        assert!(state.has_slot_for(MediaKind::Video));
        state.claim_slot(MediaKind::Video);
        assert!(!state.has_slot_for(MediaKind::Video));
        state.release_slot(MediaKind::Image);
        assert!(state.has_slot_for(MediaKind::Image));
    }

    #[test]
    fn queue_state_is_idle_without_jobs() {
        let state = QueueState::new();
        assert!(state.is_idle());
    }

    // ── Queue integration (requires GLib main context) ────────────────────────
    // Run with: `cargo test -- --include-ignored converter::queue`
    // Requires a thread with a running GLib default main context.

    fn make_test_jobs(dir: &std::path::Path, count: usize) -> Vec<ConversionJob> {
        let preset = crate::converter::preset_by_id("to_jpeg").unwrap();
        (0..count)
            .map(|i| {
                let src = dir.join(format!("photo{i}.png"));
                std::fs::write(&src, b"").unwrap();
                ConversionJob {
                    id: i as u64,
                    source: src.clone(),
                    dest: dir.join(format!("photo{i}.jpg")),
                    preset,
                    status: ConversionJobStatus::Pending,
                    attempt: 0,
                }
            })
            .collect()
    }

    #[test]
    #[ignore = "requires a live GLib main loop; run with --include-ignored on a desktop session"]
    fn queue_runs_all_jobs_to_completion() {
        let ctx = glib::MainContext::new();
        let _guard = ctx.acquire();
        ctx.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let runner = Arc::new(MockRunner::all_succeed());
            let tools = ToolAvailability {
                ffmpeg: true,
                ffprobe: true,
                imagemagick: true,
                vips: true,
            };
            let queue = ConversionQueue::with_runner(tools, runner.clone());

            let done_flag = Rc::new(RefCell::new(false));
            let done_flag_cb = Rc::clone(&done_flag);
            let errors_out: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let errors_cb = Rc::clone(&errors_out);

            queue.connect_done(move |_op_id, errors| {
                *errors_cb.borrow_mut() = errors;
                *done_flag_cb.borrow_mut() = true;
            });

            let jobs = make_test_jobs(dir.path(), 3);
            queue.enqueue_jobs(jobs, 0);

            // Spin the main context until done or timeout
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !*done_flag.borrow() && std::time::Instant::now() < deadline {
                ctx.iteration(false);
            }

            assert!(*done_flag.borrow(), "queue did not complete in time");
            assert_eq!(errors_out.borrow().len(), 0);
            assert_eq!(runner.call_count(), 3);
        });
    }

    #[test]
    #[ignore = "requires a live GLib main loop; run with --include-ignored on a desktop session"]
    fn one_failure_does_not_stop_batch() {
        let ctx = glib::MainContext::new();
        let _guard = ctx.acquire();
        ctx.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let runner = Arc::new(MockRunner::all_succeed());
            // First job will fail, remaining succeed
            runner.then(RunResult::Failed {
                exit_code: Some(1),
                stderr: "codec error".into(),
            });
            let tools = ToolAvailability {
                ffmpeg: true,
                ffprobe: false,
                imagemagick: false,
                vips: false,
            };
            let queue = ConversionQueue::with_runner(tools, runner.clone());

            let done_flag = Rc::new(RefCell::new(false));
            let done_for_cb = Rc::clone(&done_flag);
            let errors_out: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let errors_cb = Rc::clone(&errors_out);

            queue.connect_done(move |_, errors| {
                *errors_cb.borrow_mut() = errors;
                *done_for_cb.borrow_mut() = true;
            });

            let jobs = make_test_jobs(dir.path(), 3);
            queue.enqueue_jobs(jobs, 0);

            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !*done_flag.borrow() && std::time::Instant::now() < deadline {
                ctx.iteration(false);
            }

            assert!(*done_flag.borrow(), "did not complete");
            // All 3 jobs ran (1 failed, 2 succeeded)
            assert_eq!(runner.call_count(), 3);
            assert_eq!(errors_out.borrow().len(), 1);
        });
    }

    #[test]
    #[ignore = "requires a live GLib main loop; run with --include-ignored on a desktop session"]
    fn cancellation_stops_pending_jobs() {
        let ctx = glib::MainContext::new();
        let _guard = ctx.acquire();
        ctx.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            // Use a slow mock that checks cancel before returning
            let runner = Arc::new(MockRunner::all_succeed());
            let tools = ToolAvailability {
                ffmpeg: true,
                ffprobe: false,
                imagemagick: false,
                vips: false,
            };
            let queue = ConversionQueue::with_runner(tools, runner.clone());

            let done_flag = Rc::new(RefCell::new(false));
            let done_for_cb = Rc::clone(&done_flag);
            queue.connect_done(move |_, _| {
                *done_for_cb.borrow_mut() = true;
            });

            let jobs = make_test_jobs(dir.path(), 5);
            queue.enqueue_jobs(jobs, 0);
            // Cancel immediately (before any job can be pumped off the loop)
            queue.cancel();

            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !*done_flag.borrow() && std::time::Instant::now() < deadline {
                ctx.iteration(false);
            }

            // After cancel, the batch is considered done
            let progress = queue.batch_progress();
            assert!(progress.is_finished(), "batch not finished after cancel");
        });
    }

    #[test]
    #[ignore = "requires a live GLib main loop; run with --include-ignored on a desktop session"]
    fn retry_failed_job_runs_again() {
        let ctx = glib::MainContext::new();
        let _guard = ctx.acquire();
        ctx.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let runner = Arc::new(MockRunner::all_succeed());
            // First call fails, retry succeeds
            runner.then(RunResult::Failed {
                exit_code: Some(1),
                stderr: "transient".into(),
            });

            let tools = ToolAvailability {
                ffmpeg: true,
                ffprobe: false,
                imagemagick: false,
                vips: false,
            };
            let queue = ConversionQueue::with_runner(tools, runner.clone());

            let done_count = Rc::new(RefCell::new(0u32));
            let retry_done = Rc::new(RefCell::new(false));

            let q_for_retry = queue.clone();
            let done_count_cb = Rc::clone(&done_count);
            let retry_done_cb = Rc::clone(&retry_done);

            // Capture the failed job in on_job_status, retry it once
            let failed_job: Rc<RefCell<Option<ConversionJob>>> = Rc::new(RefCell::new(None));
            let failed_job_cb = Rc::clone(&failed_job);
            let preset = crate::converter::preset_by_id("to_jpeg").unwrap();
            let src = dir.path().join("photo0.png");
            std::fs::write(&src, b"").unwrap();
            let dest = dir.path().join("photo0.jpg");

            queue.connect_job_status({
                let failed_job_cb = Rc::clone(&failed_job_cb);
                let src = src.clone();
                let dest = dest.clone();
                move |id, status| {
                    if matches!(status, ConversionJobStatus::Failed(_))
                        && failed_job_cb.borrow().is_none()
                    {
                        *failed_job_cb.borrow_mut() = Some(ConversionJob {
                            id,
                            source: src.clone(),
                            dest: dest.clone(),
                            preset,
                            status: ConversionJobStatus::Pending,
                            attempt: 0,
                        });
                    }
                }
            });

            queue.connect_done(move |_, _| {
                let mut c = done_count_cb.borrow_mut();
                *c += 1;
                if *c >= 2 {
                    *retry_done_cb.borrow_mut() = true;
                }
            });

            let jobs = vec![ConversionJob {
                id: 0,
                source: src,
                dest,
                preset,
                status: ConversionJobStatus::Pending,
                attempt: 0,
            }];
            queue.enqueue_jobs(jobs, 0);

            // Wait for first batch to finish
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while done_count.borrow().eq(&0) && std::time::Instant::now() < deadline {
                ctx.iteration(false);
            }

            // Retry the failed job
            if let Some(job) = failed_job.borrow().clone() {
                q_for_retry.retry_job(job);
            }

            let deadline2 = std::time::Instant::now() + Duration::from_secs(3);
            while !*retry_done.borrow() && std::time::Instant::now() < deadline2 {
                ctx.iteration(false);
            }

            assert_eq!(
                runner.call_count(),
                2,
                "job should have been attempted twice"
            );
        });
    }

    #[test]
    #[ignore = "requires a live GLib main loop; run with --include-ignored on a desktop session"]
    fn batch_progress_callback_fires_on_completion() {
        let ctx = glib::MainContext::new();
        let _guard = ctx.acquire();
        ctx.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let runner = Arc::new(MockRunner::all_succeed());
            let tools = ToolAvailability {
                ffmpeg: true,
                ffprobe: false,
                imagemagick: false,
                vips: false,
            };
            let queue = ConversionQueue::with_runner(tools, runner);

            let snapshots: Rc<RefCell<Vec<BatchProgress>>> = Rc::new(RefCell::new(Vec::new()));
            let snap_cb = Rc::clone(&snapshots);
            queue.connect_batch_progress(move |p| {
                snap_cb.borrow_mut().push(p);
            });

            let done = Rc::new(RefCell::new(false));
            let done_cb = Rc::clone(&done);
            queue.connect_done(move |_, _| {
                *done_cb.borrow_mut() = true;
            });

            queue.enqueue_jobs(make_test_jobs(dir.path(), 2), 0);

            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !*done.borrow() && std::time::Instant::now() < deadline {
                ctx.iteration(false);
            }

            let snaps = snapshots.borrow();
            assert!(!snaps.is_empty(), "batch progress callback never fired");
            // Last snapshot should be fully finished
            let last = snaps.last().unwrap();
            assert!(last.is_finished());
        });
    }
}
