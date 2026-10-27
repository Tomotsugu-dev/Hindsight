//! X11 (desktops other than GNOME): queries the X11 server's screensaver and DPMS extensions.
//! Uses a pure Rust implementation of the X11 protocol, with no system libraries. When a query
//! fails, it reports "user present, screen visible", the same as having no implementation, so a
//! session is never sealed by mistake.

use x11rb::connection::Connection as _;
use x11rb::errors::{ConnectError, ReplyError};
use x11rb::protocol::dpms::{self, ConnectionExt as _};
use x11rb::protocol::screensaver::{self, ConnectionExt as _};
use x11rb::protocol::xproto::Window;
use x11rb::rust_connection::RustConnection;

use super::super::IdleSource;

pub(super) struct X11Idle {
    conn: RustConnection,
    /// Root window of the default screen, needed for screensaver extension queries
    root: Window,
}

impl X11Idle {
    /// Connects to the X11 server that `DISPLAY` points to.
    pub(super) fn connect() -> Result<Self, ConnectError> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        Ok(Self { conn, root })
    }

    /// Screensaver extension state: time since the last input, and whether the screensaver is running
    fn screensaver_info(&self) -> Result<screensaver::QueryInfoReply, ReplyError> {
        self.conn.screensaver_query_info(self.root)?.reply()
    }

    /// DPMS (display power management) state
    fn dpms_info(&self) -> Result<dpms::InfoReply, ReplyError> {
        self.conn.dpms_info()?.reply()
    }
}

impl IdleSource for X11Idle {
    fn idle_secs(&self) -> u64 {
        match self.screensaver_info() {
            Ok(info) => u64::from(info.ms_since_user_input / 1000),
            Err(e) => {
                log::debug!("Failed to read the idle time: {e}");
                0
            }
        }
    }

    fn screen_unavailable(&self) -> bool {
        match self.screensaver_info() {
            Ok(info) if screensaver_running(info.state) => return true,
            Ok(_) => {}
            Err(e) => log::debug!("Failed to read the screensaver state: {e}"),
        }
        match self.dpms_info() {
            Ok(info) => display_off(info.state, info.power_level),
            Err(e) => {
                log::debug!("Failed to read the display power state: {e}");
                false
            }
        }
    }

    /// X11 has no general way to check this, so it returns false and idle detection relies on
    /// keyboard and mouse input only.
    fn display_keepawake_active(&self) -> bool {
        false
    }
}

/// Whether the screensaver extension's `state` means the screensaver is running
fn screensaver_running(state: u8) -> bool {
    let state = screensaver::State::from(state);
    // ON and CYCLE mean the screensaver is running
    state == screensaver::State::ON || state == screensaver::State::CYCLE
}

/// The display counts as off when DPMS is enabled and the power level is not `ON`
/// (standby, suspend or off).
fn display_off(dpms_enabled: bool, level: dpms::DPMSMode) -> bool {
    dpms_enabled && level != dpms::DPMSMode::ON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_screensaver_states() {
        assert!(!screensaver_running(u8::from(screensaver::State::OFF)));
        assert!(screensaver_running(u8::from(screensaver::State::ON)));
        assert!(screensaver_running(u8::from(screensaver::State::CYCLE)));
        // 用户关掉了屏保，不是在运行
        assert!(!screensaver_running(u8::from(screensaver::State::DISABLED)));
    }

    #[test]
    fn dpms_display_off() {
        assert!(!display_off(true, dpms::DPMSMode::ON));
        assert!(display_off(true, dpms::DPMSMode::STANDBY));
        assert!(display_off(true, dpms::DPMSMode::SUSPEND));
        assert!(display_off(true, dpms::DPMSMode::OFF));
        // 没开 DPMS 时电源状态没有意义
        assert!(!display_off(false, dpms::DPMSMode::OFF));
    }

    /// 手动验证：在 X11 会话里跑，动鼠标、启动屏保、关显示器时看值的变化。
    ///
    /// 跑法：`cargo test --lib x11_idle_manual -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn x11_idle_manual() {
        let idle = X11Idle::connect().expect("连不上 X 服务器");
        match idle.screensaver_info() {
            Ok(i) => println!(
                "screensaver: state = {}, ms_since_user_input = {}, ms_until_server = {}",
                i.state, i.ms_since_user_input, i.ms_until_server
            ),
            Err(e) => println!("screensaver: {e:?}"),
        }
        match idle.dpms_info() {
            Ok(i) => println!(
                "dpms: enabled = {}, power_level = {:?}",
                i.state,
                u16::from(i.power_level)
            ),
            Err(e) => println!("dpms: {e:?}"),
        }
        println!(
            "idle_secs = {}, screen_unavailable = {}",
            idle.idle_secs(),
            idle.screen_unavailable()
        );
    }
}
