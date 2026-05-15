mod app;
mod ui;

use gtk::prelude::*;
use gtk::Application;

const APP_ID: &str = "com.lattice.filemanager";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(app::on_activate);
    app.run()
}
