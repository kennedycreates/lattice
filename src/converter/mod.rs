#![allow(dead_code)]

pub mod command;
pub mod progress;
pub mod queue;
pub mod settings;

// Re-exports consumed by code outside the converter module.
pub use command::cleanup_orphaned_temps_in;
pub use queue::{BatchProgress, ConversionQueue};
pub use settings::ConvertSettings;

// Test-only re-exports (command execution internals used by integration tests).
#[cfg(test)]
pub use command::{
    build_commands, build_ffprobe, cleanup_temp, finalize_output, temp_dest, CommandRunner,
    CommandSpec, MockRunner, ProcessOutput, RealRunner, RunResult,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ── MediaKind ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    Unknown,
}

// ── ConversionTool ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionTool {
    Ffmpeg,
    ImageMagick,
    Vips,
}

impl ConversionTool {
    /// Human-readable tool name for display.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::ImageMagick => "ImageMagick (magick / convert)",
            Self::Vips => "vips",
        }
    }

    /// Short install hint for the two documented distros.
    pub fn install_hint(self) -> &'static str {
        match self {
            Self::Ffmpeg => {
                "Install ffmpeg — Ubuntu: sudo apt install ffmpeg  |  Arch: sudo pacman -S ffmpeg"
            }
            Self::ImageMagick => {
                "Install ImageMagick — Ubuntu: sudo apt install imagemagick  |  Arch: sudo pacman -S imagemagick"
            }
            Self::Vips => {
                "Install libvips — Ubuntu: sudo apt install libvips-tools  |  Arch: sudo pacman -S libvips"
            }
        }
    }
}

// ── ConversionPreset ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ConversionPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: MediaKind,
    pub tool: ConversionTool,
    /// Output file extension without the leading dot.
    pub ext: &'static str,
    /// Arguments for ffmpeg between `-i <source>` and `<dest>`.
    /// Ignored for non-ffmpeg presets.
    pub ffmpeg_args: &'static [&'static str],
}

impl ConversionPreset {
    pub fn tool_available(&self, avail: &ToolAvailability) -> bool {
        avail.has(self.tool)
    }
}

static PRESETS: &[ConversionPreset] = &[
    // ── Images ──────────────────────────────────────────────────────────────
    ConversionPreset {
        id: "to_jpeg",
        label: "Convert to JPEG",
        kind: MediaKind::Image,
        tool: ConversionTool::Ffmpeg,
        ext: "jpg",
        ffmpeg_args: &["-q:v", "2", "-f", "mjpeg"],
    },
    ConversionPreset {
        id: "to_png",
        label: "Convert to PNG",
        kind: MediaKind::Image,
        tool: ConversionTool::Ffmpeg,
        ext: "png",
        ffmpeg_args: &["-f", "apng"],
    },
    ConversionPreset {
        id: "to_webp",
        label: "Convert to WebP",
        kind: MediaKind::Image,
        tool: ConversionTool::Ffmpeg,
        ext: "webp",
        ffmpeg_args: &["-c:v", "libwebp", "-quality", "85"],
    },
    ConversionPreset {
        id: "to_avif",
        label: "Convert to AVIF",
        kind: MediaKind::Image,
        tool: ConversionTool::ImageMagick,
        ext: "avif",
        ffmpeg_args: &[],
    },
    ConversionPreset {
        id: "web_jpeg",
        label: "Web-sized JPEG (1920px, q85)",
        kind: MediaKind::Image,
        tool: ConversionTool::Ffmpeg,
        ext: "jpg",
        ffmpeg_args: &["-vf", "scale='min(1920,iw)':-2", "-q:v", "2", "-f", "mjpeg"],
    },
    ConversionPreset {
        id: "web_webp",
        label: "Web-sized WebP (1920px)",
        kind: MediaKind::Image,
        tool: ConversionTool::Ffmpeg,
        ext: "webp",
        ffmpeg_args: &[
            "-vf",
            "scale='min(1920,iw)':-2",
            "-c:v",
            "libwebp",
            "-quality",
            "82",
        ],
    },
    // ── Audio ────────────────────────────────────────────────────────────────
    ConversionPreset {
        id: "to_mp3",
        label: "Convert to MP3",
        kind: MediaKind::Audio,
        tool: ConversionTool::Ffmpeg,
        ext: "mp3",
        ffmpeg_args: &["-c:a", "libmp3lame", "-b:a", "192k", "-vn"],
    },
    ConversionPreset {
        id: "to_flac",
        label: "Convert to FLAC",
        kind: MediaKind::Audio,
        tool: ConversionTool::Ffmpeg,
        ext: "flac",
        ffmpeg_args: &["-c:a", "flac", "-vn"],
    },
    ConversionPreset {
        id: "to_opus",
        label: "Convert to Opus",
        kind: MediaKind::Audio,
        tool: ConversionTool::Ffmpeg,
        ext: "opus",
        ffmpeg_args: &["-c:a", "libopus", "-b:a", "128k", "-vn"],
    },
    // ── Video ────────────────────────────────────────────────────────────────
    ConversionPreset {
        id: "mp4_compatible",
        label: "Compatible MP4 (H.264)",
        kind: MediaKind::Video,
        tool: ConversionTool::Ffmpeg,
        ext: "mp4",
        ffmpeg_args: &[
            "-c:v", "libx264", "-crf", "23", "-c:a", "aac", "-b:a", "128k",
        ],
    },
    ConversionPreset {
        id: "mp4_small",
        label: "Smaller MP4 (H.264, higher compression)",
        kind: MediaKind::Video,
        tool: ConversionTool::Ffmpeg,
        ext: "mp4",
        ffmpeg_args: &[
            "-c:v", "libx264", "-crf", "30", "-c:a", "aac", "-b:a", "96k",
        ],
    },
    ConversionPreset {
        id: "to_webm",
        label: "WebM (VP9)",
        kind: MediaKind::Video,
        tool: ConversionTool::Ffmpeg,
        ext: "webm",
        ffmpeg_args: &[
            "-c:v",
            "libvpx-vp9",
            "-crf",
            "30",
            "-b:v",
            "0",
            "-c:a",
            "libopus",
        ],
    },
    ConversionPreset {
        id: "extract_mp3",
        label: "Extract audio as MP3",
        kind: MediaKind::Video,
        tool: ConversionTool::Ffmpeg,
        ext: "mp3",
        ffmpeg_args: &["-vn", "-c:a", "libmp3lame", "-b:a", "192k"],
    },
    ConversionPreset {
        id: "extract_wav",
        label: "Extract audio as WAV",
        kind: MediaKind::Video,
        tool: ConversionTool::Ffmpeg,
        ext: "wav",
        ffmpeg_args: &["-vn", "-c:a", "pcm_s16le"],
    },
];

pub fn all_presets() -> &'static [ConversionPreset] {
    PRESETS
}

pub fn preset_by_id(id: &str) -> Option<&'static ConversionPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// All presets matching `kind` whose required tool is available.
pub fn presets_for(kind: MediaKind, avail: &ToolAvailability) -> Vec<&'static ConversionPreset> {
    PRESETS
        .iter()
        .filter(|p| p.kind == kind && p.tool_available(avail))
        .collect()
}

// ── Tool availability ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct ToolAvailability {
    pub ffmpeg: bool,
    pub ffprobe: bool,
    pub imagemagick: bool,
    pub vips: bool,
}

impl ToolAvailability {
    pub fn has(&self, tool: ConversionTool) -> bool {
        match tool {
            ConversionTool::Ffmpeg => self.ffmpeg,
            ConversionTool::ImageMagick => self.imagemagick,
            ConversionTool::Vips => self.vips,
        }
    }

    pub fn any_available(&self) -> bool {
        self.ffmpeg || self.imagemagick || self.vips
    }
}

pub type ToolProber = fn(&str) -> bool;

pub fn probe_tool_real(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn detect_tools_with(prober: ToolProber) -> ToolAvailability {
    let ffmpeg = prober("ffmpeg");
    let ffprobe = prober("ffprobe");
    let imagemagick = prober("magick") || prober("convert");
    let vips = std::process::Command::new("vips")
        .arg("--vips-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ToolAvailability {
        ffmpeg,
        ffprobe,
        imagemagick,
        vips,
    }
}

pub fn detect_tools() -> ToolAvailability {
    detect_tools_with(probe_tool_real)
}

pub fn imagemagick_binary() -> Option<&'static str> {
    for name in ["magick", "convert"] {
        if probe_tool_real(name) {
            return Some(name);
        }
    }
    None
}

// ── MediaKind detection ───────────────────────────────────────────────────────

pub fn media_kind_from_ext(ext: &str) -> MediaKind {
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "tiff" | "tif" | "bmp" | "heic" | "avif" => {
            MediaKind::Image
        }
        "wav" | "flac" | "mp3" | "m4a" | "aac" | "ogg" | "opus" => MediaKind::Audio,
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => MediaKind::Video,
        _ => MediaKind::Unknown,
    }
}

pub fn media_kind_from_path(path: &Path, content_type: Option<&str>) -> MediaKind {
    if let Some(ct) = content_type {
        if ct.starts_with("image/") {
            return MediaKind::Image;
        }
        if ct.starts_with("audio/") {
            return MediaKind::Audio;
        }
        if ct.starts_with("video/") {
            return MediaKind::Video;
        }
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    media_kind_from_ext(ext)
}

// ── Policy types ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputConflictPolicy {
    AutoRename,
    Skip,
    Overwrite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputLocationMode {
    NextToSource,
    ChosenFolder(PathBuf),
    Subfolder(String),
}

// ── ConversionError ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversionError {
    ToolNotFound(ConversionTool),
    SourceNotFound,
    UnsupportedKind,
    ProcessFailed(String),
    Io(String),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolNotFound(t) => write!(f, "Required tool not found: {t:?}"),
            Self::SourceNotFound => write!(f, "Source file not found"),
            Self::UnsupportedKind => write!(f, "File type not supported by this preset"),
            Self::ProcessFailed(msg) => write!(f, "{msg}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

/// Produce a human-readable error summary from raw tool stderr output.
///
/// Returns a short sentence describing the likely cause, followed by the
/// original stderr (truncated) so the user still has the raw detail available.
pub fn format_job_error(raw_stderr: &str) -> String {
    let lower = raw_stderr.to_ascii_lowercase();

    let summary = if lower.contains("permission denied") || lower.contains("access denied") {
        "Output location is not writable. Check folder permissions."
    } else if lower.contains("no such file") || lower.contains("file not found") {
        "Source or output file path not found. The file may have been moved or deleted."
    } else if lower.contains("no decode delegate")
        || lower.contains("unable to open image")
        || lower.contains("no streams")
    {
        "This file format is not supported. The tool may need additional codec support."
    } else if lower.contains("invalid data found")
        || lower.contains("invalid argument")
        || lower.contains("moov atom not found")
    {
        "The file appears to be corrupted or is not a valid media file."
    } else if lower.contains("codec not currently supported")
        || lower.contains("encoder not found")
        || lower.contains("unknown encoder")
        || lower.contains("decoder not found")
    {
        "Required codec is not available in this build of the tool."
    } else if lower.contains("no space left") {
        "No disk space left on the output device."
    } else if lower.contains("output file is empty") || lower.contains("nothing was encoded") {
        "Conversion produced an empty output file. The source may be empty or unsupported."
    } else {
        // No recognisable pattern — show the raw text directly
        return raw_stderr.to_string();
    };

    if raw_stderr.is_empty() {
        summary.to_string()
    } else {
        // Trim the raw detail to a reasonable length for the detail view
        let detail = truncate_stderr(raw_stderr, 400);
        format!("{summary}\n\nTool output:\n{detail}")
    }
}

fn truncate_stderr(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ── Job types ─────────────────────────────────────────────────────────────────

pub type ConversionJobId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversionJobStatus {
    Pending,
    Running,
    Done,
    Failed(String),
    Cancelled,
    Skipped(String),
}

impl ConversionJobStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Pending)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed(_) | Self::Cancelled | Self::Skipped(_)
        )
    }
}

#[derive(Clone, Debug)]
pub struct ConversionJob {
    pub id: ConversionJobId,
    pub source: PathBuf,
    /// Planned output path. For `Skipped` jobs this shows the path that *would*
    /// have been used, useful for preview UI.
    pub dest: PathBuf,
    pub preset: &'static ConversionPreset,
    pub status: ConversionJobStatus,
    pub attempt: u32,
}

impl ConversionJob {
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

// ── ConversionBatch ───────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ConversionBatch {
    pub jobs: Vec<ConversionJob>,
    pub preset: &'static ConversionPreset,
    pub output_mode: OutputLocationMode,
    pub conflict_policy: OutputConflictPolicy,
}

impl ConversionBatch {
    pub fn active_jobs(&self) -> impl Iterator<Item = &ConversionJob> {
        self.jobs.iter().filter(|j| j.is_active())
    }

    pub fn skipped_jobs(&self) -> impl Iterator<Item = &ConversionJob> {
        self.jobs
            .iter()
            .filter(|j| matches!(j.status, ConversionJobStatus::Skipped(_)))
    }

    pub fn active_count(&self) -> usize {
        self.active_jobs().count()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped_jobs().count()
    }

    pub fn into_active_jobs(self) -> Vec<ConversionJob> {
        self.jobs.into_iter().filter(|j| j.is_active()).collect()
    }

    /// Best guess at the output directory for "Open Output" navigation.
    pub fn representative_output_dir(&self) -> Option<PathBuf> {
        match &self.output_mode {
            OutputLocationMode::ChosenFolder(p) => Some(p.clone()),
            OutputLocationMode::Subfolder(_) | OutputLocationMode::NextToSource => self
                .jobs
                .iter()
                .find(|j| j.is_active())
                .or_else(|| self.jobs.first())
                .and_then(|j| j.dest.parent().map(|p| p.to_path_buf())),
        }
    }
}

// ── ConvertItem ───────────────────────────────────────────────────────────────

/// Pre-classified file for the convert panel.
#[derive(Clone, Debug)]
pub struct ConvertItem {
    pub path: PathBuf,
    pub kind: MediaKind,
}

// ── Output path resolution ────────────────────────────────────────────────────

/// Conflict-safe output path computation.
///
/// AutoRename: appends ` 2`, ` 3` … before the extension.
/// Skip: returns `None` if output already exists or is reserved.
/// Overwrite: always returns the base path.
pub fn resolve_output_path(
    source: &Path,
    ext: &str,
    dest_dir: &Path,
    policy: OutputConflictPolicy,
    reserved: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let stem = source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let base = dest_dir.join(format!("{stem}.{ext}"));

    match policy {
        OutputConflictPolicy::Overwrite => return Some(base),
        OutputConflictPolicy::Skip => {
            return if base.exists() || reserved.contains(&base) {
                None
            } else {
                Some(base)
            };
        }
        OutputConflictPolicy::AutoRename => {}
    }

    if !base.exists() && !reserved.contains(&base) {
        return Some(base);
    }
    for n in 2u32.. {
        let candidate = dest_dir.join(format!("{stem} {n}.{ext}"));
        if !candidate.exists() && !reserved.contains(&candidate) {
            return Some(candidate);
        }
    }
    unreachable!("resolve_output_path: exhausted all candidates")
}

/// Convenience: AutoRename, always returns a path.
pub fn resolve_dest(
    source: &Path,
    preset: &ConversionPreset,
    dest_dir: &Path,
    reserved: &HashSet<PathBuf>,
) -> PathBuf {
    resolve_output_path(
        source,
        preset.ext,
        dest_dir,
        OutputConflictPolicy::AutoRename,
        reserved,
    )
    .expect("AutoRename always returns Some")
}

// ── Batch planning ────────────────────────────────────────────────────────────

/// Produce a `ConversionBatch` from raw source paths.
///
/// `content_types` maps path → MIME string for files where MIME is already
/// known (e.g., from GIO). Pass an empty map if not available.
pub fn plan_batch(
    sources: &[PathBuf],
    preset: &'static ConversionPreset,
    output_mode: OutputLocationMode,
    conflict_policy: OutputConflictPolicy,
    avail: &ToolAvailability,
    content_types: &HashMap<PathBuf, String>,
) -> ConversionBatch {
    let tool_ok = avail.has(preset.tool);
    let mut reserved: HashSet<PathBuf> = HashSet::new();
    let mut next_id: ConversionJobId = 0;
    let mut jobs = Vec::with_capacity(sources.len());

    for source in sources {
        let id = next_id;
        next_id += 1;

        let ct = content_types.get(source).map(String::as_str);
        let kind = media_kind_from_path(source, ct);

        if !tool_ok {
            jobs.push(ConversionJob {
                id,
                source: source.clone(),
                dest: source.clone(),
                preset,
                status: ConversionJobStatus::Skipped(format!(
                    "{} is not installed",
                    preset.tool.display_name()
                )),
                attempt: 0,
            });
            continue;
        }

        if kind != preset.kind {
            let reason = if kind == MediaKind::Unknown {
                "Unsupported file type — not image, audio, or video".to_string()
            } else {
                let kind_name = match kind {
                    MediaKind::Image => "image",
                    MediaKind::Audio => "audio",
                    MediaKind::Video => "video",
                    MediaKind::Unknown => "unknown",
                };
                let expected_name = match preset.kind {
                    MediaKind::Image => "image",
                    MediaKind::Audio => "audio",
                    MediaKind::Video => "video",
                    MediaKind::Unknown => "unknown",
                };
                format!("This is a {kind_name} file; preset converts {expected_name}")
            };
            jobs.push(ConversionJob {
                id,
                source: source.clone(),
                dest: source.clone(),
                preset,
                status: ConversionJobStatus::Skipped(reason),
                attempt: 0,
            });
            continue;
        }

        let dest_dir: PathBuf = match &output_mode {
            OutputLocationMode::NextToSource => {
                source.parent().unwrap_or(Path::new("/")).to_path_buf()
            }
            OutputLocationMode::ChosenFolder(f) => f.clone(),
            OutputLocationMode::Subfolder(name) => {
                source.parent().unwrap_or(Path::new("/")).join(name)
            }
        };

        let dest_opt = resolve_output_path(
            source,
            preset.ext,
            &dest_dir,
            conflict_policy.clone(),
            &reserved,
        );

        match dest_opt {
            None => {
                let stem = source.file_stem().unwrap_or_default().to_string_lossy();
                jobs.push(ConversionJob {
                    id,
                    source: source.clone(),
                    dest: dest_dir.join(format!("{stem}.{}", preset.ext)),
                    preset,
                    status: ConversionJobStatus::Skipped(
                        "Output already exists (skipped — change conflict policy to rename or overwrite)".to_string(),
                    ),
                    attempt: 0,
                });
            }
            Some(dest) => {
                reserved.insert(dest.clone());
                jobs.push(ConversionJob {
                    id,
                    source: source.clone(),
                    dest,
                    preset,
                    status: ConversionJobStatus::Pending,
                    attempt: 0,
                });
            }
        }
    }

    ConversionBatch {
        jobs,
        preset,
        output_mode,
        conflict_policy,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn avail_ffmpeg_only() -> ToolAvailability {
        ToolAvailability {
            ffmpeg: true,
            ffprobe: true,
            imagemagick: false,
            vips: false,
        }
    }

    fn avail_none() -> ToolAvailability {
        ToolAvailability {
            ffmpeg: false,
            ffprobe: false,
            imagemagick: false,
            vips: false,
        }
    }

    fn avail_all() -> ToolAvailability {
        ToolAvailability {
            ffmpeg: true,
            ffprobe: true,
            imagemagick: true,
            vips: true,
        }
    }

    fn no_ct() -> HashMap<PathBuf, String> {
        HashMap::new()
    }

    // ── MediaKind ─────────────────────────────────────────────────────────────

    #[test]
    fn kind_images() {
        for ext in [
            "jpg", "jpeg", "png", "webp", "gif", "tiff", "tif", "bmp", "heic", "avif",
        ] {
            assert_eq!(media_kind_from_ext(ext), MediaKind::Image, ".{ext}");
        }
    }

    #[test]
    fn kind_audio() {
        for ext in ["wav", "flac", "mp3", "m4a", "aac", "ogg", "opus"] {
            assert_eq!(media_kind_from_ext(ext), MediaKind::Audio, ".{ext}");
        }
    }

    #[test]
    fn kind_video() {
        for ext in ["mp4", "mov", "mkv", "webm", "avi", "m4v"] {
            assert_eq!(media_kind_from_ext(ext), MediaKind::Video, ".{ext}");
        }
    }

    #[test]
    fn kind_unknown() {
        assert_eq!(media_kind_from_ext("xyz"), MediaKind::Unknown);
        assert_eq!(media_kind_from_ext(""), MediaKind::Unknown);
    }

    #[test]
    fn kind_case_insensitive() {
        assert_eq!(media_kind_from_ext("JPG"), MediaKind::Image);
        assert_eq!(media_kind_from_ext("MP4"), MediaKind::Video);
    }

    #[test]
    fn kind_content_type_override() {
        let path = PathBuf::from("data.dat");
        assert_eq!(
            media_kind_from_path(&path, Some("video/mp4")),
            MediaKind::Video
        );
        assert_eq!(
            media_kind_from_path(&path, Some("image/png")),
            MediaKind::Image
        );
        assert_eq!(
            media_kind_from_path(&path, Some("audio/flac")),
            MediaKind::Audio
        );
        assert_eq!(
            media_kind_from_path(&path, Some("application/octet-stream")),
            MediaKind::Unknown
        );
    }

    // ── Presets ───────────────────────────────────────────────────────────────

    #[test]
    fn preset_ids_are_unique() {
        let ids: HashSet<&str> = PRESETS.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), PRESETS.len());
    }

    #[test]
    fn preset_exts_have_no_dot() {
        for p in PRESETS {
            assert!(!p.ext.is_empty());
            assert!(!p.ext.contains('.'));
        }
    }

    #[test]
    fn presets_for_filters_kind_and_tool() {
        let images = presets_for(MediaKind::Image, &avail_ffmpeg_only());
        assert!(images.iter().all(|p| p.kind == MediaKind::Image));
        assert!(!images.iter().any(|p| p.id == "to_avif")); // requires imagemagick
        assert!(images.iter().any(|p| p.id == "to_jpeg"));
    }

    // ── Tool detection (mocked) ───────────────────────────────────────────────

    #[test]
    fn detect_with_mock_all() {
        let avail = detect_tools_with(|_| true);
        assert!(avail.ffmpeg && avail.imagemagick);
        assert!(avail.any_available());
    }

    #[test]
    fn detect_with_mock_none() {
        let avail = detect_tools_with(|_| false);
        assert!(!avail.any_available());
    }

    #[test]
    fn detect_real_does_not_panic() {
        let _ = detect_tools();
    }

    // ── resolve_output_path ───────────────────────────────────────────────────

    #[test]
    fn resolve_no_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let p = resolve_output_path(
            Path::new("/src/photo.png"),
            "jpg",
            dir.path(),
            OutputConflictPolicy::AutoRename,
            &HashSet::new(),
        );
        assert_eq!(p, Some(dir.path().join("photo.jpg")));
    }

    #[test]
    fn resolve_autorename_space_number() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.jpg"), b"").unwrap();
        let p = resolve_output_path(
            Path::new("/src/photo.png"),
            "jpg",
            dir.path(),
            OutputConflictPolicy::AutoRename,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(p, dir.path().join("photo 2.jpg"));
    }

    #[test]
    fn resolve_skip_returns_none_on_conflict() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.jpg"), b"").unwrap();
        let p = resolve_output_path(
            Path::new("/src/photo.png"),
            "jpg",
            dir.path(),
            OutputConflictPolicy::Skip,
            &HashSet::new(),
        );
        assert_eq!(p, None);
    }

    #[test]
    fn resolve_overwrite_ignores_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.jpg"), b"existing").unwrap();
        let p = resolve_output_path(
            Path::new("/src/photo.png"),
            "jpg",
            dir.path(),
            OutputConflictPolicy::Overwrite,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(p, dir.path().join("photo.jpg"));
    }

    #[test]
    fn resolve_batch_reserved_set() {
        let dir = tempfile::tempdir().unwrap();
        let mut reserved = HashSet::new();
        let results: Vec<_> = (0..3)
            .map(|_| {
                let p = resolve_output_path(
                    Path::new("/src/photo.png"),
                    "jpg",
                    dir.path(),
                    OutputConflictPolicy::AutoRename,
                    &reserved,
                )
                .unwrap();
                reserved.insert(p.clone());
                p
            })
            .collect();
        assert_eq!(results[0], dir.path().join("photo.jpg"));
        assert_eq!(results[1], dir.path().join("photo 2.jpg"));
        assert_eq!(results[2], dir.path().join("photo 3.jpg"));
    }

    // ── plan_batch ────────────────────────────────────────────────────────────

    #[test]
    fn plan_batch_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let sources: Vec<PathBuf> = ["a.png", "b.jpg", "c.webp"]
            .iter()
            .map(|f| {
                let p = dir.path().join(f);
                std::fs::write(&p, b"").unwrap();
                p
            })
            .collect();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &sources,
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        assert_eq!(batch.active_count(), 3);
        assert_eq!(batch.skipped_count(), 0);
    }

    #[test]
    fn plan_batch_skips_incompatible_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let sources: Vec<PathBuf> = ["photo.png", "clip.mp4", "song.flac"]
            .iter()
            .map(|f| {
                let p = dir.path().join(f);
                std::fs::write(&p, b"").unwrap();
                p
            })
            .collect();
        let preset = preset_by_id("to_jpeg").unwrap(); // image preset
        let batch = plan_batch(
            &sources,
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        assert_eq!(batch.active_count(), 1);
        assert_eq!(batch.skipped_count(), 2);
    }

    #[test]
    fn plan_batch_skips_unknown_extension() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("data.xyz");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        assert_eq!(batch.active_count(), 0);
        let skip = batch.skipped_jobs().next().unwrap();
        if let ConversionJobStatus::Skipped(ref reason) = skip.status {
            assert!(
                reason.contains("Unsupported") || reason.contains("type"),
                "skip reason should mention type/unsupported: {reason}"
            );
        }
    }

    #[test]
    fn plan_batch_skips_when_tool_missing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.png");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_avif").unwrap(); // requires imagemagick
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        assert_eq!(batch.active_count(), 0);
        let skip_status = batch.skipped_jobs().next().unwrap().status.clone();
        if let ConversionJobStatus::Skipped(reason) = skip_status {
            assert!(reason.contains("ImageMagick"));
        }
    }

    #[test]
    fn plan_batch_chosen_folder_output() {
        let src_dir = tempfile::tempdir().unwrap();
        let out_dir = tempfile::tempdir().unwrap();
        let source = src_dir.path().join("photo.png");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::ChosenFolder(out_dir.path().to_path_buf()),
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        let job = batch.active_jobs().next().unwrap();
        assert_eq!(job.dest.parent().unwrap(), out_dir.path());
    }

    #[test]
    fn plan_batch_subfolder_output() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.png");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::Subfolder("Converted".to_string()),
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        let job = batch.active_jobs().next().unwrap();
        assert_eq!(job.dest.parent().unwrap(), dir.path().join("Converted"));
    }

    #[test]
    fn plan_batch_content_type_used() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("media.dat");
        std::fs::write(&source, b"").unwrap();
        let mut ct = HashMap::new();
        ct.insert(source.clone(), "image/png".to_string());
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &ct,
        );
        assert_eq!(batch.active_count(), 1);
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn plan_batch_empty_sources_returns_empty_batch() {
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        assert_eq!(batch.active_count(), 0);
        assert_eq!(batch.skipped_count(), 0);
        assert!(batch.jobs.is_empty());
    }

    #[test]
    fn plan_batch_filename_with_spaces_preserved_in_dest() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("my photo with spaces.png");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source.clone()],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        let job = batch.active_jobs().next().unwrap();
        let dest_name = job.dest.file_name().unwrap().to_string_lossy();
        assert_eq!(dest_name, "my photo with spaces.jpg");
    }

    #[test]
    fn plan_batch_unicode_filename_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("日本語写真.png");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        let job = batch.active_jobs().next().unwrap();
        let dest_name = job.dest.file_name().unwrap().to_string_lossy();
        assert_eq!(dest_name, "日本語写真.jpg");
    }

    #[test]
    fn plan_batch_special_chars_in_filename() {
        let dir = tempfile::tempdir().unwrap();
        // Apostrophe, parentheses, semicolon — safe in filenames, must not be escaped
        let source = dir.path().join("my's file (v2); final.png");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        let job = batch.active_jobs().next().unwrap();
        let dest_name = job.dest.file_name().unwrap().to_string_lossy();
        assert_eq!(dest_name, "my's file (v2); final.jpg");
    }

    #[test]
    fn plan_batch_no_extension_source_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("README"); // no extension
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(),
            &no_ct(),
        );
        assert_eq!(batch.active_count(), 0);
        assert_eq!(batch.skipped_count(), 1);
        let skip = batch.skipped_jobs().next().unwrap();
        if let ConversionJobStatus::Skipped(ref reason) = skip.status {
            assert!(
                reason.contains("Unsupported") || reason.contains("type"),
                "skip reason should mention type: {reason}"
            );
        } else {
            panic!("expected Skipped status");
        }
    }

    #[test]
    fn plan_batch_skip_reason_mentions_tool_name_not_debug_repr() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.png");
        std::fs::write(&source, b"").unwrap();
        let preset = preset_by_id("to_avif").unwrap(); // requires ImageMagick
        let batch = plan_batch(
            &[source],
            preset,
            OutputLocationMode::NextToSource,
            OutputConflictPolicy::AutoRename,
            &avail_ffmpeg_only(), // ImageMagick not available
            &no_ct(),
        );
        let skip = batch.skipped_jobs().next().unwrap();
        if let ConversionJobStatus::Skipped(ref reason) = skip.status {
            // Should say "ImageMagick" not "ImageMagick" (debug repr would be just "ImageMagick"
            // which is fine) — crucially should NOT say "Ffmpeg"
            assert!(
                !reason.contains("Ffmpeg"),
                "skip reason must not use debug format 'Ffmpeg': {reason}"
            );
            assert!(
                reason.to_lowercase().contains("imagemagick")
                    || reason.contains("magick")
                    || reason.contains("convert"),
                "skip reason should name ImageMagick: {reason}"
            );
        } else {
            panic!("expected Skipped status");
        }
    }

    #[test]
    fn resolve_output_path_unicode_stem_autorename() {
        let dir = tempfile::tempdir().unwrap();
        // First output exists
        std::fs::write(dir.path().join("写真.jpg"), b"").unwrap();
        let p = resolve_output_path(
            Path::new("/src/写真.png"),
            "jpg",
            dir.path(),
            OutputConflictPolicy::AutoRename,
            &HashSet::new(),
        )
        .unwrap();
        // Must be "写真 2.jpg"
        assert_eq!(p, dir.path().join("写真 2.jpg"));
    }

    #[test]
    fn representative_output_dir_chosen_folder() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("output");
        let preset = preset_by_id("to_jpeg").unwrap();
        let batch = ConversionBatch {
            jobs: Vec::new(),
            preset,
            output_mode: OutputLocationMode::ChosenFolder(out.clone()),
            conflict_policy: OutputConflictPolicy::AutoRename,
        };
        assert_eq!(batch.representative_output_dir(), Some(out));
    }

    // ── format_job_error ──────────────────────────────────────────────────────

    #[test]
    fn format_job_error_permission_denied() {
        let msg = format_job_error("av_interleaved_write_frame(): Permission denied");
        assert!(
            msg.to_lowercase().contains("permission") || msg.to_lowercase().contains("writable"),
            "should mention permissions: {msg}"
        );
    }

    #[test]
    fn format_job_error_no_decode_delegate() {
        let msg = format_job_error("no decode delegate for this image format");
        assert!(
            msg.to_lowercase().contains("format")
                || msg.to_lowercase().contains("codec")
                || msg.to_lowercase().contains("supported"),
            "should mention format/codec: {msg}"
        );
    }

    #[test]
    fn format_job_error_unknown_returns_raw() {
        let raw = "some completely unknown error xyz123";
        let msg = format_job_error(raw);
        // Unknown patterns pass through unchanged
        assert_eq!(msg, raw);
    }

    #[test]
    fn format_job_error_empty_input() {
        // Should not panic
        let msg = format_job_error("");
        let _ = msg; // just ensure no panic
    }

    #[test]
    fn format_job_error_invalid_data() {
        let msg = format_job_error("Invalid data found when processing input");
        assert!(
            msg.to_lowercase().contains("corrupt")
                || msg.to_lowercase().contains("valid")
                || msg.to_lowercase().contains("media"),
            "should mention corruption/validity: {msg}"
        );
    }

    // ── tool display names ────────────────────────────────────────────────────

    #[test]
    fn tool_display_names_are_lowercase_friendly() {
        use crate::converter::ConversionTool;
        // Must not use debug-style capitalization ("Ffmpeg", "ImageMagick", "Vips")
        assert!(ConversionTool::Ffmpeg.display_name().contains("ffmpeg"));
        assert!(ConversionTool::ImageMagick
            .display_name()
            .to_lowercase()
            .contains("imagemagick"));
        assert!(ConversionTool::Vips.display_name().contains("vips"));
    }

    #[test]
    fn tool_install_hints_non_empty() {
        use crate::converter::ConversionTool;
        for tool in [
            ConversionTool::Ffmpeg,
            ConversionTool::ImageMagick,
            ConversionTool::Vips,
        ] {
            let hint = tool.install_hint();
            assert!(
                !hint.is_empty(),
                "install hint must not be empty for {tool:?}"
            );
            // Should mention at least one package manager
            assert!(
                hint.contains("apt") || hint.contains("pacman") || hint.contains("brew"),
                "hint should include install command: {hint}"
            );
        }
    }
}
