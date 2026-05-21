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
//! over D-Bus. See docs/file_picker_portal.md for design, limitations, and
//! install instructions.

use gio::prelude::*;
use gio::{
    BusNameOwnerFlags, BusType, DBusConnection, DBusMethodInvocation, DBusNodeInfo, Subprocess,
    SubprocessFlags,
};
use glib::{Variant, VariantDict};
use std::path::{Path, PathBuf};

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.lattice";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE_NAME: &str = "org.freedesktop.impl.portal.FileChooser";

const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_ERROR: u32 = 2;

const INTROSPECTION_XML: &str = r#"
<node>
  <interface name="org.freedesktop.impl.portal.FileChooser">
    <method name="OpenFile">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFile">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFiles">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
  </interface>
</node>
"#;

enum PortalCall {
    OpenFile {
        app_id: String,
        title: String,
        options: VariantDict,
    },
    SaveFile {
        app_id: String,
        title: String,
        options: VariantDict,
    },
    SaveFiles {
        app_id: String,
        title: String,
        options: VariantDict,
    },
}

impl PortalCall {
    fn parse(method: &str, parameters: &Variant) -> Option<Self> {
        let app_id = parameters.try_child_get::<String>(1).ok().flatten()?;
        let title = parameters.try_child_get::<String>(3).ok().flatten()?;
        let options = parameters
            .try_child_value(4)
            .and_then(|v| VariantDict::from_variant(&v))?;

        match method {
            "OpenFile" => Some(Self::OpenFile {
                app_id,
                title,
                options,
            }),
            "SaveFile" => Some(Self::SaveFile {
                app_id,
                title,
                options,
            }),
            "SaveFiles" => Some(Self::SaveFiles {
                app_id,
                title,
                options,
            }),
            _ => None,
        }
    }
}

async fn handle_call(call: PortalCall) -> Variant {
    match call {
        PortalCall::OpenFile {
            app_id,
            title,
            options,
        } => open_file(app_id, title, options).await,
        PortalCall::SaveFile {
            app_id,
            title,
            options,
        } => save_file(app_id, title, options).await,
        PortalCall::SaveFiles {
            app_id,
            title,
            options,
        } => save_files(app_id, title, options).await,
    }
}

async fn open_file(app_id: String, title: String, options: VariantDict) -> Variant {
    let multiple = opt_bool(&options, "multiple");
    let directory = opt_bool(&options, "directory");
    let initial_dir = opt_ay_path(&options, "current_folder");
    let accept_label = opt_str(&options, "accept_label");

    log_filters(&options);
    for key in &["choices", "writable"] {
        if options.contains(key) {
            eprintln!("[lattice-portal] note: '{key}' option not supported - ignored");
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
            return portal_result(RESPONSE_ERROR, vec![]);
        }
    };

    if !output.success {
        eprintln!(
            "[lattice-portal] picker exited {} - cancelled",
            output.exit_status
        );
        return portal_result(RESPONSE_CANCELLED, vec![]);
    }

    let uris: Vec<String> = output
        .stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let canon = std::fs::canonicalize(line).ok()?;
            path_to_file_uri(&canon)
        })
        .collect();

    if uris.is_empty() {
        eprintln!("[lattice-portal] no valid file:// URIs after normalization");
        return portal_result(RESPONSE_CANCELLED, vec![]);
    }

    eprintln!("[lattice-portal] OpenFile: returning {} URI(s)", uris.len());
    portal_result(RESPONSE_SUCCESS, uris)
}

async fn save_file(app_id: String, title: String, options: VariantDict) -> Variant {
    let initial_dir =
        opt_ay_path(&options, "current_folder").or_else(|| opt_current_file_dir(&options));
    let suggested_name = opt_str(&options, "current_name")
        .or_else(|| opt_current_file_name(&options))
        .unwrap_or_default();

    log_filters(&options);
    if options.contains("choices") {
        eprintln!("[lattice-portal] note: 'choices' option not supported - ignored");
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
            return portal_result(RESPONSE_ERROR, vec![]);
        }
    };

    if !output.success {
        eprintln!(
            "[lattice-portal] picker exited {} - cancelled",
            output.exit_status
        );
        return portal_result(RESPONSE_CANCELLED, vec![]);
    }

    let Some(uri) = output
        .stdout
        .lines()
        .find_map(|line| path_to_save_uri(Path::new(line.trim())))
    else {
        eprintln!("[lattice-portal] SaveFile: no valid save URI after normalization");
        return portal_result(RESPONSE_CANCELLED, vec![]);
    };

    eprintln!("[lattice-portal] SaveFile: returning URI {uri:?}");
    portal_result(RESPONSE_SUCCESS, vec![uri])
}

async fn save_files(app_id: String, title: String, options: VariantDict) -> Variant {
    let initial_dir = opt_ay_path(&options, "current_folder");
    let filenames = opt_save_filenames(&options);

    eprintln!(
        "[lattice-portal] SaveFiles from {app_id:?}: title={title:?} \
         initial_dir={initial_dir:?} files={filenames:?}"
    );

    let output = match spawn_picker("folder", initial_dir.as_deref(), None).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[lattice-portal] failed to spawn picker: {e}");
            return portal_result(RESPONSE_ERROR, vec![]);
        }
    };

    if !output.success {
        eprintln!(
            "[lattice-portal] picker exited {} - cancelled",
            output.exit_status
        );
        return portal_result(RESPONSE_CANCELLED, vec![]);
    }

    let Some(chosen_dir) = output
        .stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .and_then(|p| std::fs::canonicalize(p).ok())
        .filter(|p| p.is_dir())
    else {
        eprintln!("[lattice-portal] SaveFiles: no valid destination folder returned");
        return portal_result(RESPONSE_CANCELLED, vec![]);
    };

    let uris: Vec<String> = if filenames.is_empty() {
        path_to_file_uri(&chosen_dir).into_iter().collect()
    } else {
        filenames
            .iter()
            .filter_map(|name| path_to_file_uri(&chosen_dir.join(name)))
            .collect()
    };

    if uris.is_empty() {
        eprintln!("[lattice-portal] SaveFiles: URI list is empty after construction");
        return portal_result(RESPONSE_ERROR, vec![]);
    }

    eprintln!(
        "[lattice-portal] SaveFiles: returning {} URI(s)",
        uris.len()
    );
    portal_result(RESPONSE_SUCCESS, uris)
}

struct PickerOutput {
    success: bool,
    exit_status: i32,
    stdout: String,
}

/// Spawn `lattice --picker <subcommand> [--path <path>] [--name <name>]`.
async fn spawn_picker(
    subcommand: &str,
    path: Option<&Path>,
    name: Option<&str>,
) -> Result<PickerOutput, glib::Error> {
    let lattice_bin = find_lattice_bin();
    eprintln!("[lattice-portal] spawning {lattice_bin:?} --picker {subcommand}");

    let mut args = vec![
        lattice_bin.into_os_string(),
        "--picker".into(),
        subcommand.into(),
    ];
    if let Some(p) = path {
        args.push("--path".into());
        args.push(p.as_os_str().into());
    }
    if let Some(n) = name {
        args.push("--name".into());
        args.push(n.into());
    }

    let argv: Vec<&std::ffi::OsStr> = args.iter().map(|s| s.as_os_str()).collect();
    let process = Subprocess::newv(
        &argv,
        SubprocessFlags::STDOUT_PIPE | SubprocessFlags::STDERR_PIPE,
    )?;
    let (stdout, stderr) = process.communicate_utf8_future(None).await?;

    if let Some(stderr) = stderr.filter(|s| !s.trim().is_empty()) {
        eprintln!("[lattice-portal] picker stderr: {}", stderr.trim());
    }

    Ok(PickerOutput {
        success: process.is_successful(),
        exit_status: process.exit_status(),
        stdout: stdout.map(|s| s.to_string()).unwrap_or_default(),
    })
}

fn opt_bool(options: &VariantDict, key: &str) -> bool {
    options.lookup::<bool>(key).ok().flatten().unwrap_or(false)
}

fn opt_str(options: &VariantDict, key: &str) -> Option<String> {
    options.lookup::<String>(key).ok().flatten()
}

/// Decode any `ay` (null-terminated byte array) option as an absolute path.
fn opt_ay_path(options: &VariantDict, key: &str) -> Option<PathBuf> {
    let bytes = options.lookup::<Vec<u8>>(key).ok().flatten()?;
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

fn opt_current_file_dir(options: &VariantDict) -> Option<PathBuf> {
    let file = opt_ay_path(options, "current_file")?;
    let parent = file.parent().map(PathBuf::from)?;
    if parent.is_dir() {
        Some(parent)
    } else {
        None
    }
}

fn opt_current_file_name(options: &VariantDict) -> Option<String> {
    opt_ay_path(options, "current_file")
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Decode the `files` option (`aay`) into a list of bare filenames.
/// Entries that contain a path separator are rejected for safety.
fn opt_save_filenames(options: &VariantDict) -> Vec<String> {
    options
        .lookup::<Vec<Vec<u8>>>("files")
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|bytes| {
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

fn log_filters(options: &VariantDict) {
    if options.contains("filters") {
        eprintln!(
            "[lattice-portal] note: filters requested - not applied (picker has no filter bar)"
        );
    }
}

/// Encode a local absolute path as a `file://` URI, percent-encoding any byte
/// that is not in the unreserved set or `/`. Non-UTF-8 paths return `None`.
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

/// Build a `file://` URI for a save path. The file may not exist yet;
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

fn portal_result(response: u32, uris: Vec<String>) -> Variant {
    let results = VariantDict::new(None);
    if !uris.is_empty() {
        results.insert("uris", uris);
    }
    Variant::tuple_from_iter([response.to_variant(), results.end()])
}

/// Find the `lattice` executable. Prefers a sibling binary next to this
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

fn register_portal_object(connection: &DBusConnection) -> Result<(), glib::Error> {
    let node = DBusNodeInfo::for_xml(INTROSPECTION_XML)?;
    let interface = node
        .lookup_interface(INTERFACE_NAME)
        .expect("portal introspection XML must define FileChooser");

    connection
        .register_object(OBJECT_PATH, &interface)
        .method_call(
            |_connection,
             _sender,
             _object_path,
             _interface_name,
             method,
             parameters,
             invocation: DBusMethodInvocation| {
                let Some(call) = PortalCall::parse(method, &parameters) else {
                    eprintln!("[lattice-portal] unsupported or malformed method call: {method}");
                    invocation.return_result(Ok(Some(portal_result(RESPONSE_ERROR, vec![]))));
                    return;
                };

                invocation.return_future_local(async move { Ok(Some(handle_call(call).await)) });
            },
        )
        .build()?;

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[lattice-portal] EXPERIMENTAL - OpenFile + SaveFile + SaveFiles");
    eprintln!("[lattice-portal] registering {INTERFACE_NAME} on session bus");

    let main_loop = glib::MainLoop::new(None, false);
    let loop_for_lost = main_loop.clone();

    let _owner = gio::bus_own_name(
        BusType::Session,
        BUS_NAME,
        BusNameOwnerFlags::NONE,
        |connection, _name| match register_portal_object(&connection) {
            Ok(()) => eprintln!("[lattice-portal] registered object {OBJECT_PATH}"),
            Err(err) => eprintln!("[lattice-portal] failed to register object: {err}"),
        },
        |_connection, name| {
            eprintln!("[lattice-portal] acquired D-Bus name {name}");
        },
        move |_connection, name| {
            eprintln!("[lattice-portal] lost D-Bus name {name}");
            loop_for_lost.quit();
        },
    );

    eprintln!("[lattice-portal] ready - waiting for portal requests");
    main_loop.run();
    Ok(())
}

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
        assert!(uri.contains("%23"));
    }

    #[test]
    fn opt_bool_missing_key_is_false() {
        let options = VariantDict::new(None);
        assert!(!opt_bool(&options, "multiple"));
        assert!(!opt_bool(&options, "directory"));
    }

    #[test]
    fn opt_save_filenames_empty_options() {
        let options = VariantDict::new(None);
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
        let path = Path::new("/tmp/lattice-portal-test-output.txt");
        let uri = path_to_save_uri(path);
        assert!(uri.is_some());
        let uri = uri.unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("lattice-portal-test-output.txt"));
    }
}
