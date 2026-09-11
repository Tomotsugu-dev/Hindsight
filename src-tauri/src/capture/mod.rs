pub mod browser_url;
#[cfg(target_os = "macos")]
pub mod bundle;
pub mod ignore;
pub mod screenshot;
#[cfg(target_os = "macos")]
pub mod screenshot_macos;
pub mod screenshot_policy;
pub mod service;
pub mod window;

pub use service::{CaptureService, CaptureStatus};
pub use window::WindowInfo;
