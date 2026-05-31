use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_millis(180);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerroirError {
    Unavailable(String),
    Protocol(String),
    Api(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirStatus {
    #[serde(default)]
    pub daemon: Option<String>,
    #[serde(default)]
    pub workspaces: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirWorkspaceEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub water_file: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirBrokenRef {
    #[serde(default)]
    pub workspace_name: String,
    #[serde(default)]
    pub object_id: String,
    #[serde(default)]
    pub object_title: String,
    #[serde(default)]
    pub target_path: String,
    #[serde(default)]
    pub resolved_path: String,
    #[serde(default)]
    pub health: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirDoctorSummary {
    #[serde(default)]
    pub counts: TerroirDoctorCounts,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct TerroirDoctorCounts {
    #[serde(default)]
    pub info: usize,
    #[serde(default)]
    pub warning: usize,
    #[serde(default)]
    pub broken: usize,
    #[serde(default)]
    pub dangerous: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirReindexResult {
    #[serde(default)]
    pub indexed_workspaces: usize,
    #[serde(default)]
    pub errors: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirContext {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub workspaces: Vec<TerroirWorkspace>,
    #[serde(default)]
    pub palettes: Vec<TerroirPalette>,
    #[serde(default)]
    pub objects: Vec<TerroirObject>,
    #[serde(default)]
    pub refs: Vec<TerroirRef>,
    #[serde(default)]
    pub health: Option<String>,
    #[serde(default)]
    pub broken: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirWorkspace {
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub workspace_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirPalette {
    #[serde(default)]
    pub palette_id: String,
    #[serde(default)]
    pub palette_name: String,
    #[serde(default)]
    pub workspace_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirObject {
    #[serde(default)]
    pub object_id: String,
    #[serde(default)]
    pub object_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub workspace_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerroirRef {
    #[serde(default)]
    pub target_path: String,
    #[serde(default)]
    pub resolved_path: Option<String>,
    #[serde(default)]
    pub health: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse {
    ok: bool,
    result: Option<Value>,
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    code: String,
    message: String,
}

pub fn status() -> Result<TerroirStatus, TerroirError> {
    let result = call("status", json!({}))?;
    serde_json::from_value(result).map_err(|error| TerroirError::Protocol(error.to_string()))
}

pub fn context_for_path(path: &Path) -> Result<TerroirContext, TerroirError> {
    let result = call(
        "context_for_path",
        json!({ "path": path.to_string_lossy().to_string() }),
    )?;
    serde_json::from_value(result).map_err(|error| TerroirError::Protocol(error.to_string()))
}

pub fn list_workspaces() -> Result<Vec<TerroirWorkspaceEntry>, TerroirError> {
    let result = call("list_workspaces", json!({}))?;
    let workspaces = result
        .get("workspaces")
        .cloned()
        .unwrap_or_else(|| json!([]));
    serde_json::from_value(workspaces).map_err(|error| TerroirError::Protocol(error.to_string()))
}

pub fn list_palettes() -> Result<Vec<TerroirPalette>, TerroirError> {
    let result = call("list_palettes", json!({}))?;
    let palettes = result.get("palettes").cloned().unwrap_or_else(|| json!([]));
    serde_json::from_value(palettes).map_err(|error| TerroirError::Protocol(error.to_string()))
}

pub fn broken_refs() -> Result<Vec<TerroirBrokenRef>, TerroirError> {
    let result = call("broken_refs", json!({}))?;
    let refs = result.get("refs").cloned().unwrap_or_else(|| json!([]));
    serde_json::from_value(refs).map_err(|error| TerroirError::Protocol(error.to_string()))
}

pub fn doctor_summary() -> Result<TerroirDoctorSummary, TerroirError> {
    let result = call("doctor_summary", json!({}))?;
    serde_json::from_value(result).map_err(|error| TerroirError::Protocol(error.to_string()))
}

pub fn reindex() -> Result<TerroirReindexResult, TerroirError> {
    let result = call("reindex", json!({}))?;
    serde_json::from_value(result).map_err(|error| TerroirError::Protocol(error.to_string()))
}

fn call(method: &str, params: Value) -> Result<Value, TerroirError> {
    let mut stream = UnixStream::connect(socket_path())
        .map_err(|error| TerroirError::Unavailable(error.to_string()))?;
    stream
        .set_read_timeout(Some(REQUEST_TIMEOUT))
        .map_err(|error| TerroirError::Unavailable(error.to_string()))?;
    stream
        .set_write_timeout(Some(REQUEST_TIMEOUT))
        .map_err(|error| TerroirError::Unavailable(error.to_string()))?;

    let request = json!({ "method": method, "params": params }).to_string();
    stream
        .write_all(request.as_bytes())
        .map_err(|error| TerroirError::Unavailable(error.to_string()))?;
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| TerroirError::Unavailable(error.to_string()))?;
    parse_response(&response)
}

fn parse_response(response: &str) -> Result<Value, TerroirError> {
    let response: ApiResponse = serde_json::from_str(response)
        .map_err(|error| TerroirError::Protocol(error.to_string()))?;
    if response.ok {
        Ok(response.result.unwrap_or_else(|| json!({})))
    } else if let Some(error) = response.error {
        Err(TerroirError::Api(format!(
            "{}: {}",
            error.code, error.message
        )))
    } else {
        Err(TerroirError::Protocol(
            "Terroir returned an error without details.".to_string(),
        ))
    }
}

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("watercolor")
        .join("terroir")
        .join("terroir.sock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::thread;

    #[test]
    fn parses_context_for_path_response() {
        let response = r#"{
            "ok": true,
            "result": {
                "path": "/tmp/example.txt",
                "workspaces": [{"workspace_id": "wcw_demo", "workspace_name": "Demo"}],
                "palettes": [{"palette_id": "wcp_main", "palette_name": "Main", "workspace_name": "Demo"}],
                "objects": [{"object_id": "wco_note", "object_type": "note", "title": "Brief", "workspace_name": "Demo"}],
                "refs": [{"target_path": "/tmp/example.txt", "resolved_path": "/tmp/example.txt", "health": "ok"}],
                "health": "ok",
                "broken": false
            }
        }"#;

        let value = parse_response(response).expect("response");
        let context: TerroirContext = serde_json::from_value(value).expect("context");

        assert_eq!(context.workspaces[0].workspace_name, "Demo");
        assert_eq!(context.palettes[0].palette_name, "Main");
        assert_eq!(context.objects[0].title, "Brief");
    }

    #[test]
    fn unavailable_socket_is_graceful() {
        let result = UnixStream::connect("/tmp/lattice-terroir-test-missing.sock")
            .map_err(|error| TerroirError::Unavailable(error.to_string()));

        assert!(matches!(result, Err(TerroirError::Unavailable(_))));
    }

    #[test]
    fn error_response_is_non_panicking() {
        let temp = tempfile::tempdir().expect("tempdir");
        let socket = temp.path().join("terroir.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let socket_for_client = socket.clone();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = String::new();
            let _ = stream.read_to_string(&mut request);
            let _ = stream.write_all(
                br#"{"ok":false,"error":{"code":"unknown_method","message":"Unknown method"}}"#,
            );
        });

        let mut stream = UnixStream::connect(socket_for_client).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("read timeout");
        stream
            .write_all(b"{\"method\":\"x\",\"params\":{}}")
            .expect("write");
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        handle.join().expect("join");
        let parsed = parse_response(&response);
        let _ = fs::remove_file(socket);

        assert!(matches!(parsed, Err(TerroirError::Api(_))));
    }

    #[test]
    fn context_response_tolerates_missing_optional_arrays() {
        let response = r#"{
            "ok": true,
            "result": {
                "path": "/tmp/example.txt"
            }
        }"#;

        let value = parse_response(response).expect("response");
        let context: TerroirContext = serde_json::from_value(value).expect("context");

        assert_eq!(context.path, "/tmp/example.txt");
        assert!(context.workspaces.is_empty());
        assert!(context.palettes.is_empty());
        assert!(context.objects.is_empty());
        assert!(context.refs.is_empty());
        assert!(!context.broken);
    }

    #[test]
    fn list_item_response_tolerates_missing_fields() {
        let response = r#"{
            "ok": true,
            "result": {
                "refs": [{}],
                "palettes": [{}],
                "workspaces": [{}]
            }
        }"#;
        let value = parse_response(response).expect("response");
        let refs: Vec<TerroirBrokenRef> =
            serde_json::from_value(value["refs"].clone()).expect("refs");
        let palettes: Vec<TerroirPalette> =
            serde_json::from_value(value["palettes"].clone()).expect("palettes");
        let workspaces: Vec<TerroirWorkspaceEntry> =
            serde_json::from_value(value["workspaces"].clone()).expect("workspaces");

        assert_eq!(refs[0].health, "");
        assert_eq!(palettes[0].palette_name, "");
        assert_eq!(workspaces[0].state, "");
    }
}
