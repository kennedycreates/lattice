use crate::ui::main_window::MainWindow;
use gtk::prelude::*;
use gtk::{Application, CssProvider};

pub fn on_activate(app: &Application) {
    load_css();
    let window = MainWindow::new(app);
    window.present();
}

fn load_css() {
    let provider = CssProvider::new();

    let css_path = std::path::Path::new("themes/default.css");
    if css_path.exists() {
        provider.load_from_path(css_path);
    } else {
        // Fallback: try relative to the binary location
        if let Ok(exe) = std::env::current_exe() {
            let fallback = exe
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.join("themes/default.css"));
            if let Some(path) = fallback {
                if path.exists() {
                    provider.load_from_path(&path);
                }
            }
        }
    }

    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("Could not connect to display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
