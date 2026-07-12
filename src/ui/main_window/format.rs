//! Error, file-size and MIME/type formatting helpers, split out of main_window.

use super::TRASH_GVFS_DIAGNOSTIC;
use crate::metadata::TintRecord;
use crate::ui::file_grid::{FileItem, FileKind};

pub(super) fn command_exists(command: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path_var).any(|directory| directory.join(command).is_file())
}

pub(super) fn friendly_error(error: &glib::Error) -> (&'static str, String) {
    match error.kind::<gio::IOErrorEnum>() {
        Some(gio::IOErrorEnum::PermissionDenied) => (
            "Permission Denied",
            "Lattice does not have permission to complete this operation.".to_string(),
        ),
        Some(gio::IOErrorEnum::NotFound) => (
            "File Not Found",
            "The selected file or folder no longer exists.".to_string(),
        ),
        Some(gio::IOErrorEnum::Exists) => (
            "Name Conflict",
            "An item with that name already exists in this location.".to_string(),
        ),
        _ => ("Operation Failed", error.message().to_string()),
    }
}

pub(super) fn friendly_error_detail(error: &glib::Error) -> String {
    let (title, detail) = friendly_error(error);
    format!("{title}: {detail}")
}

pub(super) fn trash_operation_error_detail(error: &glib::Error) -> String {
    let base = friendly_error_detail(error);
    match error.kind::<gio::IOErrorEnum>() {
        Some(gio::IOErrorEnum::NotSupported) => format!(
            "{base}. This filesystem or mount may not support Trash. Permanent Delete is available only through the explicit confirmation flow."
        ),
        Some(gio::IOErrorEnum::NotMounted) => {
            format!("{base}. The mount may have disappeared or is not available.")
        }
        Some(gio::IOErrorEnum::PermissionDenied) => {
            format!("{base}. Check mount permissions or GVfs/portal access.")
        }
        _ => format!("{base}. {TRASH_GVFS_DIAGNOSTIC}"),
    }
}

pub(super) fn format_modified_time(time: Option<glib::DateTime>) -> Option<String> {
    time.and_then(|value| value.format("%Y-%m-%d %H:%M").ok())
        .map(|value| value.to_string())
}

pub(super) fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0usize;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

pub(super) fn preview_type_label(item: &FileItem, mime: &str) -> String {
    let mime_lower = mime.to_ascii_lowercase();
    if let Some(friendly) = mime_to_friendly_type(&mime_lower) {
        return friendly.to_string();
    }
    // Broad fallback for unrecognised audio files classified as Unknown
    if item.kind == FileKind::Unknown && mime_lower.starts_with("audio/") {
        return "Audio".to_string();
    }
    item.kind.label().to_string()
}

pub(super) fn tint_name_and_color(
    tints: &[TintRecord],
    tint_id: i64,
    fallback_color: Option<String>,
) -> (String, Option<String>) {
    if let Some(tint) = tints.iter().find(|tint| tint.id == tint_id) {
        return (
            tint.name.clone(),
            fallback_color.or_else(|| tint.color.clone()),
        );
    }

    if let Some(default) = tints.iter().find(|tint| tint.is_default) {
        return (
            default.name.clone(),
            fallback_color.or_else(|| default.color.clone()),
        );
    }

    ("Beige".to_string(), fallback_color)
}

fn mime_to_friendly_type(mime: &str) -> Option<&'static str> {
    match mime {
        // Images
        "image/jpeg" | "image/jpg" => Some("JPEG Image"),
        "image/png" => Some("PNG Image"),
        "image/gif" => Some("GIF Image"),
        "image/webp" => Some("WebP Image"),
        "image/bmp" | "image/x-bmp" => Some("Bitmap Image"),
        "image/tiff" | "image/x-tiff" => Some("TIFF Image"),
        "image/svg+xml" => Some("SVG Image"),
        "image/avif" => Some("AVIF Image"),
        "image/heic" | "image/heif" => Some("HEIC Image"),
        "image/vnd.microsoft.icon" | "image/ico" | "image/x-icon" => Some("ICO Image"),
        // Video
        "video/mp4" | "video/x-m4v" => Some("MPEG-4 Video"),
        "video/x-matroska" => Some("Matroska Video"),
        "video/quicktime" => Some("QuickTime Video"),
        "video/x-msvideo" => Some("AVI Video"),
        "video/webm" => Some("WebM Video"),
        "video/mpeg" | "video/x-mpeg" => Some("MPEG Video"),
        "video/ogg" | "video/x-ogm+ogg" => Some("OGG Video"),
        "video/x-ms-wmv" => Some("WMV Video"),
        // Audio
        "audio/mpeg" | "audio/mp3" | "audio/x-mp3" => Some("MP3 Audio"),
        "audio/flac" | "audio/x-flac" => Some("FLAC Audio"),
        "audio/wav" | "audio/x-wav" => Some("WAV Audio"),
        "audio/ogg" | "audio/vorbis" | "audio/x-vorbis+ogg" => Some("OGG Audio"),
        "audio/opus" | "audio/x-opus+ogg" => Some("Opus Audio"),
        "audio/aac" | "audio/x-aac" | "audio/mp4" => Some("AAC Audio"),
        "audio/x-m4a" => Some("M4A Audio"),
        "audio/x-ms-wma" => Some("WMA Audio"),
        // Documents
        "application/pdf" => Some("PDF Document"),
        "application/msword" => Some("Word Document"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            Some("Word Document")
        }
        "application/vnd.oasis.opendocument.text" => Some("ODF Text"),
        "application/vnd.ms-excel" => Some("Excel Spreadsheet"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            Some("Excel Spreadsheet")
        }
        "application/epub+zip" => Some("EPUB E-Book"),
        // Archives
        "application/zip" | "application/x-zip-compressed" => Some("ZIP Archive"),
        "application/x-tar" => Some("TAR Archive"),
        "application/gzip" | "application/x-gzip" => Some("GZip Archive"),
        "application/x-bzip2" => Some("BZip2 Archive"),
        "application/x-7z-compressed" => Some("7-Zip Archive"),
        "application/vnd.rar" | "application/x-rar-compressed" => Some("RAR Archive"),
        "application/java-archive" => Some("JAR Archive"),
        // Text / code (broad enough to be useful)
        "text/plain" => Some("Plain Text"),
        "text/html" | "text/xhtml" | "application/xhtml+xml" => Some("HTML Document"),
        "text/css" => Some("CSS Stylesheet"),
        "text/javascript" | "application/javascript" => Some("JavaScript Source"),
        "application/json" => Some("JSON Data"),
        "application/xml" | "text/xml" => Some("XML Document"),
        "text/x-python" | "application/x-python" => Some("Python Source"),
        "text/x-rust" | "application/x-rust" => Some("Rust Source"),
        "text/x-csrc" | "text/x-c" => Some("C Source"),
        "text/x-c++src" => Some("C++ Source"),
        "text/x-shellscript" | "application/x-shellscript" => Some("Shell Script"),
        "text/markdown" => Some("Markdown Text"),
        _ => None,
    }
}

