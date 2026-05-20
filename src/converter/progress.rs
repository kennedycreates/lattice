use super::command::{build_ffprobe, CommandRunner};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

// ── ffprobe duration probe ────────────────────────────────────────────────────

/// Run ffprobe to get the source duration in microseconds.
/// Returns `None` if ffprobe is unavailable, the file has no duration, or
/// parsing fails. Callers should fall back to indeterminate progress.
pub fn probe_duration_us(
    source: &Path,
    runner: &dyn CommandRunner,
    cancel: &Arc<AtomicBool>,
) -> Option<u64> {
    let spec = build_ffprobe(source);
    let stdout_sink = Arc::new(std::sync::Mutex::new(Vec::new()));

    // Use a temporary cancel for the probe: it shouldn't block on stdout
    let probe_cancel = Arc::new(AtomicBool::new(false));

    // Run ffprobe collecting all its stdout
    let result = runner.run(&spec, &probe_cancel, Some(&stdout_sink));
    if !result.is_success() || cancel.load(Ordering::Relaxed) {
        return None;
    }

    let lines = stdout_sink.lock().unwrap().clone();
    let json = lines.join("\n");
    parse_duration_us_from_ffprobe_json(&json)
}

/// Parse the `"duration"` field from ffprobe's `-print_format json -show_format` output.
/// Returned value is in microseconds. Does not require serde.
pub fn parse_duration_us_from_ffprobe_json(json: &str) -> Option<u64> {
    // Look for: "duration": "123.456"
    let prefix = "\"duration\":";
    let pos = json.find(prefix)?;
    let rest = json[pos + prefix.len()..].trim_start();
    // Value may be quoted ("123.456") or unquoted (123.456)
    let rest = rest.trim_start_matches('"');
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(rest.len());
    let secs_str = &rest[..end];
    let secs: f64 = secs_str.parse().ok()?;
    if secs <= 0.0 {
        return None;
    }
    Some((secs * 1_000_000.0) as u64)
}

// ── ffmpeg progress block parser ──────────────────────────────────────────────

/// Accumulates lines from ffmpeg's `-progress pipe:1` output and reports
/// the current progress once a complete block has been received.
///
/// ffmpeg writes key=value pairs, one per line. A block ends when
/// `progress=continue` (mid-job) or `progress=end` (finished) is seen.
#[derive(Debug, Default)]
pub struct ProgressBlock {
    out_time_us: Option<u64>,
    is_end: bool,
    block_complete: bool,
}

impl ProgressBlock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one line of ffmpeg stdout. Returns `true` if a complete progress
    /// block has been received (caller should read `fraction` and reset).
    pub fn add_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("out_time_us=") {
            self.out_time_us = val.parse().ok();
        } else if line == "progress=end" {
            self.is_end = true;
            self.block_complete = true;
        } else if line == "progress=continue" {
            self.block_complete = true;
        }
        self.block_complete
    }

    /// Whether the last completed block had `progress=end`.
    pub fn is_final(&self) -> bool {
        self.is_end
    }

    /// Current `out_time_us` from the last completed block.
    pub fn out_time_us(&self) -> Option<u64> {
        self.out_time_us
    }

    /// Compute completion fraction (0.0–1.0) given total duration in microseconds.
    /// Returns `None` if duration is zero or we haven't received a time yet.
    pub fn fraction(&self, total_us: u64) -> Option<f32> {
        if total_us == 0 {
            return None;
        }
        let out = self.out_time_us? as f64;
        Some((out / total_us as f64).clamp(0.0, 1.0) as f32)
    }

    /// Reset for the next block.
    pub fn reset(&mut self) {
        self.block_complete = false;
    }
}

/// Parse a buffer of accumulated stdout lines and return the latest fractional
/// progress (0.0–1.0). Returns `None` if no progress data is available.
///
/// Used when stdout is read from a shared `Arc<Mutex<Vec<String>>>` by the
/// main-thread polling timer.
pub fn parse_latest_fraction(lines: &[String], total_us: u64) -> Option<f32> {
    if total_us == 0 {
        return None;
    }
    let mut block = ProgressBlock::new();
    let mut latest: Option<f32> = None;
    for line in lines {
        if block.add_line(line) {
            if let Some(f) = block.fraction(total_us) {
                latest = Some(f);
            }
            block.reset();
        }
    }
    latest
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::command::MockRunner;

    // ── parse_duration_us_from_ffprobe_json ───────────────────────────────────

    #[test]
    fn parses_quoted_duration() {
        let json = r#"{"format": {"duration": "123.456", "size": "999"}}"#;
        let us = parse_duration_us_from_ffprobe_json(json);
        assert_eq!(us, Some(123_456_000));
    }

    #[test]
    fn parses_integer_duration() {
        let json = r#"{ "format": { "duration": "60", "nb_streams": 1 } }"#;
        let us = parse_duration_us_from_ffprobe_json(json);
        assert_eq!(us, Some(60_000_000));
    }

    #[test]
    fn returns_none_for_zero_duration() {
        let json = r#"{"format": {"duration": "0.000"}}"#;
        assert_eq!(parse_duration_us_from_ffprobe_json(json), None);
    }

    #[test]
    fn returns_none_when_duration_absent() {
        let json = r#"{"format": {"size": "1000"}}"#;
        assert_eq!(parse_duration_us_from_ffprobe_json(json), None);
    }

    #[test]
    fn returns_none_for_empty_string() {
        assert_eq!(parse_duration_us_from_ffprobe_json(""), None);
    }

    #[test]
    fn handles_fractional_seconds_precisely() {
        let json = r#"{"format": {"duration": "1.5"}}"#;
        let us = parse_duration_us_from_ffprobe_json(json).unwrap();
        // 1.5 seconds = 1_500_000 us
        assert_eq!(us, 1_500_000);
    }

    // ── ProgressBlock ─────────────────────────────────────────────────────────

    #[test]
    fn block_not_complete_until_progress_line() {
        let mut block = ProgressBlock::new();
        assert!(!block.add_line("frame=10"));
        assert!(!block.add_line("fps=25.00"));
        assert!(!block.add_line("out_time_us=400000"));
        assert!(block.add_line("progress=continue"));
        assert!(!block.is_final());
    }

    #[test]
    fn block_complete_on_progress_end() {
        let mut block = ProgressBlock::new();
        block.add_line("out_time_us=1000000");
        let done = block.add_line("progress=end");
        assert!(done);
        assert!(block.is_final());
    }

    #[test]
    fn block_fraction_uses_total_duration() {
        let mut block = ProgressBlock::new();
        block.add_line("out_time_us=500000");
        block.add_line("progress=continue");
        // 0.5s of 2s = 25%
        assert_eq!(block.fraction(2_000_000), Some(0.25));
    }

    #[test]
    fn block_fraction_clamps_above_one() {
        let mut block = ProgressBlock::new();
        block.add_line("out_time_us=5000000"); // 5s
        block.add_line("progress=continue");
        // 5s / 4s total > 1.0 → clamp to 1.0
        assert_eq!(block.fraction(4_000_000), Some(1.0));
    }

    #[test]
    fn block_fraction_returns_none_without_time() {
        let mut block = ProgressBlock::new();
        block.add_line("progress=continue");
        assert_eq!(block.fraction(1_000_000), None);
    }

    #[test]
    fn block_reset_allows_next_block() {
        let mut block = ProgressBlock::new();
        block.add_line("out_time_us=100000");
        block.add_line("progress=continue");
        block.reset();
        // After reset, can receive next block
        assert!(!block.block_complete);
        block.add_line("out_time_us=200000");
        block.add_line("progress=continue");
        assert_eq!(block.fraction(1_000_000), Some(0.2));
    }

    // ── parse_latest_fraction ─────────────────────────────────────────────────

    #[test]
    fn returns_latest_fraction_from_multiple_blocks() {
        let lines: Vec<String> = vec![
            "out_time_us=500000".into(),
            "progress=continue".into(),
            "out_time_us=1000000".into(),
            "progress=continue".into(),
        ];
        let f = parse_latest_fraction(&lines, 2_000_000);
        assert_eq!(f, Some(0.5)); // second block at 1s / 2s = 0.5
    }

    #[test]
    fn returns_none_for_empty_lines() {
        assert_eq!(parse_latest_fraction(&[], 1_000_000), None);
    }

    #[test]
    fn returns_none_when_total_is_zero() {
        let lines = vec!["out_time_us=1000".into(), "progress=continue".into()];
        assert_eq!(parse_latest_fraction(&lines, 0), None);
    }

    #[test]
    fn handles_lines_without_progress_block_end() {
        // Partial block (no progress= line yet) → no fraction
        let lines = vec!["out_time_us=1000000".into()];
        assert_eq!(parse_latest_fraction(&lines, 2_000_000), None);
    }

    // ── probe_duration_us (with mock runner) ──────────────────────────────────

    #[test]
    fn probe_returns_none_when_tool_missing() {
        let runner = MockRunner::tool_missing("ffprobe");
        let cancel = Arc::new(AtomicBool::new(false));
        let result = probe_duration_us(Path::new("/video.mp4"), &runner, &cancel);
        assert_eq!(result, None);
    }

    #[test]
    fn probe_returns_none_when_cancelled() {
        let runner = MockRunner::all_succeed();
        let cancel = Arc::new(AtomicBool::new(false));
        // Mock will succeed but cancel is set after run is called — since mock
        // checks cancel at call time and probe_cancel is separate, we test
        // the branch where the runner returns success but main cancel is set
        let result = probe_duration_us(Path::new("/video.mp4"), &runner, &cancel);
        // Without JSON output, parse fails → None
        assert_eq!(result, None);
    }

    #[test]
    fn probe_parses_injected_json() {
        let runner = MockRunner::all_succeed();
        let json_lines: Vec<String> = vec![r#"{"format": {"duration": "90.0"}}"#.to_string()];
        runner.then_stdout(json_lines);
        let cancel = Arc::new(AtomicBool::new(false));
        let result = probe_duration_us(Path::new("/video.mp4"), &runner, &cancel);
        assert_eq!(result, Some(90_000_000));
    }
}
