pub mod browser_url;
#[cfg(target_os = "macos")]
pub mod bundle;
pub mod focus;
pub mod idle;
pub mod ignore;
pub mod screenshot;
#[cfg(target_os = "macos")]
pub mod screenshot_macos;
pub mod screenshot_policy;
pub mod service;

pub use focus::WindowInfo;
pub use service::{CaptureService, CaptureStatus, FocusChange, FocusedWindow};
