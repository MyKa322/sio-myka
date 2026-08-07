//! Application wiring for the SIO desktop app.
//!
//! Deliberately thin. The composition root lives here; behaviour lives in the `sio-*`
//! crates. This is what keeps the app testable without a window and makes the
//! elevation strategy swappable in one place.

mod commands;
mod error;

pub use error::{CommandError, CommandResult};

/// Build and run the application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::system_snapshot,
            commands::app_version,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the SIO window");
}

/// Logging goes to stderr, filtered by the `SIO_LOG` environment variable.
///
/// Defaults to warnings only: this is a desktop app, and a chatty default drowns the
/// signal when someone actually needs a log to diagnose a failed install.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("SIO_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,sio_desktop_lib=info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
