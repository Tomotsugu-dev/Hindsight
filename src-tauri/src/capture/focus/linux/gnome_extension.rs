//! Whether the Hindsight GNOME extension is running, and turning it on for the user. Asks GNOME
//! Shell's `org.gnome.Shell.Extensions`, the same service the Extensions app uses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use super::super::GnomeExtensionStatus;
use crate::error::{Error, Result};

/// The extension's UUID, the same as in `metadata.json` and as its install folder's name
const UUID: &str = "hindsight-focus@tomotsugu-dev.github.com";
const BUS_NAME: &str = "org.gnome.Shell.Extensions";
const OBJECT_PATH: &str = "/org/gnome/Shell/Extensions";
const INTERFACE: &str = "org.gnome.Shell.Extensions";
/// The longest wait for one call, so a stuck gnome-shell does not hold up the prompt
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// GNOME's extension states, see `ExtensionState` in gnome-shell
mod state {
    /// The extension is running
    pub const ACTIVE: f64 = 1.0;
    /// The extension hit an error
    pub const ERROR: f64 = 3.0;
    /// The extension does not support this GNOME version and needs an update
    pub const OUT_OF_DATE: f64 = 4.0;
    /// The extension is being turned on
    pub const ACTIVATING: f64 = 8.0;
}

/// Returns the extension's state. Returns [`GnomeExtensionStatus::Ready`] when GNOME Shell cannot
/// be asked: no prompt when unsure.
pub(super) fn status() -> GnomeExtensionStatus {
    match query() {
        Ok((user_extensions_enabled, info)) => status_of(user_extensions_enabled, &info, installed),
        Err(e) => {
            log::warn!(
                "Could not ask GNOME Shell for the extension state, so no prompt is shown: {e}"
            );
            GnomeExtensionStatus::Ready
        }
    }
}

/// Turns the extension on, as the Extensions app does.
///
/// # Errors
///
/// GNOME Shell cannot be asked, or it did not turn the extension on: [`Error::Capture`]
pub(super) fn enable() -> Result<()> {
    let enabled = connection()
        .and_then(|conn| {
            conn.call_method(
                Some(BUS_NAME),
                OBJECT_PATH,
                Some(INTERFACE),
                "EnableExtension",
                &(UUID,),
            )
        })
        .and_then(|reply| reply.body().deserialize::<bool>())
        .map_err(|e| {
            Error::Capture(format!(
                "Could not turn on the Hindsight GNOME extension: {e}"
            ))
        })?;
    if enabled {
        Ok(())
    } else {
        Err(Error::Capture(
            "GNOME did not turn on the Hindsight GNOME extension".into(),
        ))
    }
}

fn connection() -> zbus::Result<Connection> {
    zbus::blocking::connection::Builder::session()?
        .method_timeout(CALL_TIMEOUT)
        .build()
}

/// Returns whether GNOME's switch for all extensions (`UserExtensionsEnabled`) is on, and what
/// GNOME knows about this extension; that is empty when GNOME has not found it.
fn query() -> zbus::Result<(bool, HashMap<String, OwnedValue>)> {
    let conn = connection()?;
    let user_extensions_enabled: OwnedValue = conn
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(INTERFACE, "UserExtensionsEnabled"),
        )?
        .body()
        .deserialize()?;
    let info = conn
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "GetExtensionInfo",
            &(UUID,),
        )?
        .body()
        .deserialize()?;
    Ok((bool::try_from(&user_extensions_enabled)?, info))
}

/// Returns the extension's state from what GNOME reports.
///
/// `user_extensions_enabled` is GNOME's switch for all extensions, `info` is what
/// `GetExtensionInfo` returns, and `installed` returns whether the extension's files are on disk.
fn status_of(
    user_extensions_enabled: bool,
    info: &HashMap<String, OwnedValue>,
    installed: impl Fn() -> bool,
) -> GnomeExtensionStatus {
    if info.is_empty() {
        return if installed() {
            GnomeExtensionStatus::NeedsRelogin
        } else {
            GnomeExtensionStatus::Missing
        };
    }
    let state = info.get("state").and_then(number);
    if state == Some(state::ACTIVE) || state == Some(state::ACTIVATING) {
        return GnomeExtensionStatus::Ready;
    }
    if !user_extensions_enabled {
        return GnomeExtensionStatus::AllOff;
    }
    if state == Some(state::OUT_OF_DATE) {
        return GnomeExtensionStatus::Unsupported;
    }
    if state == Some(state::ERROR) {
        return GnomeExtensionStatus::Failed;
    }
    GnomeExtensionStatus::Off
}

/// GNOME sends the state as a float; an integer is accepted too
fn number(v: &OwnedValue) -> Option<f64> {
    f64::try_from(v)
        .ok()
        .or_else(|| u32::try_from(v).ok().map(f64::from))
        .or_else(|| i32::try_from(v).ok().map(f64::from))
}

/// Whether the extension's files are in a folder GNOME looks in: the user's data folder, or a system
/// folder in `XDG_DATA_DIRS`
fn installed() -> bool {
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs::data_dir()
        .into_iter()
        .chain(
            system
                .split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
        )
        .any(|dir| {
            dir.join("gnome-shell/extensions")
                .join(UUID)
                .join("metadata.json")
                .is_file()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use zbus::zvariant::Value;

    fn info(state: Value<'_>) -> HashMap<String, OwnedValue> {
        HashMap::from([
            (
                "uuid".to_string(),
                Value::from(UUID).try_to_owned().unwrap(),
            ),
            ("state".to_string(), state.try_to_owned().unwrap()),
        ])
    }

    #[test]
    fn running_or_activating_is_ready() {
        assert_eq!(
            status_of(true, &info(Value::from(1.0)), || true),
            GnomeExtensionStatus::Ready
        );
        assert_eq!(
            status_of(true, &info(Value::from(8.0)), || true),
            GnomeExtensionStatus::Ready
        );
    }

    #[test]
    fn unknown_extension_needs_relogin_or_is_missing() {
        // GNOME 没认出扩展时 GetExtensionInfo 返回空字典
        let empty = HashMap::new();
        assert_eq!(
            status_of(true, &empty, || true),
            GnomeExtensionStatus::NeedsRelogin
        );
        assert_eq!(
            status_of(true, &empty, || false),
            GnomeExtensionStatus::Missing
        );
    }

    #[test]
    fn inactive_or_initialized_is_off() {
        assert_eq!(
            status_of(true, &info(Value::from(2.0)), || true),
            GnomeExtensionStatus::Off
        );
        assert_eq!(
            status_of(true, &info(Value::from(6.0)), || true),
            GnomeExtensionStatus::Off
        );
    }

    #[test]
    fn all_extensions_off_wins_over_off() {
        // 总开关关了，扩展的状态也是没打开；要提示的是总开关
        assert_eq!(
            status_of(false, &info(Value::from(2.0)), || true),
            GnomeExtensionStatus::AllOff
        );
        // 在运行就不用提示，总开关的值不用管
        assert_eq!(
            status_of(false, &info(Value::from(1.0)), || true),
            GnomeExtensionStatus::Ready
        );
    }

    #[test]
    fn out_of_date_or_error() {
        assert_eq!(
            status_of(true, &info(Value::from(4.0)), || true),
            GnomeExtensionStatus::Unsupported
        );
        assert_eq!(
            status_of(true, &info(Value::from(3.0)), || true),
            GnomeExtensionStatus::Failed
        );
    }

    #[test]
    fn integer_state_is_read_too() {
        assert_eq!(
            status_of(true, &info(Value::from(1u32)), || true),
            GnomeExtensionStatus::Ready
        );
        assert_eq!(
            status_of(true, &info(Value::from(2i32)), || true),
            GnomeExtensionStatus::Off
        );
    }

    /// 手动验证：在 GNOME 会话里跑，看 GNOME Shell 报的扩展状态；没打开的话替你打开，再看一次。
    /// 不管是不是 Wayland 都会问。
    ///
    /// 跑法：`cargo test --lib gnome_extension_manual -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn gnome_extension_manual() {
        let (user_extensions_enabled, info) =
            query().expect("问不到 GNOME Shell：要在 GNOME 会话里跑");
        println!("总开关: {user_extensions_enabled}，信息: {info:?}");
        let before = status_of(user_extensions_enabled, &info, installed);
        println!("状态: {before:?}");
        if before == GnomeExtensionStatus::Off {
            enable().expect("打开扩展失败");
            println!("打开后: {:?}", status());
        }
    }

    #[test]
    fn missing_state_is_off() {
        // 读不懂状态就当没打开：提示一下，用户点了打开也无害
        let mut i = info(Value::from(1.0));
        i.remove("state");
        assert_eq!(status_of(true, &i, || true), GnomeExtensionStatus::Off);
    }
}
