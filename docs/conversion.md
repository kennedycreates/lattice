# Lattice — Media Conversion

## User Guide

### What it does

The Convert feature lets you batch-convert image, audio, and video files to a different format. Right-click any file selection that includes media files and choose **Convert…** to open the conversion panel.

Originals are **never modified**. Conversion always writes new output files. You can safely convert in place.

### Required tools

Conversion runs external command-line tools. Lattice does not ship any codecs or encoders itself.

| Tool | Used for | Install (Ubuntu) | Install (Arch / CachyOS) |
|------|----------|------------------|--------------------------|
| **ffmpeg** | Most image, audio, and video presets | `sudo apt install ffmpeg` | `sudo pacman -S ffmpeg` |
| **ImageMagick** | AVIF output (`to_avif` preset) | `sudo apt install imagemagick` | `sudo pacman -S imagemagick` |
| **vips** | (reserved, not yet exposed in presets) | `sudo apt install libvips-tools` | `sudo pacman -S libvips` |

ffmpeg is required for almost everything. Install it first. If a preset's required tool is missing, the panel shows a warning and disables the Convert button for that preset.

### Supported formats

Formats depend on what your installed version of ffmpeg (or ImageMagick) supports. Lattice does not impose its own format restrictions beyond the preset list.

Common supported inputs: JPEG, PNG, WebP, GIF, TIFF, BMP, HEIC, AVIF, WAV, FLAC, MP3, M4A, AAC, OGG, Opus, MP4, MOV, MKV, WebM, AVI, M4V.

Files with unrecognised extensions, or files whose extension does not match the selected preset's media kind, are shown as skipped in the preview table and are not touched.

### Output location

| Option | Where output goes |
|--------|-------------------|
| **Next to originals** | Same folder as each source file |
| **Converted subfolder** | `Converted/` subfolder inside each source file's folder (created automatically) |
| **Choose folder…** | A single folder you pick with the native file picker |

### Conflict policy

| Option | What happens when the output path already exists |
|--------|--------------------------------------------------|
| **Auto-rename** *(default)* | Appends ` 2`, ` 3`, … before the extension |
| **Skip existing** | The file is left as-is; the conversion job is marked skipped |
| **Overwrite** | The existing file is replaced |

### Progress and errors

Once you click **Convert**, a progress panel slides up at the bottom of the window. You can continue browsing while conversions run.

- **Cancel** — stops the batch after the currently-running jobs finish their 50 ms poll cycle. Already-completed files are kept.
- **Retry Failed** — appears after the batch finishes if any jobs failed. Click it to re-queue all failed jobs.
- **Open Output** — navigates the active pane to the output directory.
- **Dismiss** — closes the panel. The panel auto-dismisses after 4 seconds if all files succeeded.

Failed jobs show expandable error detail. Click **Copy error** to copy the full tool output to the clipboard for troubleshooting.

### Settings persistence

The last-used preset (per media kind), output location mode, and conflict policy are remembered in `~/.config/lattice/convert_settings.toml`. The chosen-folder path is not persisted — it resets to "Next to originals" on next launch.

### Temporary files

During conversion, each job writes to a hidden `.lattice_converting_<id>.<ext>` file in the output directory. On success, this file is atomically renamed to the final destination. On failure or cancellation, the temp file is deleted.

If Lattice crashes mid-conversion, orphaned `.lattice_converting_*` files may remain. Lattice scans common user directories (Downloads, Pictures, Videos, etc.) on startup and removes any it finds.

---

## Developer Reference

### Module architecture

```
src/converter/
  mod.rs        — Public types, preset table, tool detection, batch planning
  command.rs    — Shell-free command building, subprocess execution, temp/finalize helpers
  progress.rs   — ffmpeg -progress pipe:1 output parser, ffprobe duration probe
  queue.rs      — Background conversion queue, concurrency limits, per-job callbacks
  settings.rs   — Lightweight settings persistence (load/save convert_settings.toml)
```

### Preset table

All presets live in `PRESETS: &[ConversionPreset]` in `src/converter/mod.rs`. Each preset is a static struct:

```rust
ConversionPreset {
    id: &'static str,       // unique slug, used in settings and skip messages
    label: &'static str,    // shown in the dropdown
    kind: MediaKind,        // Image | Audio | Video
    tool: ConversionTool,   // Ffmpeg | ImageMagick | Vips
    ext: &'static str,      // output extension without dot
    ffmpeg_args: &'static [&'static str], // args between -i <src> and <dest>
}
```

### Adding a new preset

1. Add a new `ConversionPreset` entry to `PRESETS` in `mod.rs`.
2. Set `tool` to the appropriate `ConversionTool` variant.
3. For ffmpeg presets: add the required ffmpeg args to `ffmpeg_args`. These are inserted between `-i <source>` and `<dest>` in the command, with `-y -loglevel error -progress pipe:1` prepended automatically.
4. For ImageMagick: set `tool: ConversionTool::ImageMagick`. The command builder runs `magick convert <src> <dest>` (with IM6 `convert` fallback).
5. For vips: set `tool: ConversionTool::Vips`. The command builder runs `vips copy <src> <dest>`.
6. No UI changes needed — the dropdown is built from `all_presets()` at runtime.

Do not add custom presets configurable by the user until a future milestone explicitly includes that scope.

### Tool detection

`detect_tools() -> ToolAvailability` probes each tool by running it with `-version` (or `--vips-version` for vips). This runs synchronously at queue construction time, which happens once on window open. Results are stored in `ConversionQueue::tools` and passed to `plan_batch` and the UI panel.

The detection is injectable for tests via `detect_tools_with(prober: ToolProber)` where `ToolProber = fn(&str) -> bool`.

ImageMagick has two supported CLI forms: IM7 (`magick convert …`) and IM6 (`convert …`). Detection checks `magick` first, then `convert`. `RealRunner` handles the IM7→IM6 fallback at execution time: if `magick` returns `ToolNotFound`, it retries with `convert` and drops the leading `convert` sub-command argument.

### Queue execution

`ConversionQueue` is an Rc-based struct that lives in `BrowserController`. It is **not** thread-safe by itself — all mutation happens on the GTK main thread.

Job execution flow:

1. `enqueue_jobs(jobs, ops_id)` — pushes jobs into the `VecDeque`, sets up `BatchProgress`, calls `pump()`.
2. `pump()` loops to find a job whose media kind has a free concurrency slot (4 image, 1 A/V), claims it, and launches an async block via `glib::MainContext::spawn_local`.
3. The async block: (a) runs ffprobe off-thread to get duration; (b) starts a 200 ms timer on the main thread to parse ffmpeg progress output; (c) runs the conversion command off-thread via `gio::spawn_blocking`; (d) on completion, finalizes or cleans up, fires callbacks, releases the slot, calls `pump()` again.
4. Cancellation uses an `Arc<AtomicBool>`. The blocking thread polls it every 50 ms. On cancel, `kill()` is called on the child process.

Callbacks (all fire on the main thread):
- `connect_progress(op_id, fraction, filename)` — periodic + per-job completion, drives OpsPanel
- `connect_done(op_id, errors)` — fires when the entire batch reaches a terminal state
- `connect_batch_progress(BatchProgress)` — fires on every job state change
- `connect_job_status(job_id, status)` — fires on every job transition
- `connect_job_progress(job_id, fraction)` — fires periodically during A/V encoding

### Temp file naming

Temp files use the pattern `.lattice_converting_<job_id>.<ext>` in the same directory as the destination file. The leading `.` makes them hidden on Linux. The `job_id` ensures uniqueness within a batch. On startup, `cleanup_orphaned_temps_in(dir)` removes any matching files from common user directories.

### Error formatting

Raw tool stderr passes through `format_job_error(stderr)` in `mod.rs` before being stored in `ConversionJobStatus::Failed`. This function detects common patterns (permission denied, missing codec, corrupted file, no disk space) and prepends a human-readable summary sentence. Unknown patterns pass through unchanged.
