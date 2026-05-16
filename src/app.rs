use crate::config::AppConfig;
use crate::launch::LaunchConfig;
use crate::ui::main_window::MainWindow;
use gtk::prelude::*;
use gtk::{Application, CssProvider};
use std::path::PathBuf;

pub fn on_activate(app: &Application, launch: &LaunchConfig) {
    // Disable overlay scrolling so scrollbars stay at a fixed size and position.
    // Without this, GTK physically expands the scrollbar widget on hover regardless
    // of any CSS min-width overrides.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_overlay_scrolling(false);
    }
    let config = AppConfig::load();
    load_css(&config.theme);
    let window = MainWindow::new(app, launch, config);
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
