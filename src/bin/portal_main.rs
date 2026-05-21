//! EXPERIMENTAL xdg-desktop-portal FileChooser backend for Lattice.
//!
//! Implements: org.freedesktop.impl.portal.FileChooser
//!   - OpenFile  ✅  (single file, multi-file, directory)
//!   - SaveFile  ✅  (save path with suggested name)
//!   - SaveFiles ✅  (folder-picker destination for a list of files)
//!
//! D-Bus name:  org.freedesktop.impl.portal.desktop.lattice
//! Object path: /org/freedesktop/portal/desktop
//!
//! Delegates all UI to `lattice --picker` subprocess; marshals result back
//! over D-Bus.  See docs/file_picker_portal.md for design, limitations, and
//! install instructions.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

// ── Response codes (org.freedesktop.impl.portal spec) ─────────────────────────

const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_ERROR: u32 = 2;

type PortalResult = (u32, HashMap<String, OwnedValue>);

// ── D-Bus interface ───────────────────────────────────────────────────────────

struct FileChooserPortal;

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserPortal {
    /// OpenFile: present the Lattice file/folder picker and return selected URIs.
    ///
    /// Supported options:
    ///   multiple (b), directory (b), current_folder (ay), accept_label (s — logged only)
    /// Logged but not applied:
    ///   filters (a(sa(us))), choices, writable
    async fn open_file(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> PortalResult {
        let multiple = opt_bool(&options, "multiple");
        let directory = opt_bool(&options, "directory");
        let initial_dir = opt_ay_path(&options, "current_folder");
        let accept_label = opt_str(&options, "accept_label");

        log_filters(&options);
        for key in &["choices", "writable"] {
            if options.contains_key(*key) {
                eprintln!("[lattice-portal] note: '{key}' option not supported — ignored");
            }
        }

        eprintln!(
            "[lattice-portal] OpenFile from {app_id:?}: title={title:?} \
             multiple={multiple} directory={directory} \
             initial_dir={initial_dir:?} accept_label={accept_label:?}"
        );

        let subcommand = if directory {
            "folder"
        } else if multiple {
            "open-files"
        } else {
            "open"
        };

        let output = match spawn_picker(subcommand, initial_dir.as_deref(), None).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("[lattice-portal] failed to spawn picker: {e}");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };

        if !output.status.success() {
            eprintln!(
                "[lattice-portal] picker exited {} — cancelled",
                output.status
            );
            return (RESPONSE_CANCELLED, HashMap::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let uris: Vec<String> = stdout
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                // Files must exist for open; canonicalize resolves symlinks and verifies.
                let canon = std::fs::canonicalize(line).ok()?;
                path_to_file_uri(&canon)
            })
            .collect();

        if uris.is_empty() {
            eprintln!("[lattice-portal] no valid file:// URIs after normalization");
            return (RESPONSE_CANCELLED, HashMap::new());
        }

        eprintln!("[lattice-portal] OpenFile: returning {} URI(s)", uris.len());
        uris_result(uris)
    }

    /// SaveFile: let the user choose a save location and filename.
    ///
    /// Supported options:
    ///   current_folder (ay) — initial directory
    ///   current_file   (ay) — fallback: use parent dir + filename if current_folder absent
    ///   current_name   (s)  — suggested filename (overrides current_file name)
    /// Logged but not applied:
    ///   filters (a(sa(us))), choices
    async fn save_file(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> PortalResult {
        // current_folder takes priority; fall back to parent dir of current_file
        let initial_dir =
            opt_ay_path(&options, "current_folder").or_else(|| opt_current_file_dir(&options));

        // current_name takes priority; fall back to filename of current_file
        let suggested_name = opt_str(&options, "current_name")
            .or_else(|| opt_current_file_name(&options))
            .unwrap_or_default();

        log_filters(&options);
        if options.contains_key("choices") {
            eprintln!("[lattice-portal] note: 'choices' option not supported — ignored");
        }

        eprintln!(
            "[lattice-portal] SaveFile from {app_id:?}: title={title:?} \
             initial_dir={initial_dir:?} suggested_name={suggested_name:?}"
        );

        let name_arg = if suggested_name.is_empty() {
            None
        } else {
            Some(suggested_name.as_str())
        };

        let output = match spawn_picker("save", initial_dir.as_deref(), name_arg).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("[lattice-portal] failed to spawn picker: {e}");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };

        if !output.status.success() {
            eprintln!(
                "[lattice-portal] picker exited {} — cancelled",
                output.status
            );
            return (RESPONSE_CANCELLED, HashMap::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let uris: Vec<String> = stdout
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                // Save paths may not exist yet; canonicalize parent only.
                path_to_save_uri(Path::new(line))
            })
            .take(1) // SaveFile returns exactly one URI
            .collect();

        if uris.is_empty() {
            eprintln!("[lattice-portal] SaveFile: no valid save URI after normalization");
            return (RESPONSE_CANCELLED, HashMap::new());
        }

        eprintln!("[lattice-portal] SaveFile: returning URI {:?}", uris[0]);
        uris_result(uris)
    }

    /// SaveFiles: let the user choose a destination folder for a list of files.
    ///
    /// Supported options:
    ///   current_folder (ay)  — initial directory for the folder picker
    ///   files          (aay) — list of filenames to save
    ///                          URIs returned are folder/each-filename
    ///                          If absent, returns the chosen folder URI itself.
    /// Logged but not applied:
    ///   filters (a(sa(us)))
    async fn save_files(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> PortalResult {
        let initial_dir = opt_ay_path(&options, "current_folder");
        let filenames = opt_save_filenames(&options);

        log_filters(&options);

        eprintln!(
            "[lattice-portal] SaveFiles from {app_id:?}: title={title:?} \
             initial_dir={initial_dir:?} files={filenames:?}"
        );

        let output = match spawn_picker("folder", initial_dir.as_deref(), None).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("[lattice-portal] failed to spawn picker: {e}");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };

        if !output.status.success() {
            eprintln!(
                "[lattice-portal] picker exited {} — cancelled",
                output.status
            );
            return (RESPONSE_CANCELLED, HashMap::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let chosen_dir = match stdout.lines().find_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let path = PathBuf::from(line);
            std::fs::canonicalize(&path).ok()
        }) {
            Some(p) => p,
            None => {
                eprintln!("[lattice-portal] SaveFiles: no valid destination folder returned");
                return (RESPONSE_CANCELLED, HashMap::new());
            }
        };

        let uris: Vec<String> = if filenames.is_empty() {
            // No files list: return the folder itself
            path_to_file_uri(&chosen_dir).into_iter().collect()
        } else {
            // Construct one URI per filename inside the chosen folder
            filenames
                .iter()
                .filter_map(|name| path_to_file_uri(&chosen_dir.join(name)))
                .collect()
        };

        if uris.is_empty() {
            eprintln!("[lattice-portal] SaveFiles: URI list is empty after construction");
            return (RESPONSE_ERROR, HashMap::new());
        }

        eprintln!(
            "[lattice-portal] SaveFiles: returning {} URI(s)",
            uris.len()
        );
        uris_result(uris)
    }
}

// ── Subprocess helper ─────────────────────────────────────────────────────────

/// Spawn `lattice --picker <subcommand> [--path <path>] [--name <name>]`.
async fn spawn_picker(
    subcommand: &str,
    path: Option<&Path>,
    name: Option<&str>,
) -> std::io::Result<std::process::Output> {
    let lattice_bin = find_lattice_bin();
    eprintln!("[lattice-portal] spawning {lattice_bin:?} --picker {subcommand}");
    let mut cmd = tokio::process::Command::new(&lattice_bin);
    cmd.arg("--picker").arg(subcommand);
    if let Some(p) = path {
        cmd.arg("--path").arg(p);
    }
    if let Some(n) = name {
        cmd.arg("--name").arg(n);
    }
    cmd.output().await
}

// ── Option helpers ────────────────────────────────────────────────────────────
// OwnedValue does not implement Clone; access the inner Value via Deref.

fn opt_bool(options: &HashMap<String, OwnedValue>, key: &str) -> bool {
    options
        .get(key)
        .and_then(|v| match v.deref() {
            Value::Bool(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false)
}

fn opt_str(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options.get(key).and_then(|v| match v.deref() {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    })
}

/// Decode any `ay` (null-terminated byte array) option as an absolute path.
fn opt_ay_path(options: &HashMap<String, OwnedValue>, key: &str) -> Option<PathBuf> {
    let val = options.get(key)?;
    let bytes: Vec<u8> = match val.deref() {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| match v {
                Value::U8(b) => Some(*b),
                _ => None,
            })
            .collect(),
        _ => return None,
    };
    let trimmed: Vec<u8> = bytes.into_iter().take_while(|&b| b != 0).collect();
    let s = String::from_utf8(trimmed).ok()?;
    if s.is_empty() {
        return None;
    }
    let path = PathBuf::from(&s);
    if path.is_absolute() {
        Some(path)
    } else {
        None
    }
}

/// Get the parent directory of `current_file`, if that directory exists.
fn opt_current_file_dir(options: &HashMap<String, OwnedValue>) -> Option<PathBuf> {
    let file = opt_ay_path(options, "current_file")?;
    let parent = file.parent().map(PathBuf::from)?;
    if parent.is_dir() {
        Some(parent)
    } else {
        None
    }
}

/// Get the filename component of `current_file`.
fn opt_current_file_name(options: &HashMap<String, OwnedValue>) -> Option<String> {
    opt_ay_path(options, "current_file")
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Decode the `files` option (`aay`) into a list of bare filenames.
/// Entries that contain a path separator are rejected for safety.
fn opt_save_filenames(options: &HashMap<String, OwnedValue>) -> Vec<String> {
    let Some(val) = options.get("files") else {
        return vec![];
    };
    let outer = match val.deref() {
        Value::Array(a) => a,
        _ => return vec![],
    };
    outer
        .iter()
        .filter_map(|item| {
            let inner = match item {
                Value::Array(a) => a,
                _ => return None,
            };
            let bytes: Vec<u8> = inner
                .iter()
                .filter_map(|v| match v {
                    Value::U8(b) => Some(*b),
                    _ => None,
                })
                .collect();
            let trimmed: Vec<u8> = bytes.into_iter().take_while(|&b| b != 0).collect();
            let s = String::from_utf8(trimmed).ok()?;
            if s.is_empty() || s.contains('/') {
                None
            } else {
                Some(s)
            }
        })
        .collect()
}

/// Decode and log `filters` (a(sa(us))): names are printed; values are never applied.
fn log_filters(options: &HashMap<String, OwnedValue>) {
    let Some(val) = options.get("filters") else {
        return;
    };
    let outer = match val.deref() {
        Value::Array(a) => a,
        _ => {
            eprintln!("[lattice-portal] note: unrecognized 'filters' format — ignored");
            return;
        }
    };
    let names: Vec<String> = outer
        .iter()
        .filter_map(|item| match item {
            Value::Structure(s) => match s.fields().first() {
                Some(Value::Str(name)) => Some(name.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    if names.is_empty() {
        eprintln!("[lattice-portal] note: 'filters' option present but no names decoded — ignored");
    } else {
        eprintln!(
            "[lattice-portal] note: filters requested: {names:?} — not applied (picker has no filter bar)"
        );
    }
}

// ── URI helpers ───────────────────────────────────────────────────────────────

/// Encode a local absolute path as a `file://` URI, percent-encoding any byte
/// that is not in the unreserved set or `/`.  Non-UTF-8 paths return `None`.
fn path_to_file_uri(path: &Path) -> Option<String> {
    let path_str = path.to_str()?;
    let mut uri = String::with_capacity(path_str.len() + 7);
    uri.push_str("file://");
    for &b in path_str.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(b as char)
            }
            other => {
                use std::fmt::Write as _;
                let _ = write!(uri, "%{other:02X}");
            }
        }
    }
    Some(uri)
}

/// Build a `file://` URI for a save path.  The file may not exist yet;
/// only the parent directory is canonicalized (must exist).
fn path_to_save_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let parent = path.parent()?;
    let filename = path.file_name()?;
    let canon_parent = std::fs::canonicalize(parent).ok()?;
    path_to_file_uri(&canon_parent.join(filename))
}

/// Wrap a list of URI strings into a success `PortalResult`.
fn uris_result(uris: Vec<String>) -> PortalResult {
    let val: Value<'static> = uris.into();
    match OwnedValue::try_from(val).ok() {
        Some(uris_val) => {
            let mut results = HashMap::new();
            results.insert("uris".to_string(), uris_val);
            (RESPONSE_SUCCESS, results)
        }
        None => {
            eprintln!("[lattice-portal] internal error: failed to build D-Bus result value");
            (RESPONSE_ERROR, HashMap::new())
        }
    }
}

// ── Binary discovery ──────────────────────────────────────────────────────────

/// Find the `lattice` executable.  Prefers a sibling binary next to this
/// process (installed case); falls back to searching `$PATH`.
fn find_lattice_bin() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("lattice");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("lattice")
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[lattice-portal] EXPERIMENTAL — OpenFile + SaveFile + SaveFiles");
    eprintln!(
        "[lattice-portal] registering org.freedesktop.impl.portal.FileChooser on session bus"
    );

    let _conn = zbus::connection::Builder::session()?
        .name("org.freedesktop.impl.portal.desktop.lattice")?
        .serve_at("/org/freedesktop/portal/desktop", FileChooserPortal)?
        .build()
        .await?;

    eprintln!("[lattice-portal] ready — waiting for portal requests");
    std::future::pending::<()>().await;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_ascii_path() {
        let path = Path::new("/home/user/documents/report.pdf");
        assert_eq!(
            path_to_file_uri(path),
            Some("file:///home/user/documents/report.pdf".into())
        );
    }

    #[test]
    fn file_uri_encodes_spaces() {
        let path = Path::new("/home/user/my documents/file.txt");
        assert_eq!(
            path_to_file_uri(path),
            Some("file:///home/user/my%20documents/file.txt".into())
        );
    }

    #[test]
    fn file_uri_encodes_special_chars() {
        let path = Path::new("/home/user/file#1.txt");
        let uri = path_to_file_uri(path).unwrap();
        assert!(uri.starts_with("file://"));
        assert!(uri.contains("%23")); // '#' → %23
    }

    #[test]
    fn opt_bool_missing_key_is_false() {
        let options: HashMap<String, OwnedValue> = HashMap::new();
        assert!(!opt_bool(&options, "multiple"));
        assert!(!opt_bool(&options, "directory"));
    }

    #[test]
    fn opt_save_filenames_empty_options() {
        let options: HashMap<String, OwnedValue> = HashMap::new();
        assert!(opt_save_filenames(&options).is_empty());
    }

    #[test]
    fn path_to_save_uri_rejects_relative() {
        let path = Path::new("relative/path/file.txt");
        assert!(path_to_save_uri(path).is_none());
    }

    #[test]
    fn path_to_save_uri_requires_parent_to_exist() {
        let path = Path::new("/nonexistent/directory/file.txt");
        assert!(path_to_save_uri(path).is_none());
    }

    #[test]
    fn path_to_save_uri_tmp() {
        // /tmp always exists; the file itself need not
        let path = Path::new("/tmp/lattice-portal-test-output.txt");
        let uri = path_to_save_uri(path);
        assert!(uri.is_some());
        let uri = uri.unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("lattice-portal-test-output.txt"));
    }
}
