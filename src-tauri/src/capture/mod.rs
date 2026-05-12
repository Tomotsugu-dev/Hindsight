pub mod browser_url;
pub mod privacy;
pub mod screenshot;
#[cfg(target_os = "macos")]
pub mod screenshot_macos;
pub mod service;
pub mod window;

pub use service::{CaptureService, CaptureStatus};
pub use window::WindowInfo;
