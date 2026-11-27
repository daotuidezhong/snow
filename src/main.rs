#![cfg_attr(windows, windows_subsystem = "windows")]

mod art;
mod config;
mod pile;
mod snow;
mod visitors;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod scan;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Snow runs on Windows only. On other systems you can still run `cargo test`.");
}
