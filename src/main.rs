mod action_plan;
mod app;
mod config;
mod converter;
mod launch;
mod metadata;
mod rclone;
mod thumbnail;
mod ui;
mod view_state;

use gtk::prelude::*;
use gtk::Application;

const APP_ID: &str = "com.lattice.filemanager";

fn main() -> glib::ExitCode {
    glib::set_application_name("Lattice");
    let launch_mode = launch::LaunchMode::from_env();
    let app = Application::builder().application_id(APP_ID).build();
    match launch_mode {
        launch::LaunchMode::Browser(launch) => {
            app.connect_activate(move |app| app::on_activate(app, &launch));
        }
        launch::LaunchMode::Picker(picker_launch) => {
            app.connect_activate(move |app| app::on_activate_picker(app, &picker_launch));
        }
    }
    // Pass only the binary name to GTK so our custom flags don't
    // trigger GLib's unknown-argument warnings.
    let prog = std::env::args().next().unwrap_or_default();
    app.run_with_args(&[prog])
}
