// Release builds attach no console window; debug builds keep one so `tracing` output
// is visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sio_desktop_lib::run();
}
