use crate::config::AppConfig;
use crate::launch::LaunchConfig;
use crate::ui::main_window::MainWindow;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, CssProvider};
use std::fs;
use std::path::{Path, PathBuf};

pub fn on_activate(app: &Application, launch: &LaunchConfig) {
    // Disable overlay scrolling so scrollbars stay at a fixed size and position.
    // Without this, GTK physically expands the scrollbar widget on hover regardless
    // of any CSS min-width overrides.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_overlay_scrolling(false);
    }
    let config = AppConfig::load();
    load_css(&config.theme);
    install_dev_assets();
    let window = MainWindow::new(app, launch, config);
    setup_icon(&window);
    window.present();
}

fn load_css(theme: &str) {
    let provider = CssProvider::new();

    if let Some(path) = find_theme(theme) {
        provider.load_from_path(&path);
    } else if theme != "default" {
        eprintln!("lattice: theme '{theme}' not found, falling back to default");
        if let Some(path) = find_theme("default") {
            provider.load_from_path(&path);
        }
    }

    let Some(display) = gtk::gdk::Display::default() else {
        eprintln!("lattice: could not connect to a display for CSS loading");
        return;
    };

    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Search for a theme CSS file by name. Resolution order:
/// 1. `~/.config/lattice/themes/<name>.css` — user override
/// 2. `themes/<name>.css` relative to CWD — dev/run-in-place
/// 3. `<exe>/../../../themes/<name>.css` — cargo build layout
/// 4. `<exe>/../share/lattice/themes/<name>.css` — installed layout
fn find_theme(theme: &str) -> Option<PathBuf> {
    let filename = format!("{theme}.css");

    let user_path = AppConfig::themes_dir().join(&filename);
    if user_path.exists() {
        return Some(user_path);
    }

    let cwd_path = PathBuf::from("themes").join(&filename);
    if cwd_path.exists() {
        return Some(cwd_path);
    }

    if let Ok(exe) = std::env::current_exe() {
        // cargo build layout: target/{debug,release}/lattice → ../../../themes/
        if let Some(dev) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("themes").join(&filename))
        {
            if dev.exists() {
                return Some(dev);
            }
        }

        // installed layout: <prefix>/bin/lattice → <prefix>/share/lattice/themes/
        if let Some(inst) = exe.parent().and_then(|p| p.parent()).map(|p| {
            p.join("share")
                .join("lattice")
                .join("themes")
                .join(&filename)
        }) {
            if inst.exists() {
                return Some(inst);
            }
        }
    }

    None
}

fn copy_icon_as_square(src: &Path, dest: &Path, size: i32) {
    let Ok(src_buf) = gdk_pixbuf::Pixbuf::from_file(src) else {
        return;
    };
    let Some(canvas) = gdk_pixbuf::Pixbuf::new(
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        size,
        size,
    ) else {
        return;
    };
    canvas.fill(0x00000000);

    let src_w = src_buf.width() as f64;
    let src_h = src_buf.height() as f64;
    let scale = (size as f64 / src_w).min(size as f64 / src_h);
    let scaled_w = (src_w * scale).round() as i32;
    let scaled_h = (src_h * scale).round() as i32;
    let offset_x = (size - scaled_w) / 2;
    let offset_y = (size - scaled_h) / 2;

    src_buf.scale(
        &canvas,
        offset_x,
        offset_y,
        scaled_w,
        scaled_h,
        offset_x as f64,
        offset_y as f64,
        scale,
        scale,
        gdk_pixbuf::InterpType::Bilinear,
    );

    let _ = canvas.savev(dest, "png", &[]);
}

fn install_dev_assets() {
    let Some(icons_dir) = find_icons_dir() else { return };
    let src_icon = icons_dir.join("lattice-icon.png");
    if !src_icon.exists() {
        return;
    }

    let data_dir = PathBuf::from(glib::user_data_dir());

    let icon_dest_dir = data_dir.join("icons/hicolor/256x256/apps");
    if fs::create_dir_all(&icon_dest_dir).is_ok() {
        let icon_dest = icon_dest_dir.join("lattice.png");
        let needs_update = !icon_dest.exists()
            || src_icon
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                > icon_dest
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok());
        if needs_update {
            copy_icon_as_square(&src_icon, &icon_dest, 256);
        }
    }

    let Some(desktop_src) = find_desktop_file() else { return };
    let apps_dir = data_dir.join("applications");
    if fs::create_dir_all(&apps_dir).is_ok() {
        let desktop_dest = apps_dir.join("com.lattice.filemanager.desktop");
        let needs_update = !desktop_dest.exists()
            || desktop_src
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                > desktop_dest
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok());
        if needs_update {
            let _ = fs::copy(&desktop_src, &desktop_dest);
        }
    }
}

/// Search for the desktop file. Resolution order mirrors find_theme:
/// 1. CWD — dev/run-in-place
/// 2. `<exe>/../../../` — cargo build layout
/// 3. `<exe>/../share/lattice/` — installed layout
fn find_desktop_file() -> Option<PathBuf> {
    const FILENAME: &str = "com.lattice.filemanager.desktop";

    let cwd_path = PathBuf::from(FILENAME);
    if cwd_path.exists() {
        return Some(cwd_path);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dev) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join(FILENAME))
        {
            if dev.exists() {
                return Some(dev);
            }
        }

        if let Some(inst) = exe
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("share").join("lattice").join(FILENAME))
        {
            if inst.exists() {
                return Some(inst);
            }
        }
    }

    None
}

fn setup_icon(window: &ApplicationWindow) {
    if let Some(icons_dir) = find_icons_dir() {
        if let Some(display) = gtk::gdk::Display::default() {
            let icon_theme = gtk::IconTheme::for_display(&display);
            icon_theme.add_search_path(icons_dir.to_string_lossy().as_ref());
        }
    }
    window.set_icon_name(Some("lattice"));
}

/// Search for the icons directory. Resolution order mirrors find_theme:
/// 1. `icons/` relative to CWD — dev/run-in-place
/// 2. `<exe>/../../../icons/` — cargo build layout
/// 3. `<exe>/../share/lattice/icons/` — installed layout
fn find_icons_dir() -> Option<PathBuf> {
    let cwd_path = PathBuf::from("icons");
    if cwd_path.join("lattice-icon.png").exists() {
        return cwd_path.canonicalize().ok().or(Some(cwd_path));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dev) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("icons"))
        {
            if dev.join("lattice-icon.png").exists() {
                return Some(dev);
            }
        }

        if let Some(inst) = exe
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("share").join("lattice").join("icons"))
        {
            if inst.join("lattice-icon.png").exists() {
                return Some(inst);
            }
        }
    }

    None
}
