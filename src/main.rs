#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod bootstrap;
mod cli;
mod icon;
mod model;
mod platform;
mod services;
mod ui;

fn main() -> eframe::Result<()> {
    bootstrap::run()
}
