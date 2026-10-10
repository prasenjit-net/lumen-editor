mod app;
mod bootstrap;
mod icon;
mod model;
mod platform;
mod services;
mod ui;

fn main() -> eframe::Result<()> {
    bootstrap::run()
}
