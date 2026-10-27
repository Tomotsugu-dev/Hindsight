//! GNOME: asks gnome-shell and gnome-session, on Wayland and X11 alike. When a query fails, the
//! answer is "the user is present, the screen is visible, and nothing keeps the display awake", the
//! same as without an implementation, so no session is sealed by mistake.

use std::sync::Mutex;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use super::super::IdleSource;

/// Upper bound for one call: the tick calls it directly, so a stuck gnome-shell must not stall the
/// tick
const CALL_TIMEOUT: Duration = Duration::from_millis(500);
/// The "idle" flag of gnome-session's inhibitors: keeps the screen from dimming and locking. Video
/// players set it
const INHIBIT_IDLE: u32 = 8;

pub(super) struct GnomeIdle {
    /// Session bus connection, opened on first use; if opening fails, the next call tries again
    conn: Mutex<Option<Connection>>,
}

impl GnomeIdle {
    pub(super) fn new() -> Self {
        Self {
            conn: Mutex::new(None),
        }
    }

    fn connection(&self) -> zbus::Result<Connection> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Reuse the connection if it is already open
        if let Some(c) = conn.as_ref() {
            return Ok(c.clone());
        }
        // Otherwise open a new one
        let c = zbus::blocking::connection::Builder::session()?
            .method_timeout(CALL_TIMEOUT)
            .build()?;
        *conn = Some(c.clone());
        Ok(c)
    }

    /// Milliseconds since the last input, from Mutter
    fn idle_ms(&self) -> zbus::Result<u64> {
        let reply = self.connection()?.call_method(
            Some("org.gnome.Mutter.IdleMonitor"),
            "/org/gnome/Mutter/IdleMonitor/Core",
            Some("org.gnome.Mutter.IdleMonitor"),
            "GetIdletime",
            &(),
        )?;
        reply.body().deserialize()
    }

    /// Whether the screen is locked, from gnome-shell
    fn locked(&self) -> zbus::Result<bool> {
        let reply = self.connection()?.call_method(
            Some("org.gnome.ScreenSaver"),
            "/org/gnome/ScreenSaver",
            Some("org.gnome.ScreenSaver"),
            "GetActive",
            &(),
        )?;
        reply.body().deserialize()
    }

    /// The display's power state, from Mutter; see [`display_off`] for the values
    fn power_save_mode(&self) -> zbus::Result<i32> {
        let reply = self.connection()?.call_method(
            Some("org.gnome.Mutter.DisplayConfig"),
            "/org/gnome/Mutter/DisplayConfig",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.gnome.Mutter.DisplayConfig", "PowerSaveMode"),
        )?;
        let value: OwnedValue = reply.body().deserialize()?;
        Ok(i32::try_from(&value)?)
    }

    /// Whether an application asks the session not to go idle, from gnome-session
    fn idle_inhibited(&self) -> zbus::Result<bool> {
        let reply = self.connection()?.call_method(
            Some("org.gnome.SessionManager"),
            "/org/gnome/SessionManager",
            Some("org.gnome.SessionManager"),
            "IsInhibited",
            &(INHIBIT_IDLE,),
        )?;
        reply.body().deserialize()
    }
}

impl IdleSource for GnomeIdle {
    fn idle_secs(&self) -> u64 {
        match self.idle_ms() {
            Ok(ms) => ms / 1000,
            Err(e) => {
                log::debug!("Cannot read the idle time: {e}");
                0
            }
        }
    }

    fn screen_unavailable(&self) -> bool {
        match self.locked() {
            Ok(true) => return true,
            Ok(false) => {}
            Err(e) => log::debug!("Cannot read whether the screen is locked: {e}"),
        }
        match self.power_save_mode() {
            Ok(mode) => display_off(mode),
            Err(e) => {
                log::debug!("Cannot read the display power state: {e}");
                false
            }
        }
    }

    fn display_keepawake_active(&self) -> bool {
        match self.idle_inhibited() {
            Ok(inhibited) => inhibited,
            Err(e) => {
                log::debug!("Cannot read whether the display is kept awake: {e}");
                false
            }
        }
    }
}

/// Whether a `PowerSaveMode` value means the display is off: 0 = on; 1, 2, 3 = standby, suspend,
/// off; -1 = unknown
fn display_off(mode: i32) -> bool {
    mode > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_save_mode_off_means_display_off() {
        assert!(!display_off(0));
        assert!(display_off(1));
        assert!(display_off(2));
        assert!(display_off(3));
        // 不知道的时候当作开着，不能误封会话
        assert!(!display_off(-1));
    }

    /// 手动验证：在 GNOME 会话里跑，锁屏、放视频时看值的变化。
    ///
    /// 跑法：`cargo test --lib gnome_idle_manual -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn gnome_idle_manual() {
        let idle = GnomeIdle::new();
        println!("idle_ms = {:?}", idle.idle_ms());
        println!("locked = {:?}", idle.locked());
        println!("power_save_mode = {:?}", idle.power_save_mode());
        println!("idle_inhibited = {:?}", idle.idle_inhibited());
    }
}
