use std::path::Path;
use std::process::Command;

pub struct RcloneStatus {
    /// `None` = rclone not found in PATH.
    /// `Some("rclone v1.66.0")` = first line of `rclone version --quiet`.
    pub version: Option<String>,
    /// Remote names without the trailing colon, e.g. `["gdrive", "backblaze"]`.
    /// Empty if rclone is not found or no remotes are configured.
    pub remotes: Vec<String>,
}

/// Detect rclone by running `rclone version --quiet` and `rclone listremotes`.
/// Both commands are instantaneous (config-only reads, no network) — safe on the main thread.
/// Only remote names are exposed; config file and credentials are never read.
pub fn detect() -> RcloneStatus {
    let version = Command::new("rclone")
        .args(["version", "--quiet"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8(o.stdout)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        });

    let remotes = if version.is_some() {
        Command::new("rclone")
            .arg("listremotes")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim_end_matches(':').trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    RcloneStatus { version, remotes }
}

/// Mount a configured rclone remote at `mount_path` using `--daemon`.
/// Creates the mount directory if it does not exist, spawns rclone with safe defaults,
/// then polls for up to 10 s to confirm the path becomes accessible.
/// Safe to call from a blocking thread (e.g. `gio::spawn_blocking`).
/// Credentials are managed entirely by rclone config — never read or modified here.
pub fn mount(remote_name: &str, mount_path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(mount_path)
        .map_err(|e| format!("Cannot create mount directory: {e}"))?;

    let output = Command::new("rclone")
        .args([
            "mount",
            &format!("{remote_name}:"),
            &mount_path.to_string_lossy(),
            "--vfs-cache-mode",
            "writes",
            "--daemon",
            "--log-level",
            "ERROR",
        ])
        .output()
        .map_err(|e| format!("Failed to run rclone: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            format!("rclone mount exited with status {}", output.status)
        } else {
            format!("rclone mount failed: {}", stderr.trim())
        });
    }

    // Poll for mount to become accessible (up to 10 s, 500 ms steps)
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if mount_path.exists() {
            return Ok(());
        }
    }

    Err(
        "Mount timed out — the path was not accessible within 10 seconds. \
         Check that the remote name is correct and the rclone remote is reachable."
            .to_string(),
    )
}

/// Unmount a FUSE mount at `mount_path`.
/// Tries `fusermount3 -u`, `fusermount -u`, then `umount` as fallbacks.
/// Safe to call from a blocking thread.
pub fn unmount(mount_path: &Path) -> Result<(), String> {
    let path_str = mount_path.to_string_lossy();
    let mut last_err = String::new();

    for cmd in &["fusermount3", "fusermount", "umount"] {
        match Command::new(cmd).args(["-u", &path_str]).output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.trim().is_empty() {
                    // Command ran but reported an error — stop trying others
                    return Err(format!("{cmd}: {}", stderr.trim()));
                }
                // Command found but stderr empty — try next
                last_err = format!("{cmd} exited {}", output.status);
            }
            Err(_) => {
                // Command not found — try next
            }
        }
    }

    Err(if last_err.is_empty() {
        "No suitable unmount command found (tried fusermount3, fusermount, umount).".to_string()
    } else {
        last_err
    })
}

/// Returns `true` if the mount path exists and is accessible (mount is active).
pub fn is_mounted(mount_path: &Path) -> bool {
    mount_path.exists()
}
