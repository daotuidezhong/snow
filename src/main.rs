#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod snow;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod render;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Snow runs on Windows only. On other systems you can still run `cargo test`.");
}
