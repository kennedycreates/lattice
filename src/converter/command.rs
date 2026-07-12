use super::{ConversionJob, ConversionTool};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

// ── Command specification ─────────────────────────────────────────────────────

/// A safe, shell-free command specification. No string interpolation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<OsString>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, a: impl Into<OsString>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(iter.into_iter().map(|a| a.into()));
        self
    }
}

// ── Command output ────────────────────────────────────────────────────────────

/// Output from one process execution.
#[derive(Clone, Debug)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
}

impl ProcessOutput {
    pub fn succeeded(&self) -> bool {
        !self.cancelled && self.exit_code == Some(0)
    }

    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

// ── Run result ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunResult {
    Success,
    Failed {
        exit_code: Option<i32>,
        stderr: String,
    },
    Cancelled,
    /// The required tool binary was not found in PATH.
    ToolNotFound(String),
}

impl RunResult {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    pub fn stderr_message(&self) -> Option<&str> {
        if let Self::Failed { stderr, .. } = self {
            Some(stderr.as_str())
        } else {
            None
        }
    }
}

// ── Temp output path ──────────────────────────────────────────────────────────

/// Compute a hidden temporary output path in the same directory as `dest`.
/// The file starts with `.lattice_converting_` so it is visually distinct and
/// can be glob-cleaned on startup if a previous conversion crashed.
pub fn temp_dest(dest: &Path, job_id: u64) -> PathBuf {
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = dest.parent().unwrap_or(Path::new("/"));
    parent.join(format!(".lattice_converting_{job_id}{ext}"))
}

/// Atomic-ish rename of temp → final. Falls back to copy+delete if rename
/// crosses a filesystem boundary.
pub fn finalize_output(temp: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create output folder '{}': {e}", parent.display()))?;
        }
    }
    std::fs::rename(temp, dest)
        .or_else(|_| {
            std::fs::copy(temp, dest)
                .map(|_| ())
                .and_then(|_| std::fs::remove_file(temp))
                .map(|_| ())
        })
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!(
                    "Permission denied writing to '{}'. Check folder permissions.",
                    dest.display()
                )
            } else {
                format!("Could not write output file '{}': {e}", dest.display())
            }
        })
}

/// Remove the temp file if it still exists. Errors are silently ignored.
pub fn cleanup_temp(temp: &Path) {
    let _ = std::fs::remove_file(temp);
}

/// Scan `dir` for `.lattice_converting_*` temp files left by a previous crash
/// and delete them. Non-fatal; all errors are silently ignored.
pub fn cleanup_orphaned_temps_in(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(".lattice_converting_") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ── Command building ──────────────────────────────────────────────────────────

/// Build the command sequence that converts `job.source` to `temp_dest`.
/// For a successful conversion, the caller must rename temp_dest → job.dest.
pub fn build_commands(job: &ConversionJob, temp_dest: &Path) -> Vec<CommandSpec> {
    match job.preset.tool {
        ConversionTool::Ffmpeg => build_ffmpeg(job, temp_dest),
        ConversionTool::ImageMagick => build_imagemagick(job, temp_dest),
        ConversionTool::Vips => build_vips(job, temp_dest),
    }
}

fn build_ffmpeg(job: &ConversionJob, temp_dest: &Path) -> Vec<CommandSpec> {
    let mut cmd = CommandSpec::new("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-progress")
        .arg("pipe:1")
        .arg("-i")
        .arg(&job.source);

    for arg in job.preset.ffmpeg_args {
        cmd = cmd.arg(*arg);
    }
    cmd = cmd.arg(temp_dest);
    vec![cmd]
}

fn build_imagemagick(job: &ConversionJob, temp_dest: &Path) -> Vec<CommandSpec> {
    // IM7: `magick convert <src> <dest>`
    // IM6 fallback: `convert <src> <dest>` — RealRunner handles the ENOENT retry.
    let cmd = CommandSpec::new("magick")
        .arg("convert")
        .arg(&job.source)
        .arg(temp_dest);
    vec![cmd]
}

fn build_vips(job: &ConversionJob, temp_dest: &Path) -> Vec<CommandSpec> {
    let cmd = CommandSpec::new("vips")
        .arg("copy")
        .arg(&job.source)
        .arg(temp_dest);
    vec![cmd]
}

/// Build the ffprobe command to extract format metadata as JSON.
pub fn build_ffprobe(source: &Path) -> CommandSpec {
    CommandSpec::new("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg(source)
}

// ── CommandRunner trait ───────────────────────────────────────────────────────

/// Abstraction over subprocess execution. Implementors must be `Send + Sync`
/// so they can be shared across the blocking thread pool.
///
/// `stdout_sink`: if `Some`, stdout lines are appended there as the process
/// runs. Used for ffmpeg `-progress pipe:1` line parsing.
pub trait CommandRunner: Send + Sync + 'static {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &Arc<AtomicBool>,
        stdout_sink: Option<&Arc<Mutex<Vec<String>>>>,
    ) -> RunResult;
}

// ── RealRunner ────────────────────────────────────────────────────────────────

/// Runs commands as real subprocesses with streaming stdout and cancellation.
pub struct RealRunner;

impl RealRunner {
    fn run_inner(
        &self,
        spec: &CommandSpec,
        cancel: &Arc<AtomicBool>,
        stdout_sink: Option<&Arc<Mutex<Vec<String>>>>,
    ) -> RunResult {
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;

        let stdout_piped = stdout_sink.is_some();

        let mut child = match std::process::Command::new(&spec.program)
            .args(&spec.args)
            .stdout(if stdout_piped {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return RunResult::ToolNotFound(spec.program.clone());
            }
            Err(e) => {
                return RunResult::Failed {
                    exit_code: None,
                    stderr: e.to_string(),
                };
            }
        };

        // Drain stderr in a background thread to prevent pipe deadlock.
        let stderr_raw = child.stderr.take().unwrap();
        let stderr_jh = std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            let mut src = stderr_raw;
            src.read_to_end(&mut buf).ok();
            buf
        });

        // Read stdout line-by-line (for progress parsing) in another thread.
        let stdout_jh = if let Some(sink) = stdout_sink {
            let stdout_raw = child.stdout.take().unwrap();
            let sink = Arc::clone(sink);
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stdout_raw);
                for line in reader.lines().map_while(Result::ok) {
                    sink.lock().unwrap().push(line);
                }
            }))
        } else {
            None
        };

        // Poll for exit, checking the cancel flag every 50 ms.
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(jh) = stdout_jh {
                    jh.join().ok();
                }
                stderr_jh.join().ok();
                return RunResult::Cancelled;
            }
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => {
                    if let Some(jh) = stdout_jh {
                        jh.join().ok();
                    }
                    stderr_jh.join().ok();
                    return RunResult::Failed {
                        exit_code: None,
                        stderr: "process error".into(),
                    };
                }
            }
        };

        if let Some(jh) = stdout_jh {
            jh.join().ok();
        }
        let stderr_bytes = stderr_jh.join().unwrap_or_default();
        let stderr = truncate_str(&String::from_utf8_lossy(&stderr_bytes), 500).to_string();

        if status.success() {
            RunResult::Success
        } else {
            RunResult::Failed {
                exit_code: status.code(),
                stderr,
            }
        }
    }
}

impl CommandRunner for RealRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &Arc<AtomicBool>,
        stdout_sink: Option<&Arc<Mutex<Vec<String>>>>,
    ) -> RunResult {
        let result = self.run_inner(spec, cancel, stdout_sink);

        // ImageMagick IM7 fallback: `magick` → `convert` on ToolNotFound
        if matches!(result, RunResult::ToolNotFound(_)) && spec.program == "magick" {
            let fallback = CommandSpec {
                program: "convert".into(),
                // Drop the leading "convert" arg that IM7 needs but IM6 does not
                args: spec.args.iter().skip(1).cloned().collect(),
            };
            return self.run_inner(&fallback, cancel, stdout_sink);
        }

        result
    }
}

// ── MockRunner ────────────────────────────────────────────────────────────────

/// Test double. Returns a pre-configured sequence of `RunResult`s.
/// Once the sequence is exhausted, falls back to `default_result`.
///
/// Optional `stdout_lines_to_inject`: lines pushed into the sink per call.
#[derive(Clone)]
pub struct MockRunner {
    default_result: RunResult,
    sequence: Arc<Mutex<std::collections::VecDeque<RunResult>>>,
    stdout_inject: Arc<Mutex<std::collections::VecDeque<Vec<String>>>>,
    /// All CommandSpecs that were passed to `run`, in call order.
    pub recorded: Arc<Mutex<Vec<CommandSpec>>>,
}

impl MockRunner {
    /// All jobs succeed.
    pub fn all_succeed() -> Self {
        Self::new(RunResult::Success)
    }

    /// All jobs fail with the given stderr.
    pub fn all_fail(stderr: impl Into<String>) -> Self {
        Self::new(RunResult::Failed {
            exit_code: Some(1),
            stderr: stderr.into(),
        })
    }

    /// Tool not found for every call.
    pub fn tool_missing(name: impl Into<String>) -> Self {
        Self::new(RunResult::ToolNotFound(name.into()))
    }

    fn new(default: RunResult) -> Self {
        Self {
            default_result: default,
            sequence: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            stdout_inject: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Queue a specific result to be returned on the next call (FIFO).
    pub fn then(&self, result: RunResult) -> &Self {
        self.sequence.lock().unwrap().push_back(result);
        self
    }

    /// Queue stdout lines to be injected into the sink on the next call.
    pub fn then_stdout(&self, lines: Vec<String>) -> &Self {
        self.stdout_inject.lock().unwrap().push_back(lines);
        self
    }

    pub fn recorded_calls(&self) -> Vec<CommandSpec> {
        self.recorded.lock().unwrap().clone()
    }

    pub fn call_count(&self) -> usize {
        self.recorded.lock().unwrap().len()
    }
}

impl CommandRunner for MockRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &Arc<AtomicBool>,
        stdout_sink: Option<&Arc<Mutex<Vec<String>>>>,
    ) -> RunResult {
        self.recorded.lock().unwrap().push(spec.clone());

        if cancel.load(Ordering::Relaxed) {
            return RunResult::Cancelled;
        }

        // Inject queued stdout lines
        if let Some(sink) = stdout_sink {
            if let Some(lines) = self.stdout_inject.lock().unwrap().pop_front() {
                sink.lock().unwrap().extend(lines);
            }
        }

        self.sequence
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.default_result.clone())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Truncate a string to at most `max_bytes`, walking back to a valid UTF-8
/// char boundary so the slice never panics on multi-byte characters.
fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{ConversionJobStatus, ConversionPreset, ConversionTool, MediaKind};

    fn make_job(source: &str, preset: &'static ConversionPreset) -> ConversionJob {
        ConversionJob {
            id: 0,
            source: PathBuf::from(source),
            dest: PathBuf::from("/out/photo.jpg"),
            preset,
            status: ConversionJobStatus::Pending,
            attempt: 0,
        }
    }

    fn jpeg_preset() -> &'static ConversionPreset {
        crate::converter::preset_by_id("to_jpeg").unwrap()
    }

    fn mp3_preset() -> &'static ConversionPreset {
        crate::converter::preset_by_id("to_mp3").unwrap()
    }

    fn avif_preset() -> &'static ConversionPreset {
        crate::converter::preset_by_id("to_avif").unwrap()
    }

    fn mp4_preset() -> &'static ConversionPreset {
        crate::converter::preset_by_id("mp4_compatible").unwrap()
    }

    // ── build_commands ────────────────────────────────────────────────────────

    #[test]
    fn ffmpeg_command_has_required_flags() {
        let job = make_job("/photos/photo.png", jpeg_preset());
        let temp = PathBuf::from("/tmp/.lattice_converting_0.jpg");
        let cmds = build_commands(&job, &temp);
        assert_eq!(cmds.len(), 1);
        let cmd = &cmds[0];
        assert_eq!(cmd.program, "ffmpeg");
        // Must have -y and -progress pipe:1
        let args_str: Vec<String> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args_str.contains(&"-y".to_string()));
        assert!(args_str.contains(&"-progress".to_string()));
        assert!(args_str.contains(&"pipe:1".to_string()));
        // Source is after -i
        let i_pos = args_str.iter().position(|a| a == "-i").unwrap();
        assert_eq!(args_str[i_pos + 1], "/photos/photo.png");
        // Last arg is temp dest
        assert_eq!(args_str.last().unwrap(), &temp.to_string_lossy().as_ref());
    }

    #[test]
    fn ffmpeg_audio_command_uses_preset_args() {
        let job = make_job("/music/track.flac", mp3_preset());
        let temp = PathBuf::from("/tmp/.lattice_converting_0.mp3");
        let cmds = build_commands(&job, &temp);
        let args_str: Vec<String> = cmds[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args_str.contains(&"libmp3lame".to_string()));
        assert!(args_str.contains(&"-vn".to_string()));
    }

    #[test]
    fn imagemagick_command_uses_magick_convert() {
        let job = make_job("/photos/photo.png", avif_preset());
        let temp = PathBuf::from("/tmp/.lattice_converting_0.avif");
        let cmds = build_commands(&job, &temp);
        assert_eq!(cmds.len(), 1);
        let cmd = &cmds[0];
        assert_eq!(cmd.program, "magick");
        let args_str: Vec<String> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args_str[0], "convert");
        assert_eq!(args_str[1], "/photos/photo.png");
        assert_eq!(args_str[2], temp.to_string_lossy().as_ref());
    }

    #[test]
    fn vips_command_uses_copy() {
        use crate::converter::ConversionPreset;
        static VIPS_PRESET: ConversionPreset = ConversionPreset {
            id: "vips_test",
            label: "Vips test",
            kind: MediaKind::Image,
            tool: ConversionTool::Vips,
            ext: "jpg",
            ffmpeg_args: &[],
        };
        let job = make_job("/photo.png", &VIPS_PRESET);
        let temp = PathBuf::from("/tmp/out.jpg");
        let cmds = build_commands(&job, &temp);
        assert_eq!(cmds[0].program, "vips");
        let args: Vec<_> = cmds[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "copy");
    }

    #[test]
    fn ffmpeg_video_command_includes_crf() {
        let job = make_job("/clip.mov", mp4_preset());
        let temp = PathBuf::from("/tmp/.lattice_converting_0.mp4");
        let cmds = build_commands(&job, &temp);
        let args: Vec<_> = cmds[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"23".to_string()));
    }

    #[test]
    fn ffprobe_command_structure() {
        let source = Path::new("/video.mp4");
        let spec = build_ffprobe(source);
        assert_eq!(spec.program, "ffprobe");
        let args: Vec<_> = spec
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"-print_format".to_string()));
        assert!(args.contains(&"json".to_string()));
        assert!(args.contains(&"-show_format".to_string()));
        assert_eq!(args.last().unwrap(), "/video.mp4");
    }

    // ── temp_dest ─────────────────────────────────────────────────────────────

    #[test]
    fn temp_dest_is_hidden_and_same_dir() {
        let dest = PathBuf::from("/photos/photo.jpg");
        let tmp = temp_dest(&dest, 42);
        assert!(tmp.file_name().unwrap().to_string_lossy().starts_with('.'));
        assert_eq!(tmp.parent().unwrap(), Path::new("/photos"));
        assert!(tmp.to_string_lossy().contains("42"));
        assert!(tmp.extension().unwrap() == "jpg");
    }

    #[test]
    fn temp_dest_no_extension_preserved() {
        let dest = PathBuf::from("/out/file");
        let tmp = temp_dest(&dest, 1);
        assert!(tmp.extension().is_none());
    }

    // ── MockRunner ────────────────────────────────────────────────────────────

    #[test]
    fn mock_runner_returns_default_result() {
        let runner = MockRunner::all_succeed();
        let cancel = Arc::new(AtomicBool::new(false));
        let spec = CommandSpec::new("ffmpeg").arg("-version");
        let result = runner.run(&spec, &cancel, None);
        assert_eq!(result, RunResult::Success);
        assert_eq!(runner.call_count(), 1);
    }

    #[test]
    fn mock_runner_sequence_consumed_in_order() {
        let runner = MockRunner::all_succeed();
        runner.then(RunResult::Failed {
            exit_code: Some(1),
            stderr: "oops".into(),
        });
        runner.then(RunResult::Success);
        let cancel = Arc::new(AtomicBool::new(false));
        let spec = CommandSpec::new("ffmpeg");
        let r1 = runner.run(&spec, &cancel, None);
        let r2 = runner.run(&spec, &cancel, None);
        let r3 = runner.run(&spec, &cancel, None); // falls back to default
        assert!(matches!(r1, RunResult::Failed { .. }));
        assert!(matches!(r2, RunResult::Success));
        assert!(matches!(r3, RunResult::Success)); // default
    }

    #[test]
    fn mock_runner_respects_cancel_flag() {
        let runner = MockRunner::all_succeed();
        let cancel = Arc::new(AtomicBool::new(true)); // pre-cancelled
        let spec = CommandSpec::new("ffmpeg");
        let result = runner.run(&spec, &cancel, None);
        assert_eq!(result, RunResult::Cancelled);
    }

    #[test]
    fn mock_runner_records_all_calls() {
        let runner = MockRunner::all_succeed();
        let cancel = Arc::new(AtomicBool::new(false));
        for _ in 0..3 {
            runner.run(&CommandSpec::new("ffmpeg"), &cancel, None);
        }
        assert_eq!(runner.call_count(), 3);
    }

    #[test]
    fn mock_runner_injects_stdout_lines() {
        let runner = MockRunner::all_succeed();
        runner.then_stdout(vec![
            "out_time_us=1000000".into(),
            "progress=continue".into(),
        ]);
        let cancel = Arc::new(AtomicBool::new(false));
        let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        runner.run(&CommandSpec::new("ffmpeg"), &cancel, Some(&sink));
        let lines = sink.lock().unwrap().clone();
        assert!(lines.iter().any(|l| l.contains("out_time_us")));
    }

    // ── finalize / cleanup ────────────────────────────────────────────────────

    #[test]
    fn finalize_renames_temp_to_dest() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".lattice_converting_1.jpg");
        let dest = dir.path().join("photo.jpg");
        std::fs::write(&temp, b"result").unwrap();
        finalize_output(&temp, &dest).unwrap();
        assert!(dest.exists());
        assert!(!temp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"result");
    }

    #[test]
    fn cleanup_removes_temp_silently() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".lattice_converting_2.jpg");
        std::fs::write(&temp, b"garbage").unwrap();
        cleanup_temp(&temp);
        assert!(!temp.exists());
        // calling again on non-existent file should not panic
        cleanup_temp(&temp);
    }

    // ── Path safety ───────────────────────────────────────────────────────────

    #[test]
    fn path_with_spaces_passed_as_single_arg() {
        let job = make_job("/my photos/holiday photo 2024.png", jpeg_preset());
        let temp = PathBuf::from("/tmp/.lattice_converting_0.jpg");
        let cmds = build_commands(&job, &temp);
        let args: Vec<String> = cmds[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // Source must appear as a single argument, not split on spaces
        assert!(
            args.iter()
                .any(|a| a == "/my photos/holiday photo 2024.png"),
            "path with spaces should be a single arg, got: {args:?}"
        );
        // Temp dest likewise
        assert!(
            args.last().map(|s| s.as_str()) == Some(temp.to_string_lossy().as_ref()),
            "last arg should be temp dest"
        );
    }

    #[test]
    fn path_with_unicode_passed_correctly() {
        let job = make_job("/photos/日本語ファイル名.png", jpeg_preset());
        let temp = PathBuf::from("/tmp/.lattice_converting_0.jpg");
        let cmds = build_commands(&job, &temp);
        let args: Vec<String> = cmds[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.iter().any(|a| a.contains("日本語")),
            "unicode path not preserved, got: {args:?}"
        );
    }

    #[test]
    fn path_with_special_chars_single_arg() {
        // Apostrophe, parentheses, exclamation — none should be interpreted by shell
        let job = make_job("/home/user/my's file (final)!.png", jpeg_preset());
        let temp = PathBuf::from("/tmp/.lattice_converting_0.jpg");
        let cmds = build_commands(&job, &temp);
        let args: Vec<String> = cmds[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let source_arg = "/home/user/my's file (final)!.png";
        assert!(
            args.iter().any(|a| a == source_arg),
            "special-char path should be a single arg, got: {args:?}"
        );
    }

    #[test]
    fn temp_dest_with_unicode_dest() {
        let dest = PathBuf::from("/photos/日本語写真.jpg");
        let tmp = temp_dest(&dest, 7);
        assert!(tmp.file_name().unwrap().to_string_lossy().starts_with('.'));
        assert_eq!(tmp.parent().unwrap(), Path::new("/photos"));
        assert!(tmp.extension().unwrap() == "jpg");
    }

    // ── truncate_str char boundary ────────────────────────────────────────────

    #[test]
    fn truncate_str_safe_on_ascii() {
        assert_eq!(truncate_str("hello", 3), "hel");
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello", 0), "");
    }

    #[test]
    fn truncate_str_does_not_panic_on_multibyte_boundary() {
        // "日" is 3 bytes (U+65E5). Truncating at byte 4 would split it.
        let s = "日本語"; // 9 bytes
        let result = truncate_str(s, 4);
        // Must be valid UTF-8 and ≤ 4 bytes
        assert!(result.is_empty() || result == "日"); // Either "" or "日" (3 bytes)
        let _ = result.to_string(); // Must not panic
    }

    #[test]
    fn truncate_str_exact_boundary() {
        let s = "日本語"; // exactly 9 bytes
        assert_eq!(truncate_str(s, 9), "日本語");
        assert_eq!(truncate_str(s, 3), "日");
        assert_eq!(truncate_str(s, 6), "日本");
    }

    // ── finalize_output edge cases ────────────────────────────────────────────

    #[test]
    fn finalize_output_creates_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".lattice_converting_9.jpg");
        let dest = dir.path().join("subfolder/output.jpg");
        std::fs::write(&temp, b"result").unwrap();
        finalize_output(&temp, &dest).unwrap();
        assert!(dest.exists());
        assert!(!temp.exists());
    }

    #[test]
    fn finalize_output_returns_error_on_nonexistent_temp() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".lattice_converting_nonexistent.jpg");
        let dest = dir.path().join("output.jpg");
        // temp doesn't exist — rename and copy+delete both fail
        let result = finalize_output(&temp, &dest);
        assert!(result.is_err(), "should fail when temp file is missing");
    }

    // ── cleanup_orphaned_temps_in ─────────────────────────────────────────────

    #[test]
    fn cleanup_orphaned_removes_lattice_temps() {
        let dir = tempfile::tempdir().unwrap();
        let temp1 = dir.path().join(".lattice_converting_1.jpg");
        let temp2 = dir.path().join(".lattice_converting_2.mp4");
        let keeper = dir.path().join("real_file.jpg");
        std::fs::write(&temp1, b"").unwrap();
        std::fs::write(&temp2, b"").unwrap();
        std::fs::write(&keeper, b"keep").unwrap();

        cleanup_orphaned_temps_in(dir.path());

        assert!(!temp1.exists(), "temp1 should have been removed");
        assert!(!temp2.exists(), "temp2 should have been removed");
        assert!(keeper.exists(), "real file should NOT have been removed");
    }

    #[test]
    fn cleanup_orphaned_is_safe_on_nonexistent_dir() {
        // Must not panic when given a directory that doesn't exist
        cleanup_orphaned_temps_in(Path::new("/nonexistent/dir/that/does/not/exist"));
    }
}
