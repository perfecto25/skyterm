mod app;
mod font;
mod input;
mod pty;
mod renderer;

use gtk4::prelude::*;

fn main() {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    let application = gtk4::Application::builder()
        .application_id("dev.skyterm.Skyterm")
        .build();
    application.connect_activate(app::on_activate);
    application.run();
}
