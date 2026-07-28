//! Archive (zip/tar/7z/rar) compress & extract helpers, split out of main_window.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug)]
pub(super) struct ArchiveOpResult {
    pub(super) source_paths: Vec<PathBuf>,
    pub(super) output_path: Option<PathBuf>,
    pub(super) error: Option<String>,
}

impl ArchiveOpResult {
    pub(super) fn success(source_paths: Vec<PathBuf>, output_path: PathBuf) -> Self {
        Self {
            source_paths,
            output_path: Some(output_path),
            error: None,
        }
    }

    pub(super) fn failure(error: impl Into<String>) -> Self {
        Self {
            source_paths: Vec::new(),
            output_path: None,
            error: Some(error.into()),
        }
    }
}

pub(super) fn compress_paths_to_zip(
    paths: Vec<PathBuf>,
    parent: PathBuf,
    dest: PathBuf,
) -> ArchiveOpResult {
    if paths.is_empty() {
        return ArchiveOpResult::failure("No files were selected.");
    }

    let mut args = vec![OsString::from("-r"), dest.as_os_str().to_os_string()];
    args.push(OsString::from("--"));
    for path in &paths {
        match path.strip_prefix(&parent) {
            Ok(relative) if !relative.as_os_str().is_empty() => {
                args.push(relative.as_os_str().to_os_string());
            }
            _ => args.push(path.as_os_str().to_os_string()),
        }
    }

    match run_archive_command("zip", &args, Some(&parent)) {
        Ok(()) => ArchiveOpResult::success(paths, dest),
        Err(error) => ArchiveOpResult::failure(error),
    }
}

pub(super) fn extract_archive_to_folder(archive: PathBuf, dest: PathBuf) -> ArchiveOpResult {
    if let Err(error) = std::fs::create_dir_all(&dest) {
        return ArchiveOpResult::failure(format!(
            "Could not create extraction folder '{}': {error}",
            dest.display()
        ));
    }

    let Some(kind) = archive_kind(&archive) else {
        return ArchiveOpResult::failure("Unsupported archive type.");
    };

    let result = match kind {
        ArchiveKind::Zip => run_archive_command(
            "unzip",
            &[
                OsString::from("-n"),
                archive.as_os_str().to_os_string(),
                OsString::from("-d"),
                dest.as_os_str().to_os_string(),
            ],
            None,
        ),
        ArchiveKind::Tar => run_archive_command(
            "tar",
            &[
                OsString::from("-xf"),
                archive.as_os_str().to_os_string(),
                OsString::from("-C"),
                dest.as_os_str().to_os_string(),
            ],
            None,
        ),
        ArchiveKind::SevenZip => run_archive_command(
            "7z",
            &[
                OsString::from("x"),
                archive.as_os_str().to_os_string(),
                OsString::from(format!("-o{}", dest.display())),
                OsString::from("-aos"),
            ],
            None,
        ),
    };

    match result {
        Ok(()) => ArchiveOpResult::success(vec![archive], dest),
        Err(error) => {
            let _ = std::fs::remove_dir(&dest);
            ArchiveOpResult::failure(error)
        }
    }
}

fn run_archive_command(
    program: &str,
    args: &[OsString],
    current_dir: Option<&Path>,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "Required archive tool '{program}' is not installed or is not on PATH."
            ));
        }
        Err(error) => return Err(error.to_string()),
    };

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "Archive command failed."
    };
    Err(format!(
        "{program} exited with status {}: {}",
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        truncate_archive_detail(detail, 600)
    ))
}

fn truncate_archive_detail(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArchiveKind {
    Zip,
    Tar,
    SevenZip,
}

pub(super) fn archive_kind(path: &Path) -> Option<ArchiveKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".zip") || name.ends_with(".jar") || name.ends_with(".epub") {
        Some(ArchiveKind::Zip)
    } else if name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".tar.xz")
        || name.ends_with(".txz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tbz")
        || name.ends_with(".tbz2")
    {
        Some(ArchiveKind::Tar)
    } else if name.ends_with(".7z") || name.ends_with(".rar") {
        Some(ArchiveKind::SevenZip)
    } else {
        None
    }
}

pub(super) fn is_supported_archive_path(path: &Path) -> bool {
    archive_kind(path).is_some()
}

pub(super) fn archive_output_folder_name(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Archive");
    for suffix in [
        ".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".tbz", ".zip", ".7z", ".rar",
        ".jar", ".epub", ".tar",
    ] {
        if name.to_ascii_lowercase().ends_with(suffix) && name.len() > suffix.len() {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Archive")
        .to_string()
}

pub(super) fn suggested_archive_name(paths: &[PathBuf]) -> String {
    let base = if paths.len() == 1 {
        paths[0]
            .file_stem()
            .or_else(|| paths[0].file_name())
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Archive")
            .to_string()
    } else {
        "Archive".to_string()
    };
    format!("{base}.zip")
}
