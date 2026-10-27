//! System signals used to detect inactivity: time since the last input, screen availability,
//! and whether an application is preventing the display from sleeping.
//! The capture service reads them only through [`IdleSource`], without knowing which system provides them.
//!
//! Each system has its own implementation, selected at compile time:
//! - Windows → [`windows`]
//! - macOS → [`macos`]
//! - Linux → [`linux`]

use std::sync::Arc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

/// System state used to determine whether the user is away from the computer.
/// Each system provides its own implementation.
///
/// These methods are called directly on every capture tick.
pub trait IdleSource: Send + Sync {
    /// Seconds elapsed since the last keyboard or mouse input.
    fn idle_secs(&self) -> u64;

    /// Returns `true` if the display is off, the screen is locked, or a screensaver is running.
    fn screen_unavailable(&self) -> bool;

    /// Returns `true` if an application is preventing the display from sleeping, such as a video player.
    fn display_keepawake_active(&self) -> bool;
}

/// Returns the source for this system. Called once when the capture service is created.
pub fn detect() -> Arc<dyn IdleSource> {
    imp::detect()
}

#[cfg(test)]
mod tests {
    /// 手动验证(macOS):对照系统断言表看返回值翻转。
    /// 1. `pmset -g assertions` 确认 PreventUserIdleDisplaySleep 当前为 0,跑测试应打印 false;
    /// 2. `caffeinate -d &` 挂一个显示防睡断言,再跑应打印 true;kill 后恢复 false。
    ///
    /// 跑法:`cargo test --lib keepawake_manual -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn keepawake_manual() {
        println!(
            "display_keepawake_active = {}",
            super::detect().display_keepawake_active()
        );
    }
}
