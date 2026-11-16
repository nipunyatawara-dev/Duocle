#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod automation;
mod config;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod desktop;
mod effect;
mod focus;
mod gesture;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform;

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    if let Err(error) = desktop::run() {
        eprintln!("Duocle: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("Duocle's desktop application is available on macOS and Windows.");
}
