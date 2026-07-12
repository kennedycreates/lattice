// `type_complexity` and `too_many_arguments` fire constantly on idiomatic GTK
// code — callback fields typed `RefCell<Option<Box<dyn Fn(..)>>>` and widget
// builder/loader functions that take several callbacks. The suggested "fixes"
// (a type alias per closure, a params struct per builder) tend to reduce
// clarity here, so we allow these two crate-wide rather than scatter
// suppressions. All other clippy lints are kept on.
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]

#[macro_use]
mod logging;
mod action_plan;
mod app;
mod config;
mod converter;
mod launch;
mod metadata;
mod rclone;
mod terroir_client;
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
