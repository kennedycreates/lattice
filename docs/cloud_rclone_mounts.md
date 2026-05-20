# rclone Mount Management in Lattice

Lattice can mount and unmount rclone remotes directly from the Cloud landing view. This page explains how it works and what Lattice does (and does not) touch.

## Prerequisites

1. Install rclone: https://rclone.org/install/
2. Configure at least one remote: `rclone config`
3. Add the remote as a Cloud entry in Lattice with the **Remote** field set to the rclone remote name (e.g. `gdrive`).

Lattice never creates or edits rclone credentials. All credential setup happens outside of Lattice via `rclone config`.

## What Lattice reads from rclone

- `rclone version --quiet` — to detect that rclone is installed and get the version string.
- `rclone listremotes` — to enumerate configured remote names (no credentials, no config file contents).

Nothing else is read.

## Adding an rclone Cloud entry

1. Open the **Cloud** section in the sidebar.
2. Click **⚙ rclone Remotes** to open the rclone panel.
3. For each configured remote, click **Add to Cloud** — this pre-fills the Add Cloud Drive form with the remote name and a suggested mount path (`~/Cloud/Lattice/<remote-name>`).
4. The **Remote** field in the form should contain the rclone remote name. This is what Lattice uses as the `<remote>:` argument to `rclone mount`.

Alternatively, add a Cloud entry manually and fill in the Remote field yourself.

## Mounting

When you click **Mount** on a Cloud landing page, Lattice runs:

```
rclone mount <remote>: <path> --vfs-cache-mode writes --daemon --log-level ERROR
```

- `--vfs-cache-mode writes` — caches file writes locally before uploading; balances performance and reliability.
- `--daemon` — rclone forks into the background; Lattice does not hold the process open.
- `--log-level ERROR` — suppresses informational output; errors are captured and shown in Lattice.

Lattice then polls `<path>` for up to 10 seconds to confirm the mount is accessible. If the path does not appear within 10 seconds, Lattice reports a timeout error (the rclone daemon may still be starting up — check manually with `ls <path>`).

The mount path (`<path>`) is created automatically if it does not exist.

### Network access

Most rclone remotes require internet access at mount time. Mounting will fail if the remote is unreachable or credentials have expired. Use `rclone config reconnect <remote>:` outside Lattice to refresh credentials.

## Unmounting

When you click **Unmount**, Lattice tries the following commands in order until one succeeds:

1. `fusermount3 -u <path>`
2. `fusermount -u <path>`
3. `umount <path>`

`fusermount3` is the standard unmount utility for FUSE mounts on modern Linux. The fallbacks handle older distributions.

## Mount state

Lattice determines mount state by checking whether `<path>` exists (`Path::exists()`). It does not maintain persistent mount state — if Lattice exits and relaunches, rclone daemon mounts remain active (rclone manages its own daemon lifecycle), and Lattice will correctly detect them as mounted on the next availability check.

## Mount / Unmount button visibility

The **Mount** and **Unmount** buttons appear only on Cloud entries where:
- `kind` is `rclone`, **and**
- the **Remote** field is set (non-empty).

Manual mount entries (kind = manual, pcloud, gvfs, sftp, etc.) show no mount buttons — they are mounted externally.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| Mount fails immediately | Remote name is wrong or rclone remote is not configured |
| Mount times out (10 s) | Remote is slow to connect; check internet / credentials |
| Unmount fails | Another process has the mount open; close it and retry |
| Status shows Unavailable after mount | The mount path does not match the Cloud entry path |

Use `rclone listremotes` to verify configured remote names and `gio mount -l` to inspect active FUSE mounts.
