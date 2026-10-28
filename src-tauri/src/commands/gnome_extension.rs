//! The Hindsight GNOME extension: its state and turning it on. Only Linux has these commands.

use std::time::Duration;

use crate::capture::focus::{self, GnomeExtensionStatus};

/// After turning the extension on, how many times and how far apart to check that it runs: up to
/// 1 second in all
const ENABLE_CHECKS: u32 = 10;
const ENABLE_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// Returns the state of the Hindsight GNOME extension. The frontend shows a prompt when it is not
/// `ready`.
#[tauri::command]
pub async fn get_gnome_extension_status() -> Result<GnomeExtensionStatus, String> {
    tokio::task::spawn_blocking(focus::gnome_extension_status)
        .await
        .map_err(|e| e.to_string())
}

/// Turns on the Hindsight GNOME extension and returns its state afterwards.
#[tauri::command]
pub async fn enable_gnome_extension() -> Result<GnomeExtensionStatus, String> {
    tokio::task::spawn_blocking(|| -> crate::error::Result<GnomeExtensionStatus> {
        focus::enable_gnome_extension()?;
        let mut status = focus::gnome_extension_status();
        // GNOME may start the extension a moment later
        for _ in 0..ENABLE_CHECKS {
            if status != GnomeExtensionStatus::Off {
                break;
            }
            std::thread::sleep(ENABLE_CHECK_INTERVAL);
            status = focus::gnome_extension_status();
        }
        Ok(status)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
