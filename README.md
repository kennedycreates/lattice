# Lattice

A mouse-first GTK4 file manager for custom Linux desktops, built with Rust, GTK4, GIO, and GLib.

Lattice is aimed at a slick, dark, cursor-driven workflow. The default view is an icon grid, not a keyboard-centric file list.

## Current Status

Lattice is currently in **Milestone 4: tabs and split-pane browsing**.

The Milestone 4 code path is implemented and compiling, but it has **not** been manually validated in a real desktop session from this headless environment yet.

Implemented now:

- Real local folder browsing through GIO async APIs
- Home / Downloads / Documents / optional Projects sidebar navigation
- Folder-first alphabetical sorting
- Hidden-file toggle
- Double-click to open folders and files
- Back / Up / Refresh navigation
- Breadcrumb-style location bar that flips into full-path editing on click
- Toggleable preview pane with real folder, image, and text/config previews
- Right-click context menus on file and folder cards
- New Folder, Rename, Move to Trash, Copy Path, and Open Terminal Here
- Real tabs with per-tab folder state
- Split-pane browsing with an active pane model

Not implemented yet:

- Search and rich previews
- Drag and drop
- Tags / project metadata / config system

## Requirements

- Rust stable `1.75+`
- GTK4 development libraries
- A Linux desktop session with working default app handlers

For the best experience, use a desktop environment or compositor with:

- default app associations for opening files
- a trash implementation
- an installed terminal emulator

### Installing GTK4 on Debian/Ubuntu

```sh
sudo apt install libgtk-4-dev
```

### Installing GTK4 on Arch Linux

```sh
sudo pacman -S gtk4
```

### Installing GTK4 on Fedora

```sh
sudo dnf install gtk4-devel
```

## Build & Run

```sh
cargo run
```

Running from the project root is recommended during development.

## Development Checks

```sh
cargo fmt --check
cargo check
```

## Project Workflow

- Read `AGENTS.md` and `JOURNAL.md` before starting work.
- Record each meaningful work session in `JOURNAL.md`.
- Keep `README.md` updated after each major step so it stays accurate and low-chaff.
- Follow `docs/agent_rules.md` and `docs/roadmap.md` when making changes.

## GitHub CI

The repo includes a minimal GitHub Actions workflow at `.github/workflows/ci.yml` that installs GTK4 development packages on Ubuntu and runs:

- `cargo fmt --check`
- `cargo check`

## Project Structure

```text
lattice/
  Cargo.toml
  Cargo.lock
  README.md
  AGENTS.md
  JOURNAL.md
  .github/
    workflows/
      ci.yml
  src/
    main.rs            — entry point, GTK application setup
    app.rs             — activation handler, CSS loading
    ui/
      mod.rs
      main_window.rs   — main window controller and file actions
      toolbar.rs       — top toolbar buttons and path display
      sidebar.rs       — left navigation sidebar
      file_grid.rs     — central icon-grid view
      preview_pane.rs  — right preview panel
      status_bar.rs    — bottom status/status-message bar
  themes/
    default.css        — dark cyberpunk CSS theme
  docs/
    product_brief.md
    roadmap.md
    agent_rules.md
```

## Roadmap

See [docs/roadmap.md](docs/roadmap.md) for the milestone plan and current status.
